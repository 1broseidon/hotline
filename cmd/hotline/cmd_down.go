package main

import (
	"flag"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"strings"
	"syscall"
	"time"

	"github.com/1broseidon/hotline/internal/config"
	"github.com/1broseidon/hotline/internal/supervise"
)

// cmdDown stops a running supervisor: SIGTERM (its shutdown path stops the
// harness gracefully first), then wait for the flock to free.
//
// Target resolution mirrors `hotline up` run from the same directory: the box
// is whatever the ambient environment resolves to after resolveInvocation has
// adopted this project's .mcp.json identity. Before signalling, down announces
// exactly what it is about to stop (box, root, supervisor/harness pids) and, as
// a safety net, refuses to SIGTERM the default box when the current directory
// plainly belongs to a *different* box the shell never selected — the shape of
// the 2026-07-21 incident where `hotline down`, run from a plugin-configured
// box's project dir with that box's HOTLINE_STATE_DIR absent from the shell,
// killed the unrelated default box (a long-running daily-driver agent).
//
// Escape hatches, all bypassing the guard: --state-dir <box-root> targets a box
// root directly (same meaning as `hotline tail --state-dir`), --force stops the
// resolved box as-is, and an explicit --bot is taken as a deliberate selection.
func cmdDown(botName string, botExplicit bool, args []string, dir string, stdout, stderr io.Writer) error {
	fs := flag.NewFlagSet("down", flag.ContinueOnError)
	fs.SetOutput(stderr)
	stateDir := fs.String("state-dir", "", "stop the box whose root is <dir> directly, bypassing project/env resolution (mirrors `hotline tail --state-dir`)")
	force := fs.Bool("force", false, "stop the resolved box even when this directory names a different one")
	if err := fs.Parse(args); err != nil {
		return err
	}

	boxKey := "(direct)"
	boxRoot := strings.TrimSpace(*stateDir)
	if boxRoot == "" {
		box, err := config.ResolveBox(botName)
		if err != nil {
			return err
		}
		boxRoot, boxKey = box.Root, box.Key

		// Safety guard: only when the operator neither named a box explicitly nor
		// forced the resolved one. A conflicting cwd/env pair here is the incident.
		if !botExplicit && !*force {
			switch decl, _, mismatch, err := projectBoxMismatch(dir); {
			case err != nil:
				// A malformed project config must not wedge a legitimate down; note
				// it and fall through to the env-resolved target.
				fmt.Fprintf(stderr, "hotline: warning: reading project box config: %v\n", err)
			case mismatch:
				return downMismatchError(decl, boxRoot)
			}
		}
	}

	supDir := supervise.Dir(boxRoot)
	if !supervise.Running(supDir) {
		fmt.Fprintf(stdout, "hotline: supervisor not running for box %s (%s)\n", boxKey, boxRoot)
		return nil
	}
	st, err := supervise.ReadState(supDir)
	if err != nil || st == nil || st.PID <= 0 {
		return fmt.Errorf("supervisor is running but %s is unreadable — stop it by pid manually (err: %v)", filepath.Join(supDir, "state.json"), err)
	}

	// Announce the exact target before signalling, so an operator who is about to
	// stop the wrong box sees which one it is (box root, where it was launched,
	// what it runs, and both pids) with a chance to Ctrl-C.
	announceDownTarget(stdout, boxKey, boxRoot, st)

	if err := syscall.Kill(st.PID, syscall.SIGTERM); err != nil {
		return fmt.Errorf("signalling supervisor (pid %d): %w", st.PID, err)
	}
	deadline := time.Now().Add(20 * time.Second)
	for supervise.Running(supDir) {
		if time.Now().After(deadline) {
			return fmt.Errorf("supervisor (pid %d) did not stop within 20s — check %s", st.PID, filepath.Join(supDir, supervise.SupervisorLogName))
		}
		time.Sleep(200 * time.Millisecond)
	}
	fmt.Fprintf(stdout, "hotline: supervisor stopped (pid %d)\n", st.PID)
	return nil
}

