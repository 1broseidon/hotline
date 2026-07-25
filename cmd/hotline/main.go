// Command hotline is a messaging channel for Claude Code (Telegram, Discord, Signal): an MCP server that
// relays Telegram DMs/groups to a Claude Code session and back, with access
// control (pairing/allowlist/groups), media handling, and a permission relay.
//
// Subcommands:
//
//	hotline setup        save credentials to the shared .env (run once)
//	hotline init         install the hotline plugin and enable it for this repo
//	hotline up           launch the harness supervised (foreground by default)
//	hotline start        deprecated alias for hotline up
//	hotline relay        run the outbound-only v2 app channel and pairing flow
//	hotline down         stop the supervised session
//	hotline [run]        MCP server + provider poller plumbing (default)
//	hotline pair <code>  approve a pending pairing code
//	hotline deny <code>  reject a pending pairing code
//	hotline revoke <id>  remove an approved sender from the allowlist
//	hotline status       print state-dir / token / access summary
//	hotline schedule     operator view of scheduled tasks (list/remove/pause/resume)
//	hotline loop         operator view of script loops (add/list/remove/pause/resume/logs/run)
package main

import (
	"context"
	"errors"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"time"

	"github.com/1broseidon/hotline/internal/access"
	"github.com/1broseidon/hotline/internal/app"
	"github.com/1broseidon/hotline/internal/catchup"
	"github.com/1broseidon/hotline/internal/config"
	"github.com/1broseidon/hotline/internal/discord"
	"github.com/1broseidon/hotline/internal/jobspool"
	"github.com/1broseidon/hotline/internal/lifecycle"
	"github.com/1broseidon/hotline/internal/loop"
	"github.com/1broseidon/hotline/internal/mc"
	"github.com/1broseidon/hotline/internal/mcpchan"
	"github.com/1broseidon/hotline/internal/notify"
	"github.com/1broseidon/hotline/internal/provider"
	"github.com/1broseidon/hotline/internal/schedule"
	"github.com/1broseidon/hotline/internal/signal"
	"github.com/1broseidon/hotline/internal/telegram"
	"github.com/1broseidon/hotline/internal/transcript"
)

func main() {
	// --bot <name> (or --bot=<name>) selects which bot to run/operate on; it may
	// appear anywhere and is stripped before subcommand parsing. Falls back to
	// $HOTLINE_BOT (legacy: $TELE_GO_BOT). "" is the default/unnamed bot.
	// Everything after a bare "--" is passthrough (start forwards it to
	// claude verbatim); split it off before any flag stripping touches it.
	head, passthrough := splitPassthrough(os.Args[1:])
	initialCmd := commandName(head)
	projectDir := ""
	if projectScopedCommand(initialCmd) {
		var err error
		projectDir, err = os.Getwd()
		if err != nil {
			fmt.Fprintf(os.Stderr, "hotline: resolving project directory: %v\n", err)
			os.Exit(1)
		}
	}
	botName, providerSel, args, cmd, botExplicit, invocationErr := resolveInvocation(head, projectDir, os.Stderr)
	if invocationErr != nil {
		fmt.Fprintf(os.Stderr, "hotline: %v\n", invocationErr)
		os.Exit(1)
	}

	var err error
	switch cmd {
	case "run":
		err = runChannel(botName)
	case "pair":
		err = cmdPair(providerSel, botName, args[1:])
	case "deny":
		err = cmdDeny(providerSel, botName, args[1:])
	case "revoke":
		err = cmdRevoke(providerSel, botName, args[1:])
	case "status":
		err = cmdStatus(providerSel, botName, projectDir, os.Stdout, os.Stderr)
	case "schedule":
		err = cmdSchedule(botName, args[1:])
	case "loop":
		err = cmdLoop(botName, args[1:], os.Stdout, os.Stderr)
	case "notify":
		// The send path returns a script-facing exit code (0 accepted, 3 queued,
		// 4 rejected, 2 usage, 1 internal); exit directly, bypassing the generic
		// error handler below.
		os.Exit(cmdNotify(botName, args[1:], os.Stdin, os.Stdout, os.Stderr))
	case "job":
		// Automatic job cards: the send path returns a script-facing exit code
		// (0 accepted, 4 rejected/full, 2 usage, 1 internal); exit directly.
		os.Exit(cmdJob(botName, args[1:], os.Stdout, os.Stderr))
	case "mission":
		// Mission Control CLI twin (direct-write via internal/mc): returns a
		// script-facing exit code (0 accepted, 4 rejected, 2 usage, 1 internal)
		// so hooks/scripts can branch; exit directly.
		os.Exit(cmdMission(botName, args[1:], os.Stdout, os.Stderr))
	case "source":
		err = cmdSource(botName, args[1:], os.Stdout)
	case "tail":
		os.Exit(cmdTail(botName, args[1:], os.Stdout, os.Stderr))
	case "setup":
		err = cmdSetup(botName, args[1:], os.Stdin, os.Stdout, stdinIsTTY())
	case "init":
		cwd, cerr := os.Getwd()
		if cerr != nil {
			err = cerr
			break
		}
		err = cmdInit(botName, args[1:], cwd, os.Stdout)
	case "start":
		cwd, cerr := os.Getwd()
		if cerr != nil {
			err = cerr
			break
		}
		err = cmdStart(botName, botExplicit, args[1:], passthrough, cwd, os.Stdout, os.Stderr)
	case "up":
		cwd, cerr := os.Getwd()
		if cerr != nil {
			err = cerr
			break
		}
		err = cmdUp(botName, botExplicit, args[1:], passthrough, cwd, os.Stdout, os.Stderr)
	case "relay":
		err = cmdRelay(botName, args[1:], os.Stdout, os.Stderr)
	case "fleet":
		err = cmdFleet(botName, args[1:], os.Stdout, os.Stderr)
	case "down":
		err = cmdDown(botName, botExplicit, args[1:], projectDir, os.Stdout, os.Stderr)
	case "version", "--version":
		cmdVersion()
	case "-h", "--help", "help":
		usage()
	default:
		// Unknown first arg: treat as default "run" only if it's not clearly a
		// subcommand typo. Be strict and show usage.
		fmt.Fprintf(os.Stderr, "hotline: unknown command %q\n\n", cmd)
		usage()
		os.Exit(2)
	}

	if err != nil {
		if coder, ok := err.(interface{ Code() int }); ok {
			os.Exit(coder.Code())
		}
		fmt.Fprintf(os.Stderr, "hotline: %v\n", err)
		os.Exit(1)
	}
}

