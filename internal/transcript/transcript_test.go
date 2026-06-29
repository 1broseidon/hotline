package transcript

import (
	"bufio"
	"encoding/json"
	"os"
	"path/filepath"
	"sync"
	"testing"
	"time"
)

func TestAppendWritesJSONL(t *testing.T) {
	path := filepath.Join(t.TempDir(), "transcript.jsonl")
	l := New(path)
	l.now = func() time.Time { return time.Date(2026, 6, 29, 6, 0, 0, 0, time.UTC) }

	if err := l.Append(Record{Dir: "in", ChatID: "1", User: "sam", Kind: "text", Text: "hi"}); err != nil {
		t.Fatal(err)
	}
	if err := l.Append(Record{Dir: "out", ChatID: "1", Kind: "reply", Text: "yo"}); err != nil {
		t.Fatal(err)
	}

	recs := readAll(t, path)
	if len(recs) != 2 {
		t.Fatalf("got %d records, want 2", len(recs))
	}
	if recs[0].Dir != "in" || recs[0].Text != "hi" || recs[0].User != "sam" {
		t.Errorf("record 0 = %+v", recs[0])
	}
	if recs[0].TS != "2026-06-29T06:00:00.000Z" {
		t.Errorf("TS not stamped as expected: %q", recs[0].TS)
	}
	if recs[1].Dir != "out" || recs[1].Text != "yo" {
		t.Errorf("record 1 = %+v", recs[1])
	}
}

func TestAppendPreservesGivenTS(t *testing.T) {
	path := filepath.Join(t.TempDir(), "t.jsonl")
	l := New(path)
	if err := l.Append(Record{TS: "2020-01-01T00:00:00.000Z", Dir: "in", Text: "x"}); err != nil {
		t.Fatal(err)
	}
	if got := readAll(t, path)[0].TS; got != "2020-01-01T00:00:00.000Z" {
		t.Errorf("explicit TS overwritten: %q", got)
	}
}

func TestAppendNilSafe(t *testing.T) {
	var l *Logger
	if err := l.Append(Record{Text: "x"}); err != nil {
		t.Fatalf("nil logger Append should be a no-op, got %v", err)
	}
}

func TestAppendConcurrent(t *testing.T) {
	path := filepath.Join(t.TempDir(), "t.jsonl")
	l := New(path)

	const n = 50
	var wg sync.WaitGroup
	for range n {
		wg.Go(func() {
			_ = l.Append(Record{Dir: "in", Text: "msg"})
		})
	}
	wg.Wait()

	if got := len(readAll(t, path)); got != n {
		t.Fatalf("concurrent appends produced %d lines, want %d", got, n)
	}
}

func readAll(t *testing.T, path string) []Record {
	t.Helper()
	f, err := os.Open(path)
	if err != nil {
		t.Fatal(err)
	}
	defer f.Close()
	var out []Record
	sc := bufio.NewScanner(f)
	for sc.Scan() {
		var r Record
		if err := json.Unmarshal(sc.Bytes(), &r); err != nil {
			t.Fatalf("bad JSON line %q: %v", sc.Text(), err)
		}
		out = append(out, r)
	}
	if err := sc.Err(); err != nil {
		t.Fatal(err)
	}
	return out
}
