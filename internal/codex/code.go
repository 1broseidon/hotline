package codex

import "crypto/rand"

const codeAlphabet = "abcdefghijkmnopqrstuvwxyz"

func newCode() string {
	out := make([]byte, 5)
	filled := 0
	var buf [8]byte
	for filled < len(out) {
		_, _ = rand.Read(buf[:])
		for _, x := range buf {
			if int(x) >= 256-(256%len(codeAlphabet)) {
				continue
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
