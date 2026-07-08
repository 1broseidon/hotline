package mcpchan

import (
	"encoding/json"
	"path/filepath"
	"strings"
	"testing"

	"github.com/1broseidon/hotline/internal/loop"
	"github.com/1broseidon/hotline/internal/notify"
)

func setupState(t *testing.T) string {
	t.Helper()
	dir := t.TempDir()
	t.Setenv("HOTLINE_STATE_DIR", dir)
	return dir
}

func TestSetupSchemasValidJSON(t *testing.T) {
	for name, schema := range map[string]string{
		"setup_loop":   setupLoopSchema,
		"setup_notify": setupNotifySchema,
	} {
		var v map[string]any
		if err := json.Unmarshal([]byte(schema), &v); err != nil {
			t.Fatalf("%s schema is not valid JSON: %v", name, err)
		}
		if v["type"] != "object" {
			t.Errorf("%s type = %v, want object", name, v["type"])
		}
	}
}

func TestSetupLoopCreatesPendingInNormalMode(t *testing.T) {
	stateRoot := setupState(t)
	msg, isErr := handleSetupLoop(SetupLoopInput{Label: "watch", Every: "1m", Cmd: "true"}, stateRoot)
	if isErr {
		t.Fatalf("setup_loop failed: %s", msg)
	}
	if !strings.Contains(msg, "pending") {
		t.Fatalf("message = %q, want pending", msg)
	}
	d, _ := loop.Load(loop.Path(stateRoot))
	if len(d.Loops) != 1 || d.Loops[0].Approved {
		t.Fatalf("loop should be pending: %+v", d.Loops)
	}
	sp, _ := notify.LoadSpool(notify.SpoolPath(stateRoot))
	if len(sp.Pending) != 1 || !strings.Contains(sp.Pending[0].Message, "pending approval") {
		t.Fatalf("operator notify not enqueued: %+v", sp.Pending)
	}
}

func TestSetupLoopCannotSelfApprove(t *testing.T) {
	stateRoot := setupState(t)
	var in SetupLoopInput
	raw := []byte(`{"label":"watch","every":"1m","cmd":"true","approved":true,"approve":true}`)
	if err := json.Unmarshal(raw, &in); err != nil {
		t.Fatal(err)
	}
	if _, isErr := handleSetupLoop(in, stateRoot); isErr {
		t.Fatal("setup_loop should ignore unknown approve fields")
	}
	d, _ := loop.Load(loop.Path(stateRoot))
	if d.Loops[0].Approved {
		t.Fatalf("agent-provided approved field must not self-approve: %+v", d.Loops[0])
	}
}

func TestSetupLoopYoloCreatesApprovedAndNotifies(t *testing.T) {
	stateRoot := setupState(t)
	t.Setenv("HOTLINE_YOLO", "1")
	msg, isErr := handleSetupLoop(SetupLoopInput{Label: "watch", Every: "1m", Cmd: "true"}, stateRoot)
	if isErr {
		t.Fatalf("setup_loop failed: %s", msg)
	}
	if !strings.Contains(msg, "live") {
		t.Fatalf("message = %q, want live", msg)
	}
	d, _ := loop.Load(loop.Path(stateRoot))
	if len(d.Loops) != 1 || !d.Loops[0].Approved {
		t.Fatalf("loop should be approved in yolo: %+v", d.Loops)
	}
	sp, _ := notify.LoadSpool(notify.SpoolPath(stateRoot))
	if len(sp.Pending) != 1 || !strings.Contains(sp.Pending[0].Message, "YOLO mode") {
		t.Fatalf("yolo notify not enqueued: %+v", sp.Pending)
	}
}

func TestSetupNotifyCreatesSourceWithoutReturningKey(t *testing.T) {
	stateRoot := setupState(t)
	msg, isErr := handleSetupNotify(SetupNotifyInput{Label: "ci", Cap: "low", Burst: 2, RefillMins: 10, ChatID: "123"}, notify.SourcesPath(stateRoot))
	if isErr {
		t.Fatalf("setup_notify failed: %s", msg)
	}
	reg, _ := notify.LoadRegistry(notify.SourcesPath(stateRoot))
	if len(reg.Sources) != 1 || reg.Sources[0].Label != "ci" || reg.Sources[0].Key == "" {
		t.Fatalf("source not created: %+v", reg.Sources)
	}
	if strings.Contains(msg, reg.Sources[0].Key) {
		t.Fatalf("setup_notify leaked key in result: %q", msg)
	}
}

func TestSetupToolsRegisteredWithScheduleSurface(t *testing.T) {
	names, _ := listToolNames(t, NewServer(&fakeToolSet{}, false, "/state/transcript.jsonl", []string{"telegram"}, "", "", filepath.Join(setupState(t), "schedules.json"), ""))
	for _, want := range []string{"schedule", "setup_loop", "setup_notify"} {
		if !names[want] {
			t.Fatalf("%s tool missing; tools=%v", want, names)
		}
	}
}
