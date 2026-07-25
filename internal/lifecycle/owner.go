package lifecycle

import (
	"crypto/rand"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"reflect"
	"sort"
	"strings"
	"sync"
	"syscall"
	"time"
)

const (
	// OwnerLeaseEnv carries a supervisor reservation into only the harness it
	// supervises. The hotline run child verifies the reservation before using it.
	OwnerLeaseEnv = "HOTLINE_OWNER_LEASE"

	// OwnerConflictMarker is stable so harness adapters can classify an
	// ownership refusal as fatal rather than crash-looping.
	OwnerConflictMarker = "state ownership conflict"
)

const (
	ownerRoleSupervisor = "supervisor"
	ownerRoleRuntime    = "runtime"
)

// OwnerSpec describes every filesystem resource one box runtime owns. Paths
// are made absolute and clean, deduplicated, and sorted before locking or
// comparison. BoxRoot is always included in the resource set.
type OwnerSpec struct {
	BoxRoot      string
	BoxKey       string
	Harness      string
	WorkDir      string
	Providers    []string
	ResourceDirs []string
}

// OwnerLease is a lifetime flock lease. Release is idempotent and safe to call
// from both a normal defer and force-exit cleanup.
type OwnerLease struct {
	ID string

	once  sync.Once
	files []*os.File
}

// Release drops every kernel lock held by the lease. Advisory JSON is left in
// place deliberately: it is diagnostic only and never liveness truth.
func (l *OwnerLease) Release() {
	if l == nil {
		return
	}
	l.once.Do(func() { releaseOwnerFiles(l.files) })
}

// ownerAdvisory is intentionally secret-free. Resources is recorded as part of
// the resolved identity so a child refuses if Mission Control/provider config
// changed after its supervisor reserved the box.
type ownerAdvisory struct {
	LeaseID   string    `json:"lease_id"`
	PID       int       `json:"pid"`
	BoxKey    string    `json:"box_key"`
	BoxRoot   string    `json:"box_root"`
	Harness   string    `json:"harness"`
	Providers []string  `json:"providers"`
	WorkDir   string    `json:"workdir"`
	Resources []string  `json:"resources"`
	Role      string    `json:"role"`
	Timestamp time.Time `json:"timestamp"`
}

// ReserveBox reserves every box resource for a supervisor's lifetime. The
// returned random lease ID must be passed only to the supervised harness.
func ReserveBox(spec OwnerSpec) (*OwnerLease, error) {
	norm, resources, err := normalizeOwnerSpec(spec)
	if err != nil {
		return nil, err
	}
	leaseID, err := randomLeaseID()
	if err != nil {
		return nil, fmt.Errorf("generating owner lease: %w", err)
	}
	meta := advisoryFor(norm, resources, leaseID, ownerRoleSupervisor)

	files, err := acquireResourceLocks(resources, "owner.lock", "owner.json")
	if err != nil {
		return nil, err
	}
	if err := writeAdvisories(resources, "owner.json", meta); err != nil {
		releaseOwnerFiles(files)
		return nil, err
	}
	return &OwnerLease{ID: leaseID, files: files}, nil
}

// ClaimBox claims a box for one runtime. Without an inherited lease it owns
// both owner.lock and active.lock itself. With a lease, it verifies that the
// matching supervisor still holds every owner.lock, then always takes every
// active.lock so even duplicate children under one supervisor refuse.
func ClaimBox(spec OwnerSpec, inheritedLeaseID string) (*OwnerLease, error) {
	norm, resources, err := normalizeOwnerSpec(spec)
	if err != nil {
		return nil, err
	}
	inheritedLeaseID = strings.TrimSpace(inheritedLeaseID)
	leaseID := inheritedLeaseID
	if leaseID == "" {
		leaseID, err = randomLeaseID()
		if err != nil {
			return nil, fmt.Errorf("generating owner lease: %w", err)
		}
	}

	var held []*os.File
	if inheritedLeaseID == "" {
		held, err = acquireResourceLocks(resources, "owner.lock", "owner.json")
		if err != nil {
			return nil, err
		}
		if err := writeAdvisories(resources, "owner.json", advisoryFor(norm, resources, leaseID, ownerRoleRuntime)); err != nil {
			releaseOwnerFiles(held)
			return nil, err
		}
	} else if err := verifySupervisorReservation(norm, resources, leaseID); err != nil {
		return nil, err
	}

	active, err := acquireResourceLocks(resources, "active.lock", "active.json")
	if err != nil {
		releaseOwnerFiles(held)
		return nil, err
	}
	held = append(held, active...)
	if err := writeAdvisories(resources, "active.json", advisoryFor(norm, resources, leaseID, ownerRoleRuntime)); err != nil {
		releaseOwnerFiles(held)
		return nil, err
	}

	// Close the small verification race where a dying supervisor released its
	// reservation while the child was acquiring active locks.
	if inheritedLeaseID != "" {
		if err := verifySupervisorReservation(norm, resources, leaseID); err != nil {
			releaseOwnerFiles(held)
			return nil, err
		}
	}

	return &OwnerLease{ID: leaseID, files: held}, nil
}

