package main

import (
	"bufio"
	"context"
	"errors"
	"fmt"
	"io"
	"math/rand"
	"os"
	"os/signal"
	"strings"
	"syscall"

	"github.com/mdp/qrterminal/v3"
	"rsc.io/qr"

	"github.com/1broseidon/hotline/internal/app"
	"github.com/1broseidon/hotline/internal/config"
)

// Script-facing exit codes for `relay new-link --code` (SHOULD-FIX 5), so callers
// can tell success (0) / strikeout / expiry / other (1) apart. Kept distinct from
// the notify (2/3/4) codes.
const (
	exitCodeStrikeout = 5
	exitCodeExpired   = 6
)

// runNewLinkCodeCmd drives the code-linking PAKE with a SIGINT/SIGTERM handler
// (BLOCKER 2) so Ctrl-C cancels a real context — letting RunNewLinkCode reach its
// detached final abort and invalidate the code on the relay — and maps outcomes to
// distinct exit codes. On the 503 kill-switch it falls back to the pair QR/URI for
// the already-minted room (SHOULD-FIX 4).
func runNewLinkCodeCmd(stateDir, coreURL string, link app.Link, stdout io.Writer) error {
	ctx, stop := signal.NotifyContext(context.Background(), os.Interrupt, syscall.SIGTERM)
	defer stop()

	err := app.RunNewLinkCode(ctx, stateDir, coreURL, link, stdout)
	switch {
	case errors.Is(err, app.ErrCodesUnavailable):
		// The room is already minted; don't waste it — print the standard QR/URI.
		fmt.Fprintln(stdout, "codes are temporarily unavailable — falling back to the pairing QR")
		printLink(link, stdout)
		return nil
	case errors.Is(err, app.ErrCodeStrikeout):
		return exitCodeError(exitCodeStrikeout)
	case errors.Is(err, app.ErrCodeExpired):
		return exitCodeError(exitCodeExpired)
	default:
		return err
	}
}

// defaultRendezvousURL is the hosted rendezvous pipe every box uses out of the
// box. Self-hosters override it with HOTLINE_RENDEZVOUS_URL.
const defaultRendezvousURL = "wss://relay.hotline.dev"

var runRelayProcess = func(botName string) error {
	old, had := os.LookupEnv("HOTLINE_PROVIDERS")
	providerName := "app"
	if botName != "" {
		providerName += ":" + botName
	}
	_ = os.Setenv("HOTLINE_PROVIDERS", providerName)
	defer func() {
		if had {
			_ = os.Setenv("HOTLINE_PROVIDERS", old)
		} else {
			_ = os.Unsetenv("HOTLINE_PROVIDERS")
		}
	}()
	return runChannel(botName)
}