func usage() {
	fmt.Fprint(os.Stderr, `hotline - messaging channel for Claude Code

Usage:
  hotline setup        save credentials to the shared .env (run once;
                       --telegram-token, --discord-token, --signal-account,
                       --signal-daemon-url; --show prints the current config)
  hotline init         install the hotline plugin and enable it for this repo
                       (--providers telegram,signal; --voice writes HOTLINE.md;
                       --mcp-json registers a raw .mcp.json server instead;
                       --harness claude|claude-sdk|opencode|pi selects the
                       harness — opencode scaffolds its config, pi/claude-sdk
                       skip the plugin and print their wiring steps)
  hotline up           launch the harness supervised: foreground by default,
                       restarted on crash with backoff until hotline down.
                       --background/-d detaches the headless pi and opencode
                       harnesses. Claude's development-channel consent needs an
                       attended terminal, so Claude rejects background mode;
                       run it long-lived with:
                         tmux new -s hotline -- hotline up
                       Detach with Ctrl-b d; reattach with:
                         tmux attach -t hotline
                       The old --foreground flag is a deprecated no-op.
                       --harness claude|claude-sdk|opencode|pi picks what runs
                       (exported as HOTLINE_HARNESS for this launch and every
                       respawn; precedence flag > env > state .env; the launch
                       line reports the resolved harness and its source).
                       HOTLINE_HARNESS picks what runs: claude (default, on a
                       supervisor pty; --yolo adds
                       --dangerously-skip-permissions), opencode (supervises
                       "opencode serve"; --yolo errors — set opencode.json's
                       permission block instead), or pi (supervises
                       "pi --mode rpc"; tools run unguarded and --yolo errors;
                       set HOTLINE_PI_PROVIDER / HOTLINE_PI_MODEL /
                       HOTLINE_PI_THINKING in the .env, or override per-run
                       after --; per-run flags win. HOTLINE_PI_MODELS is the
                       comma-separated pattern list scoping Ctrl+P model
                       cycling — globs allowed ("anthropic/*", "*sonnet*") —
                       and it is the SAME list the app's model row offers).
                       Logs live under <box-root>/supervisor/. Args after --
                       go to the harness: hotline up -- --continue
  hotline start        deprecated alias for attached hotline up
  hotline down         stop the supervised session for the box this directory
                       resolves to — the same resolution hotline up uses here.
                       Announces the box root and supervisor/harness pids before
                       signalling. Refuses when this directory names a box your
                       shell does not select (so it can't SIGTERM an unrelated
                       box); proceed with --state-dir <box-root>, --force, or by
                       exporting that box's HOTLINE_STATE_DIR.
  hotline relay        run the v2 app relay (outbound-only); prints a pairing QR
                       when no active device is linked
  hotline relay status show the current room and linked device states
  hotline relay revoke <device-id>
                       revoke a linked device
  hotline relay new-link
                       invalidate the old link and print a fresh pairing QR
                       Rendezvous: HOTLINE_RENDEZVOUS_URL (default wss://relay.hotline.dev)
  hotline fleet        agent-to-agent (A2A) peer links, separate from the
                       operator app and the relay-state room cap:
                       fleet link --alias <peer>   mint a fleet room + pair URI
                       fleet join <uri|-> --alias <peer>
                                                   store a peer's URI (use - to
                                                   read the URI from stdin)
                       fleet ls [--json] | rm <peer> | rename <peer> <new-alias>
  hotline [run]        plumbing for .mcp.json: start the MCP server, provider
                       pollers, and box loop ticker (default; operators launch
                       with up)
  hotline pair <code>  approve a pending pairing code
  hotline deny <code>  reject a pending pairing code
  hotline revoke <id>  remove an approved sender from the allowlist
                       (exact sender ID as shown by status, or a unique prefix)
  hotline status       print state-dir / token / access summary
  hotline schedule     operator view of scheduled tasks
                       (schedule list | remove <id> | pause <id> | resume <id>;
                       schedules are created from chat via the schedule tool)
  hotline loop         manage script loops
                       (loop add <label> --every <dur> --cmd "<shell>"
                       [-y|--approve] [--notify-llm]
                       [--source <notify-label>] [--level L] [--timeout <dur>]
                       | list | remove | pause | resume | approve | deny |
                       logs | run)
  hotline notify       enqueue a machine event from a local script for the agent
                       to triage (--source <key> [--level urgent|normal|low]
                       ["message"|stdin]; exit 0 accepted, 3 queued, 4 rejected).
                       "hotline notify list" shows the spool
  hotline job          drive an app-channel job card from a harness hook or
                       script: job start|update|done --cookie <id>
                       [--batch <id>] [--title] [--detail] [--progress f]
                       [--state ok|err|cancelled] [--notify]; workers sharing a
                       --batch roll up into one card. "hotline job list" shows
                       the spool and active cards
  hotline mission      read/write the box's Mission Control memory directly
                       (mission note --text … [--thread slug] | update --thread
                       slug [--status …] [--summary …] [--next …] [--text …] |
                       handoff --state … --next … [--beware …] [--trigger …] |
                       archive --thread slug --outcome … | show [slug] |
                       hook session-start|pre-compact). Same files, same lock as
                       the mission MCP tool — no daemon needed
  hotline source       manage notify capability keys
                       (source add <label> [--cap L] [--burst N] [--refill-mins M]
                       [--chat-id ID] | source list | source revoke <label>)
  hotline tail         watch a supervised box think, live: pretty-prints the
                       harness event log (<box-root>/supervisor/harness.log) —
                       inbound turns, tool calls, replies, per-turn usage.
                       [--state-dir <dir>] [-f|--follow] [--no-follow] [--json]
                       [-n <lines>]. Follows by default on a TTY; read-only
                       (no chat input). Rich turns are pi-harness only.
  hotline version      print the hotline version

Options:
  --bot <name>         select a named bot (isolated state under bots/<name>,
                       token from TELEGRAM_BOT_TOKEN_<NAME>). Omit for the
                       default bot. Also settable via $HOTLINE_BOT
                       (legacy: $TELE_GO_BOT).
  --provider <sel>     for pair/deny/revoke/status: which provider's state to
                       operate
                       on, as kind[:instance] (default: telegram). Example:
                       hotline pair a1b2c3 --provider discord
                       hotline status --provider signal
`)
}

