package mcpchan

import (
	"fmt"
	"strings"
	"time"

	"github.com/1broseidon/hotline/internal/loop"
	"github.com/1broseidon/hotline/internal/notify"
)

const setupLoopSchema = `{"type":"object","properties":{"label":{"type":"string","description":"Unique loop label."},"every":{"type":"string","description":"Interval duration, e.g. 10m, 1h, 6h."},"cmd":{"type":"string","description":"Shell command to run on each tick."},"notify_llm":{"type":"boolean","description":"Route non-empty stdout through the notify gate."},"source":{"type":"string","description":"Existing notify source label. Omit with notify_llm to auto-create a source named after the loop."},"level":{"type":"string","enum":["urgent","normal","low"],"description":"Notify level for notify_llm stdout. Default normal."},"timeout":{"type":"string","description":"Per-run timeout duration. Default 2m."}},"required":["label","every","cmd"]}`

const setupNotifySchema = `{"type":"object","properties":{"label":{"type":"string","description":"Unique notify source label."},"cap":{"type":"string","enum":["urgent","normal","low"],"description":"Maximum level this source may send. Default normal."},"burst":{"type":"integer","description":"Optional token-bucket burst override."},"refill_mins":{"type":"integer","description":"Optional token-bucket refill interval in minutes."},"chat_id":{"type":"string","description":"Optional default chat_id for this source."}},"required":["label"]}`

// SetupLoopInput mirrors hotline loop add, except it intentionally has no
// approve flag: agents cannot self-approve loops.
type SetupLoopInput struct {
	Label     string `json:"label"`
	Every     string `json:"every"`
	Cmd       string `json:"cmd"`
	NotifyLLM bool   `json:"notify_llm"`
	Source    string `json:"source"`
	Level     string `json:"level"`
	Timeout   string `json:"timeout"`
}

// SetupNotifyInput mirrors hotline source add without returning the minted key.
type SetupNotifyInput struct {
	Label      string `json:"label"`
	Cap        string `json:"cap"`
	Burst      int    `json:"burst"`
	RefillMins int    `json:"refill_mins"`
	ChatID     string `json:"chat_id"`
}

func handleSetupLoop(in SetupLoopInput, stateRoot string) (string, bool) {
	if strings.TrimSpace(in.Label) == "" || strings.TrimSpace(in.Every) == "" || strings.TrimSpace(in.Cmd) == "" {
		return "setup_loop failed: label, every, and cmd are required", true
	}
	res, err := loop.Setup(stateRoot, loop.Loop{
		Label:     in.Label,
		Every:     in.Every,
		Cmd:       in.Cmd,
		NotifyLLM: in.NotifyLLM,
		Source:    in.Source,
		Level:     in.Level,
		Timeout:   in.Timeout,
	}, strings.TrimSpace(in.Source) != "", false, time.Now())
	if err != nil {
		return "setup_loop failed: " + err.Error(), true
	}
	stored := res.Loop
	if stored.Approved {
		return fmt.Sprintf("Created live loop %q every %s. Operator notified.", stored.Label, stored.Every), false
	}
	return fmt.Sprintf("Created pending loop %q every %s. It will not run until the operator approves it with `hotline loop approve %s`.", stored.Label, stored.Every, stored.Label), false
}

func handleSetupNotify(in SetupNotifyInput, sourcesPath string) (string, bool) {
	cap, err := notify.ParseLevel(in.Cap)
	if err != nil {
		return "setup_notify failed: " + err.Error(), true
	}
	src, err := notify.AddSource(sourcesPath, in.Label, cap, notify.Rate{Burst: in.Burst, RefillMins: in.RefillMins}, in.ChatID, time.Now())
	if err != nil {
		return "setup_notify failed: " + err.Error(), true
	}
	return fmt.Sprintf("Created notify source %q (level cap: %s).", src.Label, src.LevelCap), false
}
