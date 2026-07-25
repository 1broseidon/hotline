package main

import (
	"context"
	"fmt"
	"os"

	"github.com/1broseidon/hotline/internal/app"
	"github.com/1broseidon/hotline/internal/harness"
	"github.com/1broseidon/hotline/internal/jobspool"
	"github.com/1broseidon/hotline/internal/lifecycle"
	"github.com/1broseidon/hotline/internal/mcpchan"
	"github.com/1broseidon/hotline/internal/notify"
	"github.com/1broseidon/hotline/internal/provider"
	"github.com/1broseidon/hotline/internal/schedule"
	"github.com/modelcontextprotocol/go-sdk/mcp"
)

// runPiHarness wires hotline to a Pi harness: the injected-harness seam with
// the "pi" label. See runInjectedHarness for the shared semantics.
func runPiHarness(router *provider.Router, sched *schedule.Scheduler, notifyDisp *notify.Dispatcher, jobDisp *jobspool.Dispatcher, jobSink jobspool.JobSink, agentInfoSink func(mcpchan.AgentInfoParams), appProvider *app.Provider, loopRunner loopService, transcriptPath, voice, publishExposure, schedulesPath string, cleanup func(), mcOpts ...mcpchan.ServerOption) error {
	return runInjectedHarness("pi", router, sched, notifyDisp, jobDisp, jobSink, agentInfoSink, appProvider, loopRunner, transcriptPath, voice, publishExposure, schedulesPath, cleanup, mcOpts...)
}

// harnessBindsSDKApply reports whether an injected-harness label implements the
// sdk_apply control, and so should have the hot forwarder + result sink bound.
// Kept as its own predicate so the binding decision is directly testable and
// stays in lockstep with internal/app's knobsFor table: a harness that takes
// remote model/effort must bind here, or every request on it would fall through
// to the restart path that the "no restart for a model change" directive bans.
func harnessBindsSDKApply(label string) bool {
	return label == "claude-sdk" || label == "pi"
}

