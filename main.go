// Command tele-go is a Telegram channel for Claude Code: an MCP server that
// relays Telegram DMs/groups to a Claude Code session and back, with access
// control (pairing/allowlist/groups), media handling, and a permission relay.
//
// Subcommands:
//
//	tele-go [run]        start the MCP server + Telegram poller (default)
//	tele-go pair <code>  approve a pending pairing code
//	tele-go deny <code>  reject a pending pairing code
//	tele-go status       print state-dir / token / access summary
package main

import (
	"context"
	"errors"
	"fmt"
	"os"
	"strconv"
	"strings"

	"example.com/tele-go/internal/access"
	"example.com/tele-go/internal/config"
	"example.com/tele-go/internal/lifecycle"
	"example.com/tele-go/internal/mcpchan"
	"example.com/tele-go/internal/telegram"
	"example.com/tele-go/internal/transcript"
)

func main() {
	// --bot <name> (or --bot=<name>) selects which bot to run/operate on; it may
	// appear anywhere and is stripped before subcommand parsing. Falls back to
	// $TELE_GO_BOT. "" is the default/unnamed bot.
	botName, args := resolveBotName(os.Args[1:])
	cmd := "run"
	if len(args) > 0 {
		cmd = args[0]
	}

	var err error
	switch cmd {
	case "run":
		err = runChannel(botName)
	case "pair":
		err = cmdPair(botName, args[1:])
	case "deny":
		err = cmdDeny(botName, args[1:])
	case "status":
		err = cmdStatus(botName)
	case "-h", "--help", "help":
		usage()
	default:
		// Unknown first arg: treat as default "run" only if it's not clearly a
		// subcommand typo. Be strict and show usage.
		fmt.Fprintf(os.Stderr, "tele-go: unknown command %q\n\n", cmd)
		usage()
		os.Exit(2)
	}

	if err != nil {
		fmt.Fprintf(os.Stderr, "tele-go: %v\n", err)
		os.Exit(1)
	}
}

func usage() {
	fmt.Fprint(os.Stderr, `tele-go — Telegram channel for Claude Code

Usage:
  tele-go [run]        start the MCP server + Telegram poller (default)
  tele-go pair <code>  approve a pending pairing code
  tele-go deny <code>  reject a pending pairing code
  tele-go status       print state-dir / token / access summary

Options:
  --bot <name>         select a named bot (isolated state under bots/<name>,
                       token from TELEGRAM_BOT_TOKEN_<NAME>). Omit for the
                       default bot. Also settable via $TELE_GO_BOT.
`)
}

// resolveBotName extracts "--bot <name>" / "--bot=<name>" from args (wherever it
// appears), returning the selected bot and the remaining args. When no flag is
// present it falls back to $TELE_GO_BOT, then "" (the default bot).
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
		botName = os.Getenv("TELE_GO_BOT")
	}
	return botName, rest
}

// runChannel is the main entry: it always runs the MCP handshake. If a token is
// configured it also starts the Telegram poller (after claiming the single
// poller slot) and declares the permission capability.
func runChannel(botName string) error {
	cfg, err := config.Load(botName)
	if err != nil {
		return err
	}
	if err := cfg.EnsureDirs(); err != nil {
		return err
	}
	fmt.Fprintf(os.Stderr, "tele-go: bot=%s state=%s\n", botLabel(cfg.BotName), cfg.StateDir)

	hasToken := cfg.Token != ""

	// Durable conversation log, shared per-token in the state dir. Both inbound
	// (handler) and outbound (tools) write to it so the assistant can recall the
	// thread across restarts and context compaction.
	log := transcript.New(cfg.TranscriptFile)

	// Token-less mode still serves the MCP handshake; the tools report
	// "no bot token configured" and no poller runs.
	tools := telegram.NewTools(nil, cfg, log)
	var handler *telegram.Handler
	var pollFn func(ctx context.Context) error

	if hasToken {
		b, err := telegram.NewBot(cfg.Token)
		if err != nil {
			return fmt.Errorf("initializing bot: %w", err)
		}
		if err := lifecycle.ClaimPollerSlot(cfg.PidFile); err != nil {
			return fmt.Errorf("claiming poller slot: %w", err)
		}
		defer lifecycle.ReleasePollerSlot(cfg.PidFile)

		tools = telegram.NewTools(b, cfg, log)
		handler = telegram.NewHandler(b, cfg, nil, log)

		pollFn = func(ctx context.Context) error {
			return telegram.Poll(ctx, b, handler.Dispatch)
		}
	}

	// The permission capability is only declared when we can authenticate the
	// replier (i.e. the access gate is active, which requires a running bot).
	permission := hasToken

	var onPerm mcpchan.PermissionHandler
	if handler != nil {
		onPerm = handler.OnPermissionRequest
	}
	transport := mcpchan.NewChannelTransport(onPerm)
	server := mcpchan.NewServer(tools, permission, cfg.TranscriptFile)

	if handler != nil {
		// Bind the notifier (valid after Connect) just before polling starts.
		base := pollFn
		pollFn = func(ctx context.Context) error {
			handler.Notifier = transport.Notifier()
			err := base(ctx)
			// Drain any burst still in the coalescing window before teardown.
			handler.FlushAll(context.Background())
			return err
		}
	}

	return lifecycle.Run(server, transport, cfg.PidFile, pollFn)
}

func cmdPair(botName string, args []string) error {
	if len(args) < 1 {
		return errors.New("usage: tele-go pair <code>")
	}
	code := args[0]
	cfg, err := config.Load(botName)
	if err != nil {
		return err
	}
	p, err := access.ApprovePairing(cfg.AccessFile, code)
	if err != nil {
		return err
	}
	fmt.Printf("Paired sender %s.\n", p.SenderID)

	// Best-effort confirmation DM (DM chat_id == sender_id).
	if cfg.Token != "" {
		if b, err := telegram.NewBot(cfg.Token); err == nil {
			if chatID, perr := strconv.ParseInt(p.ChatID, 10, 64); perr == nil {
				if _, serr := b.SendMessage(chatID, "Paired! Say hi to Claude.", nil); serr != nil {
					fmt.Fprintf(os.Stderr, "tele-go: could not send confirmation: %v\n", serr)
				}
			}
		}
	}
	return nil
}

func cmdDeny(botName string, args []string) error {
	if len(args) < 1 {
		return errors.New("usage: tele-go deny <code>")
	}
	cfg, err := config.Load(botName)
	if err != nil {
		return err
	}
	if err := access.DenyPairing(cfg.AccessFile, args[0]); err != nil {
		return err
	}
	fmt.Printf("Denied pairing %s.\n", args[0])
	return nil
}

func cmdStatus(botName string) error {
	cfg, err := config.Load(botName)
	if err != nil {
		return err
	}
	acc, err := access.Load(cfg.AccessFile)
	if err != nil {
		return err
	}
	fmt.Printf("bot:         %s\n", botLabel(cfg.BotName))
	fmt.Printf("state dir:   %s\n", cfg.StateDir)
	fmt.Printf("token:       %s\n", presence(cfg.Token != ""))
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
