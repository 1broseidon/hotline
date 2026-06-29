package telegram

import (
	"testing"

	"github.com/PaulSonOfLars/gotgbot/v2"
)

func TestExtractNoMedia(t *testing.T) {
	if _, ok := Extract(&gotgbot.Message{Text: "just text"}); ok {
		t.Fatal("expected ok=false for a text-only message")
	}
}

func TestExtractPhotoPicksLargest(t *testing.T) {
	msg := &gotgbot.Message{Photo: []gotgbot.PhotoSize{
		{FileId: "small", FileSize: 100},
		{FileId: "medium", FileSize: 500},
		{FileId: "large", FileSize: 2000},
	}}
	ex, ok := Extract(msg)
	if !ok {
		t.Fatal("expected media")
	}
	if ex.FileID != "large" {
		t.Fatalf("expected largest photo (last), got %q", ex.FileID)
	}
	if !ex.IsPhoto || ex.Kind != "photo" {
		t.Fatalf("photo flags wrong: %+v", ex)
	}
	if ex.Synthetic != "(photo)" {
		t.Fatalf("synthetic = %q", ex.Synthetic)
	}
}

func TestExtractDocumentSanitizesName(t *testing.T) {
	msg := &gotgbot.Message{Document: &gotgbot.Document{
		FileId:   "doc1",
		FileName: "evil<name>.pdf",
		MimeType: "application/pdf",
		FileSize: 1234,
	}}
	ex, ok := Extract(msg)
	if !ok {
		t.Fatal("expected media")
	}
	if ex.Kind != "document" {
		t.Fatalf("kind = %q", ex.Kind)
	}
	if ex.Name != "evil_name_.pdf" {
		t.Fatalf("name not SafeName'd: %q", ex.Name)
	}
	if ex.MIME != "application/pdf" || ex.Size != 1234 {
		t.Fatalf("metadata wrong: %+v", ex)
	}
	if ex.Synthetic != "(document: evil_name_.pdf)" {
		t.Fatalf("synthetic = %q", ex.Synthetic)
	}
}

func TestExtractDocumentNoNameFallback(t *testing.T) {
	ex, _ := Extract(&gotgbot.Message{Document: &gotgbot.Document{FileId: "d"}})
	if ex.Synthetic != "(document: file)" {
		t.Fatalf("expected fallback name, got %q", ex.Synthetic)
	}
}

func TestExtractVoice(t *testing.T) {
	ex, ok := Extract(&gotgbot.Message{Voice: &gotgbot.Voice{FileId: "v", MimeType: "audio/ogg", FileSize: 99}})
	if !ok || ex.Kind != "voice" {
		t.Fatalf("voice extract wrong: %+v ok=%v", ex, ok)
	}
	if ex.Synthetic != "(voice message)" {
		t.Fatalf("synthetic = %q", ex.Synthetic)
	}
	if ex.IsPhoto {
		t.Fatal("voice is not a photo")
	}
}

func TestExtractAudioPrefersTitle(t *testing.T) {
	ex, _ := Extract(&gotgbot.Message{Audio: &gotgbot.Audio{
		FileId:   "a",
		Title:    "My Song",
		FileName: "track01.mp3",
	}})
	if ex.Kind != "audio" {
		t.Fatalf("kind = %q", ex.Kind)
	}
	if ex.Synthetic != "(audio: My Song)" {
		t.Fatalf("audio should prefer title, got %q", ex.Synthetic)
	}
	// Name field carries the file name (sanitized), not the title.
	if ex.Name != "track01.mp3" {
		t.Fatalf("name = %q", ex.Name)
	}
}

func TestExtractAudioFallsBackToName(t *testing.T) {
	ex, _ := Extract(&gotgbot.Message{Audio: &gotgbot.Audio{FileId: "a", FileName: "track01.mp3"}})
	if ex.Synthetic != "(audio: track01.mp3)" {
		t.Fatalf("audio should fall back to name, got %q", ex.Synthetic)
	}
}

func TestExtractAudioFallsBackToLiteral(t *testing.T) {
	ex, _ := Extract(&gotgbot.Message{Audio: &gotgbot.Audio{FileId: "a"}})
	if ex.Synthetic != "(audio: audio)" {
		t.Fatalf("audio with no title/name, got %q", ex.Synthetic)
	}
}

func TestExtractVideo(t *testing.T) {
	ex, _ := Extract(&gotgbot.Message{Video: &gotgbot.Video{FileId: "vid", FileName: "clip<>.mp4", MimeType: "video/mp4"}})
	if ex.Kind != "video" || ex.Synthetic != "(video)" {
		t.Fatalf("video wrong: %+v", ex)
	}
	if ex.Name != "clip__.mp4" {
		t.Fatalf("video name not sanitized: %q", ex.Name)
	}
}

func TestExtractVideoNote(t *testing.T) {
	ex, _ := Extract(&gotgbot.Message{VideoNote: &gotgbot.VideoNote{FileId: "vn", FileSize: 7}})
	if ex.Kind != "video_note" || ex.Synthetic != "(video note)" {
		t.Fatalf("video note wrong: %+v", ex)
	}
}

func TestExtractStickerWithEmoji(t *testing.T) {
	ex, _ := Extract(&gotgbot.Message{Sticker: &gotgbot.Sticker{FileId: "s", Emoji: "🎉"}})
	if ex.Kind != "sticker" {
		t.Fatalf("kind = %q", ex.Kind)
	}
	if ex.Synthetic != "(sticker 🎉)" {
		t.Fatalf("sticker synthetic = %q", ex.Synthetic)
	}
}

func TestExtractStickerNoEmoji(t *testing.T) {
	ex, _ := Extract(&gotgbot.Message{Sticker: &gotgbot.Sticker{FileId: "s"}})
	if ex.Synthetic != "(sticker)" {
		t.Fatalf("sticker without emoji, got %q", ex.Synthetic)
	}
}

// TestExtractPhotoPriority confirms photo wins when multiple media-like fields
// are present (Extract checks Photo first).
func TestExtractPhotoPriority(t *testing.T) {
	msg := &gotgbot.Message{
		Photo:    []gotgbot.PhotoSize{{FileId: "p"}},
		Document: &gotgbot.Document{FileId: "d"},
	}
	ex, _ := Extract(msg)
	if ex.Kind != "photo" {
		t.Fatalf("photo should take priority, got %q", ex.Kind)
	}
}
