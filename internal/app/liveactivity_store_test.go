package app

import (
	"fmt"
	"testing"
	"time"
)

func liveActivityStoreDevice(t *testing.T, dir, deviceID string) (*RelayStore, Link) {
	t.Helper()
	store, err := OpenRelayStore(dir)
	if err != nil {
		t.Fatal(err)
	}
	link, err := store.MintLink(mdPipe, "pi")
	if err != nil {
		t.Fatal(err)
	}
	if result, _, err := store.VerifyAndLink(link.Room, deviceID, link.Secret); err != nil || result != VerifyActive {
		t.Fatalf("link device: result=%v err=%v", result, err)
	}
	return store, link
}

func TestLiveActivityStoreCapEvictionAndReload(t *testing.T) {
	dir := t.TempDir()
	store, _ := liveActivityStoreDevice(t, dir, "dev-aaaaaa111111")
	base := time.Date(2026, 7, 18, 12, 0, 0, 0, time.UTC)
	for i := 1; i <= maxLiveActivitiesPerDevice+1; i++ {
		if err := store.setLiveActivityAt("dev-aaaaaa111111", fmt.Sprintf("job-%d", i), fmt.Sprintf("%02x", i), base.Add(time.Duration(i)*time.Second)); err != nil {
			t.Fatal(err)
		}
	}

	reloaded, err := OpenRelayStore(dir)
	if err != nil {
		t.Fatal(err)
	}
	device, ok := reloaded.Device("dev-aaaaaa111111")
	if !ok {
		t.Fatal("device missing after reload")
	}
	if len(device.LiveActivities) != maxLiveActivitiesPerDevice {
		t.Fatalf("registrations=%d, want %d", len(device.LiveActivities), maxLiveActivitiesPerDevice)
	}
	if _, exists := device.LiveActivities["job-1"]; exists {
		t.Fatal("oldest registration was not evicted")
	}
	if got := device.LiveActivities["job-33"].Token; got != "21" {
		t.Fatalf("newest token=%q, want 21", got)
	}
}

func TestLiveActivityStoreTakeRemoveAndConditionalDrop(t *testing.T) {
	dir := t.TempDir()
	store, _ := liveActivityStoreDevice(t, dir, "dev-aaaaaa111111")
	if err := store.SetLiveActivity("dev-aaaaaa111111", "job-1", "aabb"); err != nil {
		t.Fatal(err)
	}
	if targets := store.ActiveLiveActivityTargets("job-1"); len(targets) != 1 || targets[0].Token != "aabb" {
		t.Fatalf("active targets=%+v", targets)
	}

	// Replacing while an old APNs request is in flight makes its invalid-token
	// cleanup a no-op; only a matching rejection may remove the registration.
	if err := store.SetLiveActivity("dev-aaaaaa111111", "job-1", "ccdd"); err != nil {
		t.Fatal(err)
	}
	if dropped, err := store.DropLiveActivityIfToken("dev-aaaaaa111111", "job-1", "aabb"); err != nil || dropped {
		t.Fatalf("stale conditional drop: dropped=%v err=%v", dropped, err)
	}
	if dropped, err := store.DropLiveActivityIfToken("dev-aaaaaa111111", "job-1", "ccdd"); err != nil || !dropped {
		t.Fatalf("matching conditional drop: dropped=%v err=%v", dropped, err)
	}

	if err := store.SetLiveActivity("dev-aaaaaa111111", "job-2", "eeff"); err != nil {
		t.Fatal(err)
	}
	targets, err := store.TakeLiveActivityTargets("job-2")
	if err != nil || len(targets) != 1 || targets[0].Token != "eeff" {
		t.Fatalf("take targets=%+v err=%v", targets, err)
	}
	if targets := store.ActiveLiveActivityTargets("job-2"); len(targets) != 0 {
		t.Fatalf("taken registration remains active: %+v", targets)
	}
	// Empty/unset removal is idempotent and survives a reload as absent.
	if err := store.RemoveLiveActivity("dev-aaaaaa111111", "job-2"); err != nil {
		t.Fatal(err)
	}
	reloaded, err := OpenRelayStore(dir)
	if err != nil {
		t.Fatal(err)
	}
	device, _ := reloaded.Device("dev-aaaaaa111111")
	if len(device.LiveActivities) != 0 {
		t.Fatalf("registrations survived take/reload: %+v", device.LiveActivities)
	}
}

func TestLiveActivityStoreClearsOnBindingLifecycle(t *testing.T) {
	t.Run("room change", func(t *testing.T) {
		store, _ := liveActivityStoreDevice(t, t.TempDir(), "dev-aaaaaa111111")
		if err := store.SetLiveActivity("dev-aaaaaa111111", "job-1", "aabb"); err != nil {
			t.Fatal(err)
		}
		link, err := store.MintLink(mdPipe, "pi")
		if err != nil {
			t.Fatal(err)
		}
		if result, _, err := store.VerifyAndLink(link.Room, "dev-aaaaaa111111", link.Secret); err != nil || result != VerifyActive {
			t.Fatalf("rebind: result=%v err=%v", result, err)
		}
		device, _ := store.Device("dev-aaaaaa111111")
		if len(device.LiveActivities) != 0 {
			t.Fatalf("room change retained registrations: %+v", device.LiveActivities)
		}
	})

	t.Run("rotate all", func(t *testing.T) {
		store, _ := liveActivityStoreDevice(t, t.TempDir(), "dev-aaaaaa111111")
		if err := store.SetLiveActivity("dev-aaaaaa111111", "job-1", "aabb"); err != nil {
			t.Fatal(err)
		}
		if _, err := store.RotateAll(mdPipe, "pi", false); err != nil {
			t.Fatal(err)
		}
		device, _ := store.Device("dev-aaaaaa111111")
		if device.State != DeviceUnbound || len(device.LiveActivities) != 0 {
			t.Fatalf("rotate result=%+v", device)
		}
	})

	t.Run("revoke", func(t *testing.T) {
		store, _ := liveActivityStoreDevice(t, t.TempDir(), "dev-aaaaaa111111")
		if err := store.SetLiveActivity("dev-aaaaaa111111", "job-1", "aabb"); err != nil {
			t.Fatal(err)
		}
		if _, err := store.Revoke("dev-aaaaaa111111"); err != nil {
			t.Fatal(err)
		}
		device, _ := store.Device("dev-aaaaaa111111")
		if device.State != DeviceBanned || len(device.LiveActivities) != 0 {
			t.Fatalf("revoke result=%+v", device)
		}
	})
}
