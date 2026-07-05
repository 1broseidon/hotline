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

	"github.com/1broseidon/hotline/internal/config"
	"github.com/1broseidon/hotline/internal/mcpchan"
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
	harness := fs.String("harness", "claude", "coding-agent harness to wire up: claude (default) or opencode")
	if err := fs.Parse(args); err != nil {
		return err
	}

	switch *harness {
	case "claude":
		if *mcpJSON {
			if err := initMCPJSON(botName, *providers, dir, stdout); err != nil {
				return err
			}
		} else if err := initPlugin(botName, *providers, dir, stdout); err != nil {
			return err
		}
		if *voice {
			if err := writeVoiceStarter(dir, stdout); err != nil {
				return err
			}
		}
		fmt.Fprintln(stdout, "Next: hotline start")
	case "opencode":
		// Write the starter voice first (if requested) so the agent definition
		// below can embed it: AgentInstructions reads dir/HOTLINE.md.
		if *voice {
			if err := writeVoiceStarter(dir, stdout); err != nil {
				return err
			}
		}
		if err := initOpenCode(botName, *providers, dir, stdout); err != nil {
			return err
		}
	default:
		return fmt.Errorf("unknown --harness %q (supported: claude, opencode)", *harness)
	}
	return nil
}

// writeVoiceStarter drops a starter HOTLINE.md into dir, never overwriting an
// existing one (mirrors the non-clobber discipline used elsewhere in init).
func writeVoiceStarter(dir string, stdout io.Writer) error {
	voicePath := filepath.Join(dir, "HOTLINE.md")
	if _, err := os.Stat(voicePath); err == nil {
		fmt.Fprintf(stdout, "HOTLINE.md already exists, leaving it alone.\n")
		return nil
	} else if !os.IsNotExist(err) {
		return err
	}
	if err := os.WriteFile(voicePath, []byte(voiceTemplate), 0o644); err != nil {
		return err
	}
	fmt.Fprintf(stdout, "Wrote %s (starter voice, edit to taste).\n", voicePath)
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

// managedAgentMarker identifies the hotline-managed OpenCode agent file. It
// leads the agent body (opencode requires YAML frontmatter at the very top of
// the file, so the marker cannot precede it). `hotline init` regenerates an
// agent file that carries this marker and refuses to clobber a hotline.md
// without it — that file is the user's own agent, not ours.
const managedAgentMarker = "<!-- hotline-managed agent: regenerated by 'hotline init'; edit voice via HOTLINE.md -->"

// hotlineAgentDescription is the frontmatter description of the scaffolded
// agent (a plain scalar — no colon, no em dash, so YAML parses it cleanly).
const hotlineAgentDescription = "hotline texting agent - talks like a friend over Telegram and honors the channel safety rules"

// initOpenCode wires the project for the OpenCode harness. It scaffolds a
// dedicated primary agent — .opencode/agents/hotline.md — whose entire system
// prompt is hotline's mechanics+voice, and merges opencode.json with the hotline
// MCP server (coupled to that agent via HOTLINE_OPENCODE_AGENT) and a default
// permission block. Both writes are non-destructive: a user's own hotline.md is
// never clobbered, and every other opencode.json key is preserved.
func initOpenCode(botName, providers, dir string, stdout io.Writer) error {
	// Reuse the runtime config resolution so the memory/transcript line baked
	// into the agent prompt matches exactly what `hotline run` uses for this bot.
	cfg, err := config.Load(botName)
	if err != nil {
		return err
	}

	// Voice: the repo's HOTLINE.md if present, else the built-in voice.
	voice := ""
	if data, err := os.ReadFile(filepath.Join(dir, "HOTLINE.md")); err == nil {
		voice = strings.TrimSpace(string(data))
	} else if !os.IsNotExist(err) {
		return err
	}

	body := mcpchan.AgentInstructions(cfg.TranscriptFile, voice)
	agentPath := filepath.Join(dir, ".opencode", "agents", "hotline.md")
	action, err := writeHotlineAgent(agentPath, body)
	if err != nil {
		return err
	}
	switch action {
	case "skip":
		fmt.Fprintf(stdout, "Left %s alone: it exists without the hotline-managed marker (your own agent). Delete it to regenerate.\n", agentPath)
	default:
		fmt.Fprintf(stdout, "%s %s (mode: primary; system prompt = hotline mechanics + voice).\n", action, agentPath)
	}

	ocPath := filepath.Join(dir, "opencode.json")
	updated, err := mergeOpenCodeConfig(ocPath, botName, providers)
	if err != nil {
		return err
	}
	if err := os.WriteFile(ocPath, updated, 0o644); err != nil {
		return err
	}
	fmt.Fprintf(stdout, "Wrote %s (hotline MCP server pinned to the hotline agent + permission defaults).\n", ocPath)

	fmt.Fprintln(stdout, "Next: opencode serve, then pair the bot (hotline pair <code>) and message it.")
	return nil
}

// hotlineAgentFile renders the full .opencode/agents/hotline.md content: YAML
// frontmatter (mode: primary + description) at the very top, then the managed
// marker and the mechanics+voice body.
func hotlineAgentFile(body string) string {
	return "---\n" +
		"mode: primary\n" +
		"description: " + hotlineAgentDescription + "\n" +
		"---\n\n" +
		managedAgentMarker + "\n\n" +
		body + "\n"
}

// writeHotlineAgent writes the scaffolded agent to path, creating parent
// directories. Absent file -> create ("Wrote"). Existing file carrying the
// managed marker -> regenerate in place ("Regenerated"). Existing file WITHOUT
// the marker -> leave it untouched and return "skip" (it is the user's own
// hotline agent, not ours to clobber). It never touches build.md or AGENTS.md.
func writeHotlineAgent(path, body string) (string, error) {
	content := hotlineAgentFile(body)

	data, err := os.ReadFile(path)
	if os.IsNotExist(err) {
		if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
			return "", err
		}
		if err := os.WriteFile(path, []byte(content), 0o644); err != nil {
			return "", err
		}
		return "Wrote", nil
	}
	if err != nil {
		return "", err
	}
	if !strings.Contains(string(data), managedAgentMarker) {
		return "skip", nil
	}
	if err := os.WriteFile(path, []byte(content), 0o644); err != nil {
		return "", err
	}
	return "Regenerated", nil
}