// announceDownTarget prints the box about to be stopped in a fixed, greppable
// block: box key, box root, the directory it was launched from, its argv, and
// the supervisor + harness pids.
func announceDownTarget(w io.Writer, boxKey, boxRoot string, st *supervise.State) {
	fmt.Fprintf(w, "hotline: stopping box %s\n", boxKey)
	fmt.Fprintf(w, "  box root:       %s\n", boxRoot)
	if st.WorkDir != "" {
		fmt.Fprintf(w, "  launched from:  %s\n", st.WorkDir)
	}
	if len(st.Argv) > 0 {
		fmt.Fprintf(w, "  running:        %s\n", strings.Join(st.Argv, " "))
	}
	fmt.Fprintf(w, "  supervisor pid: %d\n", st.PID)
	if st.HarnessPID > 0 {
		fmt.Fprintf(w, "  harness pid:    %d\n", st.HarnessPID)
	} else {
		fmt.Fprintln(w, "  harness pid:    (down; supervisor retrying)")
	}
}

// downMismatchError is the refusal an operator sees when the cwd belongs to a
// different box than the shell resolves. It names both candidates and every way
// to proceed, staying inside patterns the CLI already has (HOTLINE_STATE_DIR,
// --state-dir, --force).
func downMismatchError(decl projectBoxDecl, defaultRoot string) error {
	return fmt.Errorf("refusing to stop the wrong box: this directory belongs to a box your shell does not select\n"+
		"  this directory declares:  HOTLINE_STATE_DIR=%s (from %s)\n"+
		"  your shell resolves:      the default box at %s\n"+
		"`hotline down` here would SIGTERM the default box, not this project's box.\n"+
		"to stop THIS project's box:      HOTLINE_STATE_DIR=%s hotline down\n"+
		"                          (or):  hotline down --state-dir %s\n"+
		"to stop the default box anyway:  hotline down --force",
		decl.StateDir, decl.Source, defaultRoot, decl.StateDir, decl.StateDir)
}

// projectBoxDecl is what a directory's project config declares about its hotline
// box, read straight from disk without consulting the ambient environment. It is
// how down/status recover the box a directory belongs to even when the shell
// lacks the HOTLINE_STATE_DIR the box was launched with.
type projectBoxDecl struct {
	Found    bool              // a hotline project declaration was located
	Source   string            // the config file it came from
	StateDir string            // declared base state dir, "" if the declaration sets none
	Env      map[string]string // the full declared HOTLINE_*/TELE_GO_* map
}

// projectBoxMismatch reports whether dir belongs to a hotline box other than the
// one the ambient environment resolves to. It signals only when the shell
// supplied no base state dir (so resolution silently falls back to the default
// box) and the directory's project config names a different base — precisely the
// 2026-07-21 footgun. found is false (nil error) whenever there is nothing to
// warn about, so callers can branch on it directly.
func projectBoxMismatch(dir string) (decl projectBoxDecl, defaultBase string, found bool, err error) {
	// A shell base state dir means the operator (or an adopted .mcp.json) already
	// pinned the box; there is no silent default to protect against.
	if stateDirEnvSet() {
		return projectBoxDecl{}, "", false, nil
	}
	decl, err = projectBoxDeclaration(dir)
	if err != nil {
		return projectBoxDecl{}, "", false, err
	}
	if !decl.Found || decl.StateDir == "" {
		return projectBoxDecl{}, "", false, nil
	}
	base, err := config.StateRoot()
	if err != nil {
		return projectBoxDecl{}, "", false, err
	}
	if absClean(decl.StateDir) == absClean(base) {
		return projectBoxDecl{}, "", false, nil // same box, no ambiguity
	}
	return decl, base, true, nil
}