func normalizeOwnerSpec(spec OwnerSpec) (OwnerSpec, []string, error) {
	if strings.TrimSpace(spec.BoxRoot) == "" {
		return OwnerSpec{}, nil, fmt.Errorf("box root is required for ownership")
	}
	if strings.TrimSpace(spec.BoxKey) == "" {
		return OwnerSpec{}, nil, fmt.Errorf("box key is required for ownership")
	}
	if strings.TrimSpace(spec.Harness) == "" {
		return OwnerSpec{}, nil, fmt.Errorf("harness is required for ownership")
	}

	boxRoot, err := absoluteClean(spec.BoxRoot)
	if err != nil {
		return OwnerSpec{}, nil, fmt.Errorf("resolving box root %q: %w", spec.BoxRoot, err)
	}
	workDir := spec.WorkDir
	if strings.TrimSpace(workDir) == "" {
		workDir, err = os.Getwd()
		if err != nil {
			return OwnerSpec{}, nil, fmt.Errorf("resolving ownership workdir: %w", err)
		}
	}
	workDir, err = absoluteClean(workDir)
	if err != nil {
		return OwnerSpec{}, nil, fmt.Errorf("resolving ownership workdir %q: %w", spec.WorkDir, err)
	}

	seen := make(map[string]struct{}, len(spec.ResourceDirs)+1)
	resources := make([]string, 0, len(spec.ResourceDirs)+1)
	for _, resource := range append([]string{boxRoot}, spec.ResourceDirs...) {
		if strings.TrimSpace(resource) == "" {
			continue
		}
		resource, err = absoluteClean(resource)
		if err != nil {
			return OwnerSpec{}, nil, fmt.Errorf("resolving ownership resource %q: %w", resource, err)
		}
		if _, ok := seen[resource]; ok {
			continue
		}
		seen[resource] = struct{}{}
		resources = append(resources, resource)
	}
	sort.Strings(resources)

	providers := make([]string, 0, len(spec.Providers))
	for _, provider := range spec.Providers {
		if provider = strings.TrimSpace(provider); provider != "" {
			providers = append(providers, provider)
		}
	}

	norm := OwnerSpec{
		BoxRoot:      boxRoot,
		BoxKey:       strings.TrimSpace(spec.BoxKey),
		Harness:      strings.TrimSpace(spec.Harness),
		WorkDir:      workDir,
		Providers:    providers,
		ResourceDirs: resources,
	}
	return norm, resources, nil
}

func absoluteClean(path string) (string, error) {
	path = filepath.Clean(path)
	return filepath.Abs(path)
}

func randomLeaseID() (string, error) {
	var raw [16]byte
	if _, err := rand.Read(raw[:]); err != nil {
		return "", err
	}
	return hex.EncodeToString(raw[:]), nil
}

func advisoryFor(spec OwnerSpec, resources []string, leaseID, role string) ownerAdvisory {
	return ownerAdvisory{
		LeaseID:   leaseID,
		PID:       os.Getpid(),
		BoxKey:    spec.BoxKey,
		BoxRoot:   spec.BoxRoot,
		Harness:   spec.Harness,
		Providers: append([]string(nil), spec.Providers...),
		WorkDir:   spec.WorkDir,
		Resources: append([]string(nil), resources...),
		Role:      role,
		Timestamp: time.Now().UTC(),
	}
}

func acquireResourceLocks(resources []string, lockName, advisoryName string) ([]*os.File, error) {
	files := make([]*os.File, 0, len(resources))
	for _, resource := range resources {
		hiddenDir := filepath.Join(resource, ".hotline")
		if err := os.MkdirAll(hiddenDir, 0o700); err != nil {
			releaseOwnerFiles(files)
			return nil, fmt.Errorf("creating ownership directory %s: %w", hiddenDir, err)
		}
		lockPath := filepath.Join(hiddenDir, lockName)
		f, err := os.OpenFile(lockPath, os.O_CREATE|os.O_RDWR, 0o600)
		if err != nil {
			releaseOwnerFiles(files)
			return nil, fmt.Errorf("opening ownership lock %s: %w", lockPath, err)
		}
		if err := syscall.Flock(int(f.Fd()), syscall.LOCK_EX|syscall.LOCK_NB); err != nil {
			_ = f.Close()
			releaseOwnerFiles(files)
			if isLockContended(err) {
				return nil, ownershipConflict(resource, advisoryName, "resource is already held")
			}
			return nil, fmt.Errorf("locking ownership resource %s: %w", lockPath, err)
		}
		files = append(files, f)
	}
	return files, nil
}