// runInjectedHarness wires hotline to an external host that injects turns
// (the hotline-pi extension inside pi, or the claude-sdk node harness). It is
// the Claude Code path (main.go) with two deltas: (1) no permission relay —
// these hosts run every tool unguarded by design, so the capability is always
// off and the channel transport gets a nil handler; (2) inbound turns are
// pre-rendered through harness.RenderChannel into the notification content,
// so chat_id/source ride inside the <channel> envelope text the model reads
// (the host forwards that content verbatim as a user turn). The messaging
// providers, scheduler, and notify dispatcher are identical to the other
// harnesses; only this inbound-push seam differs.
func runInjectedHarness(label string, router *provider.Router, sched *schedule.Scheduler, notifyDisp *notify.Dispatcher, jobDisp *jobspool.Dispatcher, jobSink jobspool.JobSink, agentInfoSink func(mcpchan.AgentInfoParams), appProvider *app.Provider, loopRunner loopService, transcriptPath, voice, publishExposure, schedulesPath string, cleanup func(), mcOpts ...mcpchan.ServerOption) error {
	// Under `hotline up` the supervisor exports HOTLINE_SUPERVISOR_DIR into the
	// host environment, which the host passes through to the hotline run child
	// it spawns — enabling the restart tool exactly like the claude/opencode
	// paths.
	supervisorDir := os.Getenv("HOTLINE_SUPERVISOR_DIR")

	// Injected-harness hosts have no permission prompts, so the relay is
	// always off — every tool runs unguarded, always.
	const permission = false

	// The full, uncapped instruction block ships in the initialize result; the
	// host appends it to the system prompt per turn. claude-sdk gets its own
	// profile (reply contract + SDK preset neutralizer + Task-tool doctrine),
	// killing the pi-doctrine debt; every other injected label keeps the pi
	// server.
	var server *mcp.Server
	if label == "claude-sdk" {
		server = mcpchan.NewClaudeSDKServer(router, permission, transcriptPath, router.Sources(), voice, publishExposure, schedulesPath, supervisorDir, mcOpts...)
	} else {
		server = mcpchan.NewPiServer(router, permission, transcriptPath, router.Sources(), voice, publishExposure, schedulesPath, supervisorDir, mcOpts...)
	}

	fmt.Fprintf(os.Stderr, "hotline: harness=%s\n", label)

	// These hosts share Claude Code's stdio claude/channel transport, but with
	// no permission relay: a nil handler means the transport never dispatches
	// permission_request frames (the host never sends any).
	transport := mcpchan.NewChannelTransport(nil)

	// harness_info (§5.3): the host reports its resolved harness/model
	// identity over the same stdio; route it to the app provider's live
	// identity merge. On boxes without the app provider the sink is nil and
	// the notification is logged and dropped.
	transport.SetAgentInfoHandler(func(p mcpchan.AgentInfoParams) {
		if agentInfoSink == nil {
			// Model is presence-aware (nil = not reported, "" = cleared), so
			// render the distinction rather than printing a pointer.
			model := "(not reported)"
			if p.Model != nil {
				if *p.Model == "" {
					model = "(cleared)"
				} else {
					model = *p.Model
				}
			}
			fmt.Fprintf(os.Stderr, "hotline: harness_info (harness=%s model=%s) dropped — no app provider on this box\n", p.Harness, model)
			return
		}
		agentInfoSink(p)
	})

	// harness_catalog (model catalog amendment 2026-07-20): the host enumerates
	// the models it can actually select and the app renders THAT instead of a
	// list compiled into the client. Bound only where the app provider exists —
	// there is no other consumer, and unlike harness_info there is nothing worth
	// logging when a telegram-only box's harness reports one.
	if appProvider != nil {
		transport.SetHarnessCatalogHandler(appProvider.HarnessCatalogSink())
	}

	// Hot model/effort apply (SDK hot-model amendment 2026-07-19; pi hot-apply
	// amendment 2026-07-20): the hosts that implement the sdk_apply control bind
	// both directions — the forwarder the app server calls (set_sdk_config →
	// sdk_apply notification) and the result sink the transport routes back
	// (sdk_apply_result → pending resolution). claude-sdk applies it to the live
	// Agent SDK Query; pi applies it through its ExtensionAPI (setModel /
	// setThinkingLevel) and additionally restamps harness_info when the operator
	// changes model or thinking in pi's own TUI. The opencode/TUI labels bind
	// nothing: the hot path is unavailable there and the harness gate
	// (internal/app/sdkconfig.go knobsFor) refuses those boxes anyway.
	//
	// A pi box running an OLD extension (no sdk_apply handler) still binds the
	// forwarder here, so the box forwards and gets no answer — the pending timer
	// answers harness_unreachable after 10 s. That is the honest outcome: the
	// restart fallback is reserved for boxes whose WIRING never bound a
	// forwarder (telegram-only, pre-amendment binaries), which the box cannot
	// distinguish from a live-but-mute harness.
	if harnessBindsSDKApply(label) && appProvider != nil {
		transport.SetSDKApplyResultHandler(appProvider.SDKApplyResultSink())
		appProvider.SetSDKApplyForwarder(func(ctx context.Context, rid string, model, effort *string) error {
			return transport.Notifier().SendSDKApply(ctx, rid, model, effort)
		})
	}

	pollFn := func(ctx context.Context) error {
		var sink provider.InboundSink = &piSink{next: transport.Notifier()}
		replayCatchup(ctx, sink, transcriptPath)
		// FB13 auto job cards: the pi adapter (adapters/pi/, harness jobcards.ts)
		// enqueues intents via `hotline job`; this drains them like the claude path.
		return runWithLoopRunner(ctx, loopRunner, func(ctx context.Context) error {
			return runPollers(ctx, router, sched, notifyDisp, jobDisp, jobSink, sink)
		})
	}

	return lifecycle.Run(server, transport, cleanup, pollFn)
}

// piSink pre-renders each inbound turn into the <channel …> envelope before it
// goes out as a claude/channel notification, so routing keys (chat_id, source)
// ride inside the text the model sees — the same envelope Claude Code renders
// client-side from (content, meta), and the same one the OpenCode Link renders.
// Meta is dropped after rendering: everything the extension needs is now in the
// content, and it forwards that content verbatim to pi.sendUserMessage. The
// scheduler and notify dispatcher share this sink, so a schedule/notify fire
// renders as the same envelope (kind="schedule"|"notify") an inbound message
// does. It satisfies provider.InboundSink.
type piSink struct {
	next provider.InboundSink
}

func (s *piSink) SendChannel(ctx context.Context, content string, meta map[string]string) error {
	rendered := harness.RenderChannel(harness.Inbound{Content: content, Meta: meta})
	return s.next.SendChannel(ctx, rendered, nil)
}

func (s *piSink) SendVerdict(ctx context.Context, requestID, behavior string) error {
	return s.next.SendVerdict(ctx, requestID, behavior)
}
