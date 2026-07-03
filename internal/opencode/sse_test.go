package opencode

import (
	"bufio"
	"context"
	"strings"
	"testing"
)

// TestReadSSEParsesBusEvents parses OpenCode-style data-only blocks (no event:
// line) and keep-alive comments, joining multi-line data.
func TestReadSSEParsesBusEvents(t *testing.T) {
	stream := ":\n" + // keep-alive comment
		`data:{"type":"permission.asked","properties":{"id":"perm_1"}}` + "\n" +
		"\n" +
		":\n" +
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
	if !strings.Contains(got[0].Data, `"type":"permission.asked"`) {
		t.Fatalf("event 0: %+v", got[0])
	}
	if got[1].Data != "line1\nline2" {
		t.Fatalf("multi-line data: %q", got[1].Data)
	}
}
