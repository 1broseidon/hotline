package mcpchan

import (
	"strings"
	"testing"
)

// The channel-fact wording each provider set must (and must not) produce. The
// live bug that motivated provider-awareness: a pi session on the app channel
// was told "You're texting on Telegram", so it emitted Telegram HTML parse-mode
// tags that render as literal <b> text in the native/web client.
const (
	telegramChannelFactText = `You're texting on Telegram.`
	appMarkdownLiteMarker   = `format with markdown-lite`
	appNoHTMLMarker         = `never HTML tags`
	appClientMarker         = `hotline app`
	multiChannelMarker      = `reachable on more than one channel`
)

// piInstr renders the Pi initialize-instructions block for the given active
// providers — the harness that drives the app channel today.
func piInstr(providers ...string) string {
	return PiInstructions("/state/transcript.jsonl", "", providers...)
}

// TestProviderAppOnlyDropsTelegramWording is the direct regression guard for the
// live bug: an app-only session must carry the markdown-lite / never-HTML note
// and must NOT be told it is texting on Telegram.
func TestProviderAppOnlyDropsTelegramWording(t *testing.T) {
	got := piInstr("app")
	if strings.Contains(got, telegramChannelFactText) {
		t.Errorf("app-only instructions must not claim a Telegram channel:\n%s", got)
	}
	for _, want := range []string{appClientMarker, appMarkdownLiteMarker, appNoHTMLMarker} {
		if !strings.Contains(got, want) {
			t.Errorf("app-only instructions missing %q:\n%s", want, got)
		}
	}
	// The register tail is provider-independent and must survive.
	if !strings.Contains(got, registerVoiceLine) {
		t.Errorf("app-only instructions dropped the voice register:\n%s", got)
	}
}

// TestProviderTelegramOnlyUnchanged proves the default/messenger path is
// byte-identical to the no-provider render the goldens pin — no regression.
func TestProviderTelegramOnlyUnchanged(t *testing.T) {
	if got, want := piInstr("telegram"), piInstr(); got != want {
		t.Errorf("telegram-only must match the default render:\n--- telegram ---\n%s\n--- default ---\n%s", got, want)
	}
	got := piInstr("telegram")
	if !strings.Contains(got, telegramChannelFactText) {
		t.Errorf("telegram-only instructions missing the Telegram channel fact:\n%s", got)
	}
	if strings.Contains(got, appMarkdownLiteMarker) {
		t.Errorf("telegram-only instructions leaked the app formatting note:\n%s", got)
	}
}

// TestProviderMessengersKeepTelegramWording pins the deliberate carry-over:
// discord and signal sessions keep the historical Telegram wording (they are
// not being retargeted in this change), and the :instance suffix is stripped.
func TestProviderMessengersKeepTelegramWording(t *testing.T) {
	for _, p := range []string{"discord", "signal", "telegram:work"} {
		got := piInstr(p)
		if !strings.Contains(got, telegramChannelFactText) {
			t.Errorf("%s: expected the messenger (Telegram) wording:\n%s", p, got)
		}
		if strings.Contains(got, appMarkdownLiteMarker) {
			t.Errorf("%s: unexpectedly got the app formatting note", p)
		}
	}
}

// TestProviderMixComposesBoth: when a messenger and the app are both live, the
// channel line names both and points at the inbound source to disambiguate,
// while still carrying the app formatting contract.
func TestProviderMixComposesBoth(t *testing.T) {
	got := piInstr("telegram", "app")
	for _, want := range []string{multiChannelMarker, "On Telegram", appClientMarker, appMarkdownLiteMarker, appNoHTMLMarker} {
		if !strings.Contains(got, want) {
			t.Errorf("mixed-provider instructions missing %q:\n%s", want, got)
		}
	}
}

// TestProviderAppNoHTMLInMarkdownLine is a belt-and-suspenders check that the
// app channel note never itself contains an HTML tag the model could echo.
func TestProviderAppNoHTMLInMarkdownLine(t *testing.T) {
	got := channelFactLine([]string{"app"})
	if strings.Contains(got, "<b>") || strings.Contains(got, "</") {
		t.Errorf("app channel line must not contain HTML tags:\n%s", got)
	}
}
