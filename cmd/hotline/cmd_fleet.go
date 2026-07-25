package main

import (
	"bufio"
	"encoding/json"
	"fmt"
	"io"
	"net/url"
	"os"
	"strings"
	"time"

	"github.com/mdp/qrterminal/v3"
	"rsc.io/qr"

	"github.com/1broseidon/hotline/internal/app"
	"github.com/1broseidon/hotline/internal/config"
)

// cmdFleet drives the A2A (agent-to-agent) fleet registry — Lane L1 of
// a2a-design-v2 (§2). It mirrors the cmd_relay dispatch idiom: link/join mint or
// store an edge; ls/rm/rename manage the registry. Fleet edges live in
// <stateDir>/fleet/fleet.json — relay-state.json is never touched by any of
// these subcommands. Dialing (direction=dial) is Lane L3: `join` only parses +
// stores creds.
func cmdFleet(botName string, args []string, stdout, stderr io.Writer) error {
	cfg, err := config.LoadApp(botName)
	if err != nil {
		return err
	}
	if err := cfg.EnsureDirs(); err != nil {
		return err
	}
	fs, err := app.OpenFleetStore(cfg.StateDir)
	if err != nil {
		return err
	}

	if len(args) == 0 {
		return fmt.Errorf("usage: hotline fleet [link --alias <peer> | join <uri|-> --alias <peer> [--allow-relay <origin>] | ls [--json] | rm <peer> | rename <peer> <new-alias> | grant <peer> [--ttl <dur>] | revoke <peer>]")
	}

	switch args[0] {
	case "link":
		alias, err := aliasFlag(args[1:])
		if err != nil {
			return err
		}
		edge, secret, err := fs.Link(fleetRendezvous(cfg.EnvFile), alias)
		if err != nil {
			return err
		}
		uri := app.FleetPairingURI(edge.RelayURL, edge.Room, secret, fleetBoxName(cfg.StateDir, botName, alias))
		fmt.Fprintf(stdout, "Fleet edge %s (%s) minted (serve).\n", edge.EdgeID, alias)
		fmt.Fprintln(stdout, uri)
		qrterminal.GenerateHalfBlock(uri, qr.M, stdout)
		fmt.Fprintln(stdout, "Give this to the peer box: `hotline fleet join <uri> --alias <you>`")
		return nil

	case "join":
		if len(args) < 2 {
			return fmt.Errorf("usage: hotline fleet join <uri|-> --alias <peer> [--allow-relay <origin>]")
		}
		uriArg := args[1]
		alias, allowRelay, err := joinFlags(args[2:])
		if err != nil {
			return err
		}
		uri, err := readFleetURI(uriArg, os.Stdin)
		if err != nil {
			return err
		}
		opts, err := fleetJoinOptions(cfg.EnvFile, allowRelay)
		if err != nil {
			return err
		}
		edge, err := fs.Join(uri, alias, opts)
		if err != nil {
			return err
		}
		// The edge is stored locally; the running box's L3 dial manager picks it up on
		// its next poll (within a couple of seconds) and connects to the peer's room.
		fmt.Fprintf(stdout, "Fleet edge %s (%s) stored (dial).\n", edge.EdgeID, alias)
		fmt.Fprintf(stdout, "Device id %s registered; the box's dial manager connects to the peer on its next poll (~2s).\n", edge.DeviceCreds.DeviceID)
		return nil

	case "ls":
		asJSON := false
		for _, a := range args[1:] {
			if a == "--json" {
				asJSON = true
			} else {
				return fmt.Errorf("usage: hotline fleet ls [--json]")
			}
		}
		return printFleetLs(fs, cfg.StateDir, asJSON, stdout)

	case "rm":
		if len(args) != 2 {
			return fmt.Errorf("usage: hotline fleet rm <peer>")
		}
		edge, err := fs.Remove(args[1])
		if err != nil {
			return err
		}
		fmt.Fprintf(stdout, "Removed fleet edge %s (%s).\n", edge.EdgeID, edge.Alias)
		if edge.Direction == app.FleetDial {
			// F13: a dial-side removal is local only — the peer still holds the room.
			fmt.Fprintln(stdout, "Note: this unlink is LOCAL. The peer still holds the room; ask them to `hotline fleet rm` their side too.")
		}
		return nil

	case "rename":
		if len(args) != 3 {
			return fmt.Errorf("usage: hotline fleet rename <peer> <new-alias>")
		}
		edge, err := fs.Rename(args[1], args[2])
		if err != nil {
			return err
		}
		fmt.Fprintf(stdout, "Renamed fleet edge %s to %q.\n", edge.EdgeID, edge.Alias)
		return nil

	case "grant":
		// F1: the ONLY way orchestrator authority is ever created. It is an operator
		// act on THIS box's registry — nothing on the wire can reach it.
		if len(args) < 2 {
			return fmt.Errorf("usage: hotline fleet grant <peer> [--ttl <dur>]")
		}
		ttl, err := ttlFlag(args[2:])
		if err != nil {
			return err
		}
		edge, err := fs.GrantAuthority(args[1], ttl)
		if err != nil {
			return err
		}
		app.FleetAudit(cfg.StateDir, "edge=%s alias=%q AUTHORITY GRANTED by operator (key_fp=%s expires=%s)",
			edge.EdgeID, edge.Alias, edge.Authority.KeyFP, authorityExpiryLabel(edge))
		fmt.Fprintf(stdout, "Granted orchestrator authority to %s (edge %s), bound to peer key %s.\n", edge.Alias, edge.EdgeID, edge.Authority.KeyFP)
		if edge.Authority.ExpiresAt != "" {
			fmt.Fprintf(stdout, "Expires %s — after that it silently demotes to normal peer framing.\n", edge.Authority.ExpiresAt)
		} else {
			fmt.Fprintln(stdout, "No expiry: it stands until `hotline fleet revoke`.")
		}
		fmt.Fprintf(stdout, "Effect: inbound %s frames from this peer are framed to the agent as orchestrator directives.\n", "task/cancel/status_req")
		fmt.Fprintln(stdout, "It still cannot approve pairings/access, change permissions, restart this box, or authorize destructive acts — and it grants nothing on any other edge.")
		return nil

	case "revoke":
		if len(args) != 2 {
			return fmt.Errorf("usage: hotline fleet revoke <peer>")
		}
		edge, had, err := fs.RevokeAuthority(args[1])
		if err != nil {
			return err
		}
		if !had {
			fmt.Fprintf(stdout, "Fleet edge %s (%s) held no orchestrator authority; nothing to revoke.\n", edge.EdgeID, edge.Alias)
			return nil
		}
		app.FleetAudit(cfg.StateDir, "edge=%s alias=%q AUTHORITY REVOKED by operator", edge.EdgeID, edge.Alias)
		fmt.Fprintf(stdout, "Revoked orchestrator authority from %s (edge %s); it takes effect on the next inbound frame.\n", edge.Alias, edge.EdgeID)
		return nil

	default:
		return fmt.Errorf("usage: hotline fleet [link --alias <peer> | join <uri|-> --alias <peer> [--allow-relay <origin>] | ls [--json] | rm <peer> | rename <peer> <new-alias> | grant <peer> [--ttl <dur>] | revoke <peer>]")
	}
}

