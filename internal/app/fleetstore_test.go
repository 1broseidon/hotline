package app

import (
	"bytes"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"sync"
	"testing"
)

const fleetTestRelay = "wss://relay.test"

// fleetJoinOpts trusts the test relay origin (the B7 allowlist).
func fleetJoinOpts() JoinOptions { return JoinOptions{AllowedOrigins: []string{"wss://relay.test"}} }

func TestFleetLinkDoesNotTouchRelayState(t *testing.T) {
	dir := t.TempDir()
	rs, err := OpenRelayStore(dir)
	if err != nil {
		t.Fatalf("open relay store: %v", err)
	}
	if _, err := rs.MintLinkMode(fleetTestRelay, "Operator", false); err != nil {
		t.Fatalf("mint operator: %v", err)
	}
	relayPath := filepath.Join(dir, relayStateFile)
	before, err := os.ReadFile(relayPath)
	if err != nil {
		t.Fatalf("read relay-state before: %v", err)
	}

	fs, err := OpenFleetStore(dir)
	if err != nil {
		t.Fatalf("open fleet store: %v", err)
	}
	if _, _, err := fs.Link(fleetTestRelay, "peer"); err != nil {
		t.Fatalf("fleet link: %v", err)
	}

	after, err := os.ReadFile(relayPath)
	if err != nil {
		t.Fatalf("read relay-state after: %v", err)
	}
	if !bytes.Equal(before, after) {
		t.Fatalf("fleet link mutated relay-state.json")
	}
	if _, err := os.Stat(filepath.Join(dir, fleetDirName, fleetStateFile)); err != nil {
		t.Fatalf("fleet.json not written: %v", err)
	}
}

func TestFleetRegistryCRUDAtomicReloadAndTombstone(t *testing.T) {
	dir := t.TempDir()
	fs, err := OpenFleetStore(dir)
	if err != nil {
		t.Fatalf("open: %v", err)
	}
	edge, secret, err := fs.Link(fleetTestRelay, "alpha")
	if err != nil {
		t.Fatalf("link: %v", err)
	}
	if len(edge.EdgeID) != 8 || !fleetEdgeIDRE.MatchString(edge.EdgeID) {
		t.Fatalf("edge id not 8-char: %q", edge.EdgeID)
	}
	if edge.Direction != FleetServe || edge.Secret != secret || !edge.Envelope {
		t.Fatalf("serve edge shape wrong: %+v", edge)
	}
	edgeDir := filepath.Join(dir, fleetDirName, edge.EdgeID)
	if _, err := os.Stat(filepath.Join(edgeDir, fleetJournalFile)); err != nil {
		t.Fatalf("journal.jsonl missing: %v", err)
	}
	if _, err := os.Stat(filepath.Join(edgeDir, fleetEdgeStateFile)); err != nil {
		t.Fatalf("state.json missing: %v", err)
	}

	// A fresh store instance sees the edge from disk.
	fs2, err := OpenFleetStore(dir)
	if err != nil {
		t.Fatalf("reopen: %v", err)
	}
	if got, ok := fs2.LiveEdge(edge.EdgeID); !ok || got.Alias != "alpha" {
		t.Fatalf("reload did not see edge: ok=%v edge=%+v", ok, got)
	}

	renamed, err := fs2.Rename("alpha", "beta")
	if err != nil {
		t.Fatalf("rename: %v", err)
	}
	if renamed.EdgeID != edge.EdgeID || renamed.Alias != "beta" {
		t.Fatalf("rename wrong: %+v", renamed)
	}

	rm, err := fs2.Remove(edge.EdgeID)
	if err != nil {
		t.Fatalf("remove: %v", err)
	}
	if !rm.Removed() || rm.Secret != "" {
		t.Fatalf("tombstone/creds-zero failed: %+v", rm)
	}
	if _, ok := fs2.LiveEdge(edge.EdgeID); ok {
		t.Fatalf("tombstoned edge still live")
	}
	served, err := fs2.ServedFleetRooms()
	if err != nil || len(served) != 0 {
		t.Fatalf("tombstoned edge still served: %d err=%v", len(served), err)
	}
	if _, err := os.Stat(edgeDir); err != nil {
		t.Fatalf("edge dir not retained after rm: %v", err)
	}
	if _, err := fs2.Remove(edge.EdgeID); err != nil {
		t.Fatalf("second remove errored: %v", err)
	}
}

