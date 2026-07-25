package app

import (
	"bytes"
	"context"
	"crypto/rand"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net/http"
	"time"

	ref "github.com/1broseidon/hotline/protocol/code-link-v1/ref"
)

// This file is the BOX side of hotline's code-based device linking (WP-CL3): the
// `hotline relay new-link --code` flow. The box is CPace party A (initiator): it
// mints a normal envelope room additively (unchanged MintLinkMode), then runs the
// PAKE against the relay's signed /v1/link-codes/* endpoints (design §5.1) to
// deliver the resulting pair URI to a new client confidentially. The crypto is
// consumed from the frozen WP-CL0 role API (protocol/code-link-v1/ref); nothing
// here re-implements CPace, and the raw shared secret never leaves that package.

const (
	// codeLinkProto is the create-body protocol tag (design §5.1).
	codeLinkProto = "code-link-v1"
	// codeTTL is the box-enforced session lifetime (design §4.6): the CLI stops
	// at 300 s regardless of what the relay claims.
	codeTTL = 300 * time.Second
	// codePollWait is the box's per-poll long-poll budget (design §6.2). The relay
	// parks up to ~20 s; the box gives it a little slack.
	codePollWait = 25 * time.Second
	// codeMaxStrikes is the load-bearing box-side cap (design §4.6): after this
	// many failed confirm_b verifications the box aborts and invalidates the code.
	// The relay IS the adversary, so this counter MUST live box-side.
	codeMaxStrikes = 3
	// codeCreateRetries bounds channel-collision regeneration on 409 code_taken.
	codeCreateRetries = 5
	// crockfordAlphabet is the WP-CL0 code alphabet (SPEC §2): 0-9 A-Z minus I L O
	// U. 32 divides 256, so a uniform byte mod 32 is unbiased.
	crockfordAlphabet = "0123456789ABCDEFGHJKMNPQRSTVWXYZ"
)

// Sentinel outcomes so the CLI can print the right message / exit nonzero.
var (
	// ErrCodeStrikeout is the 3-strike abort (design §4.6): too many wrong codes.
	ErrCodeStrikeout = errors.New("code linking: too many wrong attempts — code cancelled")
	// ErrCodeExpired is the box-enforced TTL abort.
	ErrCodeExpired = errors.New("code linking: code expired before it was claimed")
	// errCodeTaken is the 409 channel-collision signal (design §5.1); the box
	// regenerates a fresh code and retries.
	errCodeTaken = errors.New("code linking: channel already taken")
	// ErrCodesUnavailable is the kill-switch fallback (RELAY.md §"Kill switch"):
	// create returned 503 provider_unavailable, so the caller should print the
	// standard pair URI/QR for the already-minted room instead of a code.
	ErrCodesUnavailable = errors.New("code linking: codes temporarily unavailable")
)

// ---------------------------------------------------------------------------
// coreClient additions — the three signed box endpoints of design §5.1. Each
// reuses the existing doSigned scheme (same canonical string, headers, nonce
// ring) — the relay TOFU-binds box_pub on create and verifies against it after.
// ---------------------------------------------------------------------------

func linkCodePath(channel, suffix string) string { return "/v1/link-codes/" + channel + suffix }

// pendingAttempt is one claimed client turn handed back by /poll: the client's
// CPace element and its confirmation MAC (design §5.1 poll / §5.2 confirm).
type pendingAttempt struct {
	MsgB     string `json:"msg_b"`
	ConfirmB string `json:"confirm_b"`
	// Aid is the relay-assigned per-attempt id (WP-CL2 attempt-correlation fix).
	// The box captures it on /poll and MUST echo it on the matching /finish so the
	// relay routes the verdict to the right attempt/owner (400 otherwise).
	Aid int `json:"aid"`
}