// ttlFlag extracts the optional --ttl <dur> flag for `fleet grant` (F1: the salvaged
// capability-token idea — a grant that dies on its own). Zero means no expiry.
func ttlFlag(args []string) (time.Duration, error) {
	raw := ""
	for i := 0; i < len(args); i++ {
		switch a := args[i]; {
		case a == "--ttl":
			if i+1 >= len(args) {
				return 0, fmt.Errorf("--ttl requires a duration (e.g. 12h)")
			}
			i++
			raw = args[i]
		case strings.HasPrefix(a, "--ttl="):
			raw = strings.TrimPrefix(a, "--ttl=")
		default:
			return 0, fmt.Errorf("unexpected argument %q", a)
		}
	}
	if strings.TrimSpace(raw) == "" {
		return 0, nil
	}
	d, err := time.ParseDuration(strings.TrimSpace(raw))
	if err != nil {
		return 0, fmt.Errorf("invalid --ttl %q: %w", raw, err)
	}
	if d <= 0 {
		return 0, fmt.Errorf("--ttl must be positive (got %s)", d)
	}
	return d, nil
}

// authorityExpiryLabel renders a grant's TTL horizon for the audit line.
func authorityExpiryLabel(e app.FleetEdge) string {
	if e.Authority == nil || e.Authority.ExpiresAt == "" {
		return "never"
	}
	return e.Authority.ExpiresAt
}