// splitPassthrough splits args at the first bare "--": everything before is
// hotline's, everything after goes to the child process verbatim.
func splitPassthrough(args []string) (head, tail []string) {
	for i, a := range args {
		if a == "--" {
			return args[:i], args[i+1:]
		}
	}
	return args, nil
}

// resolveInvocation adopts the current project's MCP identity before applying
// environment fallbacks, then returns the parsed operator command. Explicit
// --bot/--provider flags remain authoritative.
func resolveInvocation(head []string, projectDir string, stderr io.Writer) (botName, providerSel string, args []string, cmd string, botExplicit bool, resolveErr error) {
	cmd = commandName(head)
	identity := mcpIdentity{}
	if projectScopedCommand(cmd) && projectDir != "" {
		var err error
		identity, err = adoptProjectMCPServerEnv(projectDir, stderr)
		if err != nil {
			return "", "", nil, cmd, false, err
		}
	}
	botName, args, botExplicit = extractBotName(head)
	if !botExplicit {
		if identity.Found {
			botName = identity.BotName
		} else if botName = os.Getenv("HOTLINE_BOT"); botName == "" {
			botName = os.Getenv("TELE_GO_BOT")
		}
	}
	providerSel, args = resolveProviderFlag(args)
	cmd = "run"
	if len(args) > 0 {
		cmd = args[0]
	}
	if identity.Found && cmd == "relay" && botName == "" && !botExplicit {
		botName, resolveErr = relayProjectBotName()
		if resolveErr != nil {
			return "", providerSel, args, cmd, false, resolveErr
		}
	}
	return botName, providerSel, args, cmd, botExplicit, nil
}

// relayProjectBotName maps a project provider set onto relay's app instance.
// No named instances means the default app state; one distinct instance is
// deterministic; several instances are ambiguous and must be selected with
// --bot rather than guessed.
func relayProjectBotName() (string, error) {
	specs, err := config.Providers("")
	if err != nil {
		return "", err
	}
	instances := map[string]bool{}
	for _, spec := range specs {
		if spec.Instance != "" {
			instances[spec.Instance] = true
		}
	}
	switch len(instances) {
	case 0:
		return "", nil
	case 1:
		for instance := range instances {
			return instance, nil
		}
	default:
		return "", fmt.Errorf("the project MCP entry selects multiple named instances; use --bot <name> for hotline relay")
	}
	return "", nil
}

// commandName identifies the verb without applying HOTLINE_BOT fallbacks. It
// is used to decide whether project-local MCP identity is relevant.
func commandName(args []string) string {
	_, args, _ = extractBotName(args)
	_, args = resolveProviderFlag(args)
	if len(args) == 0 {
		return "run"
	}
	return args[0]
}

func projectScopedCommand(cmd string) bool {
	switch cmd {
	case "up", "start", "down", "status", "tail", "pair", "deny", "revoke",
		"schedule", "loop", "notify", "job", "mission", "source", "relay", "fleet":
		return true
	default:
		return false
	}
}

// extractBotName strips an explicit --bot selection without consulting the
// environment. found distinguishes an explicit flag from fallback resolution.
func extractBotName(args []string) (botName string, rest []string, found bool) {
	rest = make([]string, 0, len(args))
	for i := 0; i < len(args); i++ {
		switch a := args[i]; {
		case a == "--bot":
			if i+1 < len(args) {
				botName, found = args[i+1], true
				i++ // consume the value
			}
		case strings.HasPrefix(a, "--bot="):
			botName, found = strings.TrimPrefix(a, "--bot="), true
		default:
			rest = append(rest, a)
		}
	}
	return botName, rest, found
}

// resolveBotName applies the environment fallback only when no explicit flag
// was present.
func resolveBotName(args []string) (botName string, rest []string) {
	botName, rest, found := extractBotName(args)
	if !found {
		if botName = os.Getenv("HOTLINE_BOT"); botName == "" {
			botName = os.Getenv("TELE_GO_BOT") // legacy fallback
		}
	}
	return botName, rest
}

// resolveProviderFlag extracts "--provider <kind[:instance]>" /
// "--provider=<kind[:instance]>" from args, returning the selection ("" means
// the default, telegram) and the remaining args.
func resolveProviderFlag(args []string) (sel string, rest []string) {
	rest = make([]string, 0, len(args))
	for i := 0; i < len(args); i++ {
		switch a := args[i]; {
		case a == "--provider":
			if i+1 < len(args) {
				sel = args[i+1]
				i++
			}
		case strings.HasPrefix(a, "--provider="):
			sel = strings.TrimPrefix(a, "--provider=")
		default:
			rest = append(rest, a)
		}
	}
	return sel, rest
}

