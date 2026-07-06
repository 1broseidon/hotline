package config

import (
	"fmt"
	"os"
	"path/filepath"
	"strings"
)

// DefaultCodexApprovalPolicy is the app-server approval mode verified to emit
// review RPCs for sandbox escapes in codex-cli 0.142.5.
const DefaultCodexApprovalPolicy = "untrusted"

// DefaultCodexSandbox is the app-server sandbox mode verified with
// DefaultCodexApprovalPolicy in codex-cli 0.142.5.
const DefaultCodexSandbox = "workspace-write"

// CodexConfig holds settings for HOTLINE_HARNESS=codex. hotline owns the
// `codex app-server` subprocess and talks JSON-RPC over stdio; one persisted
// thread id is kept per hotline instance so a restart can attempt
// thread/resume before falling back to a fresh thread.
type CodexConfig struct {
	// CWD is the working directory handed to app-server and used as the runtime
	// workspace root. Defaults to the process cwd.
	CWD string
	// ThreadID pins a Codex thread. Empty means read/write ThreadFile.
	ThreadID string
	// ThreadFile stores the auto-created thread id when ThreadID is empty.
	ThreadFile string
	// ApprovalPolicy is thread/start approvalPolicy.
	ApprovalPolicy string
	// Sandbox is thread/start sandbox.
	Sandbox string
}

// LoadCodex resolves Codex harness settings from the real environment merged
// with the shared base-dir .env. Environment keys:
// HOTLINE_CODEX_CWD, HOTLINE_CODEX_THREAD_ID, HOTLINE_CODEX_APPROVAL_POLICY,
// HOTLINE_CODEX_SANDBOX.
func LoadCodex(botName string) (*CodexConfig, error) {
	baseDir, err := resolveStateDir()
	if err != nil {
		return nil, err
	}
	envFile := filepath.Join(baseDir, ".env")
	dotEnv, err := loadDotEnv(envFile)
	if err != nil {
		return nil, fmt.Errorf("reading %s: %w", envFile, err)
	}

	cwd := mergedEnv("HOTLINE_CODEX_CWD", dotEnv)
	if strings.TrimSpace(cwd) == "" {
		cwd, err = os.Getwd()
		if err != nil {
			return nil, err
		}
	}
	if cwd, err = filepath.Abs(cwd); err != nil {
		return nil, err
	}

	stateDir := baseDir
	if botName != "" {
		stateDir = filepath.Join(baseDir, "bots", botName)
	}

	c := &CodexConfig{
		CWD:            cwd,
		ThreadID:       strings.TrimSpace(mergedEnv("HOTLINE_CODEX_THREAD_ID", dotEnv)),
		ThreadFile:     filepath.Join(stateDir, "codex-thread"),
		ApprovalPolicy: strings.TrimSpace(mergedEnv("HOTLINE_CODEX_APPROVAL_POLICY", dotEnv)),
		Sandbox:        strings.TrimSpace(mergedEnv("HOTLINE_CODEX_SANDBOX", dotEnv)),
	}
	if c.ApprovalPolicy == "" {
		c.ApprovalPolicy = DefaultCodexApprovalPolicy
	}
	if c.Sandbox == "" {
		c.Sandbox = DefaultCodexSandbox
	}
	return c, nil
}