func cmdRelay(botName string, args []string, stdout, stderr io.Writer) error {
	cfg, err := config.LoadApp(botName)
	if err != nil {
		return err
	}
	if err := cfg.EnsureDirs(); err != nil {
		return err
	}
	store, err := app.OpenRelayStore(cfg.StateDir)
	if err != nil {
		return err
	}

	if len(args) > 0 {
		switch args[0] {
		case "status":
			if len(args) != 1 {
				return fmt.Errorf("usage: hotline relay status")
			}
			return printRelayStatus(store, cfg.StateDir, stdout)
		case "revoke":
			if len(args) != 2 {
				return fmt.Errorf("usage: hotline relay revoke <device-id | room-id>")
			}
			// FB27: revoke accepts a device-id OR an open room-id (unique prefix ok).
			// A bound room must still be revoked by its device-id; ResolveRevoke
			// refuses a room a live device rides, with guidance.
			res, err := store.ResolveRevoke(args[1])
			if err != nil {
				return err
			}
			if res.Kind == "room" {
				r, err := store.RevokeRoom(res.ID)
				if err != nil {
					return err
				}
				fmt.Fprintf(stdout, "Revoked open room %s.\n", shortRoomID(r.ID))
				return nil
			}
			d, err := store.Revoke(res.ID)
			if err != nil {
				return err
			}
			// Core mode: also remove the device from the core registry so it stops
			// receiving wake pushes (SPEC §2.3). Best-effort; the local ban already
			// took effect above. Target the DEVICE'S OWN room (d.Room), not the
			// global current_room — fixes the latent revoke-wrong-room bug.
			if cfg.CoreMode && d.Room != "" {
				if derr := app.DeleteDeviceViaCore(cfg.StateDir, cfg.CoreURL, d.Room, d.ID); derr != nil {
					fmt.Fprintf(stderr, "hotline: core delete-device failed: %v\n", derr)
				}
			}
			fmt.Fprintf(stdout, "Revoked %s.\n", d.ID)
			return nil
		case "new-link":
			rotateAll := false
			codeMode := false
			name := ""
			for i := 1; i < len(args); i++ {
				switch a := args[i]; a {
				case "--rotate-all":
					rotateAll = true
				case "--code":
					codeMode = true
				case "--name":
					if i+1 >= len(args) {
						return fmt.Errorf("usage: hotline relay new-link [--name <name>] [--rotate-all] [--code]")
					}
					i++
					name = args[i]
				default:
					return fmt.Errorf("usage: hotline relay new-link [--name <name>] [--rotate-all] [--code]")
				}
			}
			if codeMode && rotateAll {
				return fmt.Errorf("hotline relay new-link: --code and --rotate-all are mutually exclusive")
			}
			// SHOULD-FIX 3: fail fast BEFORE minting a room. Code linking needs core
			// mode; checking after the mint left an orphan open room consuming a slot.
			if codeMode && !cfg.CoreMode {
				return fmt.Errorf("codes need the hotline core relay; use the QR (set HOTLINE_CORE_MODE=1)")
			}
			// Default is ADDITIVE: mint a new room alongside every existing pairing.
			// --rotate-all is the destructive panic button that unbinds every device
			// and replaces all rooms with the single new one (SPEC §2.1).
			mint := store.MintLinkMode
			if rotateAll {
				mint = store.RotateAll
			}
			// FB21: rooms are no longer independently named. The box owns one
			// durable identity (seeded once here if this is first use), and every
			// minted room carries it. --name overrides ONLY this link's pre-connect
			// placeholder n= param — it does NOT change the box identity.
			boxName, err := ensureIdentity(store, botName, stdout)
			if err != nil {
				return err
			}
			roomName := boxName
			if n := strings.TrimSpace(name); n != "" {
				roomName = n
			}
			link, err := mint(relayRendezvous(cfg.EnvFile), roomName, cfg.CoreMode)
			if err != nil {
				return err
			}
			if codeMode {
				// Code-based linking (design §6.1): the room is minted identically
				// (additive envelope room above); the PAKE only delivers the pair URI
				// confidentially, so the URI/QR is NOT printed on success.
				return runNewLinkCodeCmd(cfg.StateDir, cfg.CoreURL, link, stdout)
			}
			printLink(link, stdout)
			return nil
		case "push-test":
			if len(args) > 2 {
				return fmt.Errorf("usage: hotline relay push-test [device-id]")
			}
			if !cfg.CoreMode {
				return fmt.Errorf("push-test requires core mode (HOTLINE_CORE_MODE=1)")
			}
			deviceID, err := resolvePushTestDevice(store, args)
			if err != nil {
				return err
			}
			// Resolve the room from the chosen device's own binding (MD3), not the
			// global current_room.
			dev, ok := store.Device(deviceID)
			if !ok || dev.Room == "" {
				return fmt.Errorf("device %s has no bound room", deviceID)
			}
			pushed, reason, err := app.PushTestViaCore(cfg.StateDir, cfg.CoreURL, dev.Room, deviceID)
			if err != nil {
				return err
			}
			if pushed {
				fmt.Fprintf(stdout, "Push-test sent to %s.\n", deviceID)
			} else {
				fmt.Fprintf(stdout, "Push-test not sent to %s (%s).\n", deviceID, nonEmpty(reason, "no_token"))
			}
			return nil
		default:
			return fmt.Errorf("usage: hotline relay [status | revoke <device-id | room-id> | new-link | push-test [device-id]]")
		}
	}

	boxName, err := ensureIdentity(store, botName, stderr)
	if err != nil {
		return err
	}
	if _, ok := store.CurrentRoom(); !ok {
		link, err := store.MintLinkMode(relayRendezvous(cfg.EnvFile), boxName, cfg.CoreMode)
		if err != nil {
			return err
		}
		printLink(link, stderr)
	}
	fmt.Fprintln(stderr, "hotline: relay starting; press Ctrl-C to stop")
	return runRelayProcess(botName)
}

// resolvePushTestDevice picks the target device for `relay push-test`: an
// explicit id/prefix (matched against active devices), or the sole active device
// when none is given.
func resolvePushTestDevice(store *app.RelayStore, args []string) (string, error) {
	active := store.ActiveDevices()
	if len(args) == 2 {
		var matches []string
		for _, d := range active {
			if d.ID == args[1] || strings.HasPrefix(d.ID, args[1]) {
				matches = append(matches, d.ID)
			}
		}
		switch len(matches) {
		case 1:
			return matches[0], nil
		case 0:
			return "", fmt.Errorf("no active device matching %q", args[1])
		default:
			return "", fmt.Errorf("device prefix %q is ambiguous", args[1])
		}
	}
	switch len(active) {
	case 1:
		return active[0].ID, nil
	case 0:
		return "", fmt.Errorf("no active device; pair the app first")
	default:
		return "", fmt.Errorf("multiple active devices; specify one: hotline relay push-test <device-id>")
	}
}

