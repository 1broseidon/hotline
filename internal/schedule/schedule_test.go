package schedule

import (
	"testing"
	"time"
)

// tst is a fixed non-UTC zone for deterministic wall-clock math (no reliance on
// the host's time.Local).
var tst = time.FixedZone("TST", 3*3600)

func at(loc *time.Location, y int, mo time.Month, d, hh, mm int) time.Time {
	return time.Date(y, mo, d, hh, mm, 0, 0, loc)
}

func TestParseTimeOfDay(t *testing.T) {
	ok := []struct {
		in     string
		hh, mm int
	}{
		{"09:00", 9, 0}, {"9:00", 9, 0}, {"23:59", 23, 59}, {"0:0", 0, 0}, {"00:00", 0, 0},
	}
	for _, c := range ok {
		hh, mm, err := ParseTimeOfDay(c.in)
		if err != nil || hh != c.hh || mm != c.mm {
			t.Errorf("ParseTimeOfDay(%q) = (%d,%d,%v), want (%d,%d,nil)", c.in, hh, mm, err, c.hh, c.mm)
		}
	}
	for _, bad := range []string{"", "9", "24:00", "12:60", "-1:00", "aa:bb", "9:00:00", "1200"} {
		if _, _, err := ParseTimeOfDay(bad); err == nil {
			t.Errorf("ParseTimeOfDay(%q) should error", bad)
		}
	}
}

func TestParseWeekday(t *testing.T) {
	cases := map[string]time.Weekday{
		"monday": time.Monday, "Sunday": time.Sunday, "SATURDAY": time.Saturday, " friday ": time.Friday,
	}
	for in, want := range cases {
		got, err := ParseWeekday(in)
		if err != nil || got != want {
			t.Errorf("ParseWeekday(%q) = (%v,%v), want %v", in, got, err, want)
		}
	}
	for _, bad := range []string{"", "funday", "mon"} {
		if _, err := ParseWeekday(bad); err == nil {
			t.Errorf("ParseWeekday(%q) should error", bad)
		}
	}
}

func TestParseOnceAt(t *testing.T) {
	now := at(tst, 2026, 7, 8, 8, 0)
	// Local form is interpreted in loc.
	got, err := ParseOnceAt("2026-07-08T09:00", now, tst)
	if err != nil {
		t.Fatal(err)
	}
	if want := at(tst, 2026, 7, 8, 9, 0); !got.Equal(want) {
		t.Errorf("local form = %v, want %v", got, want)
	}
	// RFC3339 keeps its explicit offset.
	got, err = ParseOnceAt("2026-07-08T09:00:00Z", now, tst)
	if err != nil {
		t.Fatal(err)
	}
	if !got.Equal(time.Date(2026, 7, 8, 9, 0, 0, 0, time.UTC)) {
		t.Errorf("rfc3339 form = %v", got)
	}
	for _, bad := range []string{"", "not-a-time", "2026-13-01T00:00"} {
		if _, err := ParseOnceAt(bad, now, tst); err == nil {
			t.Errorf("ParseOnceAt(%q) should error", bad)
		}
	}
}

func TestParseOnceAtRelative(t *testing.T) {
	now := at(tst, 2026, 7, 8, 8, 0)
	for _, tc := range []struct {
		in   string
		want time.Duration
	}{
		{"+2m", 2 * time.Minute},
		{"+45m", 45 * time.Minute},
		{"+1h30m", 90 * time.Minute},
		{"+1.5h", 90 * time.Minute}, // fractional durations are accepted (stdlib grammar)
	} {
		got, err := ParseOnceAt(tc.in, now, tst)
		if err != nil {
			t.Fatalf("ParseOnceAt(%q) = %v", tc.in, err)
		}
		if want := now.Add(tc.want); !got.Equal(want) {
			t.Errorf("ParseOnceAt(%q) = %v, want %v", tc.in, got, want)
		}
	}
	// Zero, negative, day-scale (no "d" unit in stdlib ParseDuration), and
	// malformed relative offsets must all error, not silently misfire.
	for _, bad := range []string{"+0s", "+-5m", "+2d", "+ 2m", "+", "+abc"} {
		if _, err := ParseOnceAt(bad, now, tst); err == nil {
			t.Errorf("ParseOnceAt(%q) should error", bad)
		}
	}
	// Runaway guard: over the 365-day cap must error.
	if _, err := ParseOnceAt("+9000h", now, tst); err == nil {
		t.Error("ParseOnceAt(+9000h) over the 365-day cap should error")
	}
	// Exactly at the cap is fine; one second over is not.
	if _, err := ParseOnceAt("+8760h", now, tst); err != nil { // 365*24
		t.Errorf("ParseOnceAt(+8760h) at the cap should be fine, got %v", err)
	}
}

