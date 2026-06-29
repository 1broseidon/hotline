package telegram

import (
	"errors"
	"fmt"
	"io"
	"net/http"
	"os"
	"path/filepath"
	"regexp"
	"strings"
	"time"

	"github.com/PaulSonOfLars/gotgbot/v2"
)

// maxDownloadBytes is Telegram's bot download cap (20MB).
const maxDownloadBytes = 20 * 1024 * 1024

// Extracted describes a media attachment found on an inbound message.
type Extracted struct {
	FileID  string
	Kind    string // photo|document|voice|audio|video|video_note|sticker
	Name    string // SafeName'd, may be empty
	MIME    string
	Size    int64
	IsPhoto bool
	// Synthetic is the fallback content used when the message has no caption.
	Synthetic string
}

// Extract returns the (single) media attachment on a message, if any. For
// photos the largest size (last in the array) is chosen.
func Extract(msg *gotgbot.Message) (Extracted, bool) {
	switch {
	case len(msg.Photo) > 0:
		best := msg.Photo[len(msg.Photo)-1]
		return Extracted{
			FileID:    best.FileId,
			Kind:      "photo",
			Size:      best.FileSize,
			IsPhoto:   true,
			Synthetic: "(photo)",
		}, true

	case msg.Document != nil:
		name := SafeName(msg.Document.FileName)
		return Extracted{
			FileID:    msg.Document.FileId,
			Kind:      "document",
			Name:      name,
			MIME:      msg.Document.MimeType,
			Size:      msg.Document.FileSize,
			Synthetic: fmt.Sprintf("(document: %s)", nonEmpty(name, "file")),
		}, true

	case msg.Voice != nil:
		return Extracted{
			FileID:    msg.Voice.FileId,
			Kind:      "voice",
			MIME:      msg.Voice.MimeType,
			Size:      msg.Voice.FileSize,
			Synthetic: "(voice message)",
		}, true

	case msg.Audio != nil:
		name := SafeName(msg.Audio.FileName)
		title := SafeName(msg.Audio.Title)
		return Extracted{
			FileID:    msg.Audio.FileId,
			Kind:      "audio",
			Name:      name,
			MIME:      msg.Audio.MimeType,
			Size:      msg.Audio.FileSize,
			Synthetic: fmt.Sprintf("(audio: %s)", nonEmpty(title, nonEmpty(name, "audio"))),
		}, true

	case msg.Video != nil:
		return Extracted{
			FileID:    msg.Video.FileId,
			Kind:      "video",
			Name:      SafeName(msg.Video.FileName),
			MIME:      msg.Video.MimeType,
			Size:      msg.Video.FileSize,
			Synthetic: "(video)",
		}, true

	case msg.VideoNote != nil:
		return Extracted{
			FileID:    msg.VideoNote.FileId,
			Kind:      "video_note",
			Size:      msg.VideoNote.FileSize,
			Synthetic: "(video note)",
		}, true

	case msg.Sticker != nil:
		emoji := ""
		if msg.Sticker.Emoji != "" {
			emoji = " " + msg.Sticker.Emoji
		}
		return Extracted{
			FileID:    msg.Sticker.FileId,
			Kind:      "sticker",
			Size:      msg.Sticker.FileSize,
			Synthetic: fmt.Sprintf("(sticker%s)", emoji),
		}, true
	}
	return Extracted{}, false
}

var extCleanRe = regexp.MustCompile(`[^a-zA-Z0-9]`)

// DownloadToInbox downloads a Telegram file by ID into inboxDir and returns the
// local path. The filename is <unixMillis>-<uniqueId>.<ext>. Refuses files
// larger than Telegram's 20MB bot-download cap.
func DownloadToInbox(bot *gotgbot.Bot, inboxDir, fileID string) (string, error) {
	file, err := bot.GetFile(fileID, nil)
	if err != nil {
		return "", err
	}
	if file.FilePath == "" {
		return "", fmt.Errorf("Telegram returned no file_path — file may have expired")
	}
	if file.FileSize > maxDownloadBytes {
		return "", fmt.Errorf("file too large to download: %d bytes (max 20MB)", file.FileSize)
	}

	url := fmt.Sprintf("https://api.telegram.org/file/bot%s/%s", bot.Token, file.FilePath)
	client := &http.Client{Timeout: 60 * time.Second}
	resp, err := client.Get(url)
	if err != nil {
		// net/http wraps transport failures in *url.Error, whose Error() embeds
		// the full request URL — including bot<TOKEN>. Scrub it before the error
		// escapes to logs or the tool result.
		return "", scrubToken(err, bot.Token)
	}
	defer resp.Body.Close()
	if resp.StatusCode != http.StatusOK {
		return "", fmt.Errorf("download failed: HTTP %d", resp.StatusCode)
	}

	if err := os.MkdirAll(inboxDir, 0o700); err != nil {
		return "", err
	}

	ext := "bin"
	if i := strings.LastIndex(file.FilePath, "."); i >= 0 {
		if cleaned := extCleanRe.ReplaceAllString(file.FilePath[i+1:], ""); cleaned != "" {
			ext = cleaned
		}
	}
	uniqueID := extCleanRe.ReplaceAllString(file.FileUniqueId, "")
	if uniqueID == "" {
		uniqueID = "dl"
	}
	name := fmt.Sprintf("%d-%s.%s", time.Now().UnixMilli(), uniqueID, ext)
	path := filepath.Join(inboxDir, name)

	out, err := os.Create(path)
	if err != nil {
		return "", err
	}
	defer out.Close()

	// Cap the copy defensively in case file_size was unset/under-reported.
	if _, err := io.Copy(out, io.LimitReader(resp.Body, maxDownloadBytes+1)); err != nil {
		_ = os.Remove(path)
		return "", err
	}
	return path, nil
}

// scrubToken returns an error whose message has every occurrence of token
// replaced with <TOKEN>, so the bot token never reaches logs or tool results.
// token may legitimately be empty (token-less mode); guard against replacing
// the empty string, which would corrupt the message.
func scrubToken(err error, token string) error {
	if err == nil {
		return nil
	}
	if token == "" {
		return err
	}
	msg := err.Error()
	if !strings.Contains(msg, token) {
		return err
	}
	return errors.New(strings.ReplaceAll(msg, token, "<TOKEN>"))
}

func nonEmpty(s, fallback string) string {
	if s == "" {
		return fallback
	}
	return s
}
