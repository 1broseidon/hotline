// Package catchup redelivers inbound user turns that were journaled to the
// transcript but never answered before the harness (re)started.
//
// The box journals every inbound turn to transcript.jsonl before/independent of
// the harness actually consuming it. If the harness was deaf (a room rotation
// left it bound to the old room) or simply restarted mid-turn, a message can be
// durably recorded yet never seen by any live harness — the turn is silently
// lost (soak bug 2026-07-14: George's 04:12 "status update" vanished across a
// manual harness restart).
//
// On harness (re)start ReplayUnanswered replays the unanswered turns. The
// high-water mark is a per-message "delivered" meta marker, journaled when the
// box successfully hands an inbound to a LIVE harness (see transcript.MarkDelivered
// at the provider/app delivery sites). An inbound whose message_id carries a
// delivered marker was seen by a running harness and is never replayed.
//
// This deliberately does NOT use "any outbound record acks all prior inbound":
// George's overnight email-triage/loops journal job/notify/schedule OUTBOUND
// records, and any one of those would falsely ack a user turn the harness never
// saw — the exact soak bug catch-up exists to fix (round-2 review B-1).
//
// Replay is capped: each attempt journals a "replay" meta marker keyed on the
// message_id, and a turn is redelivered at most maxReplayAttempts times so a
// turn whose processing keeps crashing the harness cannot drive an unbounded
// replay/crash loop (round-2 review S-1). A recency window bounds a cold
// transcript so ancient history is never resurrected.
package catchup

import (
	"bufio"
	"context"
	"encoding/json"
	"fmt"
	"os"
	"time"

	"github.com/1broseidon/hotline/internal/transcript"
)

// DefaultWindow bounds how far back an unanswered inbound turn may be and still
// be replayed. It exists for the cold-start case (a transcript with no outbound
// record at all, so every inbound would otherwise qualify): a box that has been
// down for longer than this replays nothing, matching "avoid replaying ancient
// history". At human texting rates the answered-turn high-water almost always
// bounds the set first; the window is the backstop.
const DefaultWindow = 6 * time.Hour

// maxReplayAttempts caps how many times a single unanswered turn is redelivered
// across restarts. After the cap the turn is logged and skipped, never replayed
// again — a poison turn that crashes the harness each time cannot loop forever
// under the supervisor (round-2 review S-1).
const maxReplayAttempts = 2

// tsFormat is transcript.Logger's timestamp layout (see transcript.Append).
const tsFormat = "2006-01-02T15:04:05.000Z07:00"

// Sink is the inbound-delivery surface catchup needs — satisfied by
// provider.InboundSink and the per-harness sinks that wrap it.
type Sink interface {
	SendChannel(ctx context.Context, content string, meta map[string]string) error
}

// skipKinds are inbound record kinds the generic catch-up must never replay.
// schedule/notify are box-generated fires (replaying re-triggers them). "fleet"
// is excluded because a fleet turn's provenance (source="fleet", the untrusted-
// peer framing, the fleet Provider) lives OUTSIDE the transcript row: a generic
// replay would re-inject a peer's words source-less into the OPERATOR provider,
// stripping the trust marker and the fleet sink. Fleet has its own, provenance-
// preserving startup replay (app.replayUndeliveredFleetInbound, keyed on the
// delivered-cid set in edge state), so it owns redelivery end-to-end (H2).
var skipKinds = map[string]bool{"schedule": true, "notify": true, "fleet": true}

