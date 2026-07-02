package signal

import (
	"strconv"
	"strings"
	"sync"
	"time"
)

// Envelope JSON shapes, matching signal-cli's org.asamk.signal.json records
// (the payload of the daemon's `event:receive` SSE events and jsonRpc receive
// notifications). Only the fields hotline consumes are declared.

// receivePayload is one SSE data payload: the envelope plus which local
// account received it.
type receivePayload struct {
	Envelope envelope `json:"envelope"`
	Account  string   `json:"account"`
}

type envelope struct {
	Source       string       `json:"source"`
	SourceNumber string       `json:"sourceNumber"`
	SourceUUID   string       `json:"sourceUuid"`
	SourceName   string       `json:"sourceName"`
	Timestamp    int64        `json:"timestamp"`
	DataMessage  *dataMessage `json:"dataMessage"`
}

type dataMessage struct {
	Timestamp   int64        `json:"timestamp"`
	Message     string       `json:"message"`
	GroupInfo   *groupInfo   `json:"groupInfo"`
	Quote       *quote       `json:"quote"`
	Attachments []attachment `json:"attachments"`
}

type groupInfo struct {
	GroupID   string `json:"groupId"`
	GroupName string `json:"groupName"`
	Type      string `json:"type"`
}

type quote struct {
	ID           int64  `json:"id"` // the quoted message's timestamp
	AuthorNumber string `json:"authorNumber"`
	Text         string `json:"text"`
}

type attachment struct {
	ContentType string `json:"contentType"`
	Filename    string `json:"filename"`
	ID          string `json:"id"`
	Size        int64  `json:"size"`
}

// Message identity. Signal has no message ids: a message is identified by its
// send timestamp plus its author. Outbound message_ids are the bare timestamp
// (author is implicitly our account); inbound message_ids are
// "<timestamp>:<authorE164>" so react can address the right author later.

// inboundMessageID renders a received message's identity.
func inboundMessageID(ts int64, author string) string {
	return strconv.FormatInt(ts, 10) + ":" + author
}

// parseMessageID splits a message_id into (timestamp, author). A bare
// timestamp yields author == "".
func parseMessageID(id string) (int64, string, bool) {
	tsPart, author, _ := strings.Cut(id, ":")
	ts, err := strconv.ParseInt(tsPart, 10, 64)
	if err != nil || ts <= 0 {
		return 0, "", false
	}
	return ts, author, true
}

// sender returns the envelope's author, preferring the E.164 number (the id
// space our allowlist and chat_ids use) over the legacy source field.
func (e *envelope) sender() string {
	if e.SourceNumber != "" {
		return e.SourceNumber
	}
	return e.Source
}

// chatID derives the hotline chat_id for a data message: the sender's E.164
// for DMs, "group:"+groupId for groups.
func (e *envelope) chatID() string {
	if e.DataMessage != nil && e.DataMessage.GroupInfo != nil && e.DataMessage.GroupInfo.GroupID != "" {
		return groupChatPrefix + e.DataMessage.GroupInfo.GroupID
	}
	return e.sender()
}

// tsRFC3339 renders the envelope timestamp (unix millis) for inbound meta.
func tsRFC3339(ms int64) string {
	return time.UnixMilli(ms).UTC().Format("2006-01-02T15:04:05.000Z07:00")
}

// optionStore remembers, per chat, the numbered options rendered by the last
// buttons-degraded reply, so a bare-number answer can be mapped back to its
// label — the closest Signal gets to a button tap.
type optionStore struct {
	mu      sync.Mutex
	pending map[string][]string
}

func newOptionStore() *optionStore {
	return &optionStore{pending: make(map[string][]string)}
}

// set records the option labels awaiting an answer in chatID.
func (o *optionStore) set(chatID string, labels []string) {
	o.mu.Lock()
	defer o.mu.Unlock()
	if len(labels) == 0 {
		delete(o.pending, chatID)
		return
	}
	o.pending[chatID] = append([]string(nil), labels...)
}

// answer maps text to a pending option label for chatID: a bare number picks
// that option (1-based) and clears the pending set. Non-numeric text (or an
// out-of-range number) leaves the options pending and returns "".
func (o *optionStore) answer(chatID, text string) string {
	t := strings.TrimSpace(text)
	// Accept "3" and "3." the way people answer numbered lists.
	t = strings.TrimSuffix(t, ".")
	n, err := strconv.Atoi(t)
	if err != nil {
		return ""
	}
	o.mu.Lock()
	defer o.mu.Unlock()
	labels := o.pending[chatID]
	if n < 1 || n > len(labels) {
		return ""
	}
	delete(o.pending, chatID)
	return labels[n-1]
}
