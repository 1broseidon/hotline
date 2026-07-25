package loop

import (
	"bytes"
	"context"
	"errors"
	"fmt"
	"io"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"sync"
	"syscall"
	"time"

	"github.com/1broseidon/hotline/internal/notify"
	"github.com/1broseidon/hotline/internal/supervise"
)

const (
	scanInterval     = 10 * time.Second
	maxStdoutCapture = 1 << 20
	defaultLogMax    = 5 << 20
)

// activeRunners is the in-process half of the loop-host singleton. `hotline
// run`'s lifecycle.OwnerLease is the cross-process guard; this map catches an
// accidental second Runner in the same owning process before it can tick.
var activeRunners sync.Map

// Runner owns loop execution. It is hosted by `hotline run` and never
// auto-restarts a failed command; failures are logged and advisory state is
// updated for the next operator list.
type Runner struct {
	StateRoot   string
	Path        string
	SourcesPath string
	SpoolPath   string
	RejectsPath string
	Sink        LoopSink
	Log         io.Writer

	now      func() time.Time
	scanTick time.Duration
	logMax   int64

	mu       sync.Mutex
	inflight map[string]bool
}

// NewRunner builds a production runner rooted at stateRoot.
func NewRunner(stateRoot string) *Runner {
	r := &Runner{
		StateRoot:   stateRoot,
		Path:        Path(stateRoot),
		SourcesPath: notify.SourcesPath(stateRoot),
		SpoolPath:   notify.SpoolPath(stateRoot),
		RejectsPath: notify.RejectsPath(stateRoot),
		Log:         os.Stderr,
		now:         time.Now,
		scanTick:    scanInterval,
		logMax:      defaultLogMax,
		inflight:    map[string]bool{},
	}
	r.Sink = NewNotifySink(r.SpoolPath, r.SourcesPath, r.RejectsPath)
	return r
}

// Run reconciles loops.json until ctx is cancelled. A never-run or overdue
// loop ticks immediately; otherwise its first tick resumes from persisted
// LastRunAt. Store read failures are logged and retried on the next scan; they
// never stop the supervisor.
func (r *Runner) Run(ctx context.Context) error {
	key, err := r.singletonKey()
	if err != nil {
		return err
	}
	if _, loaded := activeRunners.LoadOrStore(key, struct{}{}); loaded {
		return fmt.Errorf("loop runner already active for box root %s", key)
	}
	defer activeRunners.Delete(key)

	if r.inflight == nil {
		r.inflight = map[string]bool{}
	}
	workers := map[string]*loopWorker{}
	var workerWG sync.WaitGroup
	var tickWG sync.WaitGroup
	reconcile := func() {
		d, err := Load(r.Path)
		if err != nil {
			r.logf("loop scan failed: %v", err)
			return
		}
		want := map[string]Loop{}
		for _, l := range d.Loops {
			if l.Paused || !l.Approved {
				continue
			}
			if err := normalizeLoop(&l); err != nil {
				r.logf("loop %s invalid; skipped: %v", l.Label, err)
				continue
			}
			want[l.Label] = l
		}
		for label, w := range workers {
			if l, ok := want[label]; !ok || !sameRunnable(w.loop, l) {
				w.cancel()
				delete(workers, label)
			}
		}
		for label, l := range want {
			if _, ok := workers[label]; ok {
				continue
			}
			wctx, cancel := context.WithCancel(ctx)
			w := &loopWorker{loop: l, cancel: cancel}
			workers[label] = w
			workerWG.Add(1)
			go func() {
				defer workerWG.Done()
				r.runWorker(wctx, l, &tickWG)
			}()
		}
	}

	reconcile()
	t := time.NewTicker(r.scanTick)
	defer t.Stop()
	for {
		select {
		case <-ctx.Done():
			for _, w := range workers {
				w.cancel()
			}
			// Stop every worker before waiting for ticks, so no WaitGroup.Add can
			// race with tickWG.Wait. RunOnce observes the cancelled context and
			// kills its command process group before the box lease is released.
			workerWG.Wait()
			tickWG.Wait()
			return nil
		case <-t.C:
			reconcile()
		}
	}
}

type loopWorker struct {
	loop   Loop
	cancel context.CancelFunc
}

func sameRunnable(a, b Loop) bool {
	return a.Every == b.Every &&
		a.Cmd == b.Cmd &&
		a.NotifyLLM == b.NotifyLLM &&
		a.Sink == b.Sink &&
		a.Source == b.Source &&
		a.Level == b.Level &&
		a.Timeout == b.Timeout &&
		a.Approved == b.Approved
}