// mergeOpenCodeConfig returns the new opencode.json content with the hotline
// MCP server entry set and a default permission block ensured, preserving every
// other key. A malformed existing file is an error, never clobbered — the same
// discipline as mergeMCPConfig.
//
// Note: opencode's permission model is coarser than Claude Code's. There is no
// per-MCP-tool read-allow equivalent to hotline's safeAutoAllowTools (which
// pre-approves Read/Grep/Glob/… individually on the Claude path), so the
// read-only pre-approval here is a blunt category-level default: webfetch and
// external_directory allowed, edits and shell commands still gated.
func mergeOpenCodeConfig(path, botName, providers string) ([]byte, error) {
	root := map[string]any{}
	if data, err := os.ReadFile(path); err == nil {
		if err := json.Unmarshal(data, &root); err != nil {
			return nil, fmt.Errorf("%s exists but is not valid JSON (%v); fix or remove it, nothing was changed", path, err)
		}
	} else if !os.IsNotExist(err) {
		return nil, err
	}

	mcp, _ := root["mcp"].(map[string]any)
	if mcp == nil {
		mcp = map[string]any{}
	}

	// Preserve any extra keys on an existing hotline entry (e.g. enabled); only
	// (re)set the ones init owns.
	entry, _ := mcp["hotline"].(map[string]any)
	if entry == nil {
		entry = map[string]any{}
	}
	entry["type"] = "local"
	entry["command"] = hotlineOpenCodeCommand(botName)
	env, _ := entry["environment"].(map[string]any)
	if env == nil {
		env = map[string]any{}
	}
	env["HOTLINE_HARNESS"] = "opencode"
	// Couple the relay to the scaffolded agent: every inbound turn runs the
	// dedicated hotline agent (see run_opencode.go / opencode.Link).
	env["HOTLINE_OPENCODE_AGENT"] = "hotline"
	if providers != "" {
		env["HOTLINE_PROVIDERS"] = providers
	}
	entry["environment"] = env
	mcp["hotline"] = entry
	root["mcp"] = mcp

	// Default permission block: pre-approve routine reads, still gate writes and
	// commands. Leave an existing block untouched — that's the user's policy.
	if _, ok := root["permission"]; !ok {
		root["permission"] = map[string]any{
			"edit":               "ask",
			"bash":               "ask",
			"webfetch":           "allow",
			"external_directory": "allow",
		}
	}

	var buf bytes.Buffer
	enc := json.NewEncoder(&buf)
	enc.SetIndent("", "  ")
	if err := enc.Encode(root); err != nil {
		return nil, err
	}
	return buf.Bytes(), nil
}

// hotlineOpenCodeCommand is the local-MCP launch command opencode.json uses to
// start hotline: the binary plus "run" (and --bot for a named bot).
func hotlineOpenCodeCommand(botName string) []any {
	cmd := []any{"hotline", "run"}
	if botName != "" {
		cmd = append(cmd, "--bot", botName)
	}
	return cmd
}
