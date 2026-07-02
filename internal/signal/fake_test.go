package signal

import (
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"path/filepath"
	"sync"
	"testing"

	"github.com/1broseidon/hotline/internal/access"
	"github.com/1broseidon/hotline/internal/config"
	"github.com/1broseidon/hotline/internal/transcript"
)

const testAccount = "+15550000001"

// rpcCall is one recorded JSON-RPC request.
type rpcCall struct {
	Method string
	Params map[string]any
}

// fakeDaemon is an httptest mock of the signal-cli HTTP daemon's /api/v1/rpc
// endpoint. It records every call and serves canned per-method results.
type fakeDaemon struct {
	mu      sync.Mutex
	Calls   []rpcCall
	Results map[string]any // method -> result value
	Fail    map[string]string
	sendTS  int64

	srv *httptest.Server
}

func newFakeDaemon(t *testing.T) *fakeDaemon {
	t.Helper()
	d := &fakeDaemon{
		Results: make(map[string]any),
		Fail:    make(map[string]string),
		sendTS:  1700000000000,
	}
	d.srv = httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path != "/api/v1/rpc" || r.Method != http.MethodPost {
			w.WriteHeader(http.StatusNotFound)
			return
		}
		var req struct {
			ID     any            `json:"id"`
			Method string         `json:"method"`
			Params map[string]any `json:"params"`
		}
		if err := json.NewDecoder(r.Body).Decode(&req); err != nil {
			w.WriteHeader(http.StatusBadRequest)
			return
		}
		d.mu.Lock()
		d.Calls = append(d.Calls, rpcCall{Method: req.Method, Params: req.Params})
		var resp map[string]any
		if msg, bad := d.Fail[req.Method]; bad {
			resp = map[string]any{"jsonrpc": "2.0", "id": req.ID,
				"error": map[string]any{"code": -32602, "message": msg}}
		} else {
			result, ok := d.Results[req.Method]
			if !ok && req.Method == "send" {
				d.sendTS++
				result = map[string]any{"timestamp": d.sendTS}
			}
			resp = map[string]any{"jsonrpc": "2.0", "id": req.ID, "result": result}
		}
		d.mu.Unlock()
		w.Header().Set("Content-Type", "application/json")
		_ = json.NewEncoder(w).Encode(resp)
	}))
	t.Cleanup(d.srv.Close)
	return d
}

// calls returns a snapshot of recorded calls.
func (d *fakeDaemon) calls() []rpcCall {
	d.mu.Lock()
	defer d.mu.Unlock()
	return append([]rpcCall(nil), d.Calls...)
}

// callsFor filters recorded calls by method.
func (d *fakeDaemon) callsFor(method string) []rpcCall {
	var out []rpcCall
	for _, c := range d.calls() {
		if c.Method == method {
			out = append(out, c)
		}
	}
	return out
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

// testEnv builds a Handler + Tools over a fake daemon with isolated state.
func testEnv(t *testing.T, mutate func(*access.Access)) (*Handler, *Tools, *fakeDaemon, *captureSink) {
	t.Helper()
	dir := t.TempDir()
	cfg := &config.Config{
		StateDir:        dir,
		AccessFile:      filepath.Join(dir, "access.json"),
		InboxDir:        filepath.Join(dir, "inbox"),
		PidFile:         filepath.Join(dir, "bot.pid"),
		TranscriptFile:  filepath.Join(dir, "transcript.jsonl"),
		SignalAccount:   testAccount,
		SignalDaemonURL: "",
	}
	acc := access.Defaults()
	if mutate != nil {
		mutate(acc)
	}
	if err := access.Save(acc, cfg.AccessFile); err != nil {
		t.Fatal(err)
	}
	d := newFakeDaemon(t)
	cfg.SignalDaemonURL = d.srv.URL
	client := NewClient(d.srv.URL, testAccount)
	log := transcript.New(cfg.TranscriptFile)
	opts := newOptionStore()
	h := NewHandler(client, cfg, log, opts)
	sink := &captureSink{}
	h.BindNotifier(sink)
	tools := NewTools(client, cfg, log, opts)
	return h, tools, d, sink
}

// dmEnvelope builds a plain DM data-message envelope.
func dmEnvelope(sender, name, text string, ts int64) *envelope {
	return &envelope{
		SourceNumber: sender,
		SourceName:   name,
		Timestamp:    ts,
		DataMessage:  &dataMessage{Timestamp: ts, Message: text},
	}
}

// groupEnvelope builds a group data-message envelope.
func groupEnvelope(sender, groupID, text string, ts int64) *envelope {
	return &envelope{
		SourceNumber: sender,
		Timestamp:    ts,
		DataMessage: &dataMessage{
			Timestamp: ts,
			Message:   text,
			GroupInfo: &groupInfo{GroupID: groupID, Type: "DELIVER"},
		},
	}
}

// captureBursts synchronously captures coalesced bursts.
func captureBursts(h *Handler) *[][]pendingMsg {
	var bursts [][]pendingMsg
	h.coalDeliver = func(_ context.Context, msgs []pendingMsg) {
		bursts = append(bursts, msgs)
	}
	return &bursts
}
