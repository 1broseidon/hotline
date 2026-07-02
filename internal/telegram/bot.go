// Package telegram implements the Telegram long-polling transport: the bot
// wrapper, the resilient poll loop, update dispatch (access gate, inbound
// notification, permission relay), media extraction/download, and the outbound
// MCP tools (reply/react/edit_message/download_attachment).
package telegram

import (
	"os"
	"time"

	"github.com/PaulSonOfLars/gotgbot/v2"
)

// NewBot constructs a gotgbot Bot with the token-validity check ON (which
// populates Bot.User.Id and Bot.User.Username, needed for mention detection)
// and a 10s request timeout for the GetMe check.
//
// If HOTLINE_SKIP_TOKEN_CHECK=1 (legacy: TELE_GO_SKIP_TOKEN_CHECK=1) the
// validity check is disabled (no network
// call): Bot.User.Id is parsed from the token. This lets the offline
// integration smoke run without contacting Telegram.
func NewBot(token string) (*gotgbot.Bot, error) {
	opts := &gotgbot.BotOpts{
		RequestOpts: &gotgbot.RequestOpts{
			Timeout: 10 * time.Second,
		},
	}
	if os.Getenv("HOTLINE_SKIP_TOKEN_CHECK") == "1" || os.Getenv("TELE_GO_SKIP_TOKEN_CHECK") == "1" {
		opts.DisableTokenCheck = true
	}
	return gotgbot.NewBot(token, opts)
}

// SetCommands registers /start and /help scoped to all private chats. It is
// best-effort: errors are returned for the caller to log, not fatal.
func SetCommands(b *gotgbot.Bot) error {
	_, err := b.SetMyCommands(
		[]gotgbot.BotCommand{
			{Command: "start", Description: "Welcome and setup guide"},
			{Command: "help", Description: "What this bot can do"},
		},
		&gotgbot.SetMyCommandsOpts{
			Scope: gotgbot.BotCommandScopeAllPrivateChats{},
		},
	)
	return err
}
