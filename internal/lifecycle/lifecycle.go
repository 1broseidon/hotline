package lifecycle

import (
	"context"
	"fmt"
	"os"
	"os/signal"
	"sync"
	"syscall"
	"time"

	"github.com/modelcontextprotocol/go-sdk/mcp"
)

// ClaimPollerSlot is the exported entry point for the PID guard. It is called
// from main before the poller starts.
func ClaimPollerSlot(pidFile string) error { return claimPollerSlot(pidFile) }

// ReleasePollerSlot removes our pid file on a clean exit.
func ReleasePollerSlot(pidFile string) { releasePollerSlot(pidFile) }

// Run connects the MCP server over the transport, runs the poller (if any), and
// blocks until shutdown. Shutdown is triggered by any of: OS signal
// (SIGINT/SIGTERM/SIGHUP), stdin EOF (ServerSession.Wait returns), the orphan
// watchdog, or the poller giving up (e.g. persistent 409 Conflict). All paths
// funnel through one sync.Once that logs the reason, arms a 2s force-exit timer,
// and cancels the root context so the poller stops.
//
// poll may be nil (token-less mode): the handshake still runs. A non-nil error
// from poll is a give-up signal: it is routed through shutdown and returned so
// the process can exit non-zero for a supervisor to distinguish from a clean
// stop.
//
// cleanup (may be nil) runs on the force-exit path — os.Exit skips deferred
// cleanup, so callers pass their poller-slot releases here to avoid leaving
// stale PID files behind.
func Run(server *mcp.Server, transport mcp.Transport, cleanup func(), poll func(ctx context.Context) error) error {
	// The SDK owns stdio; connect with a non-cancellable context so closing the
	// session is our explicit decision, not a side effect of ctx cancellation.
	ss, err := server.Connect(context.Background(), transport, nil)
	if err != nil {
		return fmt.Errorf("connecting MCP server: %w", err)
	}

	rootCtx, cancel := context.WithCancel(context.Background())

	var once sync.Once
	shutdown := func(reason string) {
		once.Do(func() {
			fmt.Fprintf(os.Stderr, "hotline: shutting down (%s)\n", reason)
			// The current getUpdates request may take up to its long-poll
			// timeout to return; force-exit after 2s regardless. os.Exit skips
			// deferred cleanup, so run the caller's cleanup (poller-slot
			// releases) here too — otherwise a stalled shutdown leaves stale
			// PID files behind.
			time.AfterFunc(2*time.Second, func() {
				if cleanup != nil {
					cleanup()
				}
				os.Exit(0)
			})
			cancel()
		})
	}

	// OS signals. We watch a raw signal channel rather than a NotifyContext so a
	// real signal is distinguishable from defer-driven teardown: when the poll
	// loop exits on its own (e.g. persistent 409), shutdown cancels rootCtx and
	// this goroutine returns without falsely logging a "signal" shutdown reason.
	sigCh := make(chan os.Signal, 1)
	signal.Notify(sigCh, syscall.SIGINT, syscall.SIGTERM, syscall.SIGHUP)
	defer signal.Stop(sigCh)
	go func() {
		select {
		case <-sigCh:
			shutdown("signal")
		case <-rootCtx.Done():
		}
	}()

	// Stdin EOF: Wait returns when the client closes the connection.
	go func() {
		_ = ss.Wait()
		shutdown("stdin EOF")
	}()

	// Orphan watchdog.
	go orphanWatchdog(rootCtx, shutdown)

	// Run the poller in the foreground until shutdown.
	var pollErr error
	if poll != nil {
		// If the poller gives up on its own (it isn't responding to ctx
		// cancellation), funnel it through shutdown so the force-exit safety net
		// is armed and the reason is logged once.
		if pollErr = poll(rootCtx); pollErr != nil {
			shutdown("poll gave up: " + pollErr.Error())
		}
	} else {
		<-rootCtx.Done()
	}

	_ = ss.Close()
	return pollErr
}

// orphanWatchdog self-terminates if the parent process changes (the parent
// chain was severed by a crash and stdin events didn't fire).
func orphanWatchdog(ctx context.Context, shutdown func(string)) {
	bootPpid := os.Getppid()
	ticker := time.NewTicker(5 * time.Second)
	defer ticker.Stop()
	for {
		select {
		case <-ctx.Done():
			return
		case <-ticker.C:
			if os.Getppid() != bootPpid {
				shutdown("orphaned (reparented)")
				return
			}
		}
	}
}
