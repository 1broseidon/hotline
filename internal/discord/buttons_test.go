package discord

import (
	"strings"
	"testing"

	"github.com/bwmarrin/discordgo"
)

func TestSanitizeButtons(t *testing.T) {
	in := []string{" ship it ", "", "not yet", strings.Repeat("z", 100)}
	got := sanitizeButtons(in)
	if len(got) != 3 {
		t.Fatalf("want 3, got %d: %v", len(got), got)
	}
	if got[0] != "ship it" || got[1] != "not yet" {
		t.Fatalf("got %v", got)
	}
	if len([]rune(got[2])) != maxButtonLabel {
		t.Fatalf("long label not clamped: %d runes", len([]rune(got[2])))
	}
}

func TestSanitizeButtonsCap(t *testing.T) {
	in := make([]string, 20)
	for i := range in {
		in[i] = "opt"
	}
	if got := sanitizeButtons(in); len(got) != maxButtons {
		t.Fatalf("want %d, got %d", maxButtons, len(got))
	}
}

func TestButtonComponentsLayout(t *testing.T) {
	labels := []string{"a", "b", "c", "d"}
	rows := buttonComponents(labels)
	if len(rows) != 2 {
		t.Fatalf("want 2 rows, got %d", len(rows))
	}
	row0 := rows[0].(discordgo.ActionsRow)
	if len(row0.Components) != 3 {
		t.Fatalf("want 3 buttons in row 0, got %d", len(row0.Components))
	}
	btn := row0.Components[1].(discordgo.Button)
	if btn.Label != "b" || btn.CustomID != "act:1" {
		t.Fatalf("got %+v", btn)
	}
	row1 := rows[1].(discordgo.ActionsRow)
	last := row1.Components[0].(discordgo.Button)
	if last.Label != "d" || last.CustomID != "act:3" {
		t.Fatalf("got %+v", last)
	}
}

func TestButtonLabelLookup(t *testing.T) {
	// Pointer forms — what discordgo's JSON unmarshaling produces.
	components := []discordgo.MessageComponent{
		&discordgo.ActionsRow{Components: []discordgo.MessageComponent{
			&discordgo.Button{Label: "yes", CustomID: "act:0"},
			&discordgo.Button{Label: "no", CustomID: "act:1"},
		}},
	}
	if got := buttonLabel(components, "act:1"); got != "no" {
		t.Fatalf("got %q", got)
	}
	if got := buttonLabel(components, "act:9"); got != "" {
		t.Fatalf("want empty, got %q", got)
	}
	// Value forms — what our own send path constructs.
	valueComponents := buttonComponents([]string{"one", "two"})
	if got := buttonLabel(valueComponents, "act:0"); got != "one" {
		t.Fatalf("got %q", got)
	}
}
