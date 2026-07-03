package opencode

import "crypto/rand"

// codeAlphabet is a-z minus 'l' — the same alphabet the permission relay's
// PermReplyRe / PermBtnRe accept ([a-km-z]), avoiding 1/l confusion on phone
// keyboards. OpenCode's native permission ids are opaque and too long for a
// texted "yes <code>" reply, so the Link mints a short relay code in this
// alphabet and maps it back to the real (sessionID, permissionID) pair.
const codeAlphabet = "abcdefghijkmnopqrstuvwxyz"

// newCode returns a fresh 5-letter relay code drawn from codeAlphabet, matching
// the relay's fixed 5-char width. Rejection sampling keeps the distribution
// uniform (256 is not a multiple of 25, so a plain modulo would bias the first
// six letters).
func newCode() string {
	out := make([]byte, 5)
	filled := 0
	var buf [8]byte
	for filled < len(out) {
		_, _ = rand.Read(buf[:])
		for _, x := range buf {
			if int(x) >= 256-(256%len(codeAlphabet)) {
				continue // reject the biased tail
			}
			out[filled] = codeAlphabet[int(x)%len(codeAlphabet)]
			filled++
			if filled == len(out) {
				break
			}
		}
	}
	return string(out)
}