// projectBoxDeclaration walks dir and its parents for the nearest project-level
// hotline box config and reports which box it names. A raw .mcp.json hotline
// entry wins (it is what `hotline up` adopts); failing that, the Claude Code
// plugin config (.claude/settings.json[.local]) env block is consulted, because
// a plugin-launched box records its HOTLINE_STATE_DIR there and nowhere a bare
// shell would ever see it. The first directory carrying either kind of config
// wins, mirroring adoptProjectMCPServerEnv's nearest-config-wins walk.
func projectBoxDeclaration(dir string) (projectBoxDecl, error) {
	d, err := filepath.Abs(filepath.Clean(dir))
	if err != nil {
		return projectBoxDecl{}, fmt.Errorf("resolving project directory %q: %w", dir, err)
	}
	for {
		mcpPath := filepath.Join(d, ".mcp.json")
		if _, statErr := os.Stat(mcpPath); statErr == nil {
			_, env, _, _, entryFound, err := mcpServerEntry(mcpPath)
			if err != nil {
				return projectBoxDecl{}, err
			}
			if entryFound {
				return boxDeclFromEnv(mcpPath, env), nil
			}
		} else if !os.IsNotExist(statErr) {
			return projectBoxDecl{}, fmt.Errorf("checking %s: %w", mcpPath, statErr)
		}

		for _, name := range []string{"settings.json", "settings.local.json"} {
			env, ok, err := settingsEnvBlock(filepath.Join(d, ".claude", name))
			if err != nil {
				return projectBoxDecl{}, err
			}
			if ok {
				return boxDeclFromEnv(filepath.Join(d, ".claude", name), env), nil
			}
		}

		parent := filepath.Dir(d)
		if parent == d {
			return projectBoxDecl{}, nil
		}
		d = parent
	}
}

// settingsEnvBlock reads the env block of a .claude/settings*.json file. ok is
// true only when that block carries at least one HOTLINE_/TELE_GO_ key, so an
// unrelated settings file (permissions only, CLAUDE_* env only) is not mistaken
// for a hotline box declaration. A missing file is (nil, false, nil).
func settingsEnvBlock(path string) (map[string]string, bool, error) {
	root, err := readJSONMap(path)
	if err != nil {
		return nil, false, err
	}
	raw, _ := root["env"].(map[string]any)
	if len(raw) == 0 {
		return nil, false, nil
	}
	env := make(map[string]string, len(raw))
	for k, v := range raw {
		if s, ok := v.(string); ok {
			env[k] = s
		}
	}
	return env, hotlineEnvKeys(env), nil
}

// boxDeclFromEnv distills a declared env map into a projectBoxDecl, picking the
// base state dir with the same key precedence resolveStateDir uses.
func boxDeclFromEnv(source string, env map[string]string) projectBoxDecl {
	decl := projectBoxDecl{Found: true, Source: source, Env: env}
	for _, k := range []string{"HOTLINE_STATE_DIR", "TELE_GO_STATE_DIR", "TELEGRAM_STATE_DIR"} {
		if v := strings.TrimSpace(env[k]); v != "" {
			decl.StateDir = v
			break
		}
	}
	return decl
}

// hotlineEnvKeys reports whether any key is a hotline/tele-go control variable.
func hotlineEnvKeys(env map[string]string) bool {
	for k := range env {
		if strings.HasPrefix(k, "HOTLINE_") || strings.HasPrefix(k, "TELE_GO_") {
			return true
		}
	}
	return false
}

// stateDirEnvSet reports whether the shell already pins a base state dir through
// any of the three honored variables (an empty value counts as unset).
func stateDirEnvSet() bool {
	for _, k := range []string{"HOTLINE_STATE_DIR", "TELE_GO_STATE_DIR", "TELEGRAM_STATE_DIR"} {
		if os.Getenv(k) != "" {
			return true
		}
	}
	return false
}

// absClean normalizes a path for comparison, tolerating a resolution failure.
func absClean(p string) string {
	if a, err := filepath.Abs(filepath.Clean(p)); err == nil {
		return a
	}
	return filepath.Clean(p)
}
