package app

import (
	"context"
	"encoding/json"
	"io"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"
)

// TestBuildAgentStateIncludesName proves the box-owned identity (FB21 §3) rides
// the agent_state snapshot so every device renders it live.
func TestBuildAgentStateIncludesName(t *testing.T) {
	srv, _, _, _ := activeHarness(t)
	if snap := srv.buildAgentState(); snap.Name != "" {
		t.Fatalf("unseeded box leaked a name: %q", snap.Name)
	}
	if err := srv.store.SetIdentityName("Wendigo"); err != nil {
		t.Fatalf("set identity: %v", err)
	}
	if snap := srv.buildAgentState(); snap.Name != "Wendigo" {
		t.Fatalf("snap.Name = %q, want Wendigo", snap.Name)
	}
}

// setNameRide builds the FB21 rename control frame the way the app sends it: a
// device_send whose "send" text payload is the serialized set_name JSON line.
func setNameRide(t *testing.T, cid, name string) []byte {
	t.Helper()
	inner, err := json.Marshal(map[string]any{"t": "set_name", "name": name})
	if err != nil {
		t.Fatalf("marshal set_name: %v", err)
	}
	payload, err := json.Marshal(map[string]any{"t": "send", "text": string(inner)})
	if err != nil {
		t.Fatalf("marshal payload: %v", err)
	}
	raw, err := json.Marshal(map[string]any{"t": "device_send", "cid": cid, "payload": json.RawMessage(payload)})
	if err != nil {
		t.Fatalf("marshal device_send: %v", err)
	}
	return raw
}

// TestSetNameValidatePersistBroadcast drives the pinned set_name control ride
// (FB21 §4) through the real inbound dispatch: it arrives as a device_send text
// payload (like an `/el` tap), is consumed silently, invalid names are dropped,
// a valid name is trimmed + persisted, open rooms are restamped (§5), and an
// agent_state snapshot carrying the new name is broadcast to every device.
func TestSetNameValidatePersistBroadcast(t *testing.T) {
	srv, _, dev, sub := activeHarness(t)
	ctx := context.Background()
	var writes [][]byte
	write := func(b []byte) error { writes = append(writes, b); return nil }

	ride := func(cid, name string) (bad, fatal bool) {
		return srv.handleSessionInput(ctx, dev, sub, setNameRide(t, cid, name), write)
	}

	// Invalid: empty after trim → consumed silently (no bad_frame), nothing persisted.
	if bad, fatal := ride("cid-set-name-empty01", "   "); bad || fatal {
		t.Fatalf("empty rename: bad=%v fatal=%v, want silent consume (false,false)", bad, fatal)
	}
	if name, ok := srv.store.IdentityName(); ok || name != "" {
		t.Fatalf("empty rename persisted an identity: %q", name)
	}

	// Invalid: >64 runes → consumed silently, nothing persisted.
	if bad, fatal := ride("cid-set-name-long01", strings.Repeat("x", 65)); bad || fatal {
		t.Fatalf("over-long rename: bad=%v fatal=%v", bad, fatal)
	}
	if name, _ := srv.store.IdentityName(); name != "" {
		t.Fatalf("over-long rename persisted: %q", name)
	}

	// No writes at all: a rename ride never emits an error frame.
	if len(writes) != 0 {
		t.Fatalf("rename ride wrote %d frames, want 0 (silent)", len(writes))
	}

	// Valid: trimmed + persisted.
	if bad, fatal := ride("cid-set-name-ok0001", "  Wendigo  "); bad || fatal {
		t.Fatalf("valid rename: bad=%v fatal=%v", bad, fatal)
	}
	if name, ok := srv.store.IdentityName(); !ok || name != "Wendigo" {
		t.Fatalf("identity = %q (ok=%v), want Wendigo", name, ok)
	}

	// §5: the open room record was restamped to the live name.
	for _, r := range srv.store.ServedRooms() {
		if r.Name != "Wendigo" {
			t.Fatalf("open room %s name = %q, want Wendigo", r.ID, r.Name)
		}
	}

	// Broadcast: an agent_state transient carrying the new name reached the device.
	found := false
	for _, m := range drainTransients(t, sub, 300*time.Millisecond) {
		if m["t"] != "agent_state" {
			continue
		}
		if st, ok := m["state"].(map[string]any); ok && st["name"] == "Wendigo" {
			found = true
		}
	}
	if !found {
		t.Fatal("no agent_state broadcast carrying the renamed identity")
	}
}

// TestSetNameReregistersRoomWithFreshName proves the rename control ride
// re-registers every live room with the core carrying the NEW name, so the
// relay's per-room reg.name (the push-notification title on the core path) is no
// longer frozen at the box's first-register value. This is the fix for stale
// push titles ("Scylla" after renaming to "Kraken"): SetIdentityName restamps the
// local room records, and reregisterRoomsForRename re-sends the (idempotent)
// register with the restamped Name so the relay overwrites its stored title.
func TestSetNameReregistersRoomWithFreshName(t *testing.T) {
	names := make(chan string, 4)
	core := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if strings.HasSuffix(r.URL.Path, "/register") {
			body, _ := io.ReadAll(r.Body)
			var reg struct {
				Name string `json:"name"`
			}
			_ = json.Unmarshal(body, &reg)
			select {
			case names <- reg.Name:
			default:
			}
		}
		w.WriteHeader(http.StatusOK)
		_, _ = w.Write([]byte(`{"ok":true}`))
	}))
	defer core.Close()

	srv, dev, roomID := coreModeServer(t, core.URL, "")
	// The room was first registered under its original name; ensureRegistered's
	// one-shot gate is set, mirroring a box that already registered as "Scylla".
	srv.regMu.Lock()
	srv.registered[roomID] = true
	srv.regMu.Unlock()

	srv.mailbox.disk.Devices[dev] = &mailboxRecord{Floor: "0", Head: "0", Ack: "0"}
	_, _, _, sub, err := srv.mailbox.stateAndSubscribe(dev)
	if err != nil {
		t.Fatalf("subscribe: %v", err)
	}
	defer srv.mailbox.unsubscribe(dev, sub)

	write := func(b []byte) error { return nil }
	if bad, fatal := srv.handleSessionInput(context.Background(), dev, sub, setNameRide(t, "cid-rename-kraken1", "Kraken"), write); bad || fatal {
		t.Fatalf("rename ride: bad=%v fatal=%v", bad, fatal)
	}

	select {
	case got := <-names:
		if got != "Kraken" {
			t.Fatalf("re-register name = %q, want Kraken", got)
		}
	case <-time.After(2 * time.Second):
		t.Fatal("timed out waiting for a re-register carrying the fresh name")
	}
}
