package signal

import (
	"fmt"
	"mime"
	"os"
	"path/filepath"
	"regexp"
	"strings"
	"time"
)

// maxDownloadBytes caps how large an attachment we will accept from the
// daemon — 50MB, matching the other adapters.
const maxDownloadBytes = 50 * 1024 * 1024

// Extracted describes a media attachment found on an inbound message.
type Extracted struct {
	Kind      string // photo|voice|audio|video|document
	Name      string // SafeName'd, may be empty
	IsPhoto   bool
	Synthetic string // fallback content when the message has no text
}

// extract classifies one Signal attachment by content type. Signal voice
// notes arrive as audio/aac (or audio/ogg) without a filename; audio files
// keep their name.
func extract(att attachment) Extracted {
	name := SafeName(att.Filename)
	ct := att.ContentType
	switch {
	case strings.HasPrefix(ct, "image/"):
		return Extracted{Kind: "photo", Name: name, IsPhoto: true, Synthetic: "(photo)"}
	case strings.HasPrefix(ct, "audio/"):
		if name == "" {
			return Extracted{Kind: "voice", Synthetic: "(voice message)"}
		}
		return Extracted{Kind: "audio", Name: name, Synthetic: fmt.Sprintf("(audio: %s)", name)}
	case strings.HasPrefix(ct, "video/"):
		return Extracted{Kind: "video", Name: name, Synthetic: "(video)"}
	default:
		return Extracted{Kind: "document", Name: name, Synthetic: fmt.Sprintf("(document: %s)", nonEmpty(name, "file"))}
	}
}

var extCleanRe = regexp.MustCompile(`[^a-zA-Z0-9]`)

// saveToInbox writes attachment bytes into inboxDir with the same
// timestamped, sanitized naming scheme as the other adapters, deriving an
// extension from the filename or MIME type.
func saveToInbox(inboxDir string, data []byte, filename, contentType string) (string, error) {
	if len(data) > maxDownloadBytes {
		return "", fmt.Errorf("file too large to download: %d bytes (max 50MB)", len(data))
	}
	if err := os.MkdirAll(inboxDir, 0o700); err != nil {
		return "", err
	}

	ext := "bin"
	if i := strings.LastIndex(filename, "."); i >= 0 {
		if cleaned := extCleanRe.ReplaceAllString(filename[i+1:], ""); cleaned != "" {
			ext = cleaned
		}
	} else if exts, err := mime.ExtensionsByType(contentType); err == nil && len(exts) > 0 {
		if cleaned := extCleanRe.ReplaceAllString(exts[0], ""); cleaned != "" {
			ext = cleaned
		}
	}

	name := fmt.Sprintf("%d-%s.%s", time.Now().UnixMilli(), uniqueStem(filename), ext)
	path := filepath.Join(inboxDir, name)
	if err := os.WriteFile(path, data, 0o600); err != nil {
		return "", err
	}
	return path, nil
}

// uniqueStem derives a filesystem-safe stem from the attachment's name.
func uniqueStem(base string) string {
	stem := base
	if i := strings.LastIndex(base, "."); i >= 0 {
		stem = base[:i]
	}
	stem = extCleanRe.ReplaceAllString(stem, "")
	if stem == "" {
		stem = "dl"
	}
	if len(stem) > 40 {
		stem = stem[:40]
	}
	return stem
}
