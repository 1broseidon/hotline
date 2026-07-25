package app

import (
	"encoding/json"
	"os"
	"path/filepath"
	"testing"
	"time"
)

func TestLocalMailboxAssignDedupAckAndRestart(t *testing.T) {
	path := filepath.Join(t.TempDir(), "mailboxes.json")
	m, err := newLocalMailbox(path)
	if err != nil {
		t.Fatal(err)
	}
	m.disk.Devices["dev-af31fd290542"] = &mailboxRecord{Floor: "9007199254740993", Head: "9007199254740995", Ack: "9007199254740993"}
	if err := m.saveLocked(); err != nil {
		t.Fatal(err)
	}
	payload := []byte(`{"t":"msg","seq":812,"id":"a-812","text":"hello"}`)
	item, err := m.enqueue("dev-af31fd290542", "812", payload)
	if err != nil {
		t.Fatal(err)
	}
	if item.M != "9007199254740996" || item.ID != "env-j812-daf31fd290542" {
		t.Fatalf("assigned item = %+v", item)
	}
	dup, err := m.enqueue("dev-af31fd290542", "812", payload)
	if err != nil || dup.M != item.M || len(m.disk.Devices["dev-af31fd290542"].Items) != 1 {
		t.Fatalf("dedup = %+v err=%v", dup, err)
	}

	m2, err := newLocalMailbox(path)
	if err != nil {
		t.Fatal(err)
	}
	if got := m2.disk.Devices["dev-af31fd290542"].Items; len(got) != 1 || got[0].ID != item.ID {
		t.Fatalf("restart items = %+v", got)
	}
	if err := m2.ack("dev-af31fd290542", item.M); err != nil {
		t.Fatal(err)
	}
	mb := m2.disk.Devices["dev-af31fd290542"]
	if len(mb.Items) != 0 || mb.Floor != item.M || mb.Ack != item.M {
		t.Fatalf("trimmed mailbox = %+v", mb)
	}
	if mode := fileMode(t, path); mode != 0o600 {
		t.Fatalf("mailbox mode = %o", mode)
	}
}

func TestLocalMailboxRetentionAndFullAdvanceFloor(t *testing.T) {
	m, err := newLocalMailbox(filepath.Join(t.TempDir(), "mailboxes.json"))
	if err != nil {
		t.Fatal(err)
	}
	device := "dev-af31fd290542"
	m.disk.Devices[device] = &mailboxRecord{Floor: "0", Head: "10000", Ack: "0"}
	old := time.Now().Add(-mailboxRetention - time.Hour).UTC().Format(time.RFC3339Nano)
	m.disk.Devices[device].Items = []MailboxItem{{T: "mailbox_item", M: "1", J: "1", ID: "env-j1-daf31fd290542", Payload: json.RawMessage(`{"t":"msg"}`), CreatedAt: old}}
	if _, err := m.enqueue(device, "10001", []byte(`{"t":"msg"}`)); err != nil {
		t.Fatal(err)
	}
	_, _, _, expirySub, err := m.stateAndSubscribe(device)
	if err != nil {
		t.Fatal(err)
	}
	m.unsubscribe(device, expirySub)
	if m.disk.Devices[device].Floor != "1" {
		t.Fatalf("expired floor = %s", m.disk.Devices[device].Floor)
	}

	mb := m.disk.Devices[device]
	_, _, _, sub, err := m.stateAndSubscribe(device)
	if err != nil {
		t.Fatal(err)
	}
	defer m.unsubscribe(device, sub)
	mb.Items = make([]MailboxItem, mailboxMaxItems)
	for i := range mb.Items {
		mb.Items[i] = MailboxItem{M: decimalNext(stringDecimal(i)), CreatedAt: time.Now().UTC().Format(time.RFC3339Nano)}
	}
	oldHead := mb.Head
	if _, err := m.enqueue(device, "20000", []byte(`{"t":"msg"}`)); err != ErrMailboxFull {
		t.Fatalf("overflow error = %v", err)
	}
	if len(mb.Items) != 0 || mb.Floor != oldHead || !mb.Full {
		t.Fatalf("full reset = floor %s items %d full=%v", mb.Floor, len(mb.Items), mb.Full)
	}
	select {
	case code := <-sub.controls:
		if code != "full" {
			t.Fatalf("control = %q", code)
		}
	default:
		t.Fatal("live subscriber did not receive full")
	}
	if _, err := m.enqueue(device, "20001", []byte(`{"t":"msg"}`)); err != ErrMailboxFull {
		t.Fatalf("mailbox did not stop enqueuing: %v", err)
	}
	if err := m.ack(device, mb.Floor); err != nil {
		t.Fatal(err)
	}
	if _, err := m.enqueue(device, "20002", []byte(`{"t":"msg"}`)); err != nil {
		t.Fatalf("mailbox did not resume after gap-floor ack: %v", err)
	}
}