// createLinkCode performs the signed create POST (design §5.1). It TOFU-binds
// box_pub and stores msg_a under the channel. A 409 becomes errCodeTaken so the
// caller regenerates; 200 returns the relay-reported TTL seconds.
func (c *coreClient) createLinkCode(ctx context.Context, channel, msgA string) (expiresIn int, err error) {
	body, err := json.Marshal(map[string]any{
		"box_pub": publicJWKFor(c.priv),
		"msg_a":   msgA,
		"proto":   codeLinkProto,
	})
	if err != nil {
		return 0, err
	}
	status, snippet, err := c.doSigned(ctx, http.MethodPost, linkCodePath(channel, ""), body)
	if err != nil {
		return 0, err
	}
	switch status {
	case http.StatusOK:
		var r struct {
			ExpiresIn int `json:"expires_in"`
		}
		_ = json.Unmarshal(snippet, &r)
		return r.ExpiresIn, nil
	case http.StatusConflict:
		return 0, errCodeTaken
	case http.StatusServiceUnavailable:
		// Kill switch (RELAY.md): create is fail-closed. Signal the CLI to fall
		// back to printing the pair URI/QR for the room it already minted.
		if bytes.Contains(snippet, []byte("provider_unavailable")) {
			return 0, ErrCodesUnavailable
		}
		return 0, fmt.Errorf("create link-code rejected (%d): %s", status, readCodeBytes(snippet))
	default:
		return 0, fmt.Errorf("create link-code rejected (%d): %s", status, readCodeBytes(snippet))
	}
}

// pollLinkCode long-polls for the next claimed client attempt (design §5.1). It
// returns a non-nil *pendingAttempt when the relay handed one over, or (nil,nil)
// when the park elapsed with nothing pending (the box simply polls again).
//
// Response-shape tolerance is deliberate: WP-CL2's exact body is still settling,
// so this accepts both the nested {"pending":{…}} form and a flat
// {"msg_b":…,"confirm_b":…}. See the reconciliation note at the bottom of this
// file.
func (c *coreClient) pollLinkCode(ctx context.Context, channel string) (*pendingAttempt, error) {
	status, snippet, err := c.doSigned(ctx, http.MethodPost, linkCodePath(channel, "/poll"), []byte("{}"))
	if err != nil {
		return nil, err
	}
	if status != http.StatusOK {
		return nil, fmt.Errorf("poll link-code rejected (%d): %s", status, readCodeBytes(snippet))
	}
	var r struct {
		Pending  *pendingAttempt `json:"pending"`
		MsgB     string          `json:"msg_b"`
		ConfirmB string          `json:"confirm_b"`
		Aid      int             `json:"aid"`
	}
	if err := json.Unmarshal(snippet, &r); err != nil {
		return nil, fmt.Errorf("poll link-code: bad response: %w", err)
	}
	if r.Pending != nil && r.Pending.MsgB != "" {
		return r.Pending, nil
	}
	if r.MsgB != "" {
		return &pendingAttempt{MsgB: r.MsgB, ConfirmB: r.ConfirmB, Aid: r.Aid}, nil
	}
	return nil, nil
}

// finishLinkCode reports the outcome of one attempt to the relay (design §5.1).
// ok=true releases {confirm_a, payload} to the parked client and single-use
// deletes the session; ok=false is a rejected confirm; ok=false+final=true is
// the box aborting the whole session (strike cap or TTL / cancel).
func (c *coreClient) finishLinkCode(ctx context.Context, channel string, ok bool, aid int, confirmA, payloadN, payloadC string, final bool) error {
	m := map[string]any{"ok": ok}
	if ok {
		m["aid"] = aid
		m["confirm_a"] = confirmA
		m["payload"] = map[string]string{"n": payloadN, "c": payloadC}
	} else if final {
		// Session-wide abort (strike cap / TTL / cancel): NO aid — the relay treats
		// final:true as a channel-wide teardown, not an attempt-specific verdict.
		m["final"] = true
	} else {
		// Attempt-specific mismatch verdict REQUIRES the polled aid (400 otherwise).
		m["aid"] = aid
	}
	body, err := json.Marshal(m)
	if err != nil {
		return err
	}
	status, snippet, err := c.doSigned(ctx, http.MethodPost, linkCodePath(channel, "/finish"), body)
	if err != nil {
		return err
	}
	if status != http.StatusOK {
		return fmt.Errorf("finish link-code rejected (%d): %s", status, readCodeBytes(snippet))
	}
	return nil
}