// aliasFlag extracts the required --alias <peer> flag from the remaining args.
func aliasFlag(args []string) (string, error) {
	alias, _, err := parseFleetFlags(args, false)
	return alias, err
}

// joinFlags extracts --alias <peer> (required) and --allow-relay <origin>
// (optional) for `fleet join`.
func joinFlags(args []string) (alias, allowRelay string, err error) {
	return parseFleetFlags(args, true)
}

func parseFleetFlags(args []string, allowRelayOK bool) (alias, allowRelay string, err error) {
	for i := 0; i < len(args); i++ {
		switch a := args[i]; {
		case a == "--alias":
			if i+1 >= len(args) {
				return "", "", fmt.Errorf("--alias requires a value")
			}
			i++
			alias = args[i]
		case strings.HasPrefix(a, "--alias="):
			alias = strings.TrimPrefix(a, "--alias=")
		case allowRelayOK && a == "--allow-relay":
			if i+1 >= len(args) {
				return "", "", fmt.Errorf("--allow-relay requires an origin")
			}
			i++
			allowRelay = args[i]
		case allowRelayOK && strings.HasPrefix(a, "--allow-relay="):
			allowRelay = strings.TrimPrefix(a, "--allow-relay=")
		default:
			return "", "", fmt.Errorf("unexpected argument %q", a)
		}
	}
	if strings.TrimSpace(alias) == "" {
		return "", "", fmt.Errorf("--alias <peer> is required")
	}
	return strings.TrimSpace(alias), strings.TrimSpace(allowRelay), nil
}

// fleetJoinOptions builds the B7 relay-origin allowlist: the box's configured
// rendezvous origin is trusted by default; an operator can trust another with
// --allow-relay <origin>.
func fleetJoinOptions(envFile, allowRelay string) (app.JoinOptions, error) {
	origin, err := relayOrigin(fleetRendezvous(envFile))
	if err != nil {
		return app.JoinOptions{}, fmt.Errorf("resolving the box rendezvous origin: %w", err)
	}
	opts := app.JoinOptions{AllowedOrigins: []string{origin}}
	if allowRelay != "" {
		o, err := relayOrigin(allowRelay)
		if err != nil {
			return app.JoinOptions{}, fmt.Errorf("invalid --allow-relay origin: %w", err)
		}
		opts.AllowRelay = o
	}
	return opts, nil
}

// relayOrigin normalizes a ws/wss URL to its scheme://host origin.
func relayOrigin(raw string) (string, error) {
	u, err := url.Parse(strings.TrimRight(strings.TrimSpace(raw), "/"))
	if err != nil {
		return "", err
	}
	if u.Host == "" || (u.Scheme != "ws" && u.Scheme != "wss") {
		return "", fmt.Errorf("origin must be a ws:// or wss:// URL with a host")
	}
	return u.Scheme + "://" + u.Host, nil
}

// readFleetURI resolves the join URI argument: "-" reads the URI from stdin so a
// secret-bearing URI stays out of argv (§2, F15); anything else is the URI.
func readFleetURI(arg string, stdin io.Reader) (string, error) {
	if arg != "-" {
		return arg, nil
	}
	sc := bufio.NewScanner(stdin)
	sc.Buffer(make([]byte, 0, 64*1024), 1<<20)
	for sc.Scan() {
		line := strings.TrimSpace(sc.Text())
		if line != "" {
			return line, nil
		}
	}
	if err := sc.Err(); err != nil {
		return "", err
	}
	return "", fmt.Errorf("no URI read from stdin")
}

// fleetBoxName resolves the display name stamped into a fleet pair URI (n=): the
// operator box identity, falling back to the bot name then the alias. Read-only
// (never seeds/mints the identity — link must not touch relay-state.json).
func fleetBoxName(stateDir, botName, alias string) string {
	if store, err := app.OpenRelayStore(stateDir); err == nil {
		if name, ok := store.IdentityName(); ok && strings.TrimSpace(name) != "" {
			return name
		}
	}
	if strings.TrimSpace(botName) != "" {
		return botName
	}
	return alias
}