// loadOpsConfig resolves the config that pair / deny / revoke / status operate
// on: the
// telegram instance selected by --bot when --provider is absent or telegram,
// or the discord instance for --provider discord[:instance].
func loadOpsConfig(providerSel, botName string) (*config.Config, error) {
	kind, instance, _ := strings.Cut(providerSel, ":")
	switch kind {
	case "":
		specs, err := config.Providers(botName)
		if err != nil {
			return nil, err
		}
		var telegrams []config.ProviderSpec
		for _, spec := range specs {
			if spec.Kind == "telegram" {
				telegrams = append(telegrams, spec)
			}
		}
		switch len(telegrams) {
		case 0:
			return nil, fmt.Errorf("telegram is not configured for the selected box; use --provider kind[:instance]")
		case 1:
			instance = telegrams[0].Instance
		default:
			return nil, fmt.Errorf("the selected box has multiple telegram providers; use --provider telegram:<instance>")
		}
		return config.Load(instance)
	case "telegram":
		if instance == "" {
			instance = botName
		}
		return config.Load(instance)
	case "discord":
		return config.LoadDiscord(instance)
	case "signal":
		return config.LoadSignal(instance)
	case "app":
		return config.LoadApp(instance)
	default:
		return nil, fmt.Errorf("unknown provider %q (supported: telegram, discord, signal, app)", kind)
	}
}

