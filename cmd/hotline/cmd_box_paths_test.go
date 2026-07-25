package main

import (
	"bytes"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/1broseidon/hotline/internal/jobspool"
	"github.com/1broseidon/hotline/internal/loop"
	"github.com/1broseidon/hotline/internal/notify"
	"github.com/1broseidon/hotline/internal/schedule"
)

func TestNamedBoxCommandsKeepMutableStoresOffSharedRoot(t *testing.T) {
	base := t.TempDir()
	t.Setenv("HOTLINE_STATE_DIR", base)
	t.Setenv("HOTLINE_PROVIDERS", "telegram:Ada")
	t.Setenv("HOTLINE_MC_DIR", "")
	t.Setenv("HOTLINE_MISSION_CONTROL", "")
	t.Setenv("HOTLINE_YOLO", "0")
	t.Setenv("HOTLINE_HARNESS", "claude")
	boxRoot := filepath.Join(base, "bots", "Ada")

	// Schedule CLI mutations use the named box's schedules.json.
	schedulesPath := filepath.Join(boxRoot, "schedules.json")
	sc := sampleDaily("named1")
	if err := schedule.Save(&schedule.Doc{Schedules: []schedule.Schedule{sc}}, schedulesPath); err != nil {
		t.Fatal(err)
	}
	if err := cmdSchedule("", []string{"pause", sc.ID}); err != nil {
		t.Fatal(err)
	}

	// Loop metadata and per-loop execution state share the same box root.
	var out, errout bytes.Buffer
	if err := cmdLoop("", []string{"add", "named-loop", "--every", "1m", "--cmd", "printf hit", "-y"}, &out, &errout); err != nil {
		t.Fatal(err)
	}
	if err := cmdLoop("", []string{"run", "named-loop", "--once"}, &out, &errout); err != nil {
		t.Fatal(err)
	}

	// Notify registry/spool and job spool are box-owned too.
	if err := cmdSource("", []string{"add", "named-source"}, &out); err != nil {
		t.Fatal(err)
	}
	reg, err := notify.LoadRegistry(notify.SourcesPath(boxRoot))
	if err != nil || len(reg.Sources) != 1 {
		t.Fatalf("named notify registry: sources=%v err=%v", reg.Sources, err)
	}
	if code := cmdNotify("", []string{"--source", reg.Sources[0].Key, "named event"}, strings.NewReader(""), &out, &errout); code != exitAccepted {
		t.Fatalf("named notify exit=%d stderr=%s", code, errout.String())
	}
	if code := cmdJob("", []string{"start", "--cookie", "named-job", "--title", "Named job"}, &out, &errout); code != exitAccepted {
		t.Fatalf("named job exit=%d stderr=%s", code, errout.String())
	}

	// Mission Control defaults under the named box, never root mc.
	if code := cmdMission("", []string{"update", "--thread", "named-thread", "--summary", "isolated", "--next", "continue"}, &out, &errout); code != exitAccepted {
		t.Fatalf("named mission exit=%d stderr=%s", code, errout.String())
	}

	for _, path := range []string{
		schedulesPath,
		loop.Path(boxRoot),
		loop.LogPath(boxRoot, "named-loop"),
		notify.SourcesPath(boxRoot),
		notify.SpoolPath(boxRoot),
		jobspool.SpoolPath(boxRoot),
		filepath.Join(boxRoot, "mc", "threads", "named-thread.md"),
	} {
		if _, err := os.Stat(path); err != nil {
			t.Errorf("named box store missing at %s: %v", path, err)
		}
	}
	for _, name := range []string{"schedules.json", "loops.json", "loops", "notify", "jobs", "mc"} {
		if _, err := os.Stat(filepath.Join(base, name)); !os.IsNotExist(err) {
			t.Errorf("named command touched shared root %s (err=%v)", name, err)
		}
	}
}