// fleetLsConnectedWindow is how recently an edge must have been seen for the CLI to
// call it connected (§6, Lane L4). A separate CLI process cannot read the running
// box's in-memory session map, so it infers liveness from LastSeenAt freshness (the
// box touches it on connect + every inbound frame). This is a disk-honest
// APPROXIMATION: an idle-but-connected edge can read as disconnected here — the
// AUTHORITATIVE live view is the operator app's fleet_state / the fleet MCP tool,
// both of which read the real session map inside the box.
const fleetLsConnectedWindow = 2 * time.Minute

// fleetLsRow is the enriched, secret-free ls view (§6, Lane L4): the redacted
// registry entry plus the disk-derived live columns the operator's terminal cares
// about — connect liveness, pending-outbound depth, rolling 24h counters, and the
// tombstone/key-mismatch conditions. It NEVER carries a secret (embeds
// FleetEdgeView, B5).
type fleetLsRow struct {
	app.FleetEdgeView
	Connected   bool   `json:"connected"`
	Pending     int    `json:"pending"`
	Sent24h     uint64 `json:"sent_24h"`
	Recv24h     uint64 `json:"recv_24h"`
	Dropped24h  uint64 `json:"dropped_24h"`
	Dead        bool   `json:"dead,omitempty"`
	DeadReason  string `json:"dead_reason,omitempty"`
	KeyMismatch bool   `json:"key_mismatch,omitempty"`
	// L5 caps (caps-design §3): the box-attested one-line summary + local received_at,
	// plus the full stored manifest under --json.
	CapsSummary string             `json:"caps,omitempty"`
	CapsAt      string             `json:"caps_received_at,omitempty"`
	PeerCaps    *app.FleetPeerCaps `json:"peer_caps,omitempty"`
	// F1: the operator-granted orchestrator authority state on this edge —
	// "" (none) | granted | granted,expires_in=… | expired | unbound.
	Authority string `json:"authority,omitempty"`
	// F2 liveness: Unreachable is the recoverable cold-retry dead-mark (NOT a
	// tombstone); StalePending flags outbound queued for a peer with no session and no
	// recent contact — the asymmetric-death symptom.
	Unreachable  bool `json:"unreachable,omitempty"`
	StalePending bool `json:"stale_pending,omitempty"`
}

// fleetEdgeConnected reports the CLI's disk-derived liveness for an edge: not
// removed AND last seen within fleetLsConnectedWindow.
func fleetEdgeConnected(e app.FleetEdge, now time.Time) bool {
	if e.Removed() || e.LastSeenAt == "" {
		return false
	}
	ts, err := time.Parse(time.RFC3339Nano, e.LastSeenAt)
	if err != nil {
		return false
	}
	return now.Sub(ts) <= fleetLsConnectedWindow
}

// fleetLsRowFor enriches one edge with its disk-derived live columns. Best-effort:
// a per-edge state/journal read error leaves the counters zero rather than failing
// the whole listing.
func fleetLsRowFor(fs *app.FleetStore, e app.FleetEdge, now time.Time) fleetLsRow {
	r := fleetLsRow{FleetEdgeView: e.Redacted()}
	if e.Removed() {
		r.Dead = true
		if e.Tombstone != nil {
			r.DeadReason = e.Tombstone.Reason
		}
		return r
	}
	r.Connected = fleetEdgeConnected(e, now)
	r.Authority = e.AuthorityStatus(now)
	r.Unreachable = e.Unreachable != nil
	if depth, err := fs.PendingDepth(e.EdgeID); err == nil {
		r.Pending = depth
		r.StalePending = app.FleetStalePending(depth, r.Connected, e.LastSeenAt, now)
	}
	if sent, recv, dropped, err := fs.EdgeActivity(e.EdgeID); err == nil {
		r.Sent24h, r.Recv24h, r.Dropped24h = sent, recv, dropped
	}
	if st, err := fs.EdgeState(e.EdgeID); err == nil {
		r.KeyMismatch = st.KeyFPMismatch
	}
	// B4: the ONE pin-aware accessor — a mismatched/unbound manifest never renders under
	// box-attested language (the key_mismatch flag surfaces it instead).
	if pc, err := fs.BoundPeerCaps(e); err == nil && pc != nil {
		r.CapsSummary = pc.Caps.Summary()
		r.CapsAt = pc.ReceivedAt
		r.PeerCaps = pc
	}
	return r
}