func TestValidate(t *testing.T) {
	good := []Recurrence{
		{Kind: KindOnce},
		{Kind: KindDaily, TimeOfDay: "09:00"},
		{Kind: KindWeekly, TimeOfDay: "09:00", Weekday: "sunday"},
		{Kind: KindEveryNHours, EveryN: 1},
		{Kind: KindEveryNHours, EveryN: 720},
		{Kind: KindEveryNDays, EveryN: 3},
		{Kind: KindEveryNDays, EveryN: 3, TimeOfDay: "08:30"},
	}
	for _, r := range good {
		if err := r.Validate(); err != nil {
			t.Errorf("Validate(%+v) = %v, want nil", r, err)
		}
	}
	bad := []Recurrence{
		{Kind: "cron"},
		{Kind: KindDaily},
		{Kind: KindDaily, TimeOfDay: "99:99"},
		{Kind: KindWeekly, TimeOfDay: "09:00"},
		{Kind: KindWeekly, Weekday: "sunday"},
		{Kind: KindEveryNHours, EveryN: 0},
		{Kind: KindEveryNHours, EveryN: 721},
		{Kind: KindEveryNDays, EveryN: 0},
		{Kind: KindEveryNDays, EveryN: 366},
		{Kind: KindEveryNDays, EveryN: 3, TimeOfDay: "bad"},
	}
	for _, r := range bad {
		if err := r.Validate(); err == nil {
			t.Errorf("Validate(%+v) should error", r)
		}
	}
}

func TestDescribe(t *testing.T) {
	cases := map[string]Recurrence{
		"once":                      {Kind: KindOnce},
		"daily at 09:00":            {Kind: KindDaily, TimeOfDay: "9:00"},
		"weekly on sunday at 09:00": {Kind: KindWeekly, TimeOfDay: "09:00", Weekday: "sunday"},
		"every hour":                {Kind: KindEveryNHours, EveryN: 1},
		"every 4 hours":             {Kind: KindEveryNHours, EveryN: 4},
		"every day":                 {Kind: KindEveryNDays, EveryN: 1},
		"every 3 days":              {Kind: KindEveryNDays, EveryN: 3},
		"every 3 days at 08:30":     {Kind: KindEveryNDays, EveryN: 3, TimeOfDay: "8:30"},
	}
	for want, r := range cases {
		if got := Describe(r); got != want {
			t.Errorf("Describe(%+v) = %q, want %q", r, got, want)
		}
	}
}

func TestOnce(t *testing.T) {
	now := at(tst, 2026, 7, 6, 12, 0)
	r := Recurrence{Kind: KindOnce}

	future := at(tst, 2026, 7, 8, 9, 0)
	got, err := First(r, future, now, tst)
	if err != nil || !got.Equal(future) {
		t.Fatalf("First future once = (%v,%v), want %v", got, err, future)
	}
	if _, err := First(r, at(tst, 2026, 7, 5, 9, 0), now, tst); err == nil {
		t.Error("First past once should error")
	}
	if _, err := First(r, now, now, tst); err == nil {
		t.Error("First once == now should error (strictly after)")
	}
	if _, ok := Advance(r, future, now, tst); ok {
		t.Error("Advance once should return ok=false")
	}
}