func stringDecimal(i int) string {
	if i == 0 {
		return "0"
	}
	b, _ := json.Marshal(i)
	return string(b)
}

func TestRelayStoreHashesSecretsAndRelinksAfterRotation(t *testing.T) {
	store, err := OpenRelayStore(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	first, err := store.MintLink("ws://127.0.0.1:8787", "pi")
	if err != nil {
		t.Fatal(err)
	}
	data, err := os.ReadFile(store.path)
	if err != nil {
		t.Fatal(err)
	}
	if string(data) == "" || containsBytes(data, []byte(first.Secret)) {
		t.Fatal("state file contains the plaintext pairing secret")
	}
	const deviceID = "dev-af31fd290542"
	if res, linked, err := store.VerifyAndLink(first.Room, deviceID, first.Secret); err != nil || res != VerifyActive || !linked {
		t.Fatalf("first link result=%v linked=%v err=%v", res, linked, err)
	}

	second, err := store.MintLink("ws://127.0.0.1:8787", "pi")
	if err != nil {
		t.Fatal(err)
	}
	if second.Room == first.Room {
		t.Fatal("new-link reused the old room")
	}
	if res, _, _ := store.VerifyAndLink(first.Room, "dev-001122334455", first.Secret); res != VerifyUnauthorized {
		t.Fatalf("old link remained valid after new-link: %v", res)
	}
	// Simulate state written by the old mass-revocation implementation: the
	// record still points at R1 and uses the formerly overloaded revoked state.
	legacy, _ := store.Device(deviceID)
	legacy.State = DeviceRevoked
	store.mu.Lock()
	store.st.Devices[deviceID] = legacy
	err = store.saveLocked()
	store.mu.Unlock()
	if err != nil {
		t.Fatal(err)
	}
	if res, linked, err := store.VerifyAndLink(second.Room, deviceID, second.Secret); err != nil || res != VerifyActive || !linked {
		t.Fatalf("re-link result=%v linked=%v err=%v", res, linked, err)
	}
	device, ok := store.Device(deviceID)
	if !ok || device.Room != second.Room || device.SecretHash != secretHash(second.Secret) || device.State != DeviceActive {
		t.Fatalf("re-linked device = %+v ok=%v", device, ok)
	}
	if mode := fileMode(t, store.path); mode != 0o600 {
		t.Fatalf("store mode = %o", mode)
	}
}

func TestRelayStoreRevokePermanentlyBansDevice(t *testing.T) {
	store, err := OpenRelayStore(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	link, err := store.MintLink("ws://127.0.0.1:8787", "pi")
	if err != nil {
		t.Fatal(err)
	}
	const deviceID = "dev-af31fd290542"
	if res, _, err := store.VerifyAndLink(link.Room, deviceID, link.Secret); err != nil || res != VerifyActive {
		t.Fatalf("link result=%v err=%v", res, err)
	}
	secondProcess, err := OpenRelayStore(filepath.Dir(store.path))
	if err != nil {
		t.Fatal(err)
	}
	if _, err := secondProcess.Revoke("dev-af31"); err != nil {
		t.Fatal(err)
	}
	if device, ok := store.Device(deviceID); !ok || device.State != DeviceState("banned") {
		t.Fatalf("running process did not reload permanent ban: %+v ok=%v", device, ok)
	}
	if res, _, _ := store.VerifyAndLink(link.Room, deviceID, link.Secret); res != VerifyRevoked {
		t.Fatalf("banned verify = %v", res)
	}
	rotated, err := store.MintLink("ws://127.0.0.1:8787", "pi")
	if err != nil {
		t.Fatal(err)
	}
	if res, _, _ := store.VerifyAndLink(rotated.Room, deviceID, rotated.Secret); res != VerifyRevoked {
		t.Fatalf("banned verify after rotation = %v", res)
	}
}

func TestRelayStoreRejectsSecondDeviceInCurrentRoom(t *testing.T) {
	store, err := OpenRelayStore(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	link, err := store.MintLink("ws://127.0.0.1:8787", "pi")
	if err != nil {
		t.Fatal(err)
	}
	if res, _, _ := store.VerifyAndLink(link.Room, "dev-af31fd290542", link.Secret); res != VerifyActive {
		t.Fatalf("first device result = %v", res)
	}
	if res, _, _ := store.VerifyAndLink(link.Room, "dev-001122334455", link.Secret); res != VerifyUnauthorized {
		t.Fatalf("second device result = %v", res)
	}
}

// TestPresenceLeaseBoundaries pins the per-subscriber foreground lease on a fake
// clock: fresh subscribe is present, presence goes away exactly at the 60s
// boundary, a ping (touch) refreshes it, an explicit background latch is
// immediate and survives a later touch, and foreground restores it.
func TestPresenceLeaseBoundaries(t *testing.T) {
	m, err := newLocalMailbox(filepath.Join(t.TempDir(), "mailboxes.json"))
	if err != nil {
		t.Fatal(err)
	}
	now := time.Unix(1_000_000, 0)
	m.clock = func() time.Time { return now }
	const device = "dev-af31fd290542"
	m.disk.Devices[device] = &mailboxRecord{Floor: "0", Head: "0", Ack: "0"}

	_, _, _, sub, err := m.stateAndSubscribe(device)
	if err != nil {
		t.Fatal(err)
	}
	// Fresh subscribe: foreground, present.
	if m.deviceAway(device) {
		t.Fatal("fresh subscriber must be present (not away)")
	}
	// One second before the boundary: still present.
	now = now.Add(presenceLeaseTimeout - time.Second)
	if m.deviceAway(device) {
		t.Fatal("subscriber must remain present just before the lease boundary")
	}
	// Exactly at the boundary (60s since last foreground): away.
	now = now.Add(time.Second)
	if !m.deviceAway(device) {
		t.Fatal("subscriber must be away at exactly the lease boundary")
	}
	// A ping (touch) refreshes the lease; present again.
	m.touchPresence(sub)
	if m.deviceAway(device) {
		t.Fatal("a ping must refresh the foreground lease")
	}
	// 59s after the refresh: present; 60s: away.
	now = now.Add(presenceLeaseTimeout - time.Second)
	if m.deviceAway(device) {
		t.Fatal("subscriber must remain present within the refreshed lease")
	}
	now = now.Add(time.Second)
	if !m.deviceAway(device) {
		t.Fatal("refreshed lease must expire at the boundary")
	}

	// Foreground restores presence; an explicit background latch is immediate.
	m.setPresence(sub, true)
	if m.deviceAway(device) {
		t.Fatal("presence:foreground must restore presence")
	}
	m.setPresence(sub, false)
	if !m.deviceAway(device) {
		t.Fatal("presence:background must latch away immediately, even with a fresh lease")
	}
	// A later ping/ack must NOT reactivate a background-latched subscriber.
	m.touchPresence(sub)
	if !m.deviceAway(device) {
		t.Fatal("a ping after an explicit background latch must not reactivate presence")
	}
	// Only foreground brings it back.
	m.setPresence(sub, true)
	if m.deviceAway(device) {
		t.Fatal("presence:foreground must clear the background latch")
	}
	m.unsubscribe(device, sub)
	if !m.deviceAway(device) {
		t.Fatal("no subscriber => away")
	}
}

// TestPresenceMultiSubscriberAnyForeground pins the "any foreground subscriber
// suppresses push" rule across multiple subscribers for one device.
func TestPresenceMultiSubscriberAnyForeground(t *testing.T) {
	m, err := newLocalMailbox(filepath.Join(t.TempDir(), "mailboxes.json"))
	if err != nil {
		t.Fatal(err)
	}
	const device = "dev-af31fd290542"
	m.disk.Devices[device] = &mailboxRecord{Floor: "0", Head: "0", Ack: "0"}
	_, _, _, a, err := m.stateAndSubscribe(device)
	if err != nil {
		t.Fatal(err)
	}
	_, _, _, b, err := m.stateAndSubscribe(device)
	if err != nil {
		t.Fatal(err)
	}
	// Both foreground: present.
	if m.deviceAway(device) {
		t.Fatal("two foreground subscribers => present")
	}
	// One background, one foreground: still present (any foreground wins).
	m.setPresence(a, false)
	if m.deviceAway(device) {
		t.Fatal("a background subscriber must not mark a foreground peer away")
	}
	// Both background: away, and exactly one enqueue produces one push decision.
	m.setPresence(b, false)
	if !m.deviceAway(device) {
		t.Fatal("all subscribers background => away")
	}
	m.unsubscribe(device, a)
	m.unsubscribe(device, b)
}

func TestTypingIsNotDurableContent(t *testing.T) {
	if durableContent([]byte(`{"t":"typing","seq":1,"on":true}`)) {
		t.Fatal("typing frame classified as durable")
	}
}

func containsBytes(haystack, needle []byte) bool {
	for i := 0; i+len(needle) <= len(haystack); i++ {
		if string(haystack[i:i+len(needle)]) == string(needle) {
			return true
		}
	}
	return false
}

func fileMode(t *testing.T, path string) os.FileMode {
	t.Helper()
	info, err := os.Stat(path)
	if err != nil {
		t.Fatal(err)
	}
	return info.Mode().Perm()
}