func readCodeBytes(b []byte) string { return readCode(bytes.NewReader(b)) }

// ---------------------------------------------------------------------------
// Orchestration — the CLI-facing entry point.
// ---------------------------------------------------------------------------

// codeLinkOpts carries defaults plus test hooks. All zero values fall back to the
// production constants; forceCode/now/httpClient exist only for deterministic
// tests.
type codeLinkOpts struct {
	ttl        time.Duration
	pollWait   time.Duration
	maxStrikes int
	forceCode  string // a pre-picked normalized 12-char code (skips crypto/rand)
	now        func() time.Time
	httpClient *http.Client
	// afterPoll, if set, is invoked after a non-nil pending attempt is polled and
	// before it is verified. Test-only: it lets a test deterministically cancel the
	// caller ctx with a strikeout attempt already in hand (cancel-after-pending).
	afterPoll func()
}

// RunNewLinkCode runs the box-initiator PAKE for an already-minted envelope room
// (design §6.1). The room MUST have been minted with MintLinkMode(envelope=true)
// so link.Secret is present — the box registers it (so it is live before the
// device dials), generates the human code, and drives create → poll → verify →
// finish. On a claimed, verified client it seals the pair URI and posts it; the
// caller relays byte-identically into the frozen paste-link flow. Blocks until
// success, the 3-strike cap, TTL, or ctx cancellation.
func RunNewLinkCode(ctx context.Context, stateDir, coreURL string, link Link, out io.Writer) error {
	return runNewLinkCode(ctx, stateDir, coreURL, link, out, codeLinkOpts{})
}

