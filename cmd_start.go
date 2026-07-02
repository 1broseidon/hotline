package main

import (
	"encoding/json"
	"flag"
	"fmt"
	"io"
	"net/http"
	"os"
	"os/exec"
	"path/filepath"
	"syscall"
	"time"

	"github.com/1broseidon/hotline/internal/config"
)

// execProcess is the seam start goes through to launch claude. The default
// replaces the hotline process via exec so signals and the tty pass straight
// through; if exec fails it falls back to spawn+wait.
var execProcess = func(bin string, argv []string, env []string) error {
	if err := syscall.Exec(bin, argv, env); err == nil {
		return nil
	}
	cmd := exec.Command(bin, argv[1:]...)
	cmd.Env = env
	cmd.Stdin, cmd.Stdout, cmd.Stderr = os.Stdin, os.Stdout, os.Stderr
	return cmd.Run()
}

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

// cmdStart launches Claude Code with the hotline channel wired up.
// Everything after -- (already split off in main) is passed to claude
// verbatim.
func cmdStart(botName string, args, passthrough []string, dir string, stdout, stderr io.Writer) error {
	fs := flag.NewFlagSet("start", flag.ContinueOnError)
	fs.SetOutput(stdout)
	providers := fs.String("providers", "", "comma-separated provider list (exported as HOTLINE_PROVIDERS)")
	if err := fs.Parse(args); err != nil {
		return err
	}

	if *providers != "" {
		os.Setenv("HOTLINE_PROVIDERS", *providers)
	}
	if botName != "" {
		os.Setenv("HOTLINE_BOT", botName)
	}

	// Preflight. Only a missing claude binary blocks; the rest warns.
	bin, err := exec.LookPath("claude")
	if err != nil {
		return fmt.Errorf("claude not found on PATH. Install Claude Code first: https://claude.com/claude-code")
	}

	serverName, found := mcpServerName(filepath.Join(dir, ".mcp.json"))
	if !found {
		fmt.Fprintf(stderr, "hotline: warning: no .mcp.json in %s — run `hotline init` first or claude won't see the channel\n", dir)
	}

	warnMissingCreds(botName, stderr)

	argv := append([]string{"claude", "--dangerously-load-development-channels", "server:" + serverName}, passthrough...)
	return execProcess(bin, argv, os.Environ())
}

// mcpServerName reads .mcp.json and returns the name of the entry whose
// command is hotline, defaulting to "hotline". found reports whether a usable
// .mcp.json with a hotline entry exists.
func mcpServerName(path string) (name string, found bool) {
	name = "hotline"
	data, err := os.ReadFile(path)
	if err != nil {
		return name, false
	}
	var root struct {
		MCPServers map[string]struct {
			Command string `json:"command"`
		} `json:"mcpServers"`
	}
	if err := json.Unmarshal(data, &root); err != nil {
		return name, false
	}
	if _, ok := root.MCPServers["hotline"]; ok {
		return "hotline", true
	}
	for k, v := range root.MCPServers {
		if v.Command == "hotline" {
			return k, true
		}
	}
	return name, false
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
