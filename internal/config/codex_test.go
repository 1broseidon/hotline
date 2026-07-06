package config

import (
	"os"
	"path/filepath"
	"testing"
)

func TestLoadCodexDefaults(t *testing.T) {
	dir := withState(t)
	cwd := t.TempDir()
	oldwd, err := os.Getwd()
	if err != nil {
		t.Fatal(err)
	}
	if err := os.Chdir(cwd); err != nil {
		t.Fatal(err)
	}
	defer os.Chdir(oldwd)

	c, err := LoadCodex("")
	if err != nil {
		t.Fatal(err)
	}
	if c.CWD != cwd {
		t.Fatalf("CWD %q, want %q", c.CWD, cwd)
	}
	if c.ThreadID != "" {
		t.Fatalf("ThreadID %q, want empty", c.ThreadID)
	}
	if c.ThreadFile != filepath.Join(dir, "codex-thread") {
		t.Fatalf("ThreadFile %q", c.ThreadFile)
	}
	if c.ApprovalPolicy != DefaultCodexApprovalPolicy || c.Sandbox != DefaultCodexSandbox {
		t.Fatalf("defaults %+v", c)
	}
}

func TestLoadCodexNamedBotThreadFile(t *testing.T) {
	dir := withState(t)
	c, err := LoadCodex("work")
	if err != nil {
		t.Fatal(err)
	}
	if c.ThreadFile != filepath.Join(dir, "bots", "work", "codex-thread") {
		t.Fatalf("ThreadFile %q", c.ThreadFile)
	}
}

func TestLoadCodexFromEnv(t *testing.T) {
	withState(t)
	cwd := t.TempDir()
	t.Setenv("HOTLINE_CODEX_CWD", cwd)
	t.Setenv("HOTLINE_CODEX_THREAD_ID", "thread_env")
	t.Setenv("HOTLINE_CODEX_APPROVAL_POLICY", "on-request")
	t.Setenv("HOTLINE_CODEX_SANDBOX", "read-only")

	c, err := LoadCodex("")
	if err != nil {
		t.Fatal(err)
	}
	if c.CWD != cwd || c.ThreadID != "thread_env" || c.ApprovalPolicy != "on-request" || c.Sandbox != "read-only" {
		t.Fatalf("got %+v", c)
	}
}

func TestLoadCodexFromDotEnv(t *testing.T) {
	dir := withState(t)
	cwd := t.TempDir()
	env := "HOTLINE_CODEX_CWD=" + cwd + "\nHOTLINE_CODEX_THREAD_ID=thread_dot\nHOTLINE_CODEX_SANDBOX=danger-full-access\n"
	if err := os.WriteFile(filepath.Join(dir, ".env"), []byte(env), 0o600); err != nil {
		t.Fatal(err)
	}
	c, err := LoadCodex("")
	if err != nil {
		t.Fatal(err)
	}
	if c.CWD != cwd || c.ThreadID != "thread_dot" || c.Sandbox != "danger-full-access" {
		t.Fatalf("got %+v", c)
	}
}
