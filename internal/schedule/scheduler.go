package schedule

import (
	"context"
	"fmt"
	"os"
	"slices"
	"time"

	"github.com/1broseidon/hotline/internal/transcript"
)

// Sink is the inbound-injection seam: the one method the scheduler needs from
// provider.InboundSink. Declared locally so this package never imports
// internal/provider (mcpchan imports schedule for the tool handler, and
// provider imports mcpchan — importing provider here would cycle).
// *mcpchan.Notifier and cmd/hotline's opencodeSink satisfy it structurally.
type Sink interface {
	SendChannel(ctx context.Context, content string, meta map[string]string) error
}

// tickInterval is how often the scheduler scans for due fires. 10s keeps
// worst-case fire latency low relative to short relative-duration reminders
// (e.g. "+1m") at negligible cost — a live test showed a fixed check
// interval matters proportionally more for short windows than long ones.
const tickInterval = 10 * time.Second

// Scheduler owns the fire loop. now/loc/tick are fields (defaulted in
// NewScheduler) so tests inject a fixed clock and a short tick.
type Scheduler struct {
	path    string
	sources []string           // configured provider names, for source fallback
	log     *transcript.Logger // primary provider's transcript; nil is a no-op
	now     func() time.Time
	loc     *time.Location
	tick    time.Duration
}

// NewScheduler builds a Scheduler over schedules.json at path. sources is
// router.Sources(); log may be nil.
func NewScheduler(path string, sources []string, log *transcript.Logger) *Scheduler {
	return &Scheduler{
		path:    path,
		sources: sources,
		log:     log,
		now:     time.Now,
		loc:     time.Local,
		tick:    tickInterval,
	}
}

// Path returns the schedules.json path this scheduler operates on, so callers
// that also need to register the MCP tool over the same file can share it.
func (s *Scheduler) Path() string { return s.path }

// Run fires due schedules until ctx is cancelled: one eager catch-up scan
// immediately, then a scan per tick. It returns nil on ctx cancellation and
// never otherwise exits — store and injection failures are logged to stderr
// and retried on the next tick's scan, never fatal to the process.
func (s *Scheduler) Run(ctx context.Context, sink Sink) error {
	s.fireDue(ctx, sink)
	t := time.NewTicker(s.tick)
	defer t.Stop()
	for {
		select {
		case <-ctx.Done():
			return nil
		case <-t.C:
			s.fireDue(ctx, sink)
		}
	}
}

// fire is one collected due schedule, copied out of the store so injection
// happens after the lock is released.
type fire struct {
	id, prompt, source, chatID, desc string
}

func (s *Scheduler) fireDue(ctx context.Context, sink Sink) {
	now := s.now()
	var fires []fire
	err := Mutate(s.path, func(d *Doc) error {
		keep := d.Schedules[:0]
		for _, sc := range d.Schedules {
			if sc.Paused {
				keep = append(keep, sc)
				continue
			}
			nf, perr := time.Parse(time.RFC3339, sc.NextFire)
			if perr != nil || sc.Recurrence.Validate() != nil {
				sc.Paused = true // pause, don't spam or drop; operator resumes after fixing
				fmt.Fprintf(os.Stderr, "hotline: schedule %s invalid; paused\n", sc.ID)
				keep = append(keep, sc)
				continue
			}
			if nf.After(now) {
				keep = append(keep, sc)
				continue
			}
			// Due. Advance/record BEFORE injecting, under the lock.
			fires = append(fires, fire{sc.ID, sc.Prompt, sc.Source, sc.ChatID, Describe(sc.Recurrence)})
			sc.LastFired = now.UTC().Format(time.RFC3339)
			next, ok := Advance(sc.Recurrence, nf, now, s.loc)
			if !ok { // once: completed — drop it
				continue
			}
			sc.NextFire = next.UTC().Format(time.RFC3339)
			keep = append(keep, sc)
		}
		d.Schedules = keep
		return nil
	})
	if err != nil {
		// Nothing was persisted, so nothing may be injected: injecting now and
		// persisting next tick would double-fire. Drop the collected fires.
		fmt.Fprintf(os.Stderr, "hotline: schedule scan failed: %v\n", err)
		return
	}
	for _, f := range fires {
		s.inject(ctx, sink, f)
	}
}

func (s *Scheduler) inject(ctx context.Context, sink Sink, f fire) {
	content := fmt.Sprintf("⏰ Scheduled task %s fired (%s). This is a timer you set earlier — the user has not just messaged you. Do the task below and send the outcome to this chat with the reply tool.\n\n%s",
		f.id, f.desc, f.prompt)
	src := f.source
	if len(s.sources) > 0 && !slices.Contains(s.sources, src) {
		if len(s.sources) == 1 {
			src = s.sources[0] // mirror router.pick's single-provider default
		}
		fmt.Fprintf(os.Stderr, "hotline: schedule %s source %q not configured (using %q)\n", f.id, f.source, src)
	}
	meta := map[string]string{
		"source":      src,
		"chat_id":     f.chatID,
		"kind":        "schedule",
		"schedule_id": f.id,
		"ts":          s.now().UTC().Format("2006-01-02T15:04:05.000Z07:00"),
	}
	if err := sink.SendChannel(ctx, content, meta); err != nil {
		fmt.Fprintf(os.Stderr, "hotline: schedule %s fire not delivered: %v\n", f.id, err)
		return
	}
	s.log.Append(transcript.Record{Dir: "in", ChatID: f.chatID, Kind: "schedule", Text: content}) // nil-safe
}