// ReplayUnanswered redelivers every unanswered inbound user turn in the
// transcript at transcriptPath through sink, in order, each marked as catch-up.
// An inbound turn is "unanswered" when its message_id carries no delivered
// marker (it was never handed to a live harness). now/window are the recency
// bound (use time.Now() and DefaultWindow in production). log records one
// replay-attempt marker per redelivered turn so attempts are capped across
// restarts (nil is a valid no-op logger). It returns the number of turns
// replayed. A missing transcript is not an error (nothing to replay). A delivery
// error stops replay and is returned along with the count delivered so far.
func ReplayUnanswered(ctx context.Context, sink Sink, log *transcript.Logger, transcriptPath string, now time.Time, window time.Duration) (int, error) {
	recs, err := readRecords(transcriptPath)
	if err != nil {
		return 0, err
	}

	// Bookkeeping scan: which inbound message_ids were delivered to a live
	// harness, and how many replay attempts each has already burned.
	delivered := map[string]bool{}
	attempts := map[string]int{}
	for _, r := range recs {
		if r.Dir != transcript.DirMeta || r.MessageID == "" {
			continue
		}
		switch r.Kind {
		case transcript.KindDelivered:
			delivered[r.MessageID] = true
		case transcript.KindReplay:
			attempts[r.MessageID]++
		}
	}

	cutoff := now.Add(-window)
	replayed := 0
	for _, r := range recs {
		if r.Dir != "in" || skipKinds[r.Kind] {
			continue
		}
		if r.MessageID != "" && delivered[r.MessageID] {
			continue // already handed to a live harness — answered.
		}
		if ts, ok := parseTS(r.TS); ok && ts.Before(cutoff) {
			continue
		}
		if r.MessageID != "" && attempts[r.MessageID] >= maxReplayAttempts {
			// Poison/never-answered turn: stop redelivering it (S-1).
			fmt.Fprintf(os.Stderr, "hotline: catch-up giving up on message_id=%s after %d replay attempts\n", r.MessageID, attempts[r.MessageID])
			continue
		}
		// Reserve the attempt BEFORE delivery so a redelivery that crashes the
		// harness still counts against the cap on the next restart.
		if err := log.MarkReplay(r.MessageID); err != nil {
			fmt.Fprintf(os.Stderr, "hotline: catch-up mark-replay failed message_id=%s: %v\n", r.MessageID, err)
		}
		attempts[r.MessageID]++
		content, meta := renderCatchup(r)
		if err := sink.SendChannel(ctx, content, meta); err != nil {
			return replayed, fmt.Errorf("catchup replay: %w", err)
		}
		replayed++
	}
	return replayed, nil
}

// renderCatchup rebuilds the inbound (content, meta) from a transcript record and
// prefixes a catch-up marker so the agent knows this turn arrived while it was
// offline and is only now being delivered. The meta shape mirrors what the
// providers stamp on a live inbound (chat_id/user/user_id/kind/message_id/ts),
// so it flows through the identical sink path a fresh message would.
func renderCatchup(r transcript.Record) (string, map[string]string) {
	meta := map[string]string{}
	set := func(k, v string) {
		if v != "" {
			meta[k] = v
		}
	}
	set("chat_id", r.ChatID)
	set("user", r.User)
	set("user_id", r.UserID)
	set("kind", r.Kind)
	set("message_id", r.MessageID)
	set("ts", r.TS)

	marker := fmt.Sprintf("[catch-up: this message arrived at %s while I was restarting and was not yet answered]\n", r.TS)
	return marker + r.Text, meta
}

func parseTS(s string) (time.Time, bool) {
	t, err := time.Parse(tsFormat, s)
	if err != nil {
		return time.Time{}, false
	}
	return t, true
}

func readRecords(path string) ([]transcript.Record, error) {
	f, err := os.Open(path)
	if err != nil {
		if os.IsNotExist(err) {
			return nil, nil
		}
		return nil, err
	}
	defer f.Close()
	var out []transcript.Record
	sc := bufio.NewScanner(f)
	sc.Buffer(make([]byte, 0, 64*1024), 4*1024*1024)
	for sc.Scan() {
		line := sc.Bytes()
		if len(line) == 0 {
			continue
		}
		var r transcript.Record
		if json.Unmarshal(line, &r) != nil {
			continue // tolerate a partial/corrupt tail line
		}
		out = append(out, r)
	}
	if err := sc.Err(); err != nil {
		return nil, err
	}
	return out, nil
}
