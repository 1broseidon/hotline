package signal

import (
	"bufio"
	"context"
	"net/http"
	"net/http/httptest"
	"strings"
	"sync/atomic"
	"testing"
	"time"
)

func TestReadSSEParsesEventsAndKeepAlives(t *testing.T) {
	stream := ":\n" + // keep-alive comment
		"event:receive\n" +
		`data:{"envelope":{"sourceNumber":"+1555","timestamp":1}}` + "\n" +
		"\n" +
		":\n" +
		"event:receive\n" +
		"data:line1\n" +
		"data:line2\n" +
		"\n"
	var got []sseEvent
	err := readSSE(context.Background(), bufio.NewReader(strings.NewReader(stream)), func(ev sseEvent) {
		got = append(got, ev)
	})
	if err == nil || !strings.Contains(err.Error(), "EOF") {
		t.Fatalf("err %v", err)
	}
	if len(got) != 2 {
		t.Fatalf("events %v", got)
	}
	if got[0].Event != "receive" || !strings.Contains(got[0].Data, `"sourceNumber":"+1555"`) {
		t.Fatalf("event 0: %+v", got[0])
	}
	if got[1].Data != "line1\nline2" {
		t.Fatalf("multi-line data: %q", got[1].Data)
	}
}

// TestEventLoopReconnects proves the SSE consumer redials after a dropped
// connection: the mock daemon serves one event per connection, and the loop
// must collect events from at least two connections.
func TestEventLoopReconnects(t *testing.T) {
	var conns atomic.Int64
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path != "/api/v1/events" {
			w.WriteHeader(http.StatusNotFound)
			return
		}
		n := conns.Add(1)
		w.Header().Set("Content-Type", "text/event-stream")
		w.WriteHeader(http.StatusOK)
		_, _ = w.Write([]byte("event:receive\ndata:{\"n\":" + string(rune('0'+n)) + "}\n\n"))
		w.(http.Flusher).Flush()
		// Close the connection: the client must reconnect for the next event.
	}))
	defer srv.Close()

	c := NewClient(srv.URL, testAccount)
	events := make(chan sseEvent, 8)
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	done := make(chan struct{})
	go func() {
		c.runEventLoop(ctx, func(ev sseEvent) { events <- ev }, nil)
		close(done)
	}()

	for i := 0; i < 2; i++ {
		select {
		case <-events:
		case <-time.After(10 * time.Second):
			t.Fatalf("timed out waiting for event %d (conns=%d)", i+1, conns.Load())
		}
	}
	if conns.Load() < 2 {
		t.Fatalf("expected a reconnect, got %d connection(s)", conns.Load())
	}
	cancel()
	select {
	case <-done:
	case <-time.After(5 * time.Second):
		t.Fatal("event loop did not stop on cancel")
	}
}
