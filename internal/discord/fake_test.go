package discord

import (
	"context"
	"fmt"
	"path/filepath"
	"sync"
	"testing"

	"github.com/bwmarrin/discordgo"

	"github.com/1broseidon/hotline/internal/access"
	"github.com/1broseidon/hotline/internal/config"
	"github.com/1broseidon/hotline/internal/transcript"
)

// sentMsg records one ChannelMessageSendComplex call.
type sentMsg struct {
	ChannelID string
	Data      *discordgo.MessageSend
}

// fakeSession is an in-memory Session for offline tests.
type fakeSession struct {
	mu        sync.Mutex
	Sent      []sentMsg
	Typing    []string
	Reactions [][3]string // channelID, messageID, emoji
	Edits     []*discordgo.MessageEdit
	Responses []*discordgo.InteractionResponse
	DMOpened  []string

	SendErr error
	nextID  int
}

func (f *fakeSession) ChannelMessageSendComplex(channelID string, data *discordgo.MessageSend) (*discordgo.Message, error) {
	f.mu.Lock()
	defer f.mu.Unlock()
	if f.SendErr != nil {
		return nil, f.SendErr
	}
	f.nextID++
	f.Sent = append(f.Sent, sentMsg{ChannelID: channelID, Data: data})
	return &discordgo.Message{ID: fmt.Sprintf("m%d", f.nextID), ChannelID: channelID}, nil
}

func (f *fakeSession) ChannelTyping(channelID string) error {
	f.mu.Lock()
	defer f.mu.Unlock()
	f.Typing = append(f.Typing, channelID)
	return nil
}

func (f *fakeSession) ChannelMessageEditComplex(m *discordgo.MessageEdit) (*discordgo.Message, error) {
	f.mu.Lock()
	defer f.mu.Unlock()
	f.Edits = append(f.Edits, m)
	return &discordgo.Message{ID: m.ID, ChannelID: m.Channel}, nil
}

func (f *fakeSession) MessageReactionAdd(channelID, messageID, emojiID string) error {
	f.mu.Lock()
	defer f.mu.Unlock()
	f.Reactions = append(f.Reactions, [3]string{channelID, messageID, emojiID})
	return nil
}

func (f *fakeSession) InteractionRespond(i *discordgo.Interaction, resp *discordgo.InteractionResponse) error {
	f.mu.Lock()
	defer f.mu.Unlock()
	f.Responses = append(f.Responses, resp)
	return nil
}

func (f *fakeSession) UserChannelCreate(recipientID string) (*discordgo.Channel, error) {
	f.mu.Lock()
	defer f.mu.Unlock()
	f.DMOpened = append(f.DMOpened, recipientID)
	return &discordgo.Channel{ID: "dm-" + recipientID}, nil
}

// captureSink records inbound deliveries and verdicts.
type captureSink struct {
	mu       sync.Mutex
	Contents []string
	Metas    []map[string]string
	Verdicts [][2]string
}

func (c *captureSink) SendChannel(_ context.Context, content string, meta map[string]string) error {
	c.mu.Lock()
	defer c.mu.Unlock()
	c.Contents = append(c.Contents, content)
	c.Metas = append(c.Metas, meta)
	return nil
}

func (c *captureSink) SendVerdict(_ context.Context, requestID, behavior string) error {
	c.mu.Lock()
	defer c.mu.Unlock()
	c.Verdicts = append(c.Verdicts, [2]string{requestID, behavior})
	return nil
}

// testEnv builds a Handler + Tools over a fake session with isolated state.
func testEnv(t *testing.T, mutate func(*access.Access)) (*Handler, *Tools, *fakeSession, *captureSink) {
	t.Helper()
	dir := t.TempDir()
	cfg := &config.Config{
		StateDir:       dir,
		AccessFile:     filepath.Join(dir, "access.json"),
		InboxDir:       filepath.Join(dir, "inbox"),
		PidFile:        filepath.Join(dir, "bot.pid"),
		TranscriptFile: filepath.Join(dir, "transcript.jsonl"),
	}
	acc := access.Defaults()
	if mutate != nil {
		mutate(acc)
	}
	if err := access.Save(acc, cfg.AccessFile); err != nil {
		t.Fatal(err)
	}
	fs := &fakeSession{}
	log := transcript.New(cfg.TranscriptFile)
	h := NewHandler(fs, cfg, log)
	h.BotUserID = "bot1"
	sink := &captureSink{}
	h.BindNotifier(sink)
	tools := NewTools(fs, cfg, log)
	return h, tools, fs, sink
}
