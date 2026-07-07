// Package schedule implements hotline's proactive scheduling: persisted
// scheduled prompts that fire as synthetic inbound turns through the same
// sink real messages use. State lives in schedules.json at the state root,
// guarded by the same flock/atomic-write pattern as access.json.
package schedule

import (
	"fmt"
	"strconv"
	"strings"
	"time"
)

// Recurrence kinds. Deliberately a small preset enum, not cron (see scoping doc).
const (
	KindOnce        = "once"
	KindDaily       = "daily"
	KindWeekly      = "weekly"
	KindEveryNHours = "every_n_hours"
	KindEveryNDays  = "every_n_days"
)

// maxEveryNHours / maxEveryNDays bound the interval kinds; the 1-hour floor
// is the runaway guard the scoping doc calls for.
const (
	maxEveryNHours = 720 // 30 days
	maxEveryNDays  = 365
)

// Recurrence describes when a schedule fires. TimeOfDay is 24h "HH:MM"
// (required for daily/weekly, optional for every_n_days); Weekday is a
// lowercase English day name (weekly only); EveryN is the interval count
// for the every_n_* kinds. once carries no fields here — its single fire
// time lives directly in Schedule.NextFire.
type Recurrence struct {
	Kind      string `json:"kind"`
	TimeOfDay string `json:"timeOfDay,omitempty"`
	Weekday   string `json:"weekday,omitempty"`
	EveryN    int    `json:"everyN,omitempty"`
}

// Schedule is one persisted scheduled task. Times are RFC3339 UTC strings
// (matching access.json's Pending.ExpiresAt convention); recurrence math
// converts through the process-local zone.
type Schedule struct {
	ID         string     `json:"id"`                  // 6-hex, unique in the doc
	Prompt     string     `json:"prompt"`              // trusted text, authored via tool/CLI
	Source     string     `json:"source"`              // provider name the fire is addressed to
	ChatID     string     `json:"chatId"`              // chat the fire is addressed to
	CreatedBy  string     `json:"createdBy,omitempty"` // "agent" (MCP tool) — CLI has no add in v1
	CreatedAt  string     `json:"createdAt"`           // RFC3339 UTC
	Recurrence Recurrence `json:"recurrence"`
	NextFire   string     `json:"nextFire"`            // RFC3339 UTC
	LastFired  string     `json:"lastFired,omitempty"` // RFC3339 UTC
	Paused     bool       `json:"paused,omitempty"`    // zero value = active
}

// Doc is the full persisted schedules.json document.
type Doc struct {
	Schedules []Schedule `json:"schedules"`
}

// Validate checks a Recurrence is well-formed. Unknown kinds, missing
// required fields, and out-of-range intervals are errors.
func (r Recurrence) Validate() error {
	switch r.Kind {
	case KindOnce:
		return nil
	case KindDaily:
		if _, _, err := ParseTimeOfDay(r.TimeOfDay); err != nil {
			return fmt.Errorf("daily requires time_of_day: %w", err)
		}
		return nil
	case KindWeekly:
		if _, _, err := ParseTimeOfDay(r.TimeOfDay); err != nil {
			return fmt.Errorf("weekly requires time_of_day: %w", err)
		}
		if _, err := ParseWeekday(r.Weekday); err != nil {
			return fmt.Errorf("weekly requires weekday: %w", err)
		}
		return nil
	case KindEveryNHours:
		if r.EveryN < 1 || r.EveryN > maxEveryNHours {
			return fmt.Errorf("every_n_hours needs every_n in 1..%d, got %d", maxEveryNHours, r.EveryN)
		}
		return nil
	case KindEveryNDays:
		if r.EveryN < 1 || r.EveryN > maxEveryNDays {
			return fmt.Errorf("every_n_days needs every_n in 1..%d, got %d", maxEveryNDays, r.EveryN)
		}
		if r.TimeOfDay != "" {
			if _, _, err := ParseTimeOfDay(r.TimeOfDay); err != nil {
				return fmt.Errorf("every_n_days time_of_day invalid: %w", err)
			}
		}
		return nil
	default:
		return fmt.Errorf("unknown recurrence kind %q", r.Kind)
	}
}

// ParseTimeOfDay parses 24-hour "HH:MM" (leading zero optional: "9:00" ok).
// Hand-rolled rather than time.Parse, so the accepted grammar is explicit.
func ParseTimeOfDay(s string) (hh, mm int, err error) {
	h, m, ok := strings.Cut(s, ":")
	if !ok {
		return 0, 0, fmt.Errorf("time_of_day %q: want HH:MM", s)
	}
	hh, err = strconv.Atoi(strings.TrimSpace(h))
	if err != nil || hh < 0 || hh > 23 {
		return 0, 0, fmt.Errorf("time_of_day %q: hour out of range", s)
	}
	mm, err = strconv.Atoi(strings.TrimSpace(m))
	if err != nil || mm < 0 || mm > 59 {
		return 0, 0, fmt.Errorf("time_of_day %q: minute out of range", s)
	}
	return hh, mm, nil
}

