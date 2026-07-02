// Command hotline is a messaging channel for Claude Code (Telegram, Discord, Signal): an MCP server that
// relays Telegram DMs/groups to a Claude Code session and back, with access
// control (pairing/allowlist/groups), media handling, and a permission relay.
//
// Subcommands:
//
//	hotline [run]        start the MCP server + Telegram poller (default)
//	hotline pair <code>  approve a pending pairing code
//	hotline deny <code>  reject a pending pairing code
//	hotline revoke <id>  remove an approved sender from the allowlist
//	hotline status       print state-dir / token / access summary
package main

import (
	"context"
	"errors"
	"fmt"
	"os"
	"strconv"
	"strings"

	"github.com/1broseidon/hotline/internal/access"
	"github.com/1broseidon/hotline/internal/config"
	"github.com/1broseidon/hotline/internal/discord"
	"github.com/1broseidon/hotline/internal/lifecycle"
	"github.com/1broseidon/hotline/internal/mcpchan"
	"github.com/1broseidon/hotline/internal/provider"
	"github.com/1broseidon/hotline/internal/signal"
	"github.com/1broseidon/hotline/internal/telegram"
	"github.com/1broseidon/hotline/internal/transcript"
)

func main() {
	// --bot <name> (or --bot=<name>) selects which bot to run/operate on; it may
	// appear anywhere and is stripped before subcommand parsing. Falls back to
	// $HOTLINE_BOT (legacy: $TELE_GO_BOT). "" is the default/unnamed bot.
	botName, args := resolveBotName(os.Args[1:])
	// --provider <kind[:instance]> selects which provider's state pair / deny /
	// revoke / status operate on (default: telegram). "run" ignores it — the run set
	// comes from HOTLINE_PROVIDERS.
	providerSel, args := resolveProviderFlag(args)
	cmd := "run"
	if len(args) > 0 {
		cmd = args[0]
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
		err = cmdStatus(providerSel, botName)
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
		fmt.Fprintf(os.Stderr, "hotline: %v\n", err)
		os.Exit(1)
	}
}

func usage() {
	fmt.Fprint(os.Stderr, `hotline - messaging channel for Claude Code

Usage:
  hotline [run]        start the MCP server + Telegram poller (default)
  hotline pair <code>  approve a pending pairing code
  hotline deny <code>  reject a pending pairing code
  hotline revoke <id>  remove an approved sender from the allowlist
                       (exact sender ID as shown by status, or a unique prefix)
  hotline status       print state-dir / token / access summary

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

// resolveBotName extracts "--bot <name>" / "--bot=<name>" from args (wherever it
// appears), returning the selected bot and the remaining args. When no flag is
// present it falls back to $HOTLINE_BOT, then $TELE_GO_BOT (legacy, kept for
// one release), then "" (the default bot).
func resolveBotName(args []string) (botName string, rest []string) {
	rest = make([]string, 0, len(args))
	found := false
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
	case "", "telegram":
		if instance == "" {
			instance = botName
		}
		return config.Load(instance)
	case "discord":
		return config.LoadDiscord(instance)
	case "signal":
		return config.LoadSignal(instance)
	default:
		return nil, fmt.Errorf("unknown provider %q (supported: telegram, discord, signal)", kind)
	}
}

// runChannel is the main entry: it always runs the MCP handshake, then starts
// every configured provider (HOTLINE_PROVIDERS, default just "telegram") on
// the shared channel stream. Providers with a token poll their transport; the
// permission capability is declared when at least one provider can
// authenticate the replier.
func runChannel(botName string) error {
	specs, err := config.Providers(botName)
	if err != nil {
		return err
	}

	providers := make([]provider.Provider, 0, len(specs))
	var pidFiles []string
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
		default:
			return fmt.Errorf("unknown provider %q (supported: telegram, discord, signal)", spec.Kind)
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
	var onPerm mcpchan.PermissionHandler
	if permission {
		onPerm = router.OnPermissionRequest
	}

	// The transcript path baked into the channel instructions is the primary
	// (first) provider's — with one provider configured this is exactly the old
	// behavior.
	transcriptPath := ""
	if tp, ok := providers[0].(interface{ TranscriptFile() string }); ok {
		transcriptPath = tp.TranscriptFile()
	}

	// Voice override: ./HOTLINE.md in the repo, else HOTLINE.md at the state
	// root. Read once here — instructions ship at the MCP handshake, so a
	// changed file takes effect on the next restart.
	stateRoot, _ := config.StateRoot()
	voice := mcpchan.LoadVoice(stateRoot)

	transport := mcpchan.NewChannelTransport(onPerm)
	server := mcpchan.NewServer(router, permission, transcriptPath, router.Sources(), voice)

	// The poll fn starts every provider on the source-tagging router sink; the
	// notifier is valid only after Connect, which lifecycle.Run performs first.
	pollFn := func(ctx context.Context) error {
		return router.Start(ctx, transport.Notifier())
	}

	// On force-exit (the 2s shutdown safety net skips deferred cleanup) release
	// every claimed poller slot so no stale PID files survive.
	cleanup := func() {
		for _, pf := range pidFiles {
			lifecycle.ReleasePollerSlot(pf)
		}
	}

	return lifecycle.Run(server, transport, cleanup, pollFn)
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
	if strings.HasPrefix(providerSel, "discord") || strings.HasPrefix(providerSel, "signal") {
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

func cmdStatus(providerSel, botName string) error {
	cfg, err := loadOpsConfig(providerSel, botName)
	if err != nil {
		return err
	}
	acc, err := access.Load(cfg.AccessFile)
	if err != nil {
		return err
	}
	fmt.Printf("bot:         %s\n", botLabel(cfg.BotName))
	fmt.Printf("state dir:   %s\n", cfg.StateDir)
	if strings.HasPrefix(providerSel, "signal") {
		fmt.Printf("account:     %s\n", presence(cfg.SignalAccount != ""))
		fmt.Printf("daemon url:  %s\n", cfg.SignalDaemonURL)
	} else {
		fmt.Printf("token:       %s\n", presence(cfg.Token != ""))
	}
	fmt.Printf("access mode: %s\n", modeLabel(cfg.Static))
	fmt.Printf("dmPolicy:    %s\n", acc.DMPolicy)
	fmt.Printf("allowFrom:   %d user(s)\n", len(acc.AllowFrom))
	for _, id := range acc.AllowFrom {
		fmt.Printf("  - %s\n", id)
	}
	fmt.Printf("groups:      %d configured\n", len(acc.Groups))
	for id, g := range acc.Groups {
		fmt.Printf("  - %s (requireMention=%v, allowFrom=%d)\n", id, g.RequireMention, len(g.AllowFrom))
	}
	fmt.Printf("pending:     %d pairing(s)\n", len(acc.Pending))
	for code, p := range acc.Pending {
		fmt.Printf("  - %s -> sender %s (expires %s)\n", code, p.SenderID, p.ExpiresAt)
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
