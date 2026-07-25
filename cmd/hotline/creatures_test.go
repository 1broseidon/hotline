package main

import (
	"math/rand"
	"strings"
	"testing"
)

// TestCreatureTableHas100UniqueEntries locks the roll table at exactly 100
// unique, single-word, capitalized names.
func TestCreatureTableHas100UniqueEntries(t *testing.T) {
	if len(creatures) != 100 {
		t.Fatalf("creatures len = %d, want 100", len(creatures))
	}
	seen := make(map[string]bool, len(creatures))
	for _, c := range creatures {
		if c.name == "" {
			t.Fatal("empty creature name")
		}
		if seen[c.name] {
			t.Fatalf("duplicate creature name %q", c.name)
		}
		seen[c.name] = true
		if strings.ContainsAny(c.name, " \t") {
			t.Fatalf("creature name %q is not single-word", c.name)
		}
		if r := []rune(c.name)[0]; r < 'A' || r > 'Z' {
			t.Fatalf("creature name %q is not capitalized", c.name)
		}
	}
}

// TestCreatureTierCounts pins the intended rarity split.
func TestCreatureTierCounts(t *testing.T) {
	counts := map[creatureTier]int{}
	for _, c := range creatures {
		counts[c.tier]++
	}
	if counts[tierCommon] != 80 {
		t.Errorf("common count = %d, want 80", counts[tierCommon])
	}
	if counts[tierRare] != 15 {
		t.Errorf("rare count = %d, want 15", counts[tierRare])
	}
	if counts[tierExtraRare] != 5 {
		t.Errorf("extra-rare count = %d, want 5", counts[tierExtraRare])
	}
}

// TestTierWeights pins the per-name weights: common 10, rare 3, extra-rare 1,
// so a rare name is meaningfully rarer per-name than a common one.
func TestTierWeights(t *testing.T) {
	if got := tierWeight(tierCommon); got != 10 {
		t.Errorf("common weight = %d, want 10", got)
	}
	if got := tierWeight(tierRare); got != 3 {
		t.Errorf("rare weight = %d, want 3", got)
	}
	if got := tierWeight(tierExtraRare); got != 1 {
		t.Errorf("extra-rare weight = %d, want 1", got)
	}
}

// TestRollCreatureDeterministicUnderSeed proves the roll is reproducible when
// the RNG is seeded, and that every roll lands on a real table entry.
func TestRollCreatureDeterministicUnderSeed(t *testing.T) {
	valid := make(map[string]bool, len(creatures))
	for _, c := range creatures {
		valid[c.name] = true
	}
	seq := func(seed int64) []string {
		rng := rand.New(rand.NewSource(seed))
		out := make([]string, 20)
		for i := range out {
			c := rollCreature(rng)
			if !valid[c.name] {
				t.Fatalf("rollCreature returned off-table name %q", c.name)
			}
			out[i] = c.name
		}
		return out
	}
	a := seq(42)
	b := seq(42)
	for i := range a {
		if a[i] != b[i] {
			t.Fatalf("roll %d differs under same seed: %q vs %q", i, a[i], b[i])
		}
	}
	// A different seed should not produce the identical sequence (guards against
	// the RNG being ignored).
	if diff := seq(43); strings.Join(diff, ",") == strings.Join(a, ",") {
		t.Fatal("different seed produced identical sequence; RNG likely ignored")
	}
}

// TestRollTierDistribution sanity-checks that the weighting actually makes
// higher tiers rarer over many rolls (not an exact assertion, just ordering).
func TestRollTierDistribution(t *testing.T) {
	rng := rand.New(rand.NewSource(1))
	counts := map[creatureTier]int{}
	const n = 20000
	for i := 0; i < n; i++ {
		counts[rollCreature(rng).tier]++
	}
	// Expected shares: common 800/(800+45+5) ~= 94%, rare ~5.3%, extra ~0.6%.
	if !(counts[tierCommon] > counts[tierRare] && counts[tierRare] > counts[tierExtraRare]) {
		t.Fatalf("tier ordering wrong: common=%d rare=%d extra=%d",
			counts[tierCommon], counts[tierRare], counts[tierExtraRare])
	}
}

// TestResolveRelayNameExplicitWins verifies the --name flag value beats the env
// and the bot name, and does not trigger a roll.
func TestResolveRelayNameExplicitWins(t *testing.T) {
	t.Setenv("HOTLINE_ASSISTANT_NAME", "from-env")
	rng := rand.New(rand.NewSource(1))
	name, rolled, _ := resolveRelayName("Explicit", "botname", rng)
	if name != "Explicit" {
		t.Fatalf("name = %q, want Explicit", name)
	}
	if rolled {
		t.Fatal("explicit name should not be a roll")
	}
}

// TestResolveRelayNameEnvBeatsBotName verifies HOTLINE_ASSISTANT_NAME wins over
// the bot instance name when no explicit name is given.
func TestResolveRelayNameEnvBeatsBotName(t *testing.T) {
	t.Setenv("HOTLINE_ASSISTANT_NAME", "from-env")
	rng := rand.New(rand.NewSource(1))
	name, rolled, _ := resolveRelayName("", "botname", rng)
	if name != "from-env" {
		t.Fatalf("name = %q, want from-env", name)
	}
	if rolled {
		t.Fatal("env name should not be a roll")
	}
}

// TestResolveRelayNameBotNameBeatsRoll verifies the bot instance name is used
// (no roll) when set and neither explicit nor env is present.
func TestResolveRelayNameBotNameBeatsRoll(t *testing.T) {
	t.Setenv("HOTLINE_ASSISTANT_NAME", "")
	rng := rand.New(rand.NewSource(1))
	name, rolled, _ := resolveRelayName("", "botname", rng)
	if name != "botname" {
		t.Fatalf("name = %q, want botname", name)
	}
	if rolled {
		t.Fatal("bot name should not be a roll")
	}
}

// TestResolveRelayNameRollsWhenEmpty verifies that with nothing set the name is
// rolled from the table (never the old "pi" hardcode).
func TestResolveRelayNameRollsWhenEmpty(t *testing.T) {
	t.Setenv("HOTLINE_ASSISTANT_NAME", "")
	valid := make(map[string]bool, len(creatures))
	for _, c := range creatures {
		valid[c.name] = true
	}
	rng := rand.New(rand.NewSource(7))
	name, rolled, c := resolveRelayName("", "", rng)
	if !rolled {
		t.Fatal("expected a roll when nothing is set")
	}
	if name == "pi" {
		t.Fatal("rolled name is the removed \"pi\" hardcode")
	}
	if !valid[name] || c.name != name {
		t.Fatalf("rolled name %q is not a table entry", name)
	}
}

// TestRollFlairByTier pins the print flair per tier.
func TestRollFlairByTier(t *testing.T) {
	cases := []struct {
		c    creature
		want string
	}{
		{creature{"Kitsune", tierCommon}, "rolled: Kitsune"},
		{creature{"Sphinx", tierRare}, "rolled: Sphinx ✨ (rare)"},
		{creature{"Phoenix", tierExtraRare}, "rolled: Phoenix 🌟 (EXTRA RARE)"},
	}
	for _, tc := range cases {
		if got := rollFlair(tc.c); got != tc.want {
			t.Errorf("rollFlair(%q) = %q, want %q", tc.c.name, got, tc.want)
		}
	}
}
