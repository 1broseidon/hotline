package mcpchan

// Passcode gate for public publishes.
//
// Threat model, in short: tunnel hostnames (*.lhr.life, *.trycloudflare.com)
// are guessable and actively scanned, so the hostname is not an access
// control. The gate puts a 6-digit passcode form in front of every public
// publish. Nothing secret rides the URL — link previews, browser history, and
// intermediary access logs carry nothing — and a stale link to a reused
// hostname hits a fresh passcode. 6 digits is ~20 bits, brute-forceable
// against a public endpoint, so the gate hard-locks after
// publishMaxAttempts wrong guesses: after that every request (including a
// correct code) gets a plain 404 for the life of the publish.
//
// Deliberately NOT defended against: the recipient forwarding link + code
// (that is sharing the artifact, same as forwarding the file); the tunnel
// provider, which terminates TLS and sees everything; and the accept-new ssh
// leg documented in exposure.go. Anyone who can read the chat has the code —
// the chat is the product.

import (
	"crypto/rand"
	"crypto/subtle"
	"encoding/hex"
	"fmt"
	"math/big"
	"net/http"
	"os"
	"strings"
	"sync"
)

// publishMaxAttempts is the global (per-publish, not per-IP — tunnels can
// mask client addresses) budget of wrong passcode guesses before the publish
// locks. 10 gives an attacker at most a 10-in-1,000,000 chance per publish
// while leaving a fat-fingered human plenty of retries; the cost of a lock is
// one republish (fresh link, fresh code).
const publishMaxAttempts = 10

// gateCookieName is the session cookie the gate sets after a correct code.
// The name is deliberately generic: the pre-auth page must not leak what is
// behind it.
const gateCookieName = "hotline_gate"

// newPasscode returns a uniform random 6-digit code (leading zeros kept).
// Six digits is the sweet spot for phone one-time-code autofill: iOS reads it
// straight out of the recent chat message and offers it above the keyboard.
func newPasscode() (string, error) {
	n, err := rand.Int(rand.Reader, big.NewInt(1_000_000))
	if err != nil {
		return "", fmt.Errorf("generating passcode: %v", err)
	}
	return fmt.Sprintf("%06d", n.Int64()), nil
}

// newSessionToken returns a 128-bit random session token, hex-encoded. It is
// never the passcode: the code is what the user types, the token is what the
// cookie holds, and neither is ever derived from the other.
func newSessionToken() (string, error) {
	b := make([]byte, 16)
	if _, err := rand.Read(b); err != nil {
		return "", fmt.Errorf("generating session token: %v", err)
	}
	return hex.EncodeToString(b), nil
}

// passcodeGate wraps a content handler with the passcode form. Unauthed
// requests to any path get the form; a POST with the right code mints a
// session token, sets it as a cookie, and redirects back to the requested
// path (the form posts to the URL the visitor asked for, so deep links
// survive the gate). Once locked, everything — correct code included — is a
// plain 404.
type passcodeGate struct {
	next http.Handler
	code string
	// root is the published path, used only in the one stderr line emitted on
	// lock. The passcode and session tokens are never logged anywhere.
	root string

	mu        sync.Mutex
	sessions  []string
	remaining int
	locked    bool
}

func newPasscodeGate(code, root string, next http.Handler) *passcodeGate {
	return &passcodeGate{next: next, code: code, root: root, remaining: publishMaxAttempts}
}

func (g *passcodeGate) ServeHTTP(w http.ResponseWriter, r *http.Request) {
	if g.isLocked() {
		http.NotFound(w, r)
		return
	}
	if g.authed(r) {
		g.next.ServeHTTP(w, r)
		return
	}
	if r.Method == http.MethodPost {
		g.attempt(w, r)
		return
	}
	serveGateForm(w, http.StatusOK, "")
}

func (g *passcodeGate) isLocked() bool {
	g.mu.Lock()
	defer g.mu.Unlock()
	return g.locked
}

