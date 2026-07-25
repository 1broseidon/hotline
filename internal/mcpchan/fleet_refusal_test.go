package mcpchan

import (
	"context"
	"path/filepath"
	"testing"

	"github.com/1broseidon/hotline/internal/notify"
)

const fleetRefusal = "use fleet_send for fleet peers"

// TestScheduleRefusesFleetChat proves F11 on the schedule tool: a create that
// targets a fleet chat (by chat_id or by source) is refused with the exact
// message, before anything is written.
func TestScheduleRefusesFleetChat(t *testing.T) {
	path := filepath.Join(t.TempDir(), "schedules.json")
	byChat := ScheduleInput{Action: "create", Prompt: "p", ChatID: "fleet:abcd1234", Repeat: "once", At: "+1h"}
	if msg, isErr := handleSchedule(byChat, path, []string{"app"}); !isErr || msg != fleetRefusal {
		t.Fatalf("schedule by fleet chat_id: got (%q,%t)", msg, isErr)
	}
	bySource := ScheduleInput{Action: "create", Prompt: "p", ChatID: "app", Source: "fleet", Repeat: "once", At: "+1h"}
	if msg, isErr := handleSchedule(bySource, path, []string{"app", "fleet"}); !isErr || msg != fleetRefusal {
		t.Fatalf("schedule by fleet source: got (%q,%t)", msg, isErr)
	}
}

// TestSetupNotifyRefusesFleetChat proves F11 on setup_notify: a fleet default
// chat_id is refused.
func TestSetupNotifyRefusesFleetChat(t *testing.T) {
	sp := notify.SourcesPath(t.TempDir())
	if msg, isErr := handleSetupNotify(SetupNotifyInput{Label: "x", ChatID: "fleet:abcd1234"}, sp); !isErr || msg != fleetRefusal {
		t.Fatalf("setup_notify by fleet chat_id: got (%q,%t)", msg, isErr)
	}
}

// TestSetupLoopRefusesFleetSource proves M6: setup_loop rejects the fleet channel
// as a loop's notify source with the EXACT refusal, before any side effect (an
// unwritable state root would still error if it got past the precheck).
func TestSetupLoopRefusesFleetSource(t *testing.T) {
	if msg, isErr := handleSetupLoop(SetupLoopInput{Label: "x", Every: "10m", Cmd: "echo", Source: "fleet"}, "/proc/nonexistent-does-not-matter"); !isErr || msg != fleetRefusal {
		t.Fatalf("setup_loop by fleet source: got (%q,%t)", msg, isErr)
	}
}

// TestPublishRefusesFleet proves M6: publish emits the EXACT refusal for a fleet
// source or a fleet chat_id, not the generic "unknown source", before side effects.
func TestPublishRefusesFleet(t *testing.T) {
	if msg, isErr := publishForSource(context.Background(), PublishInput{Path: "/x", Source: "fleet"}, localExposure{}, nil, []string{"app", "fleet"}); !isErr || msg != fleetRefusal {
		t.Fatalf("publish by fleet source: got (%q,%t)", msg, isErr)
	}
	if msg, isErr := publishForSource(context.Background(), PublishInput{Path: "/x", ChatID: "fleet:abcd1234"}, localExposure{}, nil, []string{"app"}); !isErr || msg != fleetRefusal {
		t.Fatalf("publish by fleet chat_id: got (%q,%t)", msg, isErr)
	}
}

// TestJobRefusesFleet proves M6: job emits the EXACT refusal for a fleet source or
// chat_id, before resolvePublishSource turns it into a generic "unknown source".
func TestJobRefusesFleet(t *testing.T) {
	if msg, isErr := jobForSource(context.Background(), JobInput{Action: "start", Title: "t", Source: "fleet"}, nil, []string{"app", "fleet"}); !isErr || msg != fleetRefusal {
		t.Fatalf("job by fleet source: got (%q,%t)", msg, isErr)
	}
	if msg, isErr := jobForSource(context.Background(), JobInput{Action: "start", Title: "t", ChatID: "fleet:abcd1234"}, nil, []string{"app"}); !isErr || msg != fleetRefusal {
		t.Fatalf("job by fleet chat_id: got (%q,%t)", msg, isErr)
	}
}
