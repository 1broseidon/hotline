// Package discord adapts Discord (via the bwmarrin/discordgo gateway + REST
// client) to the provider.Provider interface: inbound MessageCreate /
// InteractionCreate events are normalized into the claude/channel shape, and
// the outbound tool set (reply/react/edit_message/download_attachment) is
// implemented against the Discord REST API. Discord supports buttons,
// reactions, edits, and typing natively, so no degradation is needed; the
// 2000-character message cap is handled by chunking.
package discord

import (
	"github.com/bwmarrin/discordgo"
)

// Session is the slice of the discordgo REST surface the adapter uses. The
// concrete *discordgo.Session methods are variadic (trailing RequestOption),
// so realSession wraps them; tests substitute an in-memory fake.
type Session interface {
	ChannelMessageSendComplex(channelID string, data *discordgo.MessageSend) (*discordgo.Message, error)
	ChannelTyping(channelID string) error
	ChannelMessageEditComplex(m *discordgo.MessageEdit) (*discordgo.Message, error)
	MessageReactionAdd(channelID, messageID, emojiID string) error
	InteractionRespond(i *discordgo.Interaction, resp *discordgo.InteractionResponse) error
	UserChannelCreate(recipientID string) (*discordgo.Channel, error)
}

// realSession adapts *discordgo.Session to the Session interface.
type realSession struct {
	s *discordgo.Session
}

func (r *realSession) ChannelMessageSendComplex(channelID string, data *discordgo.MessageSend) (*discordgo.Message, error) {
	return r.s.ChannelMessageSendComplex(channelID, data)
}

func (r *realSession) ChannelTyping(channelID string) error {
	return r.s.ChannelTyping(channelID)
}

func (r *realSession) ChannelMessageEditComplex(m *discordgo.MessageEdit) (*discordgo.Message, error) {
	return r.s.ChannelMessageEditComplex(m)
}

func (r *realSession) MessageReactionAdd(channelID, messageID, emojiID string) error {
	return r.s.MessageReactionAdd(channelID, messageID, emojiID)
}

func (r *realSession) InteractionRespond(i *discordgo.Interaction, resp *discordgo.InteractionResponse) error {
	return r.s.InteractionRespond(i, resp)
}

func (r *realSession) UserChannelCreate(recipientID string) (*discordgo.Channel, error) {
	return r.s.UserChannelCreate(recipientID)
}
