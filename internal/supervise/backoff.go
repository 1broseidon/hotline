package supervise

import "time"

// Backoff computes restart delays for the supervisor: exponential doubling
// from Initial up to a hard Max, counting consecutive short-lived runs. A run
// whose uptime reaches Healthy resets the sequence, so the common case — a
// session healthy for hours that crashes once — restarts at Initial, while a
// persistently failing harness decays to one attempt per Max. There is no
// give-up state: an always-on agent's job is to be there when the outage
// ends, and Max bounds the flap rate.
type Backoff struct {
	Initial time.Duration // first retry delay (and the delay after a healthy run)
	Max     time.Duration // delay ceiling for a persistently failing harness
	Healthy time.Duration // uptime at which a run counts as healthy, resetting the sequence

	n int // consecutive short-lived runs seen so far
}

// Next records one harness exit after the given uptime and returns how long
// to wait before the next start. A spawn failure counts as uptime 0.
func (b *Backoff) Next(uptime time.Duration) time.Duration {
	if uptime >= b.Healthy {
		b.n = 0
	}
	d := b.Initial << b.n
	if d <= 0 || d >= b.Max { // <= 0 catches shift overflow
		return b.Max
	}
	b.n++
	return d
}

// Reset clears the sequence — used after an intentional (operator- or
// chat-requested) restart, which says nothing about the harness's health.
func (b *Backoff) Reset() { b.n = 0 }
