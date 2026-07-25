package lifecycle

import (
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"syscall"
	"testing"
)

func TestClaimBoxDirectConflict(t *testing.T) {
	spec := testOwnerSpec(t, "default")
	first, err := ClaimBox(spec, "")
	if err != nil {
		t.Fatal(err)
	}
	defer first.Release()

	second, err := ClaimBox(spec, "")
	if second != nil {
		second.Release()
	}
	assertOwnerConflict(t, err)
}

func TestSupervisorReservationAdmitsOneMatchingChild(t *testing.T) {
	spec := testOwnerSpec(t, "supervised")
	reservation, err := ReserveBox(spec)
	if err != nil {
		t.Fatal(err)
	}
	defer reservation.Release()
	if reservation.ID == "" {
		t.Fatal("reservation lease id is empty")
	}

	child, err := ClaimBox(spec, reservation.ID)
	if err != nil {
		t.Fatalf("matching child claim: %v", err)
	}
	defer child.Release()

	duplicate, err := ClaimBox(spec, reservation.ID)
	if duplicate != nil {
		duplicate.Release()
	}
	assertOwnerConflict(t, err)
}

func TestSupervisorReservationRejectsWrongLeaseOrSpec(t *testing.T) {
	spec := testOwnerSpec(t, "supervised")
	reservation, err := ReserveBox(spec)
	if err != nil {
		t.Fatal(err)
	}
	defer reservation.Release()

	cases := []struct {
		name  string
		lease string
		edit  func(OwnerSpec) OwnerSpec
	}{
		{name: "lease", lease: "wrong-lease", edit: func(s OwnerSpec) OwnerSpec { return s }},
		{name: "box key", lease: reservation.ID, edit: func(s OwnerSpec) OwnerSpec { s.BoxKey = "other"; return s }},
		{name: "harness", lease: reservation.ID, edit: func(s OwnerSpec) OwnerSpec { s.Harness = "opencode"; return s }},
		{name: "providers", lease: reservation.ID, edit: func(s OwnerSpec) OwnerSpec { s.Providers = []string{"discord"}; return s }},
		{name: "resources", lease: reservation.ID, edit: func(s OwnerSpec) OwnerSpec { s.ResourceDirs = nil; return s }},
		{name: "workdir", lease: reservation.ID, edit: func(s OwnerSpec) OwnerSpec { s.WorkDir = t.TempDir(); return s }},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			claim, err := ClaimBox(tc.edit(spec), tc.lease)
			if claim != nil {
				claim.Release()
			}
			assertOwnerConflict(t, err)
		})
	}
}

func TestClaimBoxStaleJSONWithoutLockSucceeds(t *testing.T) {
	spec := testOwnerSpec(t, "stale")
	hidden := filepath.Join(spec.BoxRoot, ".hotline")
	if err := os.MkdirAll(hidden, 0o700); err != nil {
		t.Fatal(err)
	}
	stale := ownerAdvisory{
		LeaseID: "old", BoxKey: "wrong", BoxRoot: "/old", Harness: "pi",
		Role: ownerRoleSupervisor, PID: os.Getpid(),
	}
	body, _ := json.Marshal(stale)
	for _, name := range []string{"owner.json", "active.json"} {
		if err := os.WriteFile(filepath.Join(hidden, name), body, 0o600); err != nil {
			t.Fatal(err)
		}
	}

	claim, err := ClaimBox(spec, "")
	if err != nil {
		t.Fatalf("stale advisory blocked free locks: %v", err)
	}
	defer claim.Release()

	meta, err := readAdvisory(filepath.Join(hidden, "owner.json"))
	if err != nil {
		t.Fatal(err)
	}
	if meta.LeaseID != claim.ID || meta.BoxKey != spec.BoxKey || meta.Role != ownerRoleRuntime {
		t.Fatalf("owner advisory was not replaced: %+v", meta)
	}
}