func runNewLinkCode(ctx context.Context, stateDir, coreURL string, link Link, out io.Writer, opts codeLinkOpts) error {
	ttl := opts.ttl
	if ttl == 0 {
		ttl = codeTTL
	}
	pollWait := opts.pollWait
	if pollWait == 0 {
		pollWait = codePollWait
	}
	maxStrikes := opts.maxStrikes
	if maxStrikes == 0 {
		maxStrikes = codeMaxStrikes
	}
	now := opts.now
	if now == nil {
		now = time.Now
	}

	if link.Secret == "" {
		return fmt.Errorf("code linking requires an envelope room (core mode); got a plaintext room")
	}
	ci, err := ref.BuildCIFromURL(coreURL)
	if err != nil {
		return fmt.Errorf("code linking: bad relay URL: %w", err)
	}

	client := opts.httpClient
	if client == nil {
		// No tight client-level timeout: the long-poll budget is enforced per
		// request via context so a 25 s park is not killed by the 10 s push timeout.
		client = &http.Client{Timeout: pollWait + 10*time.Second}
	}
	cc, err := newCoreClient(stateDir, coreURL, client)
	if err != nil {
		return err
	}

	// Register the room so it is live before any device dials (design §6.1 step 2).
	// Idempotent/TOFU — a running relay process registering the same room is fine.
	authHash, err := deriveRoomAuthHash(link.Secret)
	if err != nil {
		return fmt.Errorf("code linking: auth_hash derive: %w", err)
	}
	rctx, rcancel := withTimeout(ctx, pushTimeout)
	regErr := cc.register(rctx, link.Room, link.Name, authHash)
	rcancel()
	if regErr != nil {
		return fmt.Errorf("code linking: register room: %w", regErr)
	}

	// Generate a code + open the session, regenerating on channel collision.
	box, channel, display, err := createSession(ctx, cc, ci, opts.forceCode)
	if err != nil {
		return err
	}

	fmt.Fprintf(out, "code: %s   (expires in %s)\n", display, mmss(ttl))
	fmt.Fprintln(out, "waiting for the code to be typed…")

	// Guaranteed-once teardown (BLOCKER 2): the code is now live on the relay, so
	// every exit from here — success, strike cap, TTL, any error, or Ctrl-C/parent
	// cancel — MUST invalidate the session exactly once, or a dropped box leaves a
	// claimable code parked. abortSession uses a detached context so it still fires
	// after cancellation. Disarmed only on the happy-path finish ok, which already
	// released and single-use deleted the session; every other exit path (strike
	// cap, TTL, error, Ctrl-C) leaves this armed so the detached teardown is the
	// sole, cancellation-proof owner of the one final abort.
	sessionDone := false
	defer func() {
		if !sessionDone {
			abortSession(cc, channel)
		}
	}()

	deadline := now().Add(ttl)
	strikes := 0
	for now().Before(deadline) {
		if ctx.Err() != nil { // Ctrl-C / parent cancel: bail before polling again.
			return ctx.Err() // deferred teardown fires the detached final abort.
		}
		remaining := deadline.Sub(now())
		wait := pollWait
		if wait > remaining {
			wait = remaining
		}
		pctx, pcancel := context.WithTimeout(ctx, wait+2*time.Second)
		pend, perr := cc.pollLinkCode(pctx, channel)
		pcancel()
		if perr != nil {
			if ctx.Err() != nil { // Ctrl-C / parent cancel.
				return ctx.Err() // deferred teardown fires the detached final abort.
			}
			// Transient relay/network hiccup: back off briefly, then re-poll (still
			// bounded by the box TTL deadline).
			select {
			case <-ctx.Done():
				return ctx.Err()
			case <-time.After(time.Second):
			}
			continue
		}
		if pend == nil {
			continue // park elapsed with nothing claimed — poll again.
		}
		if opts.afterPoll != nil {
			opts.afterPoll() // test hook: cancel-after-pending window (before verify).
		}
		// TTL recheck (BLOCKER 1): a pending attempt that arrived AFTER the deadline
		// must never be verified or sealed — a late poll cannot resurrect a code or
		// release the payload. Break to the final abort + ErrCodeExpired below.
		if !now().Before(deadline) {
			break
		}

		accepted, verr := box.Verify(pend.MsgB, pend.ConfirmB)
		if verr != nil {
			if !ref.IsFailedAttempt(verr) {
				return fmt.Errorf("code linking: verify: %w", verr)
			}
			// A failed confirm_b is THE online-guess detector — count it (design §4.6).
			strikes++
			if strikes >= maxStrikes {
				// Strike cap reached: abort the whole session and invalidate the code.
				// Do NOT fire the final abort here with the caller's (possibly already
				// cancelled) ctx, and do NOT set sessionDone — leave the single deferred
				// detached teardown as the sole owner of the final abort. That keeps the
				// "exactly one final abort on every exit path, never suppressed by
				// cancellation" invariant: a Ctrl-C that cancelled ctx after a pending
				// attempt arrived can no longer stop the strikeout teardown from reaching
				// the relay (the deferred abortSession uses context.Background()).
				fmt.Fprintln(out, "too many wrong attempts — code cancelled")
				return ErrCodeStrikeout // deferred teardown fires the detached final abort.
			}
			// Mismatch verdict for this attempt: echo the polled aid.
			fctx, fcancel := withTimeout(ctx, pushTimeout)
			_ = cc.finishLinkCode(fctx, channel, false, pend.Aid, "", "", "", false)
			fcancel()
			fmt.Fprintf(out, "wrong code entered (%d/%d) — still waiting…\n", strikes, maxStrikes)
			continue
		}

		// confirm_b verified: seal the pair URI under K_pay and release it. Only a
		// *BoxAccepted can seal, and it exists only past the confirm check.
		nonce := make([]byte, ref.NonceBytes)
		if _, rerr := rand.Read(nonce); rerr != nil {
			return rerr
		}
		plaintext, perr := json.Marshal(map[string]string{"uri": link.URI})
		if perr != nil {
			return perr
		}
		payloadN, payloadC, serr := accepted.SealPayload(nonce, plaintext)
		if serr != nil {
			return fmt.Errorf("code linking: seal payload: %w", serr)
		}
		fctx, fcancel := withTimeout(ctx, pushTimeout)
		ferr := cc.finishLinkCode(fctx, channel, true, pend.Aid, accepted.ConfirmA(), payloadN, payloadC, false)
		fcancel()
		if ferr != nil {
			return fmt.Errorf("code linking: finish: %w", ferr)
		}
		fmt.Fprintln(out, "claimed ✓ — waiting for the device to connect")
		sessionDone = true // finish ok single-use deleted the session on the relay.
		return nil
	}

	fmt.Fprintln(out, "code expired — mint a new one")
	return ErrCodeExpired // deferred teardown fires the detached final abort.
}

