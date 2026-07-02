package access

import (
	"crypto/rand"
	"encoding/hex"
	"fmt"
	"regexp"
	"slices"
	"strings"
	"time"
)

// Decision is the gate's verdict for an inbound message.
type Decision int

const (
	// Drop silently discards the message.
	Drop Decision = iota
	// Pair means the sender is unknown under a pairing policy; reply with a code.
	Pair
	// Allow means the message should be delivered to Claude.
	Allow
)

// GateInput is the gate's view of an inbound message. The gate decides on the
// SENDER, never the chat — a non-allowlisted user in an allowlisted group is
// still dropped.
type GateInput struct {
	IsGroup      bool
	ChatID       string
	SenderID     string
	Text         string
	MentionedBot bool
	RepliedToBot bool
}

// pairingTTL is how long an unapproved pairing code stays valid.
const pairingTTL = 24 * time.Hour

// maxPairingReplies caps how many times we re-prompt a pending sender before
// going silent (initial reply counts as 1).
const maxPairingReplies = 5

// maxPending caps the total number of live pending pairings, so a flood of
// requests from many distinct senders can't grow access.json unboundedly.
const maxPending = 3

// Gate decides what to do with an inbound message based on the access policy.
//
//   - disabled: Drop everything (incl. allowlisted senders and groups).
//   - DM, sender in AllowFrom: Allow.
//   - DM, pairing policy, unknown sender: Pair.
//   - DM, allowlist policy, unknown sender: Drop (silent).
//   - group not configured: Drop.
//   - group with non-empty AllowFrom and sender absent: Drop.
//   - group requiring a mention with none present: Drop.
//   - otherwise: Allow.
func Gate(a *Access, in GateInput) Decision {
	if a.DMPolicy == "disabled" {
		return Drop
	}

	if in.IsGroup {
		policy, ok := a.Groups[in.ChatID]
		if !ok {
			return Drop
		}
		if len(policy.AllowFrom) > 0 && !contains(policy.AllowFrom, in.SenderID) {
			return Drop
		}
		if policy.RequireMention && !in.MentionedBot && !in.RepliedToBot && !MatchesMentionPattern(a, in.Text) {
			return Drop
		}
		return Allow
	}

	// Direct message.
	if contains(a.AllowFrom, in.SenderID) {
		return Allow
	}
	if a.DMPolicy == "allowlist" {
		return Drop
	}
	// pairing policy with an unknown sender.
	return Pair
}

// MatchesMentionPattern reports whether text matches any configured (and valid)
// mention regexp, case-insensitively. Invalid patterns are skipped.
func MatchesMentionPattern(a *Access, text string) bool {
	for _, pat := range a.MentionPatterns {
		re, err := regexp.Compile("(?i)" + pat)
		if err != nil {
			continue
		}
		if re.MatchString(text) {
			return true
		}
	}
	return false
}

// NewPairingCode returns a fresh 6-hex-character pairing code.
func NewPairingCode() string {
	var b [3]byte
	_, _ = rand.Read(b[:])
	return hex.EncodeToString(b[:])
}

// CreatePairing records (or reuses) a pending pairing for a DM sender. It
// returns the code, whether a reply should be sent (false once the re-prompt
// rate limit is hit), and any error.
//
// The whole read-modify-write runs under the access.json flock so two poller
// processes can't clobber each other's pending map.
func CreatePairing(accessFile, senderID, chatID string) (code string, send bool, err error) {
	err = Mutate(accessFile, func(a *Access) error {
		PurgeExpired(a)

		// Reuse an existing pending code for this sender.
		for c, p := range a.Pending {
			if p.SenderID == senderID {
				if p.Replies >= maxPairingReplies {
					code, send = c, false
					return nil
				}
				p.Replies++
				a.Pending[c] = p
				code, send = c, true
				return nil
			}
		}

		// Cap the total number of live pending entries (PurgeExpired already ran,
		// so only live entries count). Drop silently once the cap is reached.
		if len(a.Pending) >= maxPending {
			code, send = "", false
			return nil
		}

		now := time.Now()
		c := NewPairingCode()
		a.Pending[c] = Pending{
			SenderID:  senderID,
			ChatID:    chatID,
			CreatedAt: now.UTC().Format(time.RFC3339),
			ExpiresAt: now.Add(pairingTTL).UTC().Format(time.RFC3339),
			Replies:   1,
		}
		code, send = c, true
		return nil
	})
	return code, send, err
}

// ApprovePairing moves a pending entry into AllowFrom and returns it.
func ApprovePairing(accessFile, code string) (Pending, error) {
	var approved Pending
	err := Mutate(accessFile, func(a *Access) error {
		p, ok := a.Pending[code]
		if !ok {
			return ErrNotPending
		}
		approved = p
		delete(a.Pending, code)
		if !contains(a.AllowFrom, p.SenderID) {
			a.AllowFrom = append(a.AllowFrom, p.SenderID)
		}
		return nil
	})
	return approved, err
}

// DenyPairing removes a pending entry without allowlisting it.
func DenyPairing(accessFile, code string) error {
	return Mutate(accessFile, func(a *Access) error {
		if _, ok := a.Pending[code]; !ok {
			return ErrNotPending
		}
		delete(a.Pending, code)
		return nil
	})
}

// RevokeSender removes an approved sender from AllowFrom and purges any of
// their pending pairings, so a revoked sender has no live code left. The id
// may be the exact sender ID or a unique prefix of one; an ambiguous prefix
// or an unknown id is an error listing the candidates. Returns the resolved
// sender ID and how many allowlisted senders remain.
func RevokeSender(accessFile, id string) (revoked string, remaining int, err error) {
	err = Mutate(accessFile, func(a *Access) error {
		match := ""
		if contains(a.AllowFrom, id) {
			match = id
		} else {
			var candidates []string
			for _, s := range a.AllowFrom {
				if strings.HasPrefix(s, id) {
					candidates = append(candidates, s)
				}
			}
			switch len(candidates) {
			case 1:
				match = candidates[0]
			case 0:
				return fmt.Errorf("%w: %q", ErrSenderNotAllowed, id)
			default:
				return fmt.Errorf("ambiguous sender %q: matches %s", id, strings.Join(candidates, ", "))
			}
		}
		a.AllowFrom = slices.DeleteFunc(a.AllowFrom, func(s string) bool { return s == match })
		for code, p := range a.Pending {
			if p.SenderID == match {
				delete(a.Pending, code)
			}
		}
		revoked, remaining = match, len(a.AllowFrom)
		return nil
	})
	return revoked, remaining, err
}

// PurgeExpired removes pending entries past their ExpiresAt.
func PurgeExpired(a *Access) {
	now := time.Now()
	for code, p := range a.Pending {
		exp, err := time.Parse(time.RFC3339, p.ExpiresAt)
		if err != nil || now.After(exp) {
			delete(a.Pending, code)
		}
	}
}

func contains(xs []string, x string) bool {
	return slices.Contains(xs, x)
}