func TestFleetJoinStoresCredsNoDial(t *testing.T) {
	dir := t.TempDir()
	r, secret, err := mintRoom(fleetTestRelay, "", true)
	if err != nil {
		t.Fatalf("mint: %v", err)
	}
	uri := FleetPairingURI(r.URL, r.ID, secret, "peerbox")

	fs, err := OpenFleetStore(dir)
	if err != nil {
		t.Fatalf("open: %v", err)
	}
	edge, err := fs.Join(uri, "peer", fleetJoinOpts())
	if err != nil {
		t.Fatalf("join: %v", err)
	}
	if edge.Direction != FleetDial {
		t.Fatalf("join direction not dial: %+v", edge)
	}
	if edge.DeviceCreds == nil || edge.DeviceCreds.DeviceID != "flt-"+edge.EdgeID {
		t.Fatalf("device id wrong: %+v", edge.DeviceCreds)
	}
	if edge.DeviceCreds.Room != r.ID || edge.DeviceCreds.Secret != secret {
		t.Fatalf("creds not persisted: %+v", edge.DeviceCreds)
	}
	stateData, err := os.ReadFile(filepath.Join(dir, fleetDirName, edge.EdgeID, fleetEdgeStateFile))
	if err != nil {
		t.Fatalf("state.json: %v", err)
	}
	var st fleetEdgeState
	if err := json.Unmarshal(stateData, &st); err != nil {
		t.Fatalf("decode state.json: %v", err)
	}
	if st.DeviceCreds == nil || st.DeviceCreds.DeviceID != "flt-"+edge.EdgeID {
		t.Fatalf("state.json creds wrong: %+v", st)
	}
	entries, err := fs.JournalEntries(edge.EdgeID)
	if err != nil || len(entries) != 0 {
		t.Fatalf("join must not journal (no dial): entries=%d err=%v", len(entries), err)
	}
}

func TestFleetJoinRefusesNonFleetURI(t *testing.T) {
	dir := t.TempDir()
	fs, _ := OpenFleetStore(dir)
	r, secret, _ := mintRoom(fleetTestRelay, "", true)
	operatorURI := PairingURIMode(r.URL, r.ID, secret, "op", true)
	if _, err := fs.Join(operatorURI, "peer", fleetJoinOpts()); err == nil {
		t.Fatalf("join accepted a non-fleet URI")
	}
}

func TestFleetJoinRefusesUntrustedRelayOrigin(t *testing.T) {
	dir := t.TempDir()
	fs, _ := OpenFleetStore(dir)
	r, secret, _ := mintRoom("wss://evil.example", "", true)
	uri := FleetPairingURI(r.URL, r.ID, secret, "peer")
	if _, err := fs.Join(uri, "peer", fleetJoinOpts()); err == nil {
		t.Fatalf("join stored an untrusted relay origin (SSRF-by-storage)")
	}
	if _, err := fs.Join(uri, "peer", JoinOptions{AllowRelay: "wss://evil.example"}); err != nil {
		t.Fatalf("join with explicit allow-relay failed: %v", err)
	}
}

