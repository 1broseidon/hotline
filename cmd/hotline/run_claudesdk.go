package main

import (
	"github.com/1broseidon/hotline/internal/app"
	"github.com/1broseidon/hotline/internal/jobspool"
	"github.com/1broseidon/hotline/internal/mcpchan"
	"github.com/1broseidon/hotline/internal/notify"
	"github.com/1broseidon/hotline/internal/provider"
	"github.com/1broseidon/hotline/internal/schedule"
)

// runClaudeSDKHarness wires hotline to the claude-sdk harness (the Agent-SDK
// managed claude edition, harness/claude-sdk): the injected-harness seam with
// the "claude-sdk" label. The node harness owns this run child exactly the way
// the hotline-pi extension does — it spawns `hotline run`, reads the uncapped
// instructions from initialize, and forwards each pre-rendered <channel>
// envelope into the SDK session's streaming input. The piSink's pre-rendered
// envelope IS the claude-sdk inbound contract, so everything is shared with
// runPiHarness via runInjectedHarness.
//
// Instruction profile: runInjectedHarness selects NewClaudeSDKServer for the
// "claude-sdk" label, so the SDK session gets its own uncapped profile — the
// reply contract, the preset neutralizer, and the Task-tool delegation
// doctrine — instead of pi's Agent-tool doctrine. This retires the earlier
// instruction-text debt (the SDK session no longer sees pi's ~/.pi/agent/agents
// text).
func runClaudeSDKHarness(router *provider.Router, sched *schedule.Scheduler, notifyDisp *notify.Dispatcher, jobDisp *jobspool.Dispatcher, jobSink jobspool.JobSink, agentInfoSink func(mcpchan.AgentInfoParams), appProvider *app.Provider, loopRunner loopService, transcriptPath, voice, publishExposure, schedulesPath string, cleanup func(), mcOpts ...mcpchan.ServerOption) error {
	return runInjectedHarness("claude-sdk", router, sched, notifyDisp, jobDisp, jobSink, agentInfoSink, appProvider, loopRunner, transcriptPath, voice, publishExposure, schedulesPath, cleanup, mcOpts...)
}