// runChannel is the main entry: it always runs the MCP handshake, then starts
// every configured provider (HOTLINE_PROVIDERS, default just "telegram") on
// the shared channel stream. Providers with a token poll their transport; the
// permission capability is declared when at least one provider can
// authenticate the replier.
func runChannel(botName string) error {
	box, err := config.ResolveBox(botName)
	if err != nil {
		return err
	}
	specs := box.Providers
	boxRoot := box.Root

	// Resolve the complete runtime identity before any provider construction or
	// state seeding. Ownership is the first mutating runtime action.
	harnessMode, err := config.Harness()
	if err != nil {
		return err
	}
	mcCfg, err := config.MissionControlForRoot(harnessMode, boxRoot)
	if err != nil {
		return err
	}
	workDir, err := os.Getwd()
	if err != nil {
		return err
	}
	ownerSpec, err := boxOwnerSpec(box, harnessMode, workDir, mcCfg)
	if err != nil {
		return err
	}
	ownerLease, err := lifecycle.ClaimBox(ownerSpec, os.Getenv(lifecycle.OwnerLeaseEnv))
	if err != nil {
		return err
	}
	defer ownerLease.Release()

	// The loop ticker belongs to the channel-serving runtime. ClaimBox always
	// locks <boxRoot>/.hotline/active.lock before we construct it, so this exact
	// box root is guarded across processes; Runner's own singleton covers only
	// accidental duplicates inside this process.
	loopRunner := loop.NewRunner(boxRoot)

	// Box identity for wire metadata (welcome §3.2 / agent_state §1.2): the
	// harness kind is authoritative from this run child's config (supervisor
	// and MCP-loader paths both set HOTLINE_HARNESS); model/effort are seeded
	// when cheaply knowable — claude-sdk from its knob family, pi from
	// HOTLINE_PI_MODEL when set. The claude TUI and opencode pick their own
	// models, so those stay empty. Resolved here, next to the provider
	// construction, so config reads stay out of internal/app; the claude-sdk
	// harness refines the RESOLVED model live via harness_info.
	agentSeed := app.AgentInfo{Harness: harnessMode}
	switch harnessMode {
	case "claude-sdk":
		sdkCfg, err := config.LoadSDKForBox(boxRoot)
		if err != nil {
			return err
		}
		agentSeed.Model, agentSeed.Effort = sdkCfg.Model, sdkCfg.Effort
	case "pi":
		knob, err := config.LoadPiModelForBox(boxRoot)
		if err != nil {
			return err
		}
		// Both knobs seed the identity (pi hot-apply amendment 2026-07-20):
		// the thinking level is pi's effort equivalent, and the app's AGENT
		// section needs it before the extension's first harness_info lands.
		// The extension refines both to the LIVE, canonical values.
		agentSeed.Model, agentSeed.Effort = knob.Model, knob.Thinking
	}

	providers := make([]provider.Provider, 0, len(specs))
	var pidFiles []string
	// The fleet (A2A) channel rides alongside the app provider, sharing its Server
	// (a2a-design-v2 §3.3/§4). Captured here so its tools can be mounted after the
	// router is built. Nil on a box with no app provider / no fleet store.
	var fleetProvider *app.FleetProvider
	// The primary (first) provider's access.json is the last-resort chat_id
	// fallback for notify (its first AllowFrom entry — the paired user).
	var primaryAccessFile string
	for _, spec := range specs {
		switch spec.Kind {
		case "telegram":
			cfg, err := config.Load(spec.Instance)
			if err != nil {
				return err
			}
			if err := cfg.EnsureDirs(); err != nil {
				return err
			}
			if primaryAccessFile == "" {
				primaryAccessFile = cfg.AccessFile
			}
			fmt.Fprintf(os.Stderr, "hotline: provider=%s bot=%s state=%s\n", spec.Name(), botLabel(cfg.BotName), cfg.StateDir)

			// Durable conversation log, shared per-token in the state dir. Both
			// inbound (handler) and outbound (tools) write to it so the assistant
			// can recall the thread across restarts and context compaction.
			log := transcript.New(cfg.TranscriptFile)

			p, err := telegram.NewProvider(spec.Name(), cfg, log)
			if err != nil {
				return err
			}
			providers = append(providers, p)
			if cfg.Token != "" {
				pidFiles = append(pidFiles, cfg.PidFile)
			}
		case "discord":
			cfg, err := config.LoadDiscord(spec.Instance)
			if err != nil {
				return err
			}
			if err := cfg.EnsureDirs(); err != nil {
				return err
			}
			if primaryAccessFile == "" {
				primaryAccessFile = cfg.AccessFile
			}
			fmt.Fprintf(os.Stderr, "hotline: provider=%s state=%s\n", spec.Name(), cfg.StateDir)

			log := transcript.New(cfg.TranscriptFile)
			p, err := discord.NewProvider(spec.Name(), cfg, log)
			if err != nil {
				return err
			}
			providers = append(providers, p)
			if cfg.Token != "" {
				pidFiles = append(pidFiles, cfg.PidFile)
			}
		case "signal":
			cfg, err := config.LoadSignal(spec.Instance)
			if err != nil {
				return err
			}
			if err := cfg.EnsureDirs(); err != nil {
				return err
			}
			if primaryAccessFile == "" {
				primaryAccessFile = cfg.AccessFile
			}
			fmt.Fprintf(os.Stderr, "hotline: provider=%s state=%s\n", spec.Name(), cfg.StateDir)

			log := transcript.New(cfg.TranscriptFile)
			p, err := signal.NewProvider(spec.Name(), cfg, log)
			if err != nil {
				return err
			}
			providers = append(providers, p)
			if cfg.SignalAccount != "" {
				pidFiles = append(pidFiles, cfg.PidFile)
			}
		case "app":
			cfg, err := config.LoadApp(spec.Instance)
			if err != nil {
				return err
			}
			// App agent-state snapshots read schedules and loops through this
			// field; keep them on the same resolved box root as the harness.
			cfg.StateRoot = boxRoot
			if err := cfg.EnsureDirs(); err != nil {
				return err
			}
			if primaryAccessFile == "" {
				primaryAccessFile = cfg.AccessFile
			}
			fmt.Fprintf(os.Stderr, "hotline: provider=%s state=%s transport=relay-v2\n", spec.Name(), cfg.StateDir)

			// FB21: seed the box-owned assistant identity once, before the provider
			// starts. Covers every box boot (an existing state dir with rooms but no
			// identity seeds lazily here); a rolled name prints its flair to stderr.
			if istore, ierr := app.OpenRelayStore(cfg.StateDir); ierr != nil {
				return ierr
			} else if _, ierr := ensureIdentity(istore, cfg.BotName, os.Stderr); ierr != nil {
				return ierr
			}

			log := transcript.New(cfg.TranscriptFile)
			// caps-design §1: stamp the box binary identity into the caps manifest's
			// bin{} — resolved here in package main and passed in, keeping the version/
			// VCS read out of internal/app.
			bv, bc, bd := versionInfo()
			p, err := app.NewProvider(spec.Name(), cfg, log, agentSeed, boxRoot, app.AppVersion{Version: bv, Commit: bc, Date: bd})
			if err != nil {
				return err
			}
			providers = append(providers, p)
			// Register the fleet channel as its own (hidden) source: inbound
			// fleet_msg is injected with source="fleet", and the fleet + fleet_send
			// tools are mounted below. Shares p's Server so the fleet room manager,
			// registry, and box identity are one.
			if fp, ok := app.NewFleetProvider(p); ok {
				fleetProvider = fp
				providers = append(providers, fp)
			}
		default:
			return fmt.Errorf("unknown provider %q (supported: telegram, discord, signal, app)", spec.Kind)
		}
	}

	router, err := provider.NewRouter(providers...)
	if err != nil {
		return err
	}

	// The permission capability is only declared when some provider can
	// authenticate the replier (for Telegram: the access gate is active, which
	// requires a running bot).
	permission := router.PermissionRelay()

	// The transcript path baked into the channel instructions is the primary
	// (first) provider's — with one provider configured this is exactly the old
	// behavior.
	transcriptPath := ""
	if tp, ok := providers[0].(interface{ TranscriptFile() string }); ok {
		transcriptPath = tp.TranscriptFile()
	}

	// Voice override: ./HOTLINE.md in the repo, else HOTLINE.md at the shared
	// base root. Read once here — instructions ship at the MCP handshake, so a
	// changed file takes effect on the next restart.
	baseRoot, err := config.StateRoot()
	if err != nil {
		return err
	}
	voice := mcpchan.LoadVoice(baseRoot)

	// Proactive scheduling is box-owned: whichever provider delivers the reply,
	// this harness session reads only its box's schedules.json. Fires ride the
	// primary provider's transcript.
	schedulesPath := filepath.Join(boxRoot, "schedules.json")
	var schedLog *transcript.Logger
	if transcriptPath != "" {
		schedLog = transcript.New(transcriptPath)
	}
	sched := schedule.NewScheduler(schedulesPath, router.Sources(), schedLog)

	// Event-driven notifies live under <box root>/notify/. The CLI gate
	// enqueues; this dispatcher injects, riding the primary provider's transcript
	// and, as a last resort, its access.json for chat_id.
	notifyDisp := notify.NewDispatcher(
		notify.SpoolPath(boxRoot), notify.SourcesPath(boxRoot),
		primaryAccessFile, router.Sources(), schedLog)

	// FB13 automatic job cards: `hotline job` enqueues intents under
	// <box root>/jobs/; this dispatcher drains them and drives the app channel's
	// job cards (batch rollup, restart-orphan sweep). It runs only when the app
	// provider is configured — that provider owns the job registry it drives.
	jobDisp := jobspool.NewDispatcher(jobspool.SpoolPath(boxRoot), jobspool.ActivePath(boxRoot))
	// FB84 auto card check-in: while a card stays running, the dispatcher
	// re-injects a "still working?" nudge through the harness inbound path every
	// interval, so closing a card never depends on the agent remembering. The
	// static half (sources for reply routing + transcript) is wired here; the
	// inbound sink is bound in runPollers where it lives.
	//
	// CardSources, not Sources: a check-in points at a job card, so it may only
	// route to a channel that can render one (the app). Sources() would hand it
	// the first configured provider — telegram on a telegram+app box — and the
	// operator would get buzzed about a card he cannot see. No card-capable
	// channel attached → the dispatcher stays silent.
	jobDisp.ConfigureNudge(router.CardSources(), schedLog, jobNudgeInterval())
	var jobSink jobspool.JobSink
	// harness_info → live identity refinement (§5.3): same discovery pattern
	// as jobSink. Nil on telegram-only boxes — the notification is then
	// logged and dropped by the run-harness wiring.
	var agentInfoSink func(mcpchan.AgentInfoParams)
	// appProvider rides along for the claude-sdk hot-apply binding (SDK
	// hot-model amendment 2026-07-19): runInjectedHarness needs both
	// directions (forwarder out, result sink in) wired against the transport
	// it builds, so it takes the provider itself. Nil on telegram-only boxes.
	var appProvider *app.Provider
	for _, p := range providers {
		if ap, ok := p.(*app.Provider); ok {
			appProvider = ap
			jobSink = ap.JobDriver()
			agentInfoSink = ap.AgentInfoSink()
			break
		}
	}

	// Which exposure backend the publish tool uses (localhost.run default,
	// cloudflared, or local/off). Resolved once here so a bad value fails loudly
	// at startup, like the harness selection above/below.
	publishExposure, err := config.PublishExposure()
	if err != nil {
		return err
	}

	// On force-exit (the 2s shutdown safety net skips deferred cleanup), remove
	// poller PID advisories and tear down published artifacts. Keep the box lease
	// held until os.Exit: the kernel releases its flocks atomically with process
	// death, leaving no window where a replacement can overlap this runtime.
	cleanup := func() {
		for _, pf := range pidFiles {
			lifecycle.ReleasePollerSlot(pf)
		}
		mcpchan.CloseAllPublished()
	}

	// Mission Control (spec §6): use the configuration resolved before the
	// ownership claim, then seed and thread the mission tool + index injection
	// into whichever server this harness builds.
	mcOpts, err := missionControlOptions(harnessMode, mcCfg)
	if err != nil {
		return err
	}
	// Mount the fleet tools (fleet + fleet_send) whenever the fleet channel is
	// present, so every harness path (default, pi, claude-sdk) exposes them.
	if fleetProvider != nil {
		mcOpts = append(mcOpts, mcpchan.WithFleetTools(fleetProvider))
	}

	if harnessMode == "opencode" {
		return runOpenCodeHarness(router, sched, notifyDisp, loopRunner, permission, transcriptPath, voice, publishExposure, cleanup, mcOpts...)
	}
	if harnessMode == "pi" {
		// Pi rides the same stdio claude/channel transport as Claude Code, but
		// with the permission relay off and the inbound envelope pre-rendered
		// (see run_pi.go). The permission the router computed above is ignored —
		// Pi has no permission prompts by design.
		return runPiHarness(router, sched, notifyDisp, jobDisp, jobSink, agentInfoSink, appProvider, loopRunner, transcriptPath, voice, publishExposure, schedulesPath, cleanup, mcOpts...)
	}
	if harnessMode == "claude-sdk" {
		// The claude-sdk node harness owns this child the way the hotline-pi
		// extension does: same injected-harness seam (permission relay off,
		// pre-rendered inbound envelope, uncapped instructions), its own label.
		return runClaudeSDKHarness(router, sched, notifyDisp, jobDisp, jobSink, agentInfoSink, appProvider, loopRunner, transcriptPath, voice, publishExposure, schedulesPath, cleanup, mcOpts...)
	}

	// Under `hotline up` the supervisor exports HOTLINE_SUPERVISOR_DIR into the
	// harness environment (we are its grandchild), which enables the restart
	// tool; an unsupervised session never sees it.
	supervisorDir := os.Getenv("HOTLINE_SUPERVISOR_DIR")

	server := mcpchan.NewServer(router, permission, transcriptPath, router.Sources(), voice, publishExposure, schedulesPath, supervisorDir, mcOpts...)

	// Claude Code: inbound + permission relay travel over the same stdio MCP
	// connection as the tools, via the custom claude/channel notifications.
	var onPerm mcpchan.PermissionHandler
	if permission {
		onPerm = router.OnPermissionRequest
	}
	transport := mcpchan.NewChannelTransport(onPerm)

	// The poll fn starts every provider on the source-tagging router sink and the
	// schedule ticker; the notifier is valid only after Connect, which
	// lifecycle.Run performs first.
	pollFn := func(ctx context.Context) error {
		var sink provider.InboundSink = transport.Notifier()
		if len(router.Sources()) > 1 {
			sink = &sourceLabelSink{next: sink}
		}
		replayCatchup(ctx, sink, transcriptPath)
		return runWithLoopRunner(ctx, loopRunner, func(ctx context.Context) error {
			return runPollers(ctx, router, sched, notifyDisp, jobDisp, jobSink, sink)
		})
	}

	return lifecycle.Run(server, transport, cleanup, pollFn)
}