// shortRoomID trims a room id to its display prefix (matches relay status output).
func shortRoomID(id string) string {
	if len(id) > 8 {
		return id[:8]
	}
	return id
}

func nonEmpty(s, fallback string) string {
	if s == "" {
		return fallback
	}
	return s
}

// resolveRelayName picks the display name for a NEWLY minted relay room. This
// is a mint-time decision only — existing rooms are re-displayed from their
// stored name, never re-resolved here.
//
// Precedence: an explicit name (the new-link --name flag) wins, then
// HOTLINE_ASSISTANT_NAME (free UTF-8, may contain characters like '-' that
// instance names cannot), then the bot instance name if present. When none of
// those is set the name is rolled from the curated mythological-creature table
// (see creatures.go). The returned bool reports whether the name came from the
// roll, and the creature carries the tier so the caller can print roll flair.
func resolveRelayName(explicit, botName string, rng *rand.Rand) (name string, rolled bool, c creature) {
	if n := strings.TrimSpace(explicit); n != "" {
		return n, false, creature{}
	}
	if n := strings.TrimSpace(os.Getenv("HOTLINE_ASSISTANT_NAME")); n != "" {
		return n, false, creature{}
	}
	if n := strings.TrimSpace(botName); n != "" {
		return n, false, creature{}
	}
	c = rollCreature(rng)
	return c.name, true, c
}

// ensureIdentity seeds the box-owned assistant name ONCE (FB21 §1) and returns
// it. If the box already has a seeded identity it is returned unchanged — a
// second boot never re-rolls. On first use the name is resolved by precedence
// (HOTLINE_ASSISTANT_NAME > bot instance name > one creature roll); when it comes
// from the roll the flair line is printed to out at seed time. Existing state
// dirs with rooms but no identity seed lazily here on the next boot.
func ensureIdentity(store *app.RelayStore, botName string, out io.Writer) (string, error) {
	if name, ok := store.IdentityName(); ok {
		return name, nil
	}
	name, rolled, c := resolveRelayName("", botName, defaultNameRNG())
	stored, seeded, err := store.SeedIdentityName(name)
	if err != nil {
		return "", err
	}
	if seeded && rolled {
		fmt.Fprintln(out, rollFlair(c))
	}
	return stored, nil
}

func relayRendezvous(envFile string) string {
	if value := strings.TrimSpace(os.Getenv("HOTLINE_RENDEZVOUS_URL")); value != "" {
		return value
	}
	f, err := os.Open(envFile)
	if err == nil {
		defer f.Close()
		s := bufio.NewScanner(f)
		for s.Scan() {
			line := strings.TrimSpace(s.Text())
			if strings.HasPrefix(line, "HOTLINE_RENDEZVOUS_URL=") {
				value := strings.Trim(strings.TrimSpace(strings.TrimPrefix(line, "HOTLINE_RENDEZVOUS_URL=")), `"'`)
				if value != "" {
					return value
				}
			}
		}
	}
	return defaultRendezvousURL
}

func printLink(link app.Link, w io.Writer) {
	fmt.Fprintln(w, link.URI)
	qrterminal.GenerateHalfBlock(link.URI, qr.M, w)
}

func printRelayStatus(store *app.RelayStore, stateDir string, w io.Writer) error {
	fmt.Fprintf(w, "state dir:   %s\n", stateDir)
	served := store.ServedRooms()
	fmt.Fprintf(w, "slots:       %d/%d used\n", len(served), app.MaxRooms())
	devices := store.Devices()
	// Index active devices by their bound room for the per-slot lines.
	byRoom := map[string]app.DeviceRecord{}
	for _, d := range devices {
		if d.State == app.DeviceActive {
			byRoom[d.Room] = d
		}
	}
	if len(served) == 0 {
		fmt.Fprintln(w, "rooms:       none")
	}
	for _, r := range served {
		short := shortRoomID(r.ID)
		line := fmt.Sprintf("  - room %s (%s) rendezvous=%s", short, store.RoomStateFor(r), r.URL)
		if d, ok := byRoom[r.ID]; ok {
			line += fmt.Sprintf(" device=%s push=%s", d.ID, nonEmpty(d.PushPlatform, "none"))
		}
		fmt.Fprintln(w, line)
	}
	// Full device roster (all states) — preserves the "<id> (<state>)" shape.
	fmt.Fprintf(w, "devices:     %d\n", len(devices))
	for _, d := range devices {
		fmt.Fprintf(w, "  - %s (%s)\n", d.ID, d.State)
	}
	return nil
}
