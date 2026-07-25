package config

import (
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"sync"
)

// envFileMu serializes the read-modify-write inside WriteEnvFile.
//
// Without it, two concurrent updates to the same .env interleave: both read the
// pre-update lines, each applies only its own keys, and whichever writes last
// wins — silently dropping the other's change. That is not theoretical here.
// The app channel's set_sdk_config writes model and effort through this on the
// hot path, so a model apply landing while an effort apply is mid-write loses
// one of them, and the box then reports a value its .env does not hold.
//
// One global mutex rather than a per-path map: .env writes are operator-paced
// (a handful per session across every box in a process), so the contention is
// nil and a map of locks would be more machinery than the problem deserves.
//
// This covers one process. Across processes the atomic rename below still
// guarantees a reader never sees a torn file — the remaining risk is a
// lost update between two hotline processes writing the SAME .env, which is
// exactly what per-box resolution removes for the knobs that move at runtime.
var envFileMu sync.Mutex

// WriteEnvFile merges updates into an .env file and drops the keys listed in
// remove, preserving every other existing line (comments, blanks, keys it
// isn't touching) in place. Updated keys keep their position; new keys append
// at the end, sorted. A key in remove has every one of its lines dropped; a
// missing file with only removals still writes (an empty or updates-only
// file). The file is written 0600 — it is the shared credential file
// (`hotline setup` posture).
//
// Moved here from cmd/hotline/cmd_setup.go so box-side code (the app channel's
// set_sdk_config control) can persist knobs without cmd-layer imports;
// `hotline setup` delegates to this with a nil remove list.
func WriteEnvFile(path string, updates map[string]string, remove []string) error {
	envFileMu.Lock()
	defer envFileMu.Unlock()
	var lines []string
	if data, err := os.ReadFile(path); err == nil {
		lines = strings.Split(strings.TrimRight(string(data), "\n"), "\n")
		if len(lines) == 1 && lines[0] == "" {
			lines = nil
		}
	} else if !os.IsNotExist(err) {
		return err
	}

	removed := make(map[string]bool, len(remove))
	for _, k := range remove {
		removed[k] = true
	}

	written := map[string]bool{}
	kept := lines[:0]
	for _, line := range lines {
		trimmed := strings.TrimSpace(line)
		if trimmed == "" || strings.HasPrefix(trimmed, "#") {
			kept = append(kept, line)
			continue
		}
		k, _, ok := strings.Cut(trimmed, "=")
		if !ok {
			kept = append(kept, line)
			continue
		}
		k = strings.TrimSpace(k)
		if removed[k] {
			continue // dropped: the key is being removed
		}
		if v, has := updates[k]; has {
			line = k + "=" + v
			written[k] = true
		}
		kept = append(kept, line)
	}
	lines = kept
	for _, k := range SortedEnvKeys(updates) {
		if !written[k] && !removed[k] {
			lines = append(lines, k+"="+updates[k])
		}
	}

	return writeFileAtomic(path, []byte(strings.Join(lines, "\n")+"\n"))
}

// writeFileAtomic replaces path's contents in one step: write a sibling temp
// file, fsync it, then rename over the target. Rename within a directory is
// atomic, so a concurrent reader — `hotline up`'s per-respawn re-resolution, or
// another box's LoadSDK — sees either the whole old file or the whole new one,
// never a half-written .env. os.WriteFile truncates in place, which leaves a
// window where a reader gets a TRUNCATED credential file; on this path that
// would read as "every knob and every token cleared".
//
// The temp file is created 0600 before it ever holds content, so the secrets in
// a shared .env are never briefly world-readable.
func writeFileAtomic(path string, content []byte) error {
	dir := filepath.Dir(path)
	tmp, err := os.CreateTemp(dir, "."+filepath.Base(path)+".tmp*")
	if err != nil {
		return fmt.Errorf("creating temp file in %s: %w", dir, err)
	}
	tmpName := tmp.Name()
	defer func() {
		tmp.Close()
		os.Remove(tmpName) // no-op once the rename succeeded
	}()
	if err := tmp.Chmod(0o600); err != nil {
		return err
	}
	if _, err := tmp.Write(content); err != nil {
		return err
	}
	if err := tmp.Sync(); err != nil {
		return err
	}
	if err := tmp.Close(); err != nil {
		return err
	}
	return os.Rename(tmpName, path)
}

// SortedEnvKeys returns m's keys sorted, for deterministic .env appends and
// display listings.
func SortedEnvKeys(m map[string]string) []string {
	keys := make([]string, 0, len(m))
	for k := range m {
		keys = append(keys, k)
	}
	sort.Strings(keys)
	return keys
}