// loopService is the lifetime contract for the box-owned loop ticker.
// Production always passes *loop.Runner; the narrow seam keeps lifecycle
// behavior directly testable without executing shell commands.
type loopService interface {
	Run(context.Context) error
}

// runWithLoopRunner ties the required ticker to the MCP server's poll context.
// Whichever side exits first cancels the other. A runner that exits while the
// server is still live is a fatal service failure rather than a silently
// ticker-less box.
func runWithLoopRunner(ctx context.Context, runner loopService, serve func(context.Context) error) error {
	ctx, cancel := context.WithCancel(ctx)
	defer cancel()

	type result struct {
		service string
		err     error
	}
	done := make(chan result, 2)
	go func() { done <- result{service: "loop runner", err: runner.Run(ctx)} }()
	go func() { done <- result{service: "channel services", err: serve(ctx)} }()

	res := <-done
	if res.service == "channel services" {
		cancel()
		// Do not let runChannel return (and release its box lease) until the
		// runner has stopped every in-flight tick.
		runnerRes := <-done
		if res.err == nil && runnerRes.err != nil {
			return fmt.Errorf("loop runner exited: %w", runnerRes.err)
		}
		return res.err
	}

	serverStopping := ctx.Err() != nil
	cancel()
	if res.err != nil {
		return fmt.Errorf("loop runner exited: %w", res.err)
	}
	if !serverStopping {
		return errors.New("loop runner exited while the MCP server was still running")
	}
	return nil
}