// printFleetLs renders the registry + disk-honest live state (§6, Lane L4). It
// serializes REDACTED rows (never the storage struct — no secrets, B5).
func printFleetLs(fs *app.FleetStore, stateDir string, asJSON bool, w io.Writer) error {
	edges, err := fs.Edges()
	if err != nil {
		return err
	}
	now := time.Now()
	rows := make([]fleetLsRow, 0, len(edges))
	for _, e := range edges {
		rows = append(rows, fleetLsRowFor(fs, e, now))
	}
	if asJSON {
		enc := json.NewEncoder(w)
		enc.SetIndent("", "  ")
		return enc.Encode(rows)
	}
	fmt.Fprintf(w, "state dir:   %s\n", stateDir)
	active := 0
	for _, e := range edges {
		if !e.Removed() {
			active++
		}
	}
	fmt.Fprintf(w, "edges:       %d/%d used\n", active, app.FleetMaxEdges)
	if len(edges) == 0 {
		fmt.Fprintln(w, "  (none)")
		return nil
	}
	for i, e := range edges {
		r := rows[i]
		state := string(e.Direction)
		if e.Removed() {
			state = "removed"
		}
		lastSeen := e.LastSeenAt
		if lastSeen == "" {
			lastSeen = "-"
		}
		fmt.Fprintf(w, "  - %s  %-16s %-8s connected=%t last_seen=%s pending=%d sent24h=%d recv24h=%d dropped24h=%d",
			e.EdgeID, e.Alias, state, r.Connected, lastSeen, r.Pending, r.Sent24h, r.Recv24h, r.Dropped24h)
		// L5: the box-attested caps summary + age, or an em dash when no manifest.
		if !e.Removed() {
			if r.CapsSummary != "" || r.PeerCaps != nil {
				caps := r.CapsSummary
				if caps == "" {
					caps = "(no summary)"
				}
				fmt.Fprintf(w, " caps=[%s%s]", caps, fleetCapsAgeSuffix(r.CapsAt, now))
			} else {
				fmt.Fprint(w, " caps=—")
			}
		}
		// F1: authority is an operator-facing column of its own — a granted edge is the
		// one place inbound framing changes, so it must be visible at a glance.
		if r.Authority != "" {
			fmt.Fprintf(w, " authority=%s", r.Authority)
		}
		var flags []string
		// F2: the two one-sided-failure signals, loudest first.
		if r.StalePending {
			flags = append(flags, "stale_pending")
		}
		if r.Unreachable {
			flags = append(flags, "unreachable:cold_retry")
		}
		if r.Dead {
			f := "dead"
			if r.DeadReason != "" {
				f = "dead:" + r.DeadReason
			}
			flags = append(flags, f)
		}
		if r.KeyMismatch {
			flags = append(flags, "key_mismatch")
		}
		if len(flags) > 0 {
			fmt.Fprintf(w, " flags=%s", strings.Join(flags, ","))
		}
		fmt.Fprintln(w)
	}
	return nil
}

// fleetCapsAgeSuffix renders " Nh ago" / " Nm ago" for a manifest received_at, plus
// ", stale" past 24h (caps-design §3/§5). Empty when the timestamp is unparsable.
func fleetCapsAgeSuffix(receivedAt string, now time.Time) string {
	if receivedAt == "" {
		return ""
	}
	t, err := time.Parse(time.RFC3339, receivedAt)
	if err != nil {
		return ""
	}
	age := now.Sub(t)
	var ago string
	switch {
	case age < time.Minute:
		ago = "just now"
	case age < time.Hour:
		ago = fmt.Sprintf("%dm ago", int(age.Minutes()))
	case age < 24*time.Hour:
		ago = fmt.Sprintf("%dh ago", int(age.Hours()))
	default:
		ago = fmt.Sprintf("%dd ago", int(age.Hours())/24)
	}
	if age > 24*time.Hour {
		return " " + ago + ", stale"
	}
	return " " + ago
}

// fleetRendezvous resolves the rendezvous URL for a freshly minted fleet room,
// sharing the operator relay precedence (HOTLINE_RENDEZVOUS_URL > .env > default).
func fleetRendezvous(envFile string) string { return relayRendezvous(envFile) }