// authed reports whether the request carries a valid session cookie. Tokens
// are compared constant-time; a cookie holding the passcode itself never
// matches because the passcode is never stored as a session.
func (g *passcodeGate) authed(r *http.Request) bool {
	c, err := r.Cookie(gateCookieName)
	if err != nil {
		return false
	}
	g.mu.Lock()
	defer g.mu.Unlock()
	ok := false
	for _, s := range g.sessions {
		if subtle.ConstantTimeCompare([]byte(c.Value), []byte(s)) == 1 {
			ok = true
		}
	}
	return ok
}

// attempt handles an unlock POST. Every POST while unauthenticated counts as
// an attempt: there is no other legitimate POST before auth.
func (g *passcodeGate) attempt(w http.ResponseWriter, r *http.Request) {
	code := r.PostFormValue("code")
	ok := subtle.ConstantTimeCompare([]byte(code), []byte(g.code)) == 1

	g.mu.Lock()
	if g.locked {
		g.mu.Unlock()
		http.NotFound(w, r)
		return
	}
	if !ok {
		g.remaining--
		justLocked := g.remaining <= 0
		if justLocked {
			g.locked = true
		}
		g.mu.Unlock()
		if justLocked {
			// The one line the operator sees. No code, no tokens, no URL.
			fmt.Fprintf(os.Stderr, "hotline: publish of %s locked after %d wrong passcode attempts; republish to share it again\n", g.root, publishMaxAttempts)
			http.NotFound(w, r)
			return
		}
		serveGateForm(w, http.StatusUnauthorized, "Wrong passcode. Try again.")
		return
	}

	tok, err := newSessionToken()
	if err != nil {
		g.mu.Unlock()
		http.Error(w, "internal error", http.StatusInternalServerError)
		return
	}
	g.sessions = append(g.sessions, tok)
	g.mu.Unlock()

	http.SetCookie(w, &http.Cookie{
		Name:     gateCookieName,
		Value:    tok,
		Path:     "/",
		HttpOnly: true,
		Secure:   true, // the public origin is always https (the tunnel terminates TLS)
		SameSite: http.SameSiteLaxMode,
	})
	http.Redirect(w, r, safeRedirectPath(r.URL.Path), http.StatusSeeOther)
}

// safeRedirectPath confines the post-unlock redirect to a same-origin path.
// A path not starting with exactly one "/" (e.g. "//host") would be treated
// by browsers as protocol-relative — an open redirect — so it collapses to "/".
func safeRedirectPath(p string) string {
	if !strings.HasPrefix(p, "/") || strings.HasPrefix(p, "//") {
		return "/"
	}
	return p
}

// serveGateForm writes the passcode form with errLine (a fixed server-side
// string, never user input) in the error slot.
func serveGateForm(w http.ResponseWriter, status int, errLine string) {
	w.Header().Set("Content-Type", "text/html; charset=utf-8")
	w.Header().Set("Cache-Control", "no-store")
	w.WriteHeader(status)
	_, _ = w.Write([]byte(strings.Replace(gateFormHTML, "{{error}}", errLine, 1)))
}