// boxOwnerSpec builds the exact resource set shared by `run` and foreground
// `up`: the box-global root, every configured provider state directory, and
// Mission Control when enabled. Provider paths are resolved without loading
// credentials, creating provider directories, or seeding state.
func boxOwnerSpec(box config.Box, harnessMode, workDir string, mcCfg config.MCConfig) (lifecycle.OwnerSpec, error) {
	resources := make([]string, 0, len(box.Providers)+1)
	providers := make([]string, 0, len(box.Providers))
	for _, spec := range box.Providers {
		stateDir, err := config.ProviderStateDir(spec)
		if err != nil {
			return lifecycle.OwnerSpec{}, err
		}
		resources = append(resources, stateDir)
		providers = append(providers, spec.Name())
	}
	if mcCfg.Enabled {
		resources = append(resources, mcCfg.Dir)
	}
	return lifecycle.OwnerSpec{
		BoxRoot:      box.Root,
		BoxKey:       box.Key,
		Harness:      harnessMode,
		WorkDir:      workDir,
		Providers:    providers,
		ResourceDirs: resources,
	}, nil
}

// missionControlOptions seeds the already-resolved Mission Control directory
// and returns the server option that threads in the mission tool + index
// injection. When MC is off, it returns nil options and the servers are
// byte-identical to the pre-MC build.
func missionControlOptions(harnessMode string, mcCfg config.MCConfig) ([]mcpchan.ServerOption, error) {
	if !mcCfg.Enabled {
		return nil, nil
	}
	store := mc.NewStore(mcCfg.Dir)
	if err := store.Seed(); err != nil {
		return nil, fmt.Errorf("seeding mission control at %s: %w", mcCfg.Dir, err)
	}
	h := mcpchan.HarnessClaude
	switch harnessMode {
	case "opencode":
		h = mcpchan.HarnessOpenCode
	case "pi":
		h = mcpchan.HarnessPi
	}
	return []mcpchan.ServerOption{mcpchan.WithMissionControl(store, mcCfg.IndexBudget, h)}, nil
}

// sourceLabelSink decorates the display name with the provider on the Claude
// Code path when several providers are configured: Claude Code's inbound
// quick view renders only "<server> · <user>", dropping meta.source, so
// "George" from telegram and "George" from signal are indistinguishable at a
// glance. Decorating user ("George · signal") is display-only — pairing and
// access decisions key on user_id, and the OpenCode path renders its own
// <channel> envelope (source attribute included) without this wrapper.
type sourceLabelSink struct {
	next provider.InboundSink
}

func (s *sourceLabelSink) SendChannel(ctx context.Context, content string, meta map[string]string) error {
	if meta["user"] != "" && meta["source"] != "" {
		decorated := make(map[string]string, len(meta))
		for k, v := range meta {
			decorated[k] = v
		}
		decorated["user"] = decorated["user"] + " · " + decorated["source"]
		meta = decorated
	}
	return s.next.SendChannel(ctx, content, meta)
}

func (s *sourceLabelSink) SendVerdict(ctx context.Context, requestID, behavior string) error {
	return s.next.SendVerdict(ctx, requestID, behavior)
}

// replayCatchup redelivers any inbound turns that were journaled but never
// answered before this harness (re)started (see internal/catchup). It runs once
// per harness start, after lifecycle.Run has connected (the sink is live) and
// before the providers begin polling, so a turn lost to a deaf/restarted harness
// is caught up in order ahead of new traffic. Best-effort: an error is logged,
// never fatal — a missing transcript or a delivery hiccup must not stop the box.
func replayCatchup(ctx context.Context, sink provider.InboundSink, transcriptPath string) {
	// A logger over the same transcript records the per-turn replay-attempt
	// markers that cap redelivery (catchup S-1); nil-safe if the path is empty.
	var log *transcript.Logger
	if transcriptPath != "" {
		log = transcript.New(transcriptPath)
	}
	n, err := catchup.ReplayUnanswered(ctx, sink, log, transcriptPath, time.Now(), catchup.DefaultWindow)
	if err != nil {
		fmt.Fprintf(os.Stderr, "hotline: catch-up replay error: %v\n", err)
	}
	if n > 0 {
		fmt.Fprintf(os.Stderr, "hotline: replayed %d unanswered inbound turn(s) after harness (re)start\n", n)
	}
}

// runPollers runs the provider fan-in, the schedule ticker, and the notify
// dispatcher concurrently, returning the first exit — the same first-error-wins
// fan-in runOpenCodeLoop uses. sched.Run and notifyDisp.Run only exit on ctx
// cancellation (nil), so in practice the providers' exit decides shutdown,
// exactly as before. Both get the bare Notifier sink (not a sourceLabelSink):
// they stamp their own source and don't set user.
func runPollers(ctx context.Context, router *provider.Router, sched *schedule.Scheduler, notifyDisp *notify.Dispatcher, jobDisp *jobspool.Dispatcher, jobSink jobspool.JobSink, sink provider.InboundSink) error {
	ctx, cancel := context.WithCancel(ctx)
	defer cancel()

	errCh := make(chan error, 4)
	go func() { errCh <- router.Start(ctx, sink) }()
	go func() { errCh <- sched.Run(ctx, sink) }()
	go func() { errCh <- notifyDisp.Run(ctx, sink) }()
	// The job dispatcher runs only with an app provider to drive; without one it
	// idles so the fan-in shape (and shutdown semantics) stays uniform.
	go func() {
		if jobSink == nil {
			<-ctx.Done()
			errCh <- nil
			return
		}
		// FB84: bind the inbound sink now that the poller owns it, so the
		// check-in nudge injects through the same seam schedule/notify fires do.
		jobDisp.SetNudgeSink(sink)
		errCh <- jobDisp.Run(ctx, jobSink)
	}()

	err := <-errCh
	cancel()
	return err
}

