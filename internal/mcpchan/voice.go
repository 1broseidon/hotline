package mcpchan

import (
	"fmt"
	"io"
	"os"
	"path/filepath"
	"strings"
)

// voiceMaxBytes caps how much of a HOTLINE.md file goes into the channel
// instructions. Anything past the cap is dropped with a stderr warning so a
// runaway file can't blow up the system prompt.
const voiceMaxBytes = 16 * 1024

// LoadVoice resolves the voice override for the channel instructions. Lookup
// order, first hit wins:
//
//  1. ./HOTLINE.md — the working directory of the MCP server process, i.e.
//     the repo Claude Code is running in
//  2. <stateRoot>/HOTLINE.md — the operator's global default voice
//
// A missing, unreadable, or whitespace-only file falls through to the next
// candidate; with no hit LoadVoice returns "" and the built-in voice applies.
// The file is read once at startup — MCP instructions ship at the initialize
// handshake — so edits take effect on the next Claude Code restart.
//
// Reads are capped at voiceMaxBytes; longer files are truncated with a stderr
// warning. When an override loads, a stderr line reports where it came from.
func LoadVoice(stateRoot string) string {
	candidates := []string{"./HOTLINE.md"}
	if stateRoot != "" {
		candidates = append(candidates, filepath.Join(stateRoot, "HOTLINE.md"))
	}
	for _, path := range candidates {
		voice, size, ok := readVoiceFile(path)
		if !ok {
			continue
		}
		fmt.Fprintf(os.Stderr, "hotline: voice override from %s (%s)\n", path, humanSize(size))
		return voice
	}
	return ""
}

// readVoiceFile reads one candidate voice file, enforcing the size cap and
// the empty-file fallthrough. It returns the trimmed voice text, the byte
// count actually used, and whether this candidate produced a usable voice.
func readVoiceFile(path string) (voice string, size int, ok bool) {
	f, err := os.Open(path)
	if err != nil {
		if !os.IsNotExist(err) {
			fmt.Fprintf(os.Stderr, "hotline: skipping voice file %s: %v\n", path, err)
		}
		return "", 0, false
	}
	defer f.Close()

	raw, err := io.ReadAll(io.LimitReader(f, voiceMaxBytes+1))
	if err != nil {
		fmt.Fprintf(os.Stderr, "hotline: skipping voice file %s: %v\n", path, err)
		return "", 0, false
	}
	if len(raw) > voiceMaxBytes {
		fmt.Fprintf(os.Stderr, "hotline: voice file %s exceeds %dKB, truncating\n", path, voiceMaxBytes/1024)
		raw = raw[:voiceMaxBytes]
	}
	text := strings.TrimSpace(string(raw))
	if text == "" {
		return "", 0, false
	}
	return text, len(text), true
}

// humanSize renders a byte count the way the startup log does: "830B" under a
// kilobyte, "1.2KB" above.
func humanSize(n int) string {
	if n < 1024 {
		return fmt.Sprintf("%dB", n)
	}
	return fmt.Sprintf("%.1fKB", float64(n)/1024)
}
