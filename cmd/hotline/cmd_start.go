package main

import (
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"time"

	"github.com/1broseidon/hotline/internal/config"
	"github.com/1broseidon/hotline/internal/mcpchan"
)

// signalCheck probes the signal-cli daemon; swapped out in tests.
var signalCheck = func(daemonURL string) error {
	client := &http.Client{Timeout: 2 * time.Second}
	resp, err := client.Get(daemonURL + "/api/v1/check")
	if err != nil {
		return err
	}
	resp.Body.Close()
	return nil
}

// routeStartToUp is the command seam used by the deprecated start alias.
var routeStartToUp = cmdUp

// cmdStart is the compatibility alias for the unified launcher. `hotline up`
// is attached by default, so the old verb can route without maintaining a
// second launch path.
func cmdStart(botName string, botExplicit bool, args, passthrough []string, dir string, stdout, stderr io.Writer) error {
	fmt.Fprintln(stderr, "hotline: `hotline start` is deprecated; use `hotline up` (foreground by default)")
	return routeStartToUp(botName, botExplicit, args, passthrough, dir, stdout, stderr)
}

// channelArgs picks how the channel is handed to claude. A raw hotline entry
// in the project's .mcp.json takes the dev-channel flag (the only form claude
// accepts for plain servers). On the plugin path the safe --channels switch is
// used when hotline is on Claude's approved channels allowlist; until then the
// dev-channel flag registers the same plugin channel.
func channelArgs(dir string, stderr io.Writer) []string {
	if serverName, found := mcpServerName(filepath.Join(dir, ".mcp.json")); found {
		fmt.Fprintf(stderr, "hotline: raw .mcp.json server — using the dev-channel flag; `hotline init` sets up the plugin path instead\n")
		return []string{"--dangerously-load-development-channels", "server:" + serverName}
	}
	if !pluginPathActive(dir) {
		fmt.Fprintf(stderr, "hotline: warning: hotline is not set up in %s — run `hotline init` first or claude won't see the channel\n", dir)
		return nil
	}
	if channelAllowlisted() {
		return []string{"--channels", channelRef}
	}
	fmt.Fprintf(stderr, "hotline: %s is not on Claude's approved channels list yet — using the dev-channel flag (switches to --channels automatically once approved)\n", pluginID)
	return []string{"--dangerously-load-development-channels", channelRef}
}

// mcpServerName reads .mcp.json and returns the name of the entry whose
// command is hotline, defaulting to "hotline". found reports whether a usable
// .mcp.json with a hotline entry exists.
func mcpServerName(path string) (name string, found bool) {
	name, _, _, _, found, _ = mcpServerEntry(path)
	return name, found
}

// mcpServerEntry reads .mcp.json and returns the hotline entry's name, env
// block, and explicit --bot argument. Claude applies the env block on top of
// the inherited environment and invokes the recorded args, so both surfaces
// participate in box identity (see adoptMCPServerEnv).
func mcpServerEntry(path string) (name string, env map[string]string, botName string, botSet, found bool, err error) {
	name = "hotline"
	data, err := os.ReadFile(path)
	if err != nil {
		if os.IsNotExist(err) {
			return name, nil, "", false, false, nil
		}
		return name, nil, "", false, false, fmt.Errorf("reading %s: %w", path, err)
	}
	var root struct {
		MCPServers map[string]struct {
			Command string            `json:"command"`
			Args    []string          `json:"args"`
			Env     map[string]string `json:"env"`
		} `json:"mcpServers"`
	}
	if err := json.Unmarshal(data, &root); err != nil {
		return name, nil, "", false, false, fmt.Errorf("parsing %s: %w", path, err)
	}

	keys := make([]string, 0, len(root.MCPServers))
	for key := range root.MCPServers {
		keys = append(keys, key)
	}
	sort.Strings(keys)
	var matches []string
	for _, key := range keys {
		entry := root.MCPServers[key]
		if filepath.Base(strings.TrimSpace(entry.Command)) == "hotline" {
			matches = append(matches, key)
			continue
		}
		if key == "hotline" {
			return name, nil, "", false, false, fmt.Errorf("%s has an entry named %q whose command is %q, not hotline", path, key, entry.Command)
		}
	}
	if len(matches) == 0 {
		return name, nil, "", false, false, nil
	}
	if len(matches) > 1 {
		return name, nil, "", false, false, fmt.Errorf("%s has multiple hotline MCP entries (%s); keep exactly one before running a box-scoped command", path, strings.Join(matches, ", "))
	}

	name = matches[0]
	entry := root.MCPServers[name]
	head, _ := splitPassthrough(entry.Args)
	botName, _, botSet = extractBotName(head)
	return name, entry.Env, botName, botSet, true, nil
}

// warnVoiceOverflow is the launch pre-flight for the capped Claude
// instruction path: it resolves the voice exactly like the MCP server will
// (mcpchan.LoadVoice) and, when the assembled instruction block would hit the
// budget and cut the voice, prints one clearly visible warning line. Purely
// advisory — it never blocks the launch.
func warnVoiceOverflow(botName string, stderr io.Writer) {
	stateRoot, err := config.StateRoot()
	if err != nil {
		return
	}
	voice := mcpchan.LoadVoice(stateRoot)
	if voice == "" {
		return
	}
	var provs []string
	if specs, err := config.Providers(botName); err == nil {
		for _, spec := range specs {
			provs = append(provs, spec.Name())
		}
	}
	// A representative transcript path: same shape and tier the server bakes in.
	transcriptPath := filepath.Join(stateRoot, "transcript.jsonl")
	kept, total := mcpchan.VoiceFit(transcriptPath, voice, provs...)
	if kept < total {
		fmt.Fprintf(stderr, "hotline: HOTLINE.md is %d bytes but only %d fit the instruction budget — trailing content will be dropped\n", total, kept)
	}
}

// warnMissingCreds checks each configured provider for its credential and the
// signal daemon for reachability, warning without blocking.
func warnMissingCreds(botName string, stderr io.Writer) {
	specs, err := config.Providers(botName)
	if err != nil {
		fmt.Fprintf(stderr, "hotline: warning: %v\n", err)
		return
	}
	for _, spec := range specs {
		switch spec.Kind {
		case "telegram":
			cfg, err := config.Load(spec.Instance)
			if err == nil && cfg.Token == "" {
				fmt.Fprintf(stderr, "hotline: warning: no telegram token for %s — run `hotline setup`\n", spec.Name())
			}
		case "discord":
			cfg, err := config.LoadDiscord(spec.Instance)
			if err == nil && cfg.Token == "" {
				fmt.Fprintf(stderr, "hotline: warning: no discord token for %s — run `hotline setup --discord-token …`\n", spec.Name())
			}
		case "signal":
			cfg, err := config.LoadSignal(spec.Instance)
			if err != nil {
				continue
			}
			if cfg.SignalAccount == "" {
				fmt.Fprintf(stderr, "hotline: warning: no signal account for %s — run `hotline setup --signal-account +…`\n", spec.Name())
			} else if err := signalCheck(cfg.SignalDaemonURL); err != nil {
				fmt.Fprintf(stderr, "hotline: warning: signal daemon not reachable at %s — start it with `signal-cli -a %s daemon --http`\n", cfg.SignalDaemonURL, cfg.SignalAccount)
			}
		}
	}
}
