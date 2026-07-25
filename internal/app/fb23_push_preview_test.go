package app

import (
	"context"
	"encoding/json"
	"testing"
)

// setPushPreviewRide builds the FB23 push-preview control frame the way the app
// sends it: a device_send whose "send" text payload is the serialized
// set_push_preview JSON line (same text-payload mechanism as set_name). clear is
// marshaled through `any` so a caller can pass a non-bool to forge a malformed
// frame.
func setPushPreviewRide(t *testing.T, cid string, clear any) []byte {
	t.Helper()
	m := map[string]any{"t": "set_push_preview"}
	if clear != nil {
		m["clear"] = clear
	}
	inner, err := json.Marshal(m)
	if err != nil {
		t.Fatalf("marshal set_push_preview: %v", err)
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

// TestSetPushPreviewParsePersistSilent drives the pinned set_push_preview control
// ride (FB23) through the real inbound dispatch: it arrives as a device_send text
// payload (like set_name), is consumed SILENTLY (no error frame, never reaches the
// harness), a valid bool is persisted to the device's own record, and a malformed
// clear is ignored while still being consumed (leaves the preference unset).
func TestSetPushPreviewParsePersistSilent(t *testing.T) {
	srv, _, dev, sub := activeHarness(t)
	ctx := context.Background()
	var writes [][]byte
	write := func(b []byte) error { writes = append(writes, b); return nil }

	ride := func(cid string, clear any) (bad, fatal bool) {
		return srv.handleSessionInput(ctx, dev, sub, setPushPreviewRide(t, cid, clear), write)
	}

	pref := func() *bool {
		d, ok := srv.store.Device(dev)
		if !ok {
			t.Fatalf("device %s vanished", dev)
		}
		return d.PushPreviewClear
	}

	// Unset by default.
	if p := pref(); p != nil {
		t.Fatalf("fresh device had a preference: %v", *p)
	}

	// Valid clear=true → consumed silently, persisted true.
	if bad, fatal := ride("cid-pp-true0000001", true); bad || fatal {
		t.Fatalf("clear=true: bad=%v fatal=%v, want silent consume", bad, fatal)
	}
	if p := pref(); p == nil || *p != true {
		t.Fatalf("clear=true not persisted: %v", p)
	}

	// Valid clear=false → persisted false (explicit, not unset).
	if bad, fatal := ride("cid-pp-false000001", false); bad || fatal {
		t.Fatalf("clear=false: bad=%v fatal=%v", bad, fatal)
	}
	if p := pref(); p == nil || *p != false {
		t.Fatalf("clear=false not persisted: %v", p)
	}

	// No writes at all: a push-preview ride never emits an error frame or echo.
	if len(writes) != 0 {
		t.Fatalf("push-preview ride wrote %d frames, want 0 (silent)", len(writes))
	}

	// Malformed clear (a string, not a bool) → consumed silently, preference left
	// at its prior explicit value (the frame is ignored, not applied).
	if bad, fatal := ride("cid-pp-bad00000001", "yes"); bad || fatal {
		t.Fatalf("malformed clear: bad=%v fatal=%v, want silent consume", bad, fatal)
	}
	if p := pref(); p == nil || *p != false {
		t.Fatalf("malformed clear must not change the preference, got %v", p)
	}
	if len(writes) != 0 {
		t.Fatalf("malformed ride emitted %d frames, want 0", len(writes))
	}

	// Missing clear key → same: consumed silently, no change.
	if bad, fatal := ride("cid-pp-missing0001", nil); bad || fatal {
		t.Fatalf("missing clear: bad=%v fatal=%v", bad, fatal)
	}
	if p := pref(); p == nil || *p != false {
		t.Fatalf("missing clear must not change the preference, got %v", p)
	}
}

// TestWakePushPreviewPrecedence proves the wake path (FB23) honors a device's own
// explicit push-preview preference over the box env default, and falls back to the
// env default when the device never set one — so today's HOTLINE_PUSH_PREVIEW
// behavior is unchanged for a preference-less device.
func TestWakePushPreviewPrecedence(t *testing.T) {
	set := func(b bool) *bool { return &b }
	cases := []struct {
		name        string
		envClear    bool  // HOTLINE_PUSH_PREVIEW=clear default
		pref        *bool // per-device preference (nil = never set)
		wantPreview bool
	}{
		{"pref-true-overrides-env-false", false, set(true), true},
		{"pref-false-overrides-env-true", true, set(false), false},
		{"unset-falls-back-to-env-true", true, nil, true},
		{"unset-falls-back-to-env-false", false, nil, false},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			cap := newWakeCapture()
			srv, deviceID, _ := previewServer(t, tc.envClear, cap)
			if tc.pref != nil {
				if err := srv.store.SetDevicePushPreview(deviceID, *tc.pref); err != nil {
					t.Fatalf("set device pref: %v", err)
				}
			}
			srv.maybePush(deviceID, textItem("yo from the phone"))
			cap.wait(t)
			m := cap.decode(t)
			_, gotPreview := m["preview"]
			if gotPreview != tc.wantPreview {
				t.Fatalf("preview present=%v, want %v (body=%s)", gotPreview, tc.wantPreview, cap.body)
			}
			if gotPreview && m["preview"] != "yo from the phone" {
				t.Fatalf("preview = %v, want the message text", m["preview"])
			}
		})
	}
}
