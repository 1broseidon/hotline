package main

import (
	"bytes"
	"encoding/json"
	"flag"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"strings"
)

const voiceTemplate = `<!-- HOTLINE.md: per-repo voice override for the hotline channel.
     Replaces the default persona. Tone only: tools, access rules, and the
     injection stance stay compiled in. Read at startup; edit + restart. -->
Terse and friendly. One bubble unless the answer genuinely needs two.
`

// cmdInit sets up the hotline channel for the current project. The default
// path installs the official Claude Code plugin (marketplace 1broseidon/hotline)
// and enables it project-wide; --mcp-json instead writes (or merges into) the
// project's .mcp.json, registering hotline as a raw MCP server. --providers
// sets HOTLINE_PROVIDERS (project settings env block on the plugin path, the
// server entry's env block on the raw path), --bot selects a named bot, and
// --voice drops a starter HOTLINE.md.
func cmdInit(botName string, args []string, dir string, stdout io.Writer) error {
	fs := flag.NewFlagSet("init", flag.ContinueOnError)
	fs.SetOutput(stdout)
	providers := fs.String("providers", "", "comma-separated provider list (sets HOTLINE_PROVIDERS)")
	voice := fs.Bool("voice", false, "also write a starter HOTLINE.md")
	mcpJSON := fs.Bool("mcp-json", false, "register a raw MCP server in .mcp.json instead of installing the plugin")
	if err := fs.Parse(args); err != nil {
		return err
	}

	if *mcpJSON {
		if err := initMCPJSON(botName, *providers, dir, stdout); err != nil {
			return err
		}
	} else if err := initPlugin(botName, *providers, dir, stdout); err != nil {
		return err
	}

	if *voice {
		voicePath := filepath.Join(dir, "HOTLINE.md")
		if _, err := os.Stat(voicePath); err == nil {
			fmt.Fprintf(stdout, "HOTLINE.md already exists, leaving it alone.\n")
		} else if os.IsNotExist(err) {
			if err := os.WriteFile(voicePath, []byte(voiceTemplate), 0o644); err != nil {
				return err
			}
			fmt.Fprintf(stdout, "Wrote %s (starter voice, edit to taste).\n", voicePath)
		} else {
			return err
		}
	}

	fmt.Fprintln(stdout, "Next: hotline start")
	return nil
}

// initPlugin is the default init path: make sure the hotline marketplace and
// plugin are installed and enabled for this project, and persist per-project
// env (providers, bot) in .claude/settings.json so the plugin-shipped server
// inherits it.
func initPlugin(botName, providers, dir string, stdout io.Writer) error {
	if pluginPathActive(dir) {
		fmt.Fprintf(stdout, "Plugin %s already enabled for this project.\n", pluginID)
	} else {
		if out, err := runClaude(dir, "plugin", "marketplace", "add", marketplaceRepo); err != nil {
			return fmt.Errorf("adding the hotline marketplace: %v\n%s", err, out)
		}
		if out, err := runClaude(dir, "plugin", "install", pluginID, "-s", "project"); err != nil {
			return fmt.Errorf("installing the hotline plugin: %v\n%s", err, out)
		}
		fmt.Fprintf(stdout, "Installed plugin %s (enabled in .claude/settings.json).\n", pluginID)
	}

	env := map[string]string{}
	if providers != "" {
		env["HOTLINE_PROVIDERS"] = providers
	}
	if botName != "" {
		env["HOTLINE_BOT"] = botName
	}
	if len(env) > 0 {
		if err := writeProjectSettingsEnv(dir, env); err != nil {
			return err
		}
		fmt.Fprintf(stdout, "Wrote env block to %s.\n", filepath.Join(dir, ".claude", "settings.json"))
	}
	// Pre-approve routine read-only tools so a remote texting user isn't buzzed for
	// every navigation step; edits and commands still prompt.
	if added, err := mergeProjectSettingsAllow(dir, safeAutoAllowTools); err != nil {
		return err
	} else if len(added) > 0 {
		fmt.Fprintf(stdout, "Pre-approved read-only tools (%s) in .claude/settings.json — edits and commands still prompt.\n", strings.Join(added, ", "))
	}
	return nil
}

// initMCPJSON is the raw fallback path (the pre-plugin behavior): register
// hotline as a plain MCP server in the project's .mcp.json.
func initMCPJSON(botName, providers, dir string, stdout io.Writer) error {
	mcpPath := filepath.Join(dir, ".mcp.json")
	updated, err := mergeMCPConfig(mcpPath, botName, providers)
	if err != nil {
		return err
	}
	if err := os.WriteFile(mcpPath, updated, 0o644); err != nil {
		return err
	}
	fmt.Fprintf(stdout, "Wrote %s (hotline registered as an MCP server).\n", mcpPath)
	return nil
}

// mergeMCPConfig returns the new .mcp.json content with the hotline server
// entry set, preserving every other server and every unrelated top-level key.
// A malformed existing file is an error, never clobbered.
func mergeMCPConfig(path, botName, providers string) ([]byte, error) {
	root := map[string]any{}
	if data, err := os.ReadFile(path); err == nil {
		if err := json.Unmarshal(data, &root); err != nil {
			return nil, fmt.Errorf("%s exists but is not valid JSON (%v); fix or remove it, nothing was changed", path, err)
		}
	} else if !os.IsNotExist(err) {
		return nil, err
	}

	servers, _ := root["mcpServers"].(map[string]any)
	if servers == nil {
		servers = map[string]any{}
	}

	// If an entry already runs hotline (any name), update it in place; keep
	// unrelated env keys it may carry. Otherwise add one named "hotline".
	name := "hotline"
	var env map[string]any
	for k, v := range servers {
		if m, ok := v.(map[string]any); ok {
			if cmd, _ := m["command"].(string); cmd == "hotline" || k == "hotline" {
				name = k
				env, _ = m["env"].(map[string]any)
				break
			}
		}
	}

	entry := map[string]any{
		"command": "hotline",
		"args":    hotlineRunArgs(botName),
	}
	if providers != "" {
		if env == nil {
			env = map[string]any{}
		}
		env["HOTLINE_PROVIDERS"] = providers
	}
	if len(env) > 0 {
		entry["env"] = env
	}
	servers[name] = entry
	root["mcpServers"] = servers

	var buf bytes.Buffer
	enc := json.NewEncoder(&buf)
	enc.SetIndent("", "  ")
	if err := enc.Encode(root); err != nil {
		return nil, err
	}
	return buf.Bytes(), nil
}

func hotlineRunArgs(botName string) []any {
	args := []any{"run"}
	if botName != "" {
		args = append(args, "--bot", botName)
	}
	return args
}
