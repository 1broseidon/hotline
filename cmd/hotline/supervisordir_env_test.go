package main

import (
	"os"
	"testing"

	"github.com/1broseidon/hotline/internal/supervise"
)

// TestMain scrubs HOTLINE_SUPERVISOR_DIR for the whole cmd/hotline test binary.
// Several tests here call config.LoadApp, which reads HOTLINE_SUPERVISOR_DIR
// from the ambient environment (the CLI composition boundary). On a supervised
// operator box that var points at the LIVE supervisor dir and is inherited by
// every descendant shell, so an un-scrubbed `go test ./cmd/hotline/` that then
// drove a relay room rotation would file a real restart.request and bounce the
// operator's live session. Unsetting it once, up front, keeps the suite inert.
// The `hotline up` env-injection tests are unaffected: they build the harness
// env from an explicitly computed supervisor dir (supervise.EnvDir+"="+supDir),
// not from this ambient var.
func TestMain(m *testing.M) {
	os.Unsetenv(supervise.EnvDir)
	os.Exit(m.Run())
}
