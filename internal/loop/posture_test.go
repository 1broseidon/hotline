package loop

import (
	"os"
	"testing"
)

// TestYoloEnabledPiHarness pins review-M2: harness=pi counts as always-yolo, so
// loop approval posture matches the harness's documented unguarded nature.
func TestYoloEnabledPiHarness(t *testing.T) {
	// Ensure the env-var short-circuit does not mask the harness arm.
	t.Setenv("HOTLINE_YOLO", "")
	os.Unsetenv("HOTLINE_YOLO")
	t.Setenv("HOTLINE_HARNESS", "pi")

	yolo, err := YoloEnabled("")
	if err != nil {
		t.Fatalf("YoloEnabled: %v", err)
	}
	if !yolo {
		t.Fatal("harness=pi must report yolo-enabled (always unguarded)")
	}
}

// TestYoloEnabledClaudeHarness confirms the default harness is NOT yolo without
// an explicit skip flag — guards against the pi arm leaking into other harnesses.
func TestYoloEnabledClaudeHarness(t *testing.T) {
	t.Setenv("HOTLINE_YOLO", "")
	os.Unsetenv("HOTLINE_YOLO")
	t.Setenv("HOTLINE_HARNESS", "claude")

	yolo, err := YoloEnabled("")
	if err != nil {
		t.Fatalf("YoloEnabled: %v", err)
	}
	if yolo {
		t.Fatal("harness=claude without a skip flag must not report yolo-enabled")
	}
}