// jobNudgeInterval resolves the FB84 check-in cadence from
// HOTLINE_JOB_NUDGE_INTERVAL: a Go duration ("20m", "1h") overrides the default,
// "0"/"off"/"disable" turns the nudge off, and an unset/invalid value keeps
// jobspool.DefaultNudgeInterval (logging the invalid case, never failing boot).
func jobNudgeInterval() time.Duration {
	raw := strings.TrimSpace(os.Getenv("HOTLINE_JOB_NUDGE_INTERVAL"))
	if raw == "" {
		return jobspool.DefaultNudgeInterval
	}
	switch strings.ToLower(raw) {
	case "0", "off", "disable", "disabled", "none":
		return 0
	}
	d, err := time.ParseDuration(raw)
	if err != nil || d < 0 {
		fmt.Fprintf(os.Stderr, "hotline: invalid HOTLINE_JOB_NUDGE_INTERVAL %q; using default\n", raw)
		return jobspool.DefaultNudgeInterval
	}
	return d
}

func cmdPair(providerSel, botName string, args []string) error {
	if len(args) < 1 {
		return errors.New("usage: hotline pair <code>")
	}
	code := args[0]
	cfg, err := loadOpsConfig(providerSel, botName)
	if err != nil {
		return err
	}
	p, err := access.ApprovePairing(cfg.AccessFile, code)
	if err != nil {
		return err
	}
	fmt.Printf("Paired sender %s.\n", p.SenderID)

	// Best-effort confirmation DM (telegram only: DM chat_id == sender_id).
	if strings.HasPrefix(providerSel, "discord") || strings.HasPrefix(providerSel, "signal") || strings.HasPrefix(providerSel, "app") {
		return nil
	}
	if cfg.Token != "" {
		if b, err := telegram.NewBot(cfg.Token); err == nil {
			if chatID, perr := strconv.ParseInt(p.ChatID, 10, 64); perr == nil {
				if _, serr := b.SendMessage(chatID, "Paired! Say hi to Claude.", nil); serr != nil {
					fmt.Fprintf(os.Stderr, "hotline: could not send confirmation: %v\n", serr)
				}
			}
		}
	}
	return nil
}

func cmdDeny(providerSel, botName string, args []string) error {
	if len(args) < 1 {
		return errors.New("usage: hotline deny <code>")
	}
	cfg, err := loadOpsConfig(providerSel, botName)
	if err != nil {
		return err
	}
	if err := access.DenyPairing(cfg.AccessFile, args[0]); err != nil {
		return err
	}
	fmt.Printf("Denied pairing %s.\n", args[0])
	return nil
}

func cmdRevoke(providerSel, botName string, args []string) error {
	if len(args) < 1 {
		return errors.New("usage: hotline revoke <sender-id>")
	}
	cfg, err := loadOpsConfig(providerSel, botName)
	if err != nil {
		return err
	}
	id, remaining, err := access.RevokeSender(cfg.AccessFile, args[0])
	if err != nil {
		return err
	}
	fmt.Printf("Revoked %s. %d sender(s) remain.\n", id, remaining)
	return nil
}

func cmdStatus(providerSel, botName, dir string, stdout, stderr io.Writer) error {
	// Read-only sibling of the `hotline down` guard: if this directory belongs to
	// a box the shell does not select, say so up front, so a status read from a
	// plugin-configured box's dir isn't mistaken for that box's status.
	if decl, _, mismatch, err := projectBoxMismatch(dir); err == nil && mismatch {
		fmt.Fprintf(stderr, "hotline: note: this directory belongs to box HOTLINE_STATE_DIR=%s (from %s), but your shell selects the default box; this status is for the default box. Re-run as `HOTLINE_STATE_DIR=%s hotline status` for this project's box.\n", decl.StateDir, decl.Source, decl.StateDir)
	}
	cfg, err := loadOpsConfig(providerSel, botName)
	if err != nil {
		return err
	}
	acc, err := access.Load(cfg.AccessFile)
	if err != nil {
		return err
	}
	fmt.Fprintf(stdout, "bot:         %s\n", botLabel(cfg.BotName))
	fmt.Fprintf(stdout, "state dir:   %s\n", cfg.StateDir)
	if strings.HasPrefix(providerSel, "signal") {
		fmt.Fprintf(stdout, "account:     %s\n", presence(cfg.SignalAccount != ""))
		fmt.Fprintf(stdout, "daemon url:  %s\n", cfg.SignalDaemonURL)
	} else if strings.HasPrefix(providerSel, "app") {
		fmt.Fprintln(stdout, "transport:   relay-v2 (outbound only)")
	} else {
		fmt.Fprintf(stdout, "token:       %s\n", presence(cfg.Token != ""))
	}
	fmt.Fprintf(stdout, "access mode: %s\n", modeLabel(cfg.Static))
	fmt.Fprintf(stdout, "dmPolicy:    %s\n", acc.DMPolicy)
	fmt.Fprintf(stdout, "allowFrom:   %d user(s)\n", len(acc.AllowFrom))
	for _, id := range acc.AllowFrom {
		fmt.Fprintf(stdout, "  - %s\n", id)
	}
	fmt.Fprintf(stdout, "groups:      %d configured\n", len(acc.Groups))
	for id, g := range acc.Groups {
		fmt.Fprintf(stdout, "  - %s (requireMention=%v, allowFrom=%d)\n", id, g.RequireMention, len(g.AllowFrom))
	}
	fmt.Fprintf(stdout, "pending:     %d pairing(s)\n", len(acc.Pending))
	for code, p := range acc.Pending {
		fmt.Fprintf(stdout, "  - %s -> sender %s (expires %s)\n", code, p.SenderID, p.ExpiresAt)
	}
	if root, err := config.BoxRoot(botName); err == nil {
		printSupervisorStatus(root, stdout)
	}
	return nil
}

func presence(ok bool) string {
	if ok {
		return "configured"
	}
	return "NOT configured"
}

func modeLabel(static bool) string {
	if static {
		return "static"
	}
	return "live"
}

func botLabel(name string) string {
	if name == "" {
		return "(default)"
	}
	return name
}