func TestDaily(t *testing.T) {
	r := Recurrence{Kind: KindDaily, TimeOfDay: "09:00"}

	// Before the time today -> today at 09:00.
	now := at(tst, 2026, 7, 6, 8, 0)
	got, _ := First(r, time.Time{}, now, tst)
	if want := at(tst, 2026, 7, 6, 9, 0); !got.Equal(want) {
		t.Errorf("before: got %v want %v", got, want)
	}
	// Exactly at the time -> tomorrow (strict After).
	now = at(tst, 2026, 7, 6, 9, 0)
	got, _ = First(r, time.Time{}, now, tst)
	if want := at(tst, 2026, 7, 7, 9, 0); !got.Equal(want) {
		t.Errorf("at: got %v want %v", got, want)
	}
	// After the time -> tomorrow.
	now = at(tst, 2026, 7, 6, 10, 0)
	got, _ = First(r, time.Time{}, now, tst)
	if want := at(tst, 2026, 7, 7, 9, 0); !got.Equal(want) {
		t.Errorf("after: got %v want %v", got, want)
	}
	// Catch-up: 3 days overdue collapses to a single next = tomorrow relative to now.
	prev := at(tst, 2026, 7, 3, 9, 0)
	now = at(tst, 2026, 7, 6, 10, 0)
	next, ok := Advance(r, prev, now, tst)
	if !ok {
		t.Fatal("daily Advance ok=false")
	}
	if want := at(tst, 2026, 7, 7, 9, 0); !next.Equal(want) {
		t.Errorf("catch-up: got %v want %v (one fire, next tomorrow)", next, want)
	}
}

func TestWeekly(t *testing.T) {
	r := Recurrence{Kind: KindWeekly, TimeOfDay: "09:00", Weekday: "sunday"}
	target := time.Sunday

	check := func(name string, now time.Time, wantDayDiffMin int) {
		got, _ := First(r, time.Time{}, now, tst)
		if got.Weekday() != target {
			t.Errorf("%s: weekday %v, want %v", name, got.Weekday(), target)
		}
		if !got.After(now) {
			t.Errorf("%s: %v not after now %v", name, got, now)
		}
		if got.Hour() != 9 || got.Minute() != 0 {
			t.Errorf("%s: time %02d:%02d, want 09:00", name, got.Hour(), got.Minute())
		}
		// Soonest such occurrence: nothing earlier within the week.
		if got.After(now.AddDate(0, 0, 8)) {
			t.Errorf("%s: %v is more than a week out from %v", name, got, now)
		}
	}

	// A Wednesday (2026-07-08 is a Wednesday) mid-week, target Sunday later.
	check("mid-week", at(tst, 2026, 7, 8, 12, 0), 0)
	// The target Sunday (2026-07-12) before 09:00 -> today.
	sun := at(tst, 2026, 7, 12, 8, 0)
	got, _ := First(r, time.Time{}, sun, tst)
	if want := at(tst, 2026, 7, 12, 9, 0); !got.Equal(want) {
		t.Errorf("sunday-before: got %v want %v", got, want)
	}
	// The target Sunday exactly at 09:00 -> +7.
	sunAt := at(tst, 2026, 7, 12, 9, 0)
	got, _ = First(r, time.Time{}, sunAt, tst)
	if want := at(tst, 2026, 7, 19, 9, 0); !got.Equal(want) {
		t.Errorf("sunday-at: got %v want %v (+7)", got, want)
	}
	// The target Sunday after 09:00 -> +7.
	sunAfter := at(tst, 2026, 7, 12, 10, 0)
	got, _ = First(r, time.Time{}, sunAfter, tst)
	if want := at(tst, 2026, 7, 19, 9, 0); !got.Equal(want) {
		t.Errorf("sunday-after: got %v want %v (+7)", got, want)
	}
	// Target earlier in the week than now (Saturday now, Sunday target) -> next Sunday.
	sat := at(tst, 2026, 7, 11, 12, 0)
	got, _ = First(r, time.Time{}, sat, tst)
	if want := at(tst, 2026, 7, 12, 9, 0); !got.Equal(want) {
		t.Errorf("saturday: got %v want %v", got, want)
	}
}

func TestEveryNHours(t *testing.T) {
	r := Recurrence{Kind: KindEveryNHours, EveryN: 4}
	now := at(tst, 2026, 7, 6, 12, 0)

	// First is one interval out.
	got, _ := First(r, time.Time{}, now, tst)
	if want := now.Add(4 * time.Hour); !got.Equal(want) {
		t.Errorf("First: got %v want %v", got, want)
	}
	// Normal step: prev+4h still in the future.
	prev := at(tst, 2026, 7, 6, 11, 0)
	next, ok := Advance(r, prev, at(tst, 2026, 7, 6, 12, 0), tst)
	if !ok || !next.Equal(at(tst, 2026, 7, 6, 15, 0)) {
		t.Errorf("normal step: got %v ok=%v want 15:00", next, ok)
	}
	// 3 intervals overdue -> exactly one next, on-grid.
	prev = at(tst, 2026, 7, 6, 0, 0)
	now = at(tst, 2026, 7, 6, 13, 30) // prev+4h=04:00; grid 04,08,12,16 -> next 16:00
	next, _ = Advance(r, prev, now, tst)
	if want := at(tst, 2026, 7, 6, 16, 0); !next.Equal(want) {
		t.Errorf("overdue: got %v want %v", next, want)
	}
	// On-grid: (next-prev) is a whole multiple of step.
	if d := next.Sub(prev); d%(4*time.Hour) != 0 {
		t.Errorf("off-grid: next-prev=%v not multiple of 4h", d)
	}
	// Exact boundary: now == prev + k*step must still advance strictly past now.
	prev = at(tst, 2026, 7, 6, 0, 0)
	now = at(tst, 2026, 7, 6, 8, 0) // exactly 2 steps out
	next, _ = Advance(r, prev, now, tst)
	if want := at(tst, 2026, 7, 6, 12, 0); !next.Equal(want) {
		t.Errorf("exact boundary: got %v want %v", next, want)
	}
}

