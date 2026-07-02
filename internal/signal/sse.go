package signal

import (
	"bufio"
	"context"
	"fmt"
	"net/http"
	"strings"
	"time"
)

// The daemon's GET /api/v1/events endpoint is a standard Server-Sent Events
// stream: each incoming message is one `event:receive` block whose `data:`
// line is the JSON the jsonRpc mode would emit as a receive notification's
// params ({"envelope":{...},"account":"+..."}). Keep-alives are bare ":"
// comment lines every 15s.

// sseEvent is one parsed SSE block.
type sseEvent struct {
	Event string
	Data  string
}

// readSSE parses an SSE stream from r line-by-line, invoking handle for each
// complete event. It returns when the stream ends or ctx is cancelled.
func readSSE(ctx context.Context, r *bufio.Reader, handle func(sseEvent)) error {
	var ev sseEvent
	var data []string
	for {
		if ctx.Err() != nil {
			return ctx.Err()
		}
		line, err := r.ReadString('\n')
		if len(line) > 0 {
			line = strings.TrimRight(line, "\r\n")
			switch {
			case line == "":
				if len(data) > 0 || ev.Event != "" {
					ev.Data = strings.Join(data, "\n")
					handle(ev)
				}
				ev = sseEvent{}
				data = nil
			case strings.HasPrefix(line, ":"):
				// keep-alive comment
			case strings.HasPrefix(line, "event:"):
				ev.Event = strings.TrimSpace(strings.TrimPrefix(line, "event:"))
			case strings.HasPrefix(line, "data:"):
				data = append(data, strings.TrimPrefix(strings.TrimPrefix(line, "data:"), " "))
			case line == "data":
				data = append(data, "")
			}
		}
		if err != nil {
			return err
		}
	}
}

// streamEvents opens the SSE stream once and feeds each event to handle,
// returning when the connection drops or ctx is cancelled.
func (c *Client) streamEvents(ctx context.Context, handle func(sseEvent)) error {
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, c.BaseURL+"/api/v1/events", nil)
	if err != nil {
		return err
	}
	req.Header.Set("Accept", "text/event-stream")

	// A dedicated client without a global timeout: the stream is long-lived.
	client := &http.Client{}
	resp, err := client.Do(req)
	if err != nil {
		return err
	}
	defer resp.Body.Close()
	if resp.StatusCode != http.StatusOK {
		return fmt.Errorf("signal daemon events: HTTP %d", resp.StatusCode)
	}
	return readSSE(ctx, bufio.NewReader(resp.Body), handle)
}

// Reconnect/backoff tuning for the event stream.
const (
	sseBackoffMin = time.Second
	sseBackoffMax = 30 * time.Second
	// sseStableAfter: a connection that lived this long resets the backoff.
	sseStableAfter = 30 * time.Second
)

// runEventLoop keeps the SSE stream alive until ctx is cancelled, redialing
// with exponential backoff and resetting the backoff after a stable
// connection. onError (optional) observes each dropped-connection error.
func (c *Client) runEventLoop(ctx context.Context, handle func(sseEvent), onError func(error)) {
	backoff := sseBackoffMin
	for ctx.Err() == nil {
		started := time.Now()
		err := c.streamEvents(ctx, handle)
		if ctx.Err() != nil {
			return
		}
		if onError != nil && err != nil {
			onError(err)
		}
		if time.Since(started) >= sseStableAfter {
			backoff = sseBackoffMin
		}
		select {
		case <-ctx.Done():
			return
		case <-time.After(backoff):
		}
		backoff *= 2
		if backoff > sseBackoffMax {
			backoff = sseBackoffMax
		}
	}
}