// weekdayNames maps a lowercase English day name to time.Weekday.
var weekdayNames = map[string]time.Weekday{
	"sunday":    time.Sunday,
	"monday":    time.Monday,
	"tuesday":   time.Tuesday,
	"wednesday": time.Wednesday,
	"thursday":  time.Thursday,
	"friday":    time.Friday,
	"saturday":  time.Saturday,
}

// ParseWeekday maps a lowercase English day name ("monday".."sunday",
// case-insensitive) to time.Weekday.
func ParseWeekday(s string) (time.Weekday, error) {
	wd, ok := weekdayNames[strings.ToLower(strings.TrimSpace(s))]
	if !ok {
		return time.Sunday, fmt.Errorf("weekday %q: want monday..sunday", s)
	}
	return wd, nil
}

// maxOnceRelative bounds a "+duration" once schedule, matching the taste of
// maxEveryNDays's 365-day runaway guard.
const maxOnceRelative = 365 * 24 * time.Hour

// ParseOnceAt parses the one-shot fire time. Three forms, tried in order: a
// "+duration" relative offset from now (stdlib time.ParseDuration grammar,
// e.g. "+2m", "+1h30m" — h/m/s units only, no day unit, since day-scale
// reminders are already unambiguous for a model to phrase as an absolute
// time); RFC3339 (timezone explicit); or "2006-01-02T15:04" interpreted in
// loc. The relative form exists so an agent asked "remind me in 2 minutes"
// never needs to shell out to check the clock — now.Add is the same
// absolute-duration arithmetic Advance's every_n_hours case already uses, so
// it's DST-proof for the same reason.
func ParseOnceAt(s string, now time.Time, loc *time.Location) (time.Time, error) {
	s = strings.TrimSpace(s)
	if s == "" {
		return time.Time{}, fmt.Errorf("at is required for once")
	}
	if rest, ok := strings.CutPrefix(s, "+"); ok {
		d, err := time.ParseDuration(rest)
		if err != nil || d <= 0 {
			return time.Time{}, fmt.Errorf("at %q: want +duration like +2m or +1h30m (units h/m/s)", s)
		}
		if d > maxOnceRelative {
			return time.Time{}, fmt.Errorf("at %q: relative offset over %s", s, maxOnceRelative)
		}
		return now.Add(d), nil
	}
	if t, err := time.Parse(time.RFC3339, s); err == nil {
		return t, nil
	}
	if t, err := time.ParseInLocation("2006-01-02T15:04", s, loc); err == nil {
		return t, nil
	}
	return time.Time{}, fmt.Errorf("at %q: want +duration, RFC3339, or YYYY-MM-DDTHH:MM", s)
}

// Describe renders a Recurrence for humans and for the fire preamble.
func Describe(r Recurrence) string {
	switch r.Kind {
	case KindOnce:
		return "once"
	case KindDaily:
		hh, mm, _ := ParseTimeOfDay(r.TimeOfDay)
		return fmt.Sprintf("daily at %02d:%02d", hh, mm)
	case KindWeekly:
		hh, mm, _ := ParseTimeOfDay(r.TimeOfDay)
		return fmt.Sprintf("weekly on %s at %02d:%02d", strings.ToLower(r.Weekday), hh, mm)
	case KindEveryNHours:
		if r.EveryN == 1 {
			return "every hour"
		}
		return fmt.Sprintf("every %d hours", r.EveryN)
	case KindEveryNDays:
		unit := "days"
		if r.EveryN == 1 {
			unit = "day"
		}
		base := fmt.Sprintf("every %d %s", r.EveryN, unit)
		if r.EveryN == 1 {
			base = "every day"
		}
		if r.TimeOfDay != "" {
			hh, mm, _ := ParseTimeOfDay(r.TimeOfDay)
			return fmt.Sprintf("%s at %02d:%02d", base, hh, mm)
		}
		return base
	default:
		return r.Kind
	}
}

