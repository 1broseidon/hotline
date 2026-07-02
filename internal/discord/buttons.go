package discord

import (
	"regexp"
	"strconv"
	"strings"

	"github.com/bwmarrin/discordgo"
)

// maxButtons caps how many buttons one reply can carry. Discord permits up to
// 25 (5 rows x 5), but a question with a dozen-plus options is a UX smell.
// Matches the telegram adapter's cap.
const maxButtons = 12

// buttonsPerRow is how many buttons share an action row. Discord caps a
// message at 5 action rows, so 3 per row keeps labels readable and fits the
// maxButtons cap in 4 rows.
const buttonsPerRow = 3

// maxButtonLabel is Discord's button label cap.
const maxButtonLabel = 80

// actBtnRe matches an action-button custom_id: "act:<index>". The index
// identifies which option was tapped; the option's label is recovered from the
// message's own components, so no server-side state is needed.
var actBtnRe = regexp.MustCompile(`^act:(\d+)$`)

// sanitizeButtons trims labels, drops blanks, clamps each label to Discord's
// 80-char cap, and caps the count. Order is preserved so custom_id indices
// line up with what was sent.
func sanitizeButtons(in []string) []string {
	out := make([]string, 0, len(in))
	for _, b := range in {
		if b = strings.TrimSpace(b); b == "" {
			continue
		}
		if r := []rune(b); len(r) > maxButtonLabel {
			b = string(r[:maxButtonLabel-1]) + "…"
		}
		out = append(out, b)
		if len(out) == maxButtons {
			break
		}
	}
	return out
}

// buttonComponents lays the labels out as native Discord button rows. Custom
// IDs are "act:<i>"; the label (which is the value relayed back to Claude) is
// read from the message's components when the button is clicked.
func buttonComponents(labels []string) []discordgo.MessageComponent {
	var rows []discordgo.MessageComponent
	for start := 0; start < len(labels); start += buttonsPerRow {
		end := min(start+buttonsPerRow, len(labels))
		row := discordgo.ActionsRow{}
		for i := start; i < end; i++ {
			row.Components = append(row.Components, discordgo.Button{
				Label:    labels[i],
				Style:    discordgo.SecondaryButton,
				CustomID: "act:" + strconv.Itoa(i),
			})
		}
		rows = append(rows, row)
	}
	return rows
}

// buttonLabel returns the label of the button whose custom_id matches data, or
// "" if the components are gone or no button matches (e.g. already answered).
func buttonLabel(components []discordgo.MessageComponent, data string) string {
	for _, c := range components {
		row, ok := c.(*discordgo.ActionsRow)
		if !ok {
			if v, okv := c.(discordgo.ActionsRow); okv {
				row = &v
			} else {
				continue
			}
		}
		for _, inner := range row.Components {
			switch btn := inner.(type) {
			case *discordgo.Button:
				if btn.CustomID == data {
					return btn.Label
				}
			case discordgo.Button:
				if btn.CustomID == data {
					return btn.Label
				}
			}
		}
	}
	return ""
}