func TestFleetRemoveZeroesCredsPreservesCursor(t *testing.T) {
	dir := t.TempDir()
	r, secret, _ := mintRoom(fleetTestRelay, "", true)
	uri := FleetPairingURI(r.URL, r.ID, secret, "peerbox")
	fs, _ := OpenFleetStore(dir)
	edge, err := fs.Join(uri, "peer", fleetJoinOpts())
	if err != nil {
		t.Fatalf("join: %v", err)
	}
	// Advance the cursor (a stand-in for L3 progress); rm must NOT reset it (B6).
	statePath := filepath.Join(dir, fleetDirName, edge.EdgeID, fleetEdgeStateFile)
	st := fleetEdgeState{DeviceCreds: edge.DeviceCreds, Cursor: "5"}
	data, _ := json.MarshalIndent(st, "", "  ")
	if err := os.WriteFile(statePath, append(data, '\n'), 0o600); err != nil {
		t.Fatalf("seed cursor: %v", err)
	}

	rm, err := fs.Remove("peer")
	if err != nil {
		t.Fatalf("remove: %v", err)
	}
	if rm.Direction != FleetDial || !rm.Removed() {
		t.Fatalf("dial remove wrong: %+v", rm)
	}
	if rm.DeviceCreds == nil || rm.DeviceCreds.Secret != "" {
		t.Fatalf("dial creds secret not zeroed: %+v", rm.DeviceCreds)
	}
	out, _ := os.ReadFile(statePath)
	var after fleetEdgeState
	if err := json.Unmarshal(out, &after); err != nil {
		t.Fatalf("decode state after rm: %v", err)
	}
	if after.Cursor != "5" {
		t.Fatalf("rm reset the cursor to %q (want preserved 5)", after.Cursor)
	}
	if after.DeviceCreds == nil || after.DeviceCreds.Secret != "" {
		t.Fatalf("rm did not zero the state.json secret: %+v", after.DeviceCreds)
	}
}

func TestFleetEdgeCap(t *testing.T) {
	dir := t.TempDir()
	fs, _ := OpenFleetStore(dir)
	for i := 0; i < fleetMaxEdges; i++ {
		if _, _, err := fs.Link(fleetTestRelay, fmt.Sprintf("peer%d", i)); err != nil {
			t.Fatalf("link %d: %v", i, err)
		}
	}
	if _, _, err := fs.Link(fleetTestRelay, "overflow"); err == nil {
		t.Fatalf("link past the %d-edge cap succeeded", fleetMaxEdges)
	}
	edges, _ := fs.Edges()
	if _, err := fs.Remove(edges[0].EdgeID); err != nil {
		t.Fatalf("remove: %v", err)
	}
	if _, _, err := fs.Link(fleetTestRelay, "reclaimed"); err != nil {
		t.Fatalf("link after freeing a slot failed: %v", err)
	}
}

func TestFleetAliasRules(t *testing.T) {
	dir := t.TempDir()
	fs, _ := OpenFleetStore(dir)
	a, _, err := fs.Link(fleetTestRelay, "alpha")
	if err != nil {
		t.Fatalf("link alpha: %v", err)
	}
	if _, _, err := fs.Link(fleetTestRelay, "alpha"); err == nil {
		t.Fatalf("duplicate active alias accepted")
	}
	if _, _, err := fs.Link(fleetTestRelay, a.EdgeID); err == nil {
		t.Fatalf("alias==edge-id accepted")
	}
	if _, _, err := fs.Link(fleetTestRelay, "bad\nline"); err == nil {
		t.Fatalf("control-char alias accepted")
	}
	long := make([]byte, fleetMaxAliasRunes+1)
	for i := range long {
		long[i] = 'a'
	}
	if _, _, err := fs.Link(fleetTestRelay, string(long)); err == nil {
		t.Fatalf("overlong alias accepted")
	}
}