func releaseOwnerFiles(files []*os.File) {
	for i := len(files) - 1; i >= 0; i-- {
		_ = syscall.Flock(int(files[i].Fd()), syscall.LOCK_UN)
		_ = files[i].Close()
	}
}

func writeAdvisories(resources []string, name string, meta ownerAdvisory) error {
	body, err := json.MarshalIndent(meta, "", "  ")
	if err != nil {
		return err
	}
	body = append(body, '\n')
	for _, resource := range resources {
		path := filepath.Join(resource, ".hotline", name)
		f, err := os.OpenFile(path, os.O_CREATE|os.O_WRONLY|os.O_TRUNC, 0o600)
		if err != nil {
			return fmt.Errorf("opening ownership advisory %s: %w", path, err)
		}
		if err := f.Chmod(0o600); err == nil {
			_, err = f.Write(body)
		} else {
			_ = f.Close()
			return fmt.Errorf("securing ownership advisory %s: %w", path, err)
		}
		closeErr := f.Close()
		if err != nil {
			return fmt.Errorf("writing ownership advisory %s: %w", path, err)
		}
		if closeErr != nil {
			return fmt.Errorf("closing ownership advisory %s: %w", path, closeErr)
		}
	}
	return nil
}

func verifySupervisorReservation(spec OwnerSpec, resources []string, leaseID string) error {
	for _, resource := range resources {
		hiddenDir := filepath.Join(resource, ".hotline")
		if err := os.MkdirAll(hiddenDir, 0o700); err != nil {
			return fmt.Errorf("creating ownership directory %s: %w", hiddenDir, err)
		}
		lockPath := filepath.Join(hiddenDir, "owner.lock")
		f, err := os.OpenFile(lockPath, os.O_CREATE|os.O_RDWR, 0o600)
		if err != nil {
			return fmt.Errorf("opening supervisor ownership lock %s: %w", lockPath, err)
		}
		err = syscall.Flock(int(f.Fd()), syscall.LOCK_EX|syscall.LOCK_NB)
		if err == nil {
			_ = syscall.Flock(int(f.Fd()), syscall.LOCK_UN)
			_ = f.Close()
			return ownershipConflict(resource, "owner.json", "inherited lease has no live supervisor reservation")
		}
		_ = f.Close()
		if !isLockContended(err) {
			return fmt.Errorf("checking supervisor ownership resource %s: %w", lockPath, err)
		}

		meta, readErr := readAdvisory(filepath.Join(resource, ".hotline", "owner.json"))
		if readErr != nil {
			return ownershipConflict(resource, "owner.json", "live supervisor advisory is unreadable: "+readErr.Error())
		}
		if !matchesSupervisor(meta, spec, resources, leaseID) {
			return ownershipConflict(resource, "owner.json", "live supervisor reservation does not match this runtime")
		}
	}
	return nil
}

func matchesSupervisor(meta ownerAdvisory, spec OwnerSpec, resources []string, leaseID string) bool {
	return meta.LeaseID == leaseID &&
		meta.BoxRoot == spec.BoxRoot &&
		meta.BoxKey == spec.BoxKey &&
		meta.Harness == spec.Harness &&
		meta.WorkDir == spec.WorkDir &&
		meta.Role == ownerRoleSupervisor &&
		reflect.DeepEqual(meta.Providers, spec.Providers) &&
		reflect.DeepEqual(meta.Resources, resources)
}

func readAdvisory(path string) (ownerAdvisory, error) {
	var meta ownerAdvisory
	body, err := os.ReadFile(path)
	if err != nil {
		return meta, err
	}
	if err := json.Unmarshal(body, &meta); err != nil {
		return meta, err
	}
	return meta, nil
}

func isLockContended(err error) bool {
	return errors.Is(err, syscall.EWOULDBLOCK) || errors.Is(err, syscall.EAGAIN)
}

func ownershipConflict(resource, advisoryName, reason string) error {
	advisoryPath := filepath.Join(resource, ".hotline", advisoryName)
	detail := advisoryDetail(advisoryPath)
	return fmt.Errorf("%s: resource %s %s; %s. Stop the active box, select another named bot with --bot, or set HOTLINE_STATE_DIR to a separate state directory", OwnerConflictMarker, resource, reason, detail)
}

func advisoryDetail(path string) string {
	meta, err := readAdvisory(path)
	if err != nil {
		return fmt.Sprintf("advisory %s unavailable (%v)", path, err)
	}
	return fmt.Sprintf("advisory %s: role=%s pid=%d box=%s root=%s harness=%s providers=%v workdir=%s since=%s",
		path, meta.Role, meta.PID, meta.BoxKey, meta.BoxRoot, meta.Harness, meta.Providers, meta.WorkDir, meta.Timestamp.Format(time.RFC3339))
}