func TestEveryNDays(t *testing.T) {
	// Without TimeOfDay: First is EveryN days out at the same clock time.
	r := Recurrence{Kind: KindEveryNDays, EveryN: 3}
	now := at(tst, 2026, 7, 6, 12, 30)
	got, _ := First(r, time.Time{}, now, tst)
	if want := at(tst, 2026, 7, 9, 12, 30); !got.Equal(want) {
		t.Errorf("no-tod First: got %v want %v", got, want)
	}
	// Advance without TimeOfDay preserves prev's clock time, steps by EveryN.
	prev := at(tst, 2026, 7, 9, 12, 30)
	next, ok := Advance(r, prev, at(tst, 2026, 7, 9, 13, 0), tst)
	if !ok || !next.Equal(at(tst, 2026, 7, 12, 12, 30)) {
		t.Errorf("no-tod Advance: got %v ok=%v", next, ok)
	}
	// Multi-interval overdue: loops until strictly after now, staying on grid.
	prev = at(tst, 2026, 7, 1, 12, 30)
	next, _ = Advance(r, prev, at(tst, 2026, 7, 12, 8, 0), tst) // grid 4,7,10,13 -> 13
	if want := at(tst, 2026, 7, 13, 12, 30); !next.Equal(want) {
		t.Errorf("overdue no-tod: got %v want %v", next, want)
	}

	// With TimeOfDay: First is next 08:30, Advance steps by EveryN at 08:30.
	r = Recurrence{Kind: KindEveryNDays, EveryN: 3, TimeOfDay: "08:30"}
	now = at(tst, 2026, 7, 6, 12, 0) // past 08:30 today -> tomorrow 08:30
	got, _ = First(r, time.Time{}, now, tst)
	if want := at(tst, 2026, 7, 7, 8, 30); !got.Equal(want) {
		t.Errorf("tod First: got %v want %v", got, want)
	}
	prev = at(tst, 2026, 7, 7, 8, 30)
	next, _ = Advance(r, prev, at(tst, 2026, 7, 7, 9, 0), tst)
	if want := at(tst, 2026, 7, 10, 8, 30); !next.Equal(want) {
		t.Errorf("tod Advance: got %v want %v", next, want)
	}
}

func TestDST(t *testing.T) {
	loc, err := time.LoadLocation("Europe/Athens")
	if err != nil {
		t.Skip("tzdata unavailable")
	}
	// Spring forward: 2026-03-29, 03:00 local jumps to 04:00 (times 03:00-03:59
	// do not exist). A daily at 03:30 that day normalizes forward, still after now.
	r := Recurrence{Kind: KindDaily, TimeOfDay: "03:30"}
	now := time.Date(2026, 3, 29, 2, 0, 0, 0, loc)
	got, _ := First(r, time.Time{}, now, loc)
	if !got.After(now) {
		t.Errorf("DST daily: %v not after %v", got, now)
	}

	// every_n_hours stays a fixed real-duration apart across the shift.
	rh := Recurrence{Kind: KindEveryNHours, EveryN: 4}
	prev := time.Date(2026, 3, 29, 1, 0, 0, 0, loc) // 01:00 local, before the jump
	next, _ := Advance(rh, prev, time.Date(2026, 3, 29, 2, 0, 0, 0, loc), loc)
	if d := next.Sub(prev); d != 4*time.Hour {
		t.Errorf("DST interval drift: next-prev=%v, want 4h real time", d)
	}
}