// createSession picks a code, derives the box CPace state, and opens the relay
// session, regenerating on a 409 channel collision (design §6.1 step 3).
func createSession(ctx context.Context, cc *coreClient, ci []byte, forceCode string) (box *ref.BoxInit, channel, display string, err error) {
	retries := codeCreateRetries
	if forceCode != "" {
		retries = 1
	}
	for attempt := 0; attempt < retries; attempt++ {
		normalized, disp, gerr := generateCode(forceCode)
		if gerr != nil {
			return nil, "", "", gerr
		}
		seed := make([]byte, ref.UniformScalarBytes)
		if _, rerr := rand.Read(seed); rerr != nil {
			return nil, "", "", rerr
		}
		sid := ref.Channel(normalized) // channel = sid = first 4 symbols (SPEC §2).
		b, berr := ref.NewBoxInit([]byte(normalized), ci, sid, seed)
		if berr != nil {
			if errors.Is(berr, ref.ErrZeroScalar) {
				attempt-- // resample the scalar; the code was fine.
				continue
			}
			return nil, "", "", fmt.Errorf("code linking: init: %w", berr)
		}
		cctx, ccancel := withTimeout(ctx, pushTimeout)
		_, cerr := cc.createLinkCode(cctx, sid, b.MsgA())
		ccancel()
		if errors.Is(cerr, errCodeTaken) {
			if forceCode != "" {
				return nil, "", "", cerr
			}
			continue // channel collision (2^20) — regenerate.
		}
		if cerr != nil {
			return nil, "", "", cerr
		}
		return b, sid, disp, nil
	}
	return nil, "", "", fmt.Errorf("code linking: could not open a free code after %d attempts", retries)
}

// abortSession best-effort tells the relay to delete the session (design §4.6 /
// §6.1 step 6). Uses a detached context so it still fires on parent cancellation.
func abortSession(cc *coreClient, channel string) {
	ctx, cancel := withTimeout(context.Background(), pushTimeout)
	defer cancel()
	_ = cc.finishLinkCode(ctx, channel, false, 0, "", "", "", true)
}

// generateCode returns a normalized 12-symbol code and its XXXX-XXXX-XXXX display
// form. With forceCode set it echoes that (test hook), validating it round-trips
// through WP-CL0 normalization.
func generateCode(forceCode string) (normalized, display string, err error) {
	if forceCode != "" {
		normalized, err = ref.NormalizeCode(forceCode)
		if err != nil {
			return "", "", err
		}
	} else {
		buf := make([]byte, 12)
		if _, err = rand.Read(buf); err != nil {
			return "", "", err
		}
		out := make([]byte, 12)
		for i, b := range buf {
			out[i] = crockfordAlphabet[int(b)%len(crockfordAlphabet)] // 256%32==0 ⇒ unbiased.
		}
		normalized = string(out)
	}
	display = normalized[:4] + "-" + normalized[4:8] + "-" + normalized[8:]
	return normalized, display, nil
}

// mmss renders a whole-second duration as M:SS for the "(expires in …)" line.
func mmss(d time.Duration) string {
	s := int(d.Round(time.Second) / time.Second)
	return fmt.Sprintf("%d:%02d", s/60, s%60)
}
