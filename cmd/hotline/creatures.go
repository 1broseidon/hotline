package main

import (
	"fmt"
	"math/rand"
)

// creatureTier ranks a rolled room name by how rare it is to roll. Rarer tiers
// carry a smaller per-name weight (see tierWeight) and a flashier print flair
// (see rollFlair).
type creatureTier int

const (
	tierCommon creatureTier = iota
	tierRare
	tierExtraRare
)

// creature is one entry in the roll table: a single-word, display-friendly
// mythological name and its rarity tier.
type creature struct {
	name string
	tier creatureTier
}

// tierWeight is the relative selection weight of a single name in the given
// tier. Common names are common (10 each), rare names are meaningfully rarer
// per-name (3 each), and extra-rare names are the jackpot (1 each).
func tierWeight(t creatureTier) int {
	switch t {
	case tierRare:
		return 3
	case tierExtraRare:
		return 1
	default:
		return 10
	}
}

// creatures is the curated roll table: exactly 100 unique, single-word,
// capitalized mythological creatures drawn from world mythologies (Greek,
// Norse, Egyptian, Japanese, Slavic, Celtic, Mesoamerican, and more).
//
// The tier split is 80 common / 15 rare / 5 extra-rare, with the most iconic,
// legendary beasts reserved for the higher tiers.
var creatures = []creature{
	// Extra-rare (5): the legends.
	{"Phoenix", tierExtraRare},
	{"Dragon", tierExtraRare},
	{"Kraken", tierExtraRare},
	{"Ouroboros", tierExtraRare},
	{"Leviathan", tierExtraRare},

	// Rare (15): the marquee monsters.
	{"Sphinx", tierRare},
	{"Basilisk", tierRare},
	{"Chimera", tierRare},
	{"Hydra", tierRare},
	{"Cerberus", tierRare},
	{"Minotaur", tierRare},
	{"Valkyrie", tierRare},
	{"Wendigo", tierRare},
	{"Quetzalcoatl", tierRare},
	{"Griffin", tierRare},
	{"Cockatrice", tierRare},
	{"Banshee", tierRare},
	{"Fenrir", tierRare},
	{"Jormungandr", tierRare},
	{"Behemoth", tierRare},

	// Common (80): the rank and file.
	{"Cyclops", tierCommon},
	{"Medusa", tierCommon},
	{"Gorgon", tierCommon},
	{"Harpy", tierCommon},
	{"Satyr", tierCommon},
	{"Centaur", tierCommon},
	{"Pegasus", tierCommon},
	{"Typhon", tierCommon},
	{"Echidna", tierCommon},
	{"Scylla", tierCommon},
	{"Siren", tierCommon},
	{"Manticore", tierCommon},
	{"Talos", tierCommon},
	{"Cetus", tierCommon},
	{"Draugr", tierCommon},
	{"Troll", tierCommon},
	{"Jotunn", tierCommon},
	{"Nidhogg", tierCommon},
	{"Ratatoskr", tierCommon},
	{"Huldra", tierCommon},
	{"Sleipnir", tierCommon},
	{"Nokk", tierCommon},
	{"Wyrm", tierCommon},
	{"Ammit", tierCommon},
	{"Bennu", tierCommon},
	{"Apep", tierCommon},
	{"Uraeus", tierCommon},
	{"Sekhmet", tierCommon},
	{"Kitsune", tierCommon},
	{"Tengu", tierCommon},
	{"Kappa", tierCommon},
	{"Oni", tierCommon},
	{"Tanuki", tierCommon},
	{"Kirin", tierCommon},
	{"Baku", tierCommon},
	{"Bakeneko", tierCommon},
	{"Raiju", tierCommon},
	{"Umibozu", tierCommon},
	{"Namazu", tierCommon},
	{"Gashadokuro", tierCommon},
	{"Jorogumo", tierCommon},
	{"Kodama", tierCommon},
	{"Orochi", tierCommon},
	{"Leshy", tierCommon},
	{"Domovoi", tierCommon},
	{"Rusalka", tierCommon},
	{"Vodyanoy", tierCommon},
	{"Kikimora", tierCommon},
	{"Alkonost", tierCommon},
	{"Zmey", tierCommon},
	{"Bannik", tierCommon},
	{"Likho", tierCommon},
	{"Sirin", tierCommon},
	{"Gamayun", tierCommon},
	{"Selkie", tierCommon},
	{"Dullahan", tierCommon},
	{"Kelpie", tierCommon},
	{"Pooka", tierCommon},
	{"Leprechaun", tierCommon},
	{"Sluagh", tierCommon},
	{"Fomorian", tierCommon},
	{"Redcap", tierCommon},
	{"Merrow", tierCommon},
	{"Ankou", tierCommon},
	{"Cailleach", tierCommon},
	{"Ahuizotl", tierCommon},
	{"Cipactli", tierCommon},
	{"Xolotl", tierCommon},
	{"Nagual", tierCommon},
	{"Camazotz", tierCommon},
	{"Yeti", tierCommon},
	{"Thunderbird", tierCommon},
	{"Golem", tierCommon},
	{"Djinn", tierCommon},
	{"Roc", tierCommon},
	{"Simurgh", tierCommon},
	{"Ifrit", tierCommon},
	{"Ghoul", tierCommon},
	{"Naga", tierCommon},
	{"Rakshasa", tierCommon},
}

// rollCreature picks one creature from the table weighted by tier (see
// tierWeight). The RNG is injected so callers can seed it for deterministic
// tests; production callers pass a time-seeded RNG (see defaultNameRNG).
func rollCreature(rng *rand.Rand) creature {
	total := 0
	for _, c := range creatures {
		total += tierWeight(c.tier)
	}
	n := rng.Intn(total)
	for _, c := range creatures {
		n -= tierWeight(c.tier)
		if n < 0 {
			return c
		}
	}
	// Unreachable: the weighted cursor always lands inside the table.
	return creatures[len(creatures)-1]
}

// defaultNameRNG returns the production RNG for a name roll: a fresh,
// time-seeded source. Room mints are rare, so a fresh source per roll is fine
// and avoids sharing mutable state.
func defaultNameRNG() *rand.Rand {
	return rand.New(rand.NewSource(rand.Int63()))
}

// rollFlair renders the one-line "rolled: ..." banner for a rolled name, with
// flair escalating by tier.
func rollFlair(c creature) string {
	switch c.tier {
	case tierExtraRare:
		return fmt.Sprintf("rolled: %s 🌟 (EXTRA RARE)", c.name)
	case tierRare:
		return fmt.Sprintf("rolled: %s ✨ (rare)", c.name)
	default:
		return fmt.Sprintf("rolled: %s", c.name)
	}
}
