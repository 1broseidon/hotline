package loop

import (
	"fmt"
	"strings"
	"time"

	"github.com/1broseidon/hotline/internal/notify"
)

const approvalNotifySource = "hotline-loop-approval"

// SetupResult is the shared creation result used by the CLI and MCP tool.
type SetupResult struct {
	Loop             Loop
	AutoSource       string
	NotifyEnqueued   bool
	NotifySuppressed bool
	Yolo             bool
}

// Setup creates a loop using the same registry surfaces as the operator CLI:
// optional notify source validation/auto-minting, then Add with the approval
// gate enabled. sourceSet distinguishes omitted --source from --source "".
func Setup(stateRoot string, l Loop, sourceSet, preApprove bool, now time.Time) (SetupResult, error) {
	if _, err := notify.ParseLevel(l.Level); err != nil {
		return SetupResult{}, fmt.Errorf("level: %w", err)
	}
	sourcesPath := notify.SourcesPath(stateRoot)
	autoSource := ""
	if sourceSet {
		if err := RequireSourceLabel(sourcesPath, l.Source); err != nil {
			return SetupResult{}, err
		}
	} else if l.NotifyLLM {
		src, err := notify.AddSource(sourcesPath, l.Label, notify.LevelNormal, notify.Rate{}, "", now)
		if err != nil {
			return SetupResult{}, fmt.Errorf("auto-adding notify source %q: %w", l.Label, err)
		}
		l.Source = src.Label
		autoSource = src.Label
	}

	yolo, err := YoloEnabled(stateRoot)
	if err != nil {
		if autoSource != "" {
			_, _ = notify.RevokeSource(sourcesPath, autoSource)
		}
		return SetupResult{}, err
	}
	stored, err := Add(Path(stateRoot), l, now, WithApprovalGate(stateRoot, preApprove))
	if err != nil {
		if autoSource != "" {
			_, _ = notify.RevokeSource(sourcesPath, autoSource)
		}
		return SetupResult{}, err
	}
	return SetupResult{
		Loop:           stored,
		AutoSource:     autoSource,
		NotifyEnqueued: !preApprove,
		Yolo:           yolo,
	}, nil
}

// RequireSourceLabel verifies a loop references an existing notify source by
// label, never by bearer key.
func RequireSourceLabel(path, label string) error {
	label = strings.TrimSpace(label)
	if label == "" {
		return fmt.Errorf("source needs a notify source label")
	}
	reg, err := notify.LoadRegistry(path)
	if err != nil {
		return err
	}
	if _, ok := reg.FindByLabel(label); !ok {
		return fmt.Errorf("notify source %q not found (create it with `hotline source add %s`)", label, label)
	}
	return nil
}

func notifyOperator(stateRoot string, l Loop, yolo bool, now time.Time) error {
	if stateRoot == "" {
		return nil
	}
	sourcesPath := notify.SourcesPath(stateRoot)
	src, err := ensureApprovalSource(sourcesPath, now)
	if err != nil {
		return err
	}
	level := notify.LevelUrgent
	body := fmt.Sprintf("Loop %q is pending approval.\nCommand: %s\nInterval: %s\nApprove with: hotline loop approve %s\nDeny with: hotline loop deny %s",
		l.Label, l.Cmd, l.Every, l.Label, l.Label)
	if yolo {
		level = notify.LevelNormal
		body = fmt.Sprintf("YOLO mode created loop %q and it is live immediately.\nCommand: %s\nInterval: %s\nRemove with: hotline loop remove %s",
			l.Label, l.Cmd, l.Every, l.Label)
	}
	reg, err := notify.LoadRegistry(sourcesPath)
	if err != nil {
		return err
	}
	_, err = notify.Enqueue(notify.SpoolPath(stateRoot), notify.RejectsPath(stateRoot), reg, src.Key, level, body, now)
	return err
}

func ensureApprovalSource(path string, now time.Time) (notify.Source, error) {
	reg, err := notify.LoadRegistry(path)
	if err != nil {
		return notify.Source{}, err
	}
	if src, ok := reg.FindByLabel(approvalNotifySource); ok {
		return src, nil
	}
	return notify.AddSource(path, approvalNotifySource, notify.LevelUrgent, notify.Rate{Burst: 20, RefillMins: 1}, "", now)
}
