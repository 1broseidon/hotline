package loop

import (
	"context"
	"fmt"
	"strings"
	"time"

	"github.com/1broseidon/hotline/internal/notify"
)

// LoopSink routes a loop's stdout after a completed run. v1 ships only the
// notify sink; richer judges can plug in behind the same interface later.
type LoopSink interface {
	Route(ctx context.Context, l Loop, stdout string, exit int) error
}

// NewNotifySink builds the v1 sink over the existing notify registry and spool.
func NewNotifySink(spoolPath, sourcesPath, rejectsPath string) LoopSink {
	return notifySink{spoolPath: spoolPath, sourcesPath: sourcesPath, rejectsPath: rejectsPath, now: time.Now}
}

type notifySink struct {
	spoolPath   string
	sourcesPath string
	rejectsPath string
	now         func() time.Time
}

func (s notifySink) Route(ctx context.Context, l Loop, stdout string, exit int) error {
	if strings.TrimSpace(stdout) == "" {
		return nil
	}
	if err := ctx.Err(); err != nil {
		return err
	}
	if strings.TrimSpace(l.Source) == "" {
		return fmt.Errorf("loop %q has notify_llm enabled but no source label", l.Label)
	}
	reg, err := notify.LoadRegistry(s.sourcesPath)
	if err != nil {
		return err
	}
	src, ok := reg.FindByLabel(l.Source)
	if !ok {
		return fmt.Errorf("notify source %q not found for loop %q", l.Source, l.Label)
	}
	level, err := notify.ParseLevel(l.Level)
	if err != nil {
		return err
	}
	_, err = notify.Enqueue(s.spoolPath, s.rejectsPath, reg, src.Key, level, stdout, s.now())
	return err
}