// First computes the initial NextFire for a newly created (or resumed)
// schedule. once is the pre-parsed one-shot time (ParseOnceAt), used only
// when r.Kind == KindOnce. The result is always strictly after now.
func First(r Recurrence, once time.Time, now time.Time, loc *time.Location) (time.Time, error) {
	if err := r.Validate(); err != nil {
		return time.Time{}, err
	}
	switch r.Kind {
	case KindOnce:
		if !once.After(now) {
			return time.Time{}, fmt.Errorf("at is in the past")
		}
		return once, nil
	case KindDaily:
		return dailyNext(r, now, loc), nil
	case KindWeekly:
		return weeklyNext(r, now, loc), nil
	case KindEveryNHours:
		// First fire is one interval out; if the user wants the task now, the
		// agent just does it now.
		return now.Add(time.Duration(r.EveryN) * time.Hour), nil
	case KindEveryNDays:
		if r.TimeOfDay != "" {
			// Next occurrence of HH:MM, then steps by EveryN (identical to daily's
			// First).
			return dailyNext(r, now, loc), nil
		}
		n := now.In(loc)
		return time.Date(n.Year(), n.Month(), n.Day()+r.EveryN, n.Hour(), n.Minute(), 0, 0, loc), nil
	default:
		return time.Time{}, fmt.Errorf("unknown recurrence kind %q", r.Kind)
	}
}

// Advance computes the fire after prev (the NextFire that just fired),
// strictly after now. ok=false means no next fire (once) — delete the
// schedule. r must already be valid.
func Advance(r Recurrence, prev, now time.Time, loc *time.Location) (next time.Time, ok bool) {
	switch r.Kind {
	case KindOnce:
		return time.Time{}, false
	case KindDaily:
		// Wall-clock-anchored: recomputing from now is equivalent once now >= prev.
		return dailyNext(r, now, loc), true
	case KindWeekly:
		return weeklyNext(r, now, loc), true
	case KindEveryNHours:
		// Anchored to the grid, no drift, single catch-up. Absolute durations
		// deliberately (an "every 4 hours" check crossing a DST shift stays 4 real
		// hours apart).
		step := time.Duration(r.EveryN) * time.Hour
		next = prev.Add(step)
		if !next.After(now) {
			k := now.Sub(next)/step + 1 // integer Duration division, truncates
			next = next.Add(time.Duration(k) * step)
			if !next.After(now) { // exact-boundary guard
				next = next.Add(step)
			}
		}
		return next, true
	case KindEveryNDays:
		// Anchored, calendar-aware. time.Date in loc gives wall-clock-preserving
		// day steps.
		p := prev.In(loc)
		hh, mm := p.Hour(), p.Minute()
		if r.TimeOfDay != "" {
			hh, mm, _ = ParseTimeOfDay(r.TimeOfDay)
		}
		next = time.Date(p.Year(), p.Month(), p.Day()+r.EveryN, hh, mm, 0, 0, loc)
		for !next.After(now) {
			next = time.Date(next.Year(), next.Month(), next.Day()+r.EveryN, hh, mm, 0, 0, loc)
		}
		return next, true
	default:
		return time.Time{}, false
	}
}

// dailyNext returns the next HH:MM occurrence strictly after now, in loc.
// time.Date normalizes day overflow and preserves wall-clock semantics across
// DST (never Add(24h) for a calendar step). A nonexistent local time
// (spring-forward 02:30) normalizes forward per Go semantics; an ambiguous
// time (fall-back) resolves to Go's default choice. Both are accepted v1.
func dailyNext(r Recurrence, now time.Time, loc *time.Location) time.Time {
	hh, mm, _ := ParseTimeOfDay(r.TimeOfDay)
	n := now.In(loc)
	cand := time.Date(n.Year(), n.Month(), n.Day(), hh, mm, 0, 0, loc)
	if !cand.After(now) {
		cand = time.Date(n.Year(), n.Month(), n.Day()+1, hh, mm, 0, 0, loc)
	}
	return cand
}

// weeklyNext returns the next Weekday-at-HH:MM occurrence strictly after now.
// The strict-After with delta==0 covers "it's the target weekday now": before
// HH:MM fires today, at/after rolls +7. The %7 with +7 handles a
// target-weekday-earlier-in-week without negative modulo.
func weeklyNext(r Recurrence, now time.Time, loc *time.Location) time.Time {
	hh, mm, _ := ParseTimeOfDay(r.TimeOfDay)
	wd, _ := ParseWeekday(r.Weekday)
	n := now.In(loc)
	delta := (int(wd) - int(n.Weekday()) + 7) % 7 // 0..6; 0 == today
	cand := time.Date(n.Year(), n.Month(), n.Day()+delta, hh, mm, 0, 0, loc)
	if !cand.After(now) { // today but already passed
		cand = time.Date(n.Year(), n.Month(), n.Day()+delta+7, hh, mm, 0, 0, loc)
	}
	return cand
}