// gateFormHTML is the self-contained unlock page: inline CSS, system font
// stacks, zero external requests. The title and copy are generic on purpose —
// the pre-auth page must not leak the artifact's name. The three attributes
// on the input are load-bearing UX, validated on a real phone: with
// autocomplete="one-time-code" and inputmode="numeric", iOS offers the
// 6-digit code straight from the recent chat message; autofocus puts the
// caret there on load.
const gateFormHTML = `<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<meta name="robots" content="noindex">
<title>Passcode required</title>
<style>
/* Hallmark · component: passcode gate · genre: atmospheric (terminal) · theme: hotline site tokens
 * states: default · hover · focus · active · error (server-rendered form — no JS, so no loading/disabled)
 * pre-emit critique: P4 H4 E4 S4 R5 V4 */
:root {
  color-scheme: light dark;
  --color-paper: oklch(16% 0.012 260);
  --color-paper-2: oklch(19% 0.014 260);
  --color-ink: oklch(90% 0.008 260);
  --color-ink-dim: oklch(68% 0.012 260);
  --color-accent: oklch(66% 0.19 25);
  --color-accent-ink: oklch(98% 0.005 25);
  --color-prompt: oklch(75% 0.14 150);
  --color-line: oklch(30% 0.014 260);
  --color-focus: oklch(75% 0.15 25);
  --font-mono: ui-monospace, "SF Mono", "Cascadia Code", "JetBrains Mono", Menlo, Consolas, monospace;
  --font-body: system-ui, -apple-system, "Segoe UI", Roboto, sans-serif;
  --space-sm: 0.5rem;
  --space-md: 1rem;
  --space-lg: 2rem;
  --radius: 6px;
  --ease-out: cubic-bezier(0.22, 1, 0.36, 1);
}
@media (prefers-color-scheme: light) {
  :root {
    --color-paper: oklch(97% 0.006 260);
    --color-paper-2: oklch(99.5% 0.002 260);
    --color-ink: oklch(24% 0.012 260);
    --color-ink-dim: oklch(46% 0.012 260);
    --color-accent: oklch(56% 0.19 25);
    --color-prompt: oklch(46% 0.13 150);
    --color-line: oklch(87% 0.008 260);
    --color-focus: oklch(56% 0.17 25);
  }
}
html, body { overflow-x: clip; }
body {
  margin: 0;
  min-height: 100svh;
  display: grid;
  place-items: center;
  background: var(--color-paper);
  color: var(--color-ink);
  font-family: var(--font-body);
  line-height: 1.6;
  -webkit-font-smoothing: antialiased;
}
main {
  width: min(22rem, calc(100vw - 2rem));
  box-sizing: border-box;
  padding: var(--space-lg);
  background: var(--color-paper-2);
  border: 1px solid var(--color-line);
  border-radius: var(--radius);
}
.wordmark {
  margin: 0 0 var(--space-lg);
  font-family: var(--font-mono);
  font-size: 0.85rem;
  color: var(--color-ink-dim);
}
.wordmark::before { content: "> "; color: var(--color-prompt); }
h1 {
  margin: 0 0 var(--space-sm);
  font-family: var(--font-mono);
  font-style: normal;
  font-weight: 600;
  font-size: 1.25rem;
  letter-spacing: -0.01em;
  overflow-wrap: anywhere;
  min-width: 0;
}
.hint {
  margin: 0 0 var(--space-md);
  color: var(--color-ink-dim);
  font-size: 0.95rem;
}
.sr {
  position: absolute;
  width: 1px; height: 1px;
  overflow: hidden;
  clip-path: inset(50%);
  white-space: nowrap;
}
input {
  display: block;
  width: 100%;
  box-sizing: border-box;
  padding: 0.55rem 0;
  font-family: var(--font-mono);
  font-size: 1.7rem;
  text-align: center;
  letter-spacing: 0.45em;
  text-indent: 0.45em;
  color: var(--color-ink);
  background: var(--color-paper);
  border: 1px solid var(--color-line);
  border-radius: var(--radius);
}
input:focus-visible, button:focus-visible {
  outline: 2px solid var(--color-focus);
  outline-offset: 2px;
}
.err {
  min-height: 1.3rem;
  margin: var(--space-sm) 0 0;
  font-size: 0.9rem;
  color: var(--color-accent);
}
button {
  display: block;
  width: 100%;
  margin-top: var(--space-md);
  padding: 0.65rem;
  font-family: var(--font-mono);
  font-size: 1rem;
  font-weight: 600;
  white-space: nowrap;
  border: 0;
  border-radius: var(--radius);
  background: var(--color-accent);
  color: var(--color-accent-ink);
  cursor: pointer;
  transition: opacity 120ms var(--ease-out), transform 120ms var(--ease-out);
}
button:hover { opacity: 0.92; }
button:active { transform: translateY(1px); }
@media (prefers-reduced-motion: reduce) {
  button { transition: none; }
}
</style>
</head>
<body>
<main>
  <p class="wordmark">hotline</p>
  <h1>Passcode required</h1>
  <p class="hint">Enter the 6-digit code from the message that sent you this link.</p>
  <form method="post">
    <label class="sr" for="code">Passcode</label>
    <input id="code" name="code" inputmode="numeric" autocomplete="one-time-code" autofocus maxlength="6" required>
    <p class="err" role="alert">{{error}}</p>
    <button>Unlock</button>
  </form>
</main>
</body>
</html>
`
