package app

import (
	"strings"
	"testing"
)

// TestResolveRevokeDeviceVsRoom covers the FB27 resolution logic: a `relay
// revoke <arg>` argument routes to a device or an open room, by unique prefix,
// with cross-kind collisions and bound rooms refused.
func TestResolveRevokeDeviceVsRoom(t *testing.T) {
	s, err := OpenRelayStore(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	// One bound room (device rides it) and one open (unredeemed) room.
	bound, err := s.MintLink(mdPipe, "pi")
	if err != nil {
		t.Fatal(err)
	}
	if res, _, err := s.VerifyAndLink(bound.Room, "dev-aaaaaa111111", bound.Secret); err != nil || res != VerifyActive {
		t.Fatalf("bind: res=%v err=%v", res, err)
	}
	open, err := s.MintLink(mdPipe, "pi")
	if err != nil {
		t.Fatal(err)
	}

	t.Run("full device id resolves to device", func(t *testing.T) {
		res, err := s.ResolveRevoke("dev-aaaaaa111111")
		if err != nil || res.Kind != "device" || res.ID != "dev-aaaaaa111111" {
			t.Fatalf("res=%+v err=%v", res, err)
		}
	})

	t.Run("device prefix resolves to device", func(t *testing.T) {
		res, err := s.ResolveRevoke("dev-aaaaaa")
		if err != nil || res.Kind != "device" {
			t.Fatalf("res=%+v err=%v", res, err)
		}
	})

	t.Run("open room id resolves to room", func(t *testing.T) {
		res, err := s.ResolveRevoke(open.Room)
		if err != nil || res.Kind != "room" || res.ID != open.Room {
			t.Fatalf("res=%+v err=%v", res, err)
		}
	})

	t.Run("open room prefix resolves to room", func(t *testing.T) {
		res, err := s.ResolveRevoke(open.Room[:10])
		if err != nil || res.Kind != "room" || res.ID != open.Room {
			t.Fatalf("res=%+v err=%v", res, err)
		}
	})

	t.Run("bound room id is refused with device guidance", func(t *testing.T) {
		_, err := s.ResolveRevoke(bound.Room)
		if err == nil || !strings.Contains(err.Error(), "dev-aaaaaa111111") {
			t.Fatalf("bound room should point at its device, err=%v", err)
		}
	})

	t.Run("unknown arg errors", func(t *testing.T) {
		if _, err := s.ResolveRevoke("nope-zzzz"); err == nil {
			t.Fatal("want no-match error")
		}
	})
}

// TestResolveRevokePrefixCollision pins the ambiguity guard: a prefix that
// matches more than one target (two devices here) is refused, never a silent
// pick. Records are created through the real bind path so they persist across
// ResolveRevoke's reload.
func TestResolveRevokePrefixCollision(t *testing.T) {
	s, err := OpenRelayStore(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	link, err := s.MintLink(mdPipe, "pi")
	if err != nil {
		t.Fatal(err)
	}
	// A room-only prefix (no device shares it — device ids are dev-*) resolves.
	if res, err := s.ResolveRevoke(link.Room[:6]); err != nil || res.Kind != "room" {
		t.Fatalf("room-only prefix should resolve to the room: res=%+v err=%v", res, err)
	}
	// Bind that room, then mint+bind a second, so two "dev-" devices exist.
	if res, _, err := s.VerifyAndLink(link.Room, "dev-111111aaaaaa", link.Secret); err != nil || res != VerifyActive {
		t.Fatalf("bind 1: res=%v err=%v", res, err)
	}
	link2, err := s.MintLink(mdPipe, "pi")
	if err != nil {
		t.Fatal(err)
	}
	if res, _, err := s.VerifyAndLink(link2.Room, "dev-222222bbbbbb", link2.Secret); err != nil || res != VerifyActive {
		t.Fatalf("bind 2: res=%v err=%v", res, err)
	}
	if _, err := s.ResolveRevoke("dev-"); err == nil || !strings.Contains(err.Error(), "ambiguous") {
		t.Fatalf("two devices under a prefix should be ambiguous, err=%v", err)
	}
}

// TestRevokeRoomDeletesOpenRoom pins the open-room kill: the record is deleted,
// the slot freed, and a bound room is refused.
func TestRevokeRoomDeletesOpenRoom(t *testing.T) {
	s, err := OpenRelayStore(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	open, err := s.MintLink(mdPipe, "pi")
	if err != nil {
		t.Fatal(err)
	}
	bound, err := s.MintLink(mdPipe, "pi")
	if err != nil {
		t.Fatal(err)
	}
	if res, _, err := s.VerifyAndLink(bound.Room, "dev-aaaaaa111111", bound.Secret); err != nil || res != VerifyActive {
		t.Fatalf("bind: res=%v err=%v", res, err)
	}
	if served := s.ServedRooms(); len(served) != 2 {
		t.Fatalf("served before revoke = %d, want 2", len(served))
	}

	r, err := s.RevokeRoom(open.Room)
	if err != nil || r.ID != open.Room {
		t.Fatalf("revoke open room: r=%+v err=%v", r, err)
	}
	if _, ok := s.st.Rooms[open.Room]; ok {
		// reload to be sure it hit disk
		s.reloadLocked()
		if _, ok := s.st.Rooms[open.Room]; ok {
			t.Fatal("open room record should be deleted")
		}
	}
	if served := s.ServedRooms(); len(served) != 1 {
		t.Fatalf("served after revoke = %d, want 1 (slot freed)", len(served))
	}
	// A bound room cannot be nuked by room-id.
	if _, err := s.RevokeRoom(bound.Room); err == nil || !strings.Contains(err.Error(), "dev-aaaaaa111111") {
		t.Fatalf("bound room revoke-by-room-id should be refused, err=%v", err)
	}
}
