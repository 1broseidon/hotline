package main

import (
	"fmt"
	"runtime/debug"
)

// Build-time version info. Set via -ldflags "-X main.version=..." by
// goreleaser/CI. Defaults below are used for `go install` / dev builds,
// where we fall back to debug.ReadBuildInfo() to surface the module
// version and VCS stamp.
var (
	version = "dev"
	commit  = ""
	date    = ""
)

// versionInfo resolves version/commit/date, preferring linker-provided
// values and falling back to runtime build info so
// `go install github.com/1broseidon/hotline@vX.Y.Z` still reports correctly.
func versionInfo() (v, c, d string) {
	v, c, d = version, commit, date
	if v != "dev" && v != "" {
		return
	}
	bi, ok := debug.ReadBuildInfo()
	if !ok {
		return
	}
	if bi.Main.Version != "" && bi.Main.Version != "(devel)" {
		v = bi.Main.Version
	}
	for _, s := range bi.Settings {
		switch s.Key {
		case "vcs.revision":
			if c == "" {
				c = s.Value
			}
		case "vcs.time":
			if d == "" {
				d = s.Value
			}
		}
	}
	return
}

func cmdVersion() {
	v, c, d := versionInfo()
	out := "hotline " + v
	if c != "" {
		short := c
		if len(short) > 7 {
			short = short[:7]
		}
		out += fmt.Sprintf(" (%s)", short)
	}
	if d != "" {
		out += " " + d
	}
	fmt.Println(out)
}
