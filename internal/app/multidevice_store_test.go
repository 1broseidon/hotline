package app

import (
	"encoding/json"
	"os"
	"path/filepath"
	"testing"
	"time"
)

const mdPipe = "wss://relay.hotline.dev"

// TestAdditiveMintPreservesExistingDeviceAndRoom is the MD1 headline: a second
// `relay new-link` mints a new room WITHOUT touching the existing room or its
// live device, and both rooms are independently bindable.
func TestAdditiveMintPreservesExistingDeviceAndRoom(t *testing.T) {
	s, err := OpenRelayStore(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	linkA, err := s.MintLink(mdPipe, "pi")
	if err != nil {
		t.Fatal(err)
	}
	if res, _, err := s.VerifyAndLink(linkA.Room, "dev-aaaaaa111111", linkA.Secret); err != nil || res != VerifyActive {
		t.Fatalf("bind A: res=%v err=%v", res, err)
	}
	// Additive mint of room B must not unbind the device on A or drop room A.
	linkB, err := s.MintLink(mdPipe, "pi")
	if err != nil {
		t.Fatal(err)
	}
	if linkB.Room == linkA.Room {
		t.Fatal("second mint reused room A")
	}
	da, ok := s.Device("dev-aaaaaa111111")
	if !ok || da.State != DeviceActive || da.Room != linkA.Room {
		t.Fatalf("device A disturbed by additive mint: %+v", da)
	}
	if served := s.ServedRooms(); len(served) != 2 {
		t.Fatalf("served rooms after additive mint = %d, want 2", len(served))
	}
	// Room B binds a second device while A stays live.
	if res, _, err := s.VerifyAndLink(linkB.Room, "dev-bbbbbb222222", linkB.Secret); err != nil || res != VerifyActive {
		t.Fatalf("bind B: res=%v err=%v", res, err)
	}
	if active := s.ActiveDevices(); len(active) != 2 {
		t.Fatalf("active devices = %d, want 2", len(active))
	}
}

// TestMintCapEnforced pins the served-room cap (HOTLINE_MAX_ROOMS) and that the
// error names `relay revoke`.
func TestMintCapEnforced(t *testing.T) {
	t.Setenv("HOTLINE_MAX_ROOMS", "2")
	s, err := OpenRelayStore(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	if _, err := s.MintLink(mdPipe, "pi"); err != nil {
		t.Fatal(err)
	}
	if _, err := s.MintLink(mdPipe, "pi"); err != nil {
		t.Fatal(err)
	}
	_, err = s.MintLink(mdPipe, "pi")
	if err == nil {
		t.Fatal("third mint at cap 2 should error")
	}
	if !containsAll(err.Error(), "cap", "revoke") {
		t.Fatalf("cap error should name the cap and `relay revoke`: %v", err)
	}
	// --rotate-all ignores the cap (it collapses to one room).
	if _, err := s.RotateAll(mdPipe, "pi", false); err != nil {
		t.Fatalf("rotate-all at cap should succeed: %v", err)
	}
	if served := s.ServedRooms(); len(served) != 1 {
		t.Fatalf("rotate-all left %d served rooms, want 1", len(served))
	}
}

// TestRotateAllReproducesOldSemantics proves --rotate-all reproduces the
// pre-multi-device behavior byte-for-byte: unbind every non-banned device,
// replace the whole rooms map with the single new room, and (for old-binary
// rollback) point current_room at it. The on-disk RoomRecord carries no state
// field (open/bound are computed), matching the legacy shape.
func TestRotateAllReproducesOldSemantics(t *testing.T) {
	dir := t.TempDir()
	s, err := OpenRelayStore(dir)
	if err != nil {
		t.Fatal(err)
	}
	linkA, _ := s.MintLink(mdPipe, "pi")
	s.MintLink(mdPipe, "pi") // a second additive room to be swept away
	if _, _, err := s.VerifyAndLink(linkA.Room, "dev-aaaaaa111111", linkA.Secret); err != nil {
		t.Fatal(err)
	}
	linkC, err := s.RotateAll(mdPipe, "pi", false)
	if err != nil {
		t.Fatal(err)
	}
	// Device unbound, single room, current_room == new room.
	d, _ := s.Device("dev-aaaaaa111111")
	if d.State != DeviceUnbound {
		t.Fatalf("rotate-all left device state %q, want unbound", d.State)
	}
	served := s.ServedRooms()
	if len(served) != 1 || served[0].ID != linkC.Room {
		t.Fatalf("rotate-all served set = %+v, want only %s", served, linkC.Room)
	}
	// The persisted room record must not carry a state field (legacy byte-shape).
	raw, _ := os.ReadFile(filepath.Join(dir, relayStateFile))
	var onDisk struct {
		CurrentRoom string                     `json:"current_room"`
		Rooms       map[string]json.RawMessage `json:"rooms"`
	}
	if err := json.Unmarshal(raw, &onDisk); err != nil {
		t.Fatal(err)
	}
	if onDisk.CurrentRoom != linkC.Room {
		t.Fatalf("current_room = %q, want %q (rollback pointer)", onDisk.CurrentRoom, linkC.Room)
	}
	if len(onDisk.Rooms) != 1 {
		t.Fatalf("rotate-all left %d rooms on disk, want 1", len(onDisk.Rooms))
	}
	for _, rr := range onDisk.Rooms {
		var m map[string]any
		json.Unmarshal(rr, &m)
		if _, hasState := m["state"]; hasState {
			t.Fatalf("rotate-all room carries a state field (breaks byte-for-byte rollback): %s", rr)
		}
	}
}

// TestRevokeFreesSlotAndTargetsDeviceRoom proves revoke bans the device, deads
// its OWN room (freeing the slot), and returns the device carrying its own room
// so the core DELETE targets the right room (fixing the CurrentRoom bug).
func TestRevokeFreesSlotAndTargetsDeviceRoom(t *testing.T) {
	s, err := OpenRelayStore(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	linkA, _ := s.MintLink(mdPipe, "pi")
	linkB, _ := s.MintLink(mdPipe, "pi")
	s.VerifyAndLink(linkA.Room, "dev-aaaaaa111111", linkA.Secret)
	s.VerifyAndLink(linkB.Room, "dev-bbbbbb222222", linkB.Secret)
	if n := len(s.ServedRooms()); n != 2 {
		t.Fatalf("served = %d, want 2", n)
	}
	// current_room is the newest mint (room B). Revoking the device on room A must
	// still target room A — not current_room.
	d, err := s.Revoke("dev-aaaaaa111111")
	if err != nil {
		t.Fatal(err)
	}
	if d.Room != linkA.Room {
		t.Fatalf("revoked device room = %q, want %q (core DELETE would hit wrong room)", d.Room, linkA.Room)
	}
	served := s.ServedRooms()
	if len(served) != 1 || served[0].ID != linkB.Room {
		t.Fatalf("revoke did not free the slot: served = %+v", served)
	}
	// Re-verifying the banned device is terminal (revoked), even on the dead room.
	if res, _, _ := s.VerifyAndLink(linkA.Room, "dev-aaaaaa111111", linkA.Secret); res != VerifyRevoked {
		t.Fatalf("banned device re-verify = %v, want VerifyRevoked", res)
	}
}

// TestPerDevicePushRoutesViaOwnRoom proves ActivePushTarget resolves each
// device's OWN room (MD3), not a global current_room.
func TestPerDevicePushRoutesViaOwnRoom(t *testing.T) {
	s, err := OpenRelayStore(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	linkA, _ := s.MintLink(mdPipe, "pi")
	linkB, _ := s.MintLink(mdPipe, "pi") // current_room is now B
	s.VerifyAndLink(linkA.Room, "dev-aaaaaa111111", linkA.Secret)
	s.VerifyAndLink(linkB.Room, "dev-bbbbbb222222", linkB.Secret)
	s.SetPush("dev-aaaaaa111111", "ExponentPushToken[a]", "ios")
	s.SetPush("dev-bbbbbb222222", "ExponentPushToken[b]", "ios")

	if _, _, room, ok := s.ActivePushTarget("dev-aaaaaa111111"); !ok || room != linkA.Room {
		t.Fatalf("device A push room = %q ok=%v, want %q (its own room, not current_room)", room, ok, linkA.Room)
	}
	if _, _, room, ok := s.ActivePushTarget("dev-bbbbbb222222"); !ok || room != linkB.Room {
		t.Fatalf("device B push room = %q ok=%v, want %q", room, ok, linkB.Room)
	}
}

// TestOldShapeRelayStateLoadsClean proves an old (single-room, current_room,
// no state field) relay-state.json loads and serves identically to N=1.
func TestOldShapeRelayStateLoadsClean(t *testing.T) {
	dir := t.TempDir()
	old := `{
  "current_room": "Ab3dEf6hIj8lMn0pQr2tUv",
  "rooms": {
    "Ab3dEf6hIj8lMn0pQr2tUv": {"id":"Ab3dEf6hIj8lMn0pQr2tUv","url":"wss://relay.hotline.dev","name":"pi","secret_hash":"deadbeef","created_at":"2026-07-10T00:00:00Z"}
  },
  "devices": {
    "dev-aaaaaa111111": {"id":"dev-aaaaaa111111","room":"Ab3dEf6hIj8lMn0pQr2tUv","secret_hash":"deadbeef","state":"active","linked_at":"2026-07-10T00:00:00Z"}
  }
}`
	if err := os.WriteFile(filepath.Join(dir, relayStateFile), []byte(old), 0o600); err != nil {
		t.Fatal(err)
	}
	s, err := OpenRelayStore(dir)
	if err != nil {
		t.Fatal(err)
	}
	served := s.ServedRooms()
	if len(served) != 1 || served[0].ID != "Ab3dEf6hIj8lMn0pQr2tUv" {
		t.Fatalf("old-shape file served = %+v, want the single legacy room", served)
	}
	if _, _, room, ok := s.ActivePushTarget("dev-aaaaaa111111"); !ok || room != "Ab3dEf6hIj8lMn0pQr2tUv" {
		t.Fatalf("old-shape device push target = %q ok=%v", room, ok)
	}
}

// TestRollbackShapePreservesRoomsUnderOldStruct proves a new-shape file (extra
// rooms + state field) round-trips through the OLD three-field struct without
// losing rooms, and that current_room is still written (the rollback pointer).
func TestRollbackShapePreservesRoomsUnderOldStruct(t *testing.T) {
	dir := t.TempDir()
	s, err := OpenRelayStore(dir)
	if err != nil {
		t.Fatal(err)
	}
	linkA, _ := s.MintLink(mdPipe, "pi")
	linkB, _ := s.MintLink(mdPipe, "pi")
	s.VerifyAndLink(linkA.Room, "dev-aaaaaa111111", linkA.Secret)
	// Dead a room so a state field is actually present on disk.
	s.Revoke("dev-aaaaaa111111")

	raw, _ := os.ReadFile(filepath.Join(dir, relayStateFile))
	// The exact pre-multi-device on-disk struct (no State field on RoomRecord).
	type oldRoom struct {
		ID  string `json:"id"`
		URL string `json:"url"`
	}
	var oldStruct struct {
		CurrentRoom string             `json:"current_room"`
		Rooms       map[string]oldRoom `json:"rooms"`
		Devices     map[string]any     `json:"devices"`
	}
	if err := json.Unmarshal(raw, &oldStruct); err != nil {
		t.Fatalf("new-shape file does not decode under the old struct: %v", err)
	}
	if oldStruct.CurrentRoom != linkB.Room {
		t.Fatalf("current_room = %q, want newest mint %q", oldStruct.CurrentRoom, linkB.Room)
	}
	// Both rooms survive the old-struct round-trip (extra rooms are not lost).
	if _, ok := oldStruct.Rooms[linkA.Room]; !ok {
		t.Fatal("room A lost under old struct")
	}
	if _, ok := oldStruct.Rooms[linkB.Room]; !ok {
		t.Fatal("room B lost under old struct")
	}
}

// TestOpenRoomExpires48h proves an open room that never linked a device is
// swept to dead at mint time after 48h, freeing its slot.
func TestOpenRoomExpires48h(t *testing.T) {
	dir := t.TempDir()
	s, err := OpenRelayStore(dir)
	if err != nil {
		t.Fatal(err)
	}
	linkOld, _ := s.MintLink(mdPipe, "pi")
	// Backdate the open room past the expiry window.
	s.mu.Lock()
	r := s.st.Rooms[linkOld.Room]
	r.CreatedAt = time.Now().Add(-49 * time.Hour).UTC().Format(time.RFC3339)
	s.st.Rooms[linkOld.Room] = r
	s.saveLocked()
	s.mu.Unlock()
	// The next mint runs expiry: the stale never-linked room goes dead.
	if _, err := s.MintLink(mdPipe, "pi"); err != nil {
		t.Fatal(err)
	}
	for _, room := range s.ServedRooms() {
		if room.ID == linkOld.Room {
			t.Fatal("stale never-linked open room was not expired")
		}
	}
}

func containsAll(s string, subs ...string) bool {
	for _, sub := range subs {
		found := false
		for i := 0; i+len(sub) <= len(s); i++ {
			if s[i:i+len(sub)] == sub {
				found = true
				break
			}
		}
		if !found {
			return false
		}
	}
	return true
}