// TestFleetConcurrentLinksFlock proves B1: N separate FleetStore instances (each
// its own flock fd, like N processes) linking concurrently never lose a write.
func TestFleetConcurrentLinksFlock(t *testing.T) {
	dir := t.TempDir()
	const n = 12
	var wg sync.WaitGroup
	errs := make(chan error, n)
	for i := 0; i < n; i++ {
		wg.Add(1)
		go func(i int) {
			defer wg.Done()
			fs, err := OpenFleetStore(dir)
			if err != nil {
				errs <- err
				return
			}
			if _, _, err := fs.Link(fleetTestRelay, fmt.Sprintf("peer%d", i)); err != nil {
				errs <- err
			}
		}(i)
	}
	wg.Wait()
	close(errs)
	for err := range errs {
		t.Fatalf("concurrent link: %v", err)
	}
	fs, _ := OpenFleetStore(dir)
	edges, err := fs.Edges()
	if err != nil {
		t.Fatalf("edges: %v", err)
	}
	if len(edges) != n {
		t.Fatalf("flock lost writes: %d edges, want %d", len(edges), n)
	}
}

// TestFleetConcurrentCapRace proves the cap holds under concurrency: 24 racing
// links against a 16-cap admit exactly 16.
func TestFleetConcurrentCapRace(t *testing.T) {
	dir := t.TempDir()
	const n = 24
	var wg sync.WaitGroup
	var mu sync.Mutex
	ok := 0
	for i := 0; i < n; i++ {
		wg.Add(1)
		go func(i int) {
			defer wg.Done()
			fs, _ := OpenFleetStore(dir)
			if _, _, err := fs.Link(fleetTestRelay, fmt.Sprintf("peer%d", i)); err == nil {
				mu.Lock()
				ok++
				mu.Unlock()
			}
		}(i)
	}
	wg.Wait()
	if ok != fleetMaxEdges {
		t.Fatalf("cap race admitted %d edges, want %d", ok, fleetMaxEdges)
	}
	fs, _ := OpenFleetStore(dir)
	edges, _ := fs.Edges()
	if len(edges) != fleetMaxEdges {
		t.Fatalf("registry has %d edges, want %d", len(edges), fleetMaxEdges)
	}
}

// TestFleetStagedCommitFailureNoGhost proves B6: a stage failure (edge dir
// uncreatable) aborts before the registry is published — no ghost edge.
func TestFleetStagedCommitFailureNoGhost(t *testing.T) {
	if os.Geteuid() == 0 {
		t.Skip("root bypasses directory permissions")
	}
	dir := t.TempDir()
	fs, _ := OpenFleetStore(dir)
	if _, _, err := fs.Link(fleetTestRelay, "alpha"); err != nil {
		t.Fatalf("seed link: %v", err)
	}
	fleetDir := filepath.Join(dir, fleetDirName)
	if err := os.Chmod(fleetDir, 0o500); err != nil {
		t.Fatalf("chmod: %v", err)
	}
	defer os.Chmod(fleetDir, 0o700)

	if _, _, err := fs.Link(fleetTestRelay, "beta"); err == nil {
		t.Fatalf("link succeeded despite an unwritable edge dir")
	}
	os.Chmod(fleetDir, 0o700)
	edges, err := fs.Edges()
	if err != nil {
		t.Fatalf("edges: %v", err)
	}
	if len(edges) != 1 {
		t.Fatalf("ghost edge published: %d edges, want 1", len(edges))
	}
}

// TestFleetAppendJournalFailureSurfaced proves B4: a journal write failure is
// returned (so the session rejects the frame), never shrugged.
func TestFleetAppendJournalFailureSurfaced(t *testing.T) {
	dir := t.TempDir()
	fs, _ := OpenFleetStore(dir)
	edgeID := "AAAAAAAA"
	if err := os.MkdirAll(filepath.Join(dir, fleetDirName), 0o700); err != nil {
		t.Fatalf("mkdir: %v", err)
	}
	// A regular FILE where the edge dir must be → MkdirAll fails inside append.
	if err := os.WriteFile(filepath.Join(dir, fleetDirName, edgeID), []byte("x"), 0o600); err != nil {
		t.Fatalf("seed file: %v", err)
	}
	if _, err := fs.AppendJournalFrame(edgeID, "in", json.RawMessage(`{"t":"fleet_msg"}`)); err == nil {
		t.Fatalf("AppendJournalFrame did not surface a write failure")
	}
}
