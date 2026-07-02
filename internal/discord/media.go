package discord

import (
	"fmt"
	"io"
	"net/http"
	"net/url"
	"os"
	"path/filepath"
	"regexp"
	"strings"
	"time"

	"github.com/bwmarrin/discordgo"
)

// maxDownloadBytes caps how large an attachment we will pull from Discord's
// CDN (Discord itself allows big uploads from boosted servers; 50MB matches
// the reply-side cap).
const maxDownloadBytes = 50 * 1024 * 1024

// allowedCDNHosts are the only hosts DownloadToInbox will fetch from. The
// file_id for Discord is an attachment URL supplied through the tool surface,
// so it must never become a generic SSRF primitive.
var allowedCDNHosts = map[string]bool{
	"cdn.discordapp.com":   true,
	"media.discordapp.net": true,
}

// Extracted describes a media attachment found on an inbound message.
type Extracted struct {
	Kind      string // photo|voice|audio|video|document
	Name      string // SafeName'd, may be empty
	IsPhoto   bool
	Synthetic string // fallback content when the message has no text
}

// extract classifies one Discord attachment by content type.
func extract(att *discordgo.MessageAttachment) Extracted {
	name := SafeName(att.Filename)
	ct := att.ContentType
	switch {
	case strings.HasPrefix(ct, "image/"):
		return Extracted{Kind: "photo", Name: name, IsPhoto: true, Synthetic: "(photo)"}
	case strings.HasPrefix(ct, "audio/"):
		kind := "audio"
		synth := fmt.Sprintf("(audio: %s)", nonEmpty(name, "audio"))
		if att.DurationSecs > 0 && att.Waveform != "" {
			kind = "voice" // Discord voice messages carry duration + waveform
			synth = "(voice message)"
		}
		return Extracted{Kind: kind, Name: name, Synthetic: synth}
	case strings.HasPrefix(ct, "video/"):
		return Extracted{Kind: "video", Name: name, Synthetic: "(video)"}
	default:
		return Extracted{Kind: "document", Name: name, Synthetic: fmt.Sprintf("(document: %s)", nonEmpty(name, "file"))}
	}
}

var extCleanRe = regexp.MustCompile(`[^a-zA-Z0-9]`)

// DownloadToInbox fetches a Discord attachment URL into inboxDir and returns
// the local path. Only Discord CDN hosts over https are accepted, and the
// copy is capped at maxDownloadBytes.
func DownloadToInbox(inboxDir, rawURL string) (string, error) {
	u, err := url.Parse(rawURL)
	if err != nil {
		return "", fmt.Errorf("invalid attachment URL: %w", err)
	}
	if u.Scheme != "https" || !allowedCDNHosts[u.Hostname()] {
		return "", fmt.Errorf("refusing to download from %q — only Discord CDN attachment URLs are supported", u.Host)
	}

	client := &http.Client{Timeout: 60 * time.Second}
	resp, err := client.Get(u.String())
	if err != nil {
		return "", err
	}
	defer resp.Body.Close()
	if resp.StatusCode != http.StatusOK {
		return "", fmt.Errorf("download failed: HTTP %d", resp.StatusCode)
	}
	if resp.ContentLength > maxDownloadBytes {
		return "", fmt.Errorf("file too large to download: %d bytes (max 50MB)", resp.ContentLength)
	}

	if err := os.MkdirAll(inboxDir, 0o700); err != nil {
		return "", err
	}

	base := filepath.Base(u.Path)
	ext := "bin"
	if i := strings.LastIndex(base, "."); i >= 0 {
		if cleaned := extCleanRe.ReplaceAllString(base[i+1:], ""); cleaned != "" {
			ext = cleaned
		}
	}
	name := fmt.Sprintf("%d-%s.%s", time.Now().UnixMilli(), uniqueStem(base), ext)
	path := filepath.Join(inboxDir, name)

	out, err := os.Create(path)
	if err != nil {
		return "", err
	}
	defer out.Close()

	n, err := io.Copy(out, io.LimitReader(resp.Body, maxDownloadBytes+1))
	if err != nil {
		_ = os.Remove(path)
		return "", err
	}
	if n > maxDownloadBytes {
		_ = os.Remove(path)
		return "", fmt.Errorf("file too large to download (max 50MB)")
	}
	return path, nil
}

// uniqueStem derives a filesystem-safe stem from the attachment's base name.
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
