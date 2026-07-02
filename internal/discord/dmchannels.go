package discord

import (
	"encoding/json"
	"os"
	"path/filepath"
	"sync"
)

// Telegram DMs have chat_id == user_id, so its outbound gate checks the
// allowlist directly. Discord DMs live in their own channel (a snowflake
// unrelated to the user), so the adapter persists which DM channels belong to
// allowlisted users: the handler records the mapping whenever a gate-approved
// DM arrives, and the outbound tools consult it before sending. State survives
// restarts in dm_channels.json next to access.json.

var dmMu sync.Mutex

// dmChannelsFile returns the registry path for a state dir.
func dmChannelsFile(stateDir string) string {
	return filepath.Join(stateDir, "dm_channels.json")
}

// loadDMChannels reads the channelID -> userID registry; missing or corrupt
// files yield an empty map.
func loadDMChannels(stateDir string) map[string]string {
	out := map[string]string{}
	raw, err := os.ReadFile(dmChannelsFile(stateDir))
	if err != nil {
		return out
	}
	_ = json.Unmarshal(raw, &out)
	return out
}

// recordDMChannel persists channelID -> userID (idempotent).
func recordDMChannel(stateDir, channelID, userID string) {
	dmMu.Lock()
	defer dmMu.Unlock()
	m := loadDMChannels(stateDir)
	if m[channelID] == userID {
		return
	}
	m[channelID] = userID
	data, err := json.MarshalIndent(m, "", "  ")
	if err != nil {
		return
	}
	tmp := dmChannelsFile(stateDir) + ".tmp"
	if err := os.WriteFile(tmp, append(data, '\n'), 0o600); err != nil {
		return
	}
	_ = os.Rename(tmp, dmChannelsFile(stateDir))
}

// dmChannelUser returns the recorded owner of a DM channel, or "".
func dmChannelUser(stateDir, channelID string) string {
	dmMu.Lock()
	defer dmMu.Unlock()
	return loadDMChannels(stateDir)[channelID]
}
