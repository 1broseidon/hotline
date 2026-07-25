package config

import (
	"fmt"
	"io"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"testing"
)

// TestWriteEnvFileRoundTrip proves the merge-writer's full contract in one
// round trip: comments and blank lines preserved verbatim, an updated key
// rewritten IN PLACE (position kept), a removed key's line dropped, unknown
// keys untouched, new keys appended sorted, and the 0600 posture kept.
func TestWriteEnvFileRoundTrip(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, ".env")
	orig := "# hotline credentials\n" +
		"TELEGRAM_BOT_TOKEN=tok123\n" +
		"\n" +
		"# sdk knobs\n" +
		"HOTLINE_SDK_MODEL=claude-opus-4-8\n" +
		"HOTLINE_CLAUDE_SDK_MODEL=claude-legacy\n" +
		"HOTLINE_SDK_MAX_TURNS=40\n"
	if err := os.WriteFile(path, []byte(orig), 0o600); err != nil {
		t.Fatal(err)
	}

	err := WriteEnvFile(path,
		map[string]string{"HOTLINE_SDK_MODEL": "claude-sonnet-4-6", "HOTLINE_SDK_EFFORT": "high"},
		[]string{"HOTLINE_CLAUDE_SDK_MODEL"})
	if err != nil {
		t.Fatal(err)
	}

	got, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	want := "# hotline credentials\n" +
		"TELEGRAM_BOT_TOKEN=tok123\n" +
		"\n" +
		"# sdk knobs\n" +
		"HOTLINE_SDK_MODEL=claude-sonnet-4-6\n" +
		"HOTLINE_SDK_MAX_TURNS=40\n" +
		"HOTLINE_SDK_EFFORT=high\n"
	if string(got) != want {
		t.Errorf("round trip mismatch:\n--- got ---\n%s--- want ---\n%s", got, want)
	}

	info, err := os.Stat(path)
	if err != nil {
		t.Fatal(err)
	}
	if perm := info.Mode().Perm(); perm != 0o600 {
		t.Errorf("mode = %o, want 0600", perm)
	}
}

// TestWriteEnvFileRemoveMissingKeyAndFile: removing a key that is absent is a
// no-op, and a missing file with only removals still writes a valid file.
func TestWriteEnvFileRemoveMissingKeyAndFile(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, ".env")
	if err := WriteEnvFile(path, nil, []string{"HOTLINE_SDK_MODEL"}); err != nil {
		t.Fatal(err)
	}
	if _, err := os.Stat(path); err != nil {
		t.Fatalf("file not written: %v", err)
	}

	if err := WriteEnvFile(path, map[string]string{"A": "1"}, []string{"NOPE"}); err != nil {
		t.Fatal(err)
	}
	got, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	if string(got) != "\nA=1\n" && string(got) != "A=1\n" {
		t.Errorf("content = %q", got)
	}
}

// TestWriteEnvFileRemoveWinsOverUpdate: a key listed in both updates and
// remove is removed, not written — removal is the stronger intent and the
// callers (UpdateSDKEnv) never mean both.
func TestWriteEnvFileRemoveWinsOverUpdate(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, ".env")
	if err := os.WriteFile(path, []byte("K=old\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	if err := WriteEnvFile(path, map[string]string{"K": "new"}, []string{"K"}); err != nil {
		t.Fatal(err)
	}
	got, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	if string(got) != "\n" {
		t.Errorf("content = %q, want the K line gone", got)
	}
}

// TestWriteEnvFileConcurrentUpdatesDoNotLoseKeys is sol #3's .env half: the
// hot-apply path writes model and effort through WriteEnvFile, and a
// read-modify-write with no lock loses whichever update commits first.
func TestWriteEnvFileConcurrentUpdatesDoNotLoseKeys(t *testing.T) {
	path := filepath.Join(t.TempDir(), ".env")
	if err := os.WriteFile(path, []byte("TELEGRAM_BOT_TOKEN=keepme\n"), 0o600); err != nil {
		t.Fatal(err)
	}

	const n = 24
	var wg sync.WaitGroup
	errs := make(chan error, n)
	for i := 0; i < n; i++ {
		wg.Add(1)
		go func(i int) {
			defer wg.Done()
			key := fmt.Sprintf("HOTLINE_KNOB_%02d", i)
			if err := WriteEnvFile(path, map[string]string{key: "v"}, nil); err != nil {
				errs <- err
			}
		}(i)
	}
	wg.Wait()
	close(errs)
	for err := range errs {
		t.Fatalf("concurrent write: %v", err)
	}

	raw, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	got := string(raw)
	if !strings.Contains(got, "TELEGRAM_BOT_TOKEN=keepme") {
		t.Errorf("an unrelated credential was lost:\n%s", got)
	}
	for i := 0; i < n; i++ {
		key := fmt.Sprintf("HOTLINE_KNOB_%02d", i)
		if !strings.Contains(got, key+"=v") {
			t.Errorf("%s lost to a concurrent write:\n%s", key, got)
		}
	}
}

// TestWriteEnvFileReplacesRatherThanTruncates: a reader must never observe a
// half-written .env. os.WriteFile truncates the target in place, which leaves a
// window where a concurrent LoadSDK reads an EMPTY credential file and concludes
// every knob and every token is unset. The fix is temp-file + rename, and this
// asserts that property deterministically: a descriptor opened on the ORIGINAL
// inode must still see the ORIGINAL bytes after the write, which is only true
// if the file was REPLACED, never rewritten in place.
func TestWriteEnvFileReplacesRatherThanTruncates(t *testing.T) {
	path := filepath.Join(t.TempDir(), ".env")
	body := "TELEGRAM_BOT_TOKEN=keepme\nHOTLINE_SDK_MODEL=claude-opus-4-8\n"
	if err := os.WriteFile(path, []byte(body), 0o600); err != nil {
		t.Fatal(err)
	}
	// Stand in for a reader that opened the file just before the write.
	old, err := os.Open(path)
	if err != nil {
		t.Fatal(err)
	}
	defer old.Close()

	if err := WriteEnvFile(path, map[string]string{"HOTLINE_SDK_EFFORT": "high"}, nil); err != nil {
		t.Fatal(err)
	}

	seen, err := io.ReadAll(old)
	if err != nil {
		t.Fatal(err)
	}
	if string(seen) != body {
		t.Fatalf("the target inode was rewritten in place; a mid-write reader would see a partial file.\ngot:  %q\nwant: %q", seen, body)
	}
	// And the new content did land.
	raw, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(string(raw), "HOTLINE_SDK_EFFORT=high") || !strings.Contains(string(raw), "TELEGRAM_BOT_TOKEN=keepme") {
		t.Fatalf("post-write content:\n%s", raw)
	}
	// No temp file left behind.
	entries, err := os.ReadDir(filepath.Dir(path))
	if err != nil {
		t.Fatal(err)
	}
	if len(entries) != 1 {
		t.Fatalf("write left %d files in the state dir, want 1", len(entries))
	}
}