func (r *Runner) runWorker(ctx context.Context, l Loop, ticks *sync.WaitGroup) {
	if delay := r.nextRunDelay(l); delay > 0 {
		t := time.NewTimer(delay)
		select {
		case <-ctx.Done():
			t.Stop()
			return
		case <-t.C:
		}
	}

	for {
		r.launch(ctx, l, ticks)
		t := time.NewTimer(l.EveryDuration())
		select {
		case <-ctx.Done():
			t.Stop()
			return
		case <-t.C:
		}
	}
}

// nextRunDelay verifies and applies the persisted run watermark. Missing,
// malformed, or overdue watermarks run now; a future watermark is capped to
// one interval so clock skew cannot wedge a loop indefinitely.
func (r *Runner) nextRunDelay(l Loop) time.Duration {
	if strings.TrimSpace(l.LastRunAt) == "" {
		return 0
	}
	last, err := time.Parse(time.RFC3339, l.LastRunAt)
	if err != nil {
		r.logf("loop %s has invalid lastRunAt %q; running now", l.Label, l.LastRunAt)
		return 0
	}
	interval := l.EveryDuration()
	delay := last.Add(interval).Sub(r.now())
	if delay <= 0 {
		return 0
	}
	if delay > interval {
		r.logf("loop %s lastRunAt is in the future; retrying in %s", l.Label, interval)
		return interval
	}
	return delay
}

func (r *Runner) singletonKey() (string, error) {
	root := strings.TrimSpace(r.StateRoot)
	if root == "" {
		return "", fmt.Errorf("loop runner state root is required")
	}
	key, err := filepath.Abs(filepath.Clean(root))
	if err != nil {
		return "", fmt.Errorf("resolving loop runner state root %q: %w", root, err)
	}
	return key, nil
}

func (r *Runner) launch(ctx context.Context, l Loop, ticks *sync.WaitGroup) {
	ticks.Add(1)
	go func() {
		defer ticks.Done()
		if _, err := r.RunOnce(ctx, l); err != nil {
			r.logf("loop %s run failed: %v", l.Label, err)
		}
	}()
}

// Result is the outcome of one foreground or supervisor-hosted tick.
type Result struct {
	ExitCode int
	Skipped  bool
	Stdout   string
	Duration time.Duration
}

// RunOnce executes one loop tick with overlap protection and advisory status
// recording. If the loop is already running, it logs and returns Skipped.
func (r *Runner) RunOnce(ctx context.Context, l Loop) (Result, error) {
	if r.inflight == nil {
		r.inflight = map[string]bool{}
	}
	if err := normalizeLoop(&l); err != nil {
		return Result{ExitCode: 1}, err
	}
	if !l.Approved {
		r.logf("loop %s skipped: pending approval", l.Label)
		return Result{Skipped: true}, nil
	}
	if !r.markInFlight(l.Label) {
		r.logf("loop %s skipped: still running", l.Label)
		return Result{Skipped: true}, nil
	}
	defer r.clearInFlight(l.Label)

	start := r.now()
	// Persist the start watermark before executing. If the owning process dies
	// mid-command, the next `up` resumes from this tick instead of immediately
	// duplicating its effects. A persistence failure refuses the tick.
	if recErr := RecordRunStart(r.Path, l.Label, start); recErr != nil {
		return Result{ExitCode: 1}, fmt.Errorf("persisting loop %s start: %w", l.Label, recErr)
	}
	// Panic barrier: a latent panic in a single loop run must never take down
	// the hosting `hotline run` process. Recover, log it, and record a failed
	// run. clearInFlight is a separate defer, so the in-flight flag is cleared
	// on this path too.
	defer func() {
		if rec := recover(); rec != nil {
			r.logf("loop %s panicked: %v", l.Label, rec)
			if recErr := RecordRunResult(r.Path, l.Label, 1, r.now().Sub(start)); recErr != nil {
				r.logf("loop %s status update failed: %v", l.Label, recErr)
			}
		}
	}()
	res, err := r.runCommand(ctx, l, start)
	res.Duration = r.now().Sub(start)
	if recErr := RecordRunResult(r.Path, l.Label, res.ExitCode, res.Duration); recErr != nil {
		r.logf("loop %s status update failed: %v", l.Label, recErr)
	}
	if err == nil && l.NotifyLLM && strings.TrimSpace(res.Stdout) != "" {
		if routeErr := r.Sink.Route(ctx, l, res.Stdout, res.ExitCode); routeErr != nil {
			r.logf("loop %s notify sink failed: %v", l.Label, routeErr)
		}
	}
	return res, err
}

func (r *Runner) markInFlight(label string) bool {
	r.mu.Lock()
	defer r.mu.Unlock()
	if r.inflight[label] {
		return false
	}
	r.inflight[label] = true
	return true
}