func TestOverlappingProviderResourceAcrossBoxesRefuses(t *testing.T) {
	root := t.TempDir()
	shared := filepath.Join(root, "providers", "telegram-work")
	specA := OwnerSpec{
		BoxRoot: filepath.Join(root, "boxes", "a"), BoxKey: "a", Harness: "pi",
		WorkDir: t.TempDir(), Providers: []string{"telegram:work", "discord:a"}, ResourceDirs: []string{shared},
	}
	specB := OwnerSpec{
		BoxRoot: filepath.Join(root, "boxes", "b"), BoxKey: "b", Harness: "pi",
		WorkDir: specA.WorkDir, Providers: []string{"telegram:work", "signal:b"}, ResourceDirs: []string{shared},
	}
	first, err := ClaimBox(specA, "")
	if err != nil {
		t.Fatal(err)
	}
	defer first.Release()

	second, err := ClaimBox(specB, "")
	if second != nil {
		second.Release()
	}
	assertOwnerConflict(t, err)
	if err != nil && !strings.Contains(err.Error(), shared) {
		t.Fatalf("conflict does not name shared resource %s: %v", shared, err)
	}
}

func TestDisjointNamedBoxesCanCoexist(t *testing.T) {
	root := t.TempDir()
	workdir := t.TempDir()
	specA := OwnerSpec{
		BoxRoot: filepath.Join(root, "bots", "a"), BoxKey: "a", Harness: "claude",
		WorkDir: workdir, Providers: []string{"telegram:a"}, ResourceDirs: []string{filepath.Join(root, "providers", "a")},
	}
	specB := OwnerSpec{
		BoxRoot: filepath.Join(root, "bots", "b"), BoxKey: "b", Harness: "claude",
		WorkDir: workdir, Providers: []string{"telegram:b"}, ResourceDirs: []string{filepath.Join(root, "providers", "b")},
	}
	first, err := ClaimBox(specA, "")
	if err != nil {
		t.Fatal(err)
	}
	defer first.Release()
	second, err := ClaimBox(specB, "")
	if err != nil {
		t.Fatalf("disjoint box claim: %v", err)
	}
	defer second.Release()
}

func TestClaimBoxPartialAcquisitionUnwinds(t *testing.T) {
	root := t.TempDir()
	boxRoot := filepath.Join(root, "a-box")
	blocked := filepath.Join(root, "z-blocked")
	hidden := filepath.Join(blocked, ".hotline")
	if err := os.MkdirAll(hidden, 0o700); err != nil {
		t.Fatal(err)
	}
	blockFile, err := os.OpenFile(filepath.Join(hidden, "owner.lock"), os.O_CREATE|os.O_RDWR, 0o600)
	if err != nil {
		t.Fatal(err)
	}
	if err := syscall.Flock(int(blockFile.Fd()), syscall.LOCK_EX|syscall.LOCK_NB); err != nil {
		t.Fatal(err)
	}
	defer func() {
		_ = syscall.Flock(int(blockFile.Fd()), syscall.LOCK_UN)
		_ = blockFile.Close()
	}()

	spec := OwnerSpec{
		BoxRoot: boxRoot, BoxKey: "partial", Harness: "pi", WorkDir: t.TempDir(),
		Providers: []string{"telegram"}, ResourceDirs: []string{blocked},
	}
	claim, err := ClaimBox(spec, "")
	if claim != nil {
		claim.Release()
	}
	assertOwnerConflict(t, err)

	// boxRoot sorts before blocked, so it was acquired first. A fresh flock must
	// succeed now, proving the failed multi-resource claim unwound it.
	firstLock := filepath.Join(boxRoot, ".hotline", "owner.lock")
	f, err := os.OpenFile(firstLock, os.O_CREATE|os.O_RDWR, 0o600)
	if err != nil {
		t.Fatal(err)
	}
	defer f.Close()
	if err := syscall.Flock(int(f.Fd()), syscall.LOCK_EX|syscall.LOCK_NB); err != nil {
		t.Fatalf("partial claim left %s locked: %v", firstLock, err)
	}
	_ = syscall.Flock(int(f.Fd()), syscall.LOCK_UN)
}

func testOwnerSpec(t *testing.T, key string) OwnerSpec {
	t.Helper()
	root := t.TempDir()
	return OwnerSpec{
		BoxRoot: root, BoxKey: key, Harness: "pi", WorkDir: t.TempDir(),
		Providers: []string{"telegram"}, ResourceDirs: []string{root, filepath.Join(root, "provider")},
	}
}

func assertOwnerConflict(t *testing.T, err error) {
	t.Helper()
	if err == nil || !strings.Contains(err.Error(), OwnerConflictMarker) {
		t.Fatalf("err = %v, want %q marker", err, OwnerConflictMarker)
	}
	if !strings.Contains(err.Error(), "--bot") || !strings.Contains(err.Error(), "HOTLINE_STATE_DIR") {
		t.Fatalf("conflict lacks actionable box guidance: %v", err)
	}
}