func (r *Runner) clearInFlight(label string) {
	r.mu.Lock()
	defer r.mu.Unlock()
	delete(r.inflight, label)
}

func (r *Runner) runCommand(ctx context.Context, l Loop, start time.Time) (Result, error) {
	if err := os.MkdirAll(StateDir(r.StateRoot, l.Label), 0o700); err != nil {
		return Result{ExitCode: 1}, err
	}
	logPath := LogPath(r.StateRoot, l.Label)
	if err := os.MkdirAll(filepath.Dir(logPath), 0o700); err != nil {
		return Result{ExitCode: 1}, err
	}
	logw, err := supervise.NewRotatingWriter(logPath, r.logMax)
	if err != nil {
		return Result{ExitCode: 1}, err
	}
	defer logw.Close()

	fmt.Fprintf(logw, "\n[%s] loop %s start\n", start.UTC().Format(time.RFC3339), l.Label)

	runCtx, cancel := context.WithTimeout(ctx, l.TimeoutDuration())
	defer cancel()
	cmd := exec.CommandContext(runCtx, "/bin/sh", "-c", l.Cmd)
	cmd.Env = append(os.Environ(),
		"HOTLINE_LOOP_STATE_DIR="+StateDir(r.StateRoot, l.Label),
		"HOTLINE_LOOP_LABEL="+l.Label,
	)
	if l.Source != "" {
		key, err := r.sourceKey(l.Source)
		switch {
		case err != nil && l.NotifyLLM:
			// Fail-closed: the notify sink genuinely needs the source key.
			fmt.Fprintf(logw, "[%s] source resolve failed: %v\n", r.now().UTC().Format(time.RFC3339), err)
			return Result{ExitCode: 1}, err
		case err != nil:
			// Best-effort: --source only populates the HOTLINE_NOTIFY_SOURCE
			// convenience env var here, so a revoked source must not stop the
			// script from running. Leave the var unset and carry on.
			fmt.Fprintf(logw, "[%s] source resolve failed (best-effort, HOTLINE_NOTIFY_SOURCE unset): %v\n", r.now().UTC().Format(time.RFC3339), err)
			r.logf("loop %s source resolve failed; running without HOTLINE_NOTIFY_SOURCE: %v", l.Label, err)
		default:
			cmd.Env = append(cmd.Env, "HOTLINE_NOTIFY_SOURCE="+key)
		}
	}
	cmd.SysProcAttr = &syscall.SysProcAttr{Setsid: true}
	cmd.Cancel = func() error {
		if cmd.Process == nil {
			return nil
		}
		return syscall.Kill(-cmd.Process.Pid, syscall.SIGKILL)
	}
	cmd.WaitDelay = 5 * time.Second

	cap := &captureWriter{max: maxStdoutCapture}
	cmd.Stdout = io.MultiWriter(logw, cap)
	cmd.Stderr = logw
	err = cmd.Run()

	exit := exitCode(err)
	if errors.Is(runCtx.Err(), context.DeadlineExceeded) {
		exit = 124
	}
	if cap.truncated {
		fmt.Fprintf(logw, "\n[stdout truncated at %d bytes]\n", maxStdoutCapture)
	}
	fmt.Fprintf(logw, "[%s] loop %s exit=%d duration=%s\n", r.now().UTC().Format(time.RFC3339), l.Label, exit, r.now().Sub(start).Round(time.Millisecond))
	return Result{ExitCode: exit, Stdout: cap.String()}, nil
}

func (r *Runner) sourceKey(label string) (string, error) {
	reg, err := notify.LoadRegistry(r.SourcesPath)
	if err != nil {
		return "", err
	}
	src, ok := reg.FindByLabel(label)
	if !ok {
		return "", fmt.Errorf("notify source %q not found", label)
	}
	return src.Key, nil
}

func exitCode(err error) int {
	if err == nil {
		return 0
	}
	var ee *exec.ExitError
	if errors.As(err, &ee) {
		return ee.ExitCode()
	}
	return 1
}

func (r *Runner) logf(format string, args ...any) {
	if r.Log == nil {
		return
	}
	fmt.Fprintf(r.Log, "hotline: %s\n", fmt.Sprintf(format, args...))
}

type captureWriter struct {
	buf       bytes.Buffer
	max       int
	truncated bool
}

func (w *captureWriter) Write(p []byte) (int, error) {
	remain := w.max - w.buf.Len()
	if remain > 0 {
		if len(p) <= remain {
			_, _ = w.buf.Write(p)
		} else {
			_, _ = w.buf.Write(p[:remain])
			w.truncated = true
		}
	} else {
		w.truncated = true
	}
	return len(p), nil
}

func (w *captureWriter) String() string { return w.buf.String() }
