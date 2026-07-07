package mcpchan

import (
	"io"
	"net/http"
	"net/http/httptest"
	"net/url"
	"os"
	"path/filepath"
	"regexp"
	"strings"
	"testing"
)

const gateTestCode = "123456"

// newGateSite builds an artifact directory (index.html referencing ./style.css,
// plus a sub page and a sibling asset), wraps it in a passcode gate with a
// fixed code, and serves it over httptest.
func newGateSite(t *testing.T) *httptest.Server {
	t.Helper()
	dir := t.TempDir()
	writeFile(t, filepath.Join(dir, "index.html"), `<link rel="stylesheet" href="./style.css"><h1>artifact-content</h1>`)
	writeFile(t, filepath.Join(dir, "style.css"), "body{color:red}")
	if err := os.MkdirAll(filepath.Join(dir, "sub"), 0o755); err != nil {
		t.Fatal(err)
	}
	writeFile(t, filepath.Join(dir, "sub", "page.html"), `<link rel="stylesheet" href="../style.css"><p>sub-page</p>`)

	gate := newPasscodeGate(gateTestCode, dir, http.FileServer(http.Dir(dir)))
	ts := httptest.NewServer(gate)
	t.Cleanup(ts.Close)
	return ts
}

// noRedirectClient returns responses as-is so tests can inspect 303s.
func noRedirectClient() *http.Client {
	return &http.Client{CheckRedirect: func(*http.Request, []*http.Request) error {
		return http.ErrUseLastResponse
	}}
}

func get(t *testing.T, c *http.Client, url, cookie string) (*http.Response, string) {
	t.Helper()
	req, err := http.NewRequest(http.MethodGet, url, nil)
	if err != nil {
		t.Fatal(err)
	}
	if cookie != "" {
		req.Header.Set("Cookie", gateCookieName+"="+cookie)
	}
	resp, err := c.Do(req)
	if err != nil {
		t.Fatal(err)
	}
	defer resp.Body.Close()
	body := readAll(t, resp)
	return resp, body
}

func postCode(t *testing.T, c *http.Client, target, code string) *http.Response {
	t.Helper()
	resp, err := c.PostForm(target, url.Values{"code": {code}})
	if err != nil {
		t.Fatal(err)
	}
	resp.Body.Close()
	return resp
}

func readAll(t *testing.T, resp *http.Response) string {
	t.Helper()
	b, err := io.ReadAll(resp.Body)
	if err != nil {
		t.Fatal(err)
	}
	return string(b)
}

// unlock performs a correct-code POST against path and returns the session
// cookie value the gate set.
func unlock(t *testing.T, ts *httptest.Server, path string) (*http.Response, string) {
	t.Helper()
	resp := postCode(t, noRedirectClient(), ts.URL+path, gateTestCode)
	for _, c := range resp.Cookies() {
		if c.Name == gateCookieName {
			return resp, c.Value
		}
	}
	t.Fatalf("no %s cookie set on correct-code POST (status %d)", gateCookieName, resp.StatusCode)
	return nil, ""
}

// TestGateServesFormToUnauthedRequests: any path, any unauthenticated GET gets
// the form — with the three load-bearing autofill attributes — and never the
// artifact.
func TestGateServesFormToUnauthedRequests(t *testing.T) {
	ts := newGateSite(t)
	c := noRedirectClient()

	for _, path := range []string{"/", "/index.html", "/style.css", "/sub/page.html", "/nope"} {
		resp, body := get(t, c, ts.URL+path, "")
		if resp.StatusCode != http.StatusOK {
			t.Errorf("GET %s unauthed = %d, want 200 form", path, resp.StatusCode)
		}
		for _, attr := range []string{`autocomplete="one-time-code"`, `inputmode="numeric"`, "autofocus"} {
			if !strings.Contains(body, attr) {
				t.Errorf("GET %s form missing %s", path, attr)
			}
		}
		if strings.Contains(body, "artifact-content") || strings.Contains(body, "sub-page") {
			t.Errorf("GET %s unauthed leaked artifact content", path)
		}
		if strings.Contains(body, gateTestCode) {
			t.Errorf("GET %s form leaked the passcode", path)
		}
	}
}

// TestGateCorrectCodeSetsSessionCookieAndServes: right code → 303 back to the
// requested path, a session cookie that is NOT the passcode, and content plus
// relative assets afterward (the regression check for the deleted StripPrefix
// design: paths are unprefixed, so ./style.css just resolves).
func TestGateCorrectCodeSetsSessionCookieAndServes(t *testing.T) {
	ts := newGateSite(t)
	c := noRedirectClient()

	resp, session := unlock(t, ts, "/index.html")
	if resp.StatusCode != http.StatusSeeOther {
		t.Fatalf("correct code status = %d, want 303", resp.StatusCode)
	}
	if loc := resp.Header.Get("Location"); loc != "/index.html" {
		t.Errorf("redirect Location = %q, want /index.html", loc)
	}
	if session == gateTestCode {
		t.Fatal("session cookie must not be the passcode")
	}
	if !regexp.MustCompile(`^[0-9a-f]{32}$`).MatchString(session) {
		t.Errorf("session token %q is not 32 hex chars (128 bits)", session)
	}
	var cookie *http.Cookie
	for _, ck := range resp.Cookies() {
		if ck.Name == gateCookieName {
			cookie = ck
		}
	}
	if !cookie.HttpOnly || !cookie.Secure || cookie.SameSite != http.SameSiteLaxMode || cookie.Path != "/" {
		t.Errorf("cookie flags = HttpOnly:%v Secure:%v SameSite:%v Path:%q, want true/true/Lax/\"/\"", cookie.HttpOnly, cookie.Secure, cookie.SameSite, cookie.Path)
	}

	// "/" rather than "/index.html": FileServer canonicalizes the index name
	// with its own (relative) redirect, which passes through the gate intact.
	for path, want := range map[string]string{
		"/":              "artifact-content",
		"/style.css":     "body{color:red}",
		"/sub/page.html": "sub-page",
	} {
		resp, body := get(t, c, ts.URL+path, session)
		if resp.StatusCode != http.StatusOK || !strings.Contains(body, want) {
			t.Errorf("authed GET %s = %d, body missing %q", path, resp.StatusCode, want)
		}
	}
}

// TestGateWrongCode: wrong code gets the 401 retry form; the visitor stays
// unauthenticated.
func TestGateWrongCode(t *testing.T) {
	ts := newGateSite(t)
	c := noRedirectClient()

	resp, err := c.PostForm(ts.URL+"/", url.Values{"code": {"000000"}})
	if err != nil {
		t.Fatal(err)
	}
	body := readAll(t, resp)
	resp.Body.Close()
	if resp.StatusCode != http.StatusUnauthorized {
		t.Errorf("wrong code status = %d, want 401", resp.StatusCode)
	}
	if !strings.Contains(body, "Wrong passcode") {
		t.Error("retry page missing the wrong-passcode line")
	}
	if len(resp.Cookies()) != 0 {
		t.Error("wrong code must not set any cookie")
	}
	if r2, b2 := get(t, c, ts.URL+"/index.html", ""); r2.StatusCode != http.StatusOK || strings.Contains(b2, "artifact-content") {
		t.Error("still-unauthed GET should get the form, not content")
	}
}

// TestGatePasscodeAsCookieRejected: a cookie carrying the passcode itself is
// not a session — the session store never contains the code.
func TestGatePasscodeAsCookieRejected(t *testing.T) {
	ts := newGateSite(t)
	c := noRedirectClient()

	for _, val := range []string{gateTestCode, "deadbeefdeadbeefdeadbeefdeadbeef"} {
		if _, body := get(t, c, ts.URL+"/index.html", val); strings.Contains(body, "artifact-content") {
			t.Errorf("cookie %q must not authenticate", val)
		}
	}
}

// TestGateLocksAfterExhaustion: publishMaxAttempts wrong guesses lock the
// publish — from then on every request 404s: GETs, wrong codes, the CORRECT
// code, and even a session that was valid before the lock.
func TestGateLocksAfterExhaustion(t *testing.T) {
	ts := newGateSite(t)
	c := noRedirectClient()

	// Mint a valid session first; it must die with the lock.
	_, session := unlock(t, ts, "/")

	for i := 1; i <= publishMaxAttempts; i++ {
		resp := postCode(t, c, ts.URL+"/", "999999")
		want := http.StatusUnauthorized
		if i == publishMaxAttempts {
			want = http.StatusNotFound // the exhausting attempt already 404s
		}
		if resp.StatusCode != want {
			t.Fatalf("wrong attempt %d status = %d, want %d", i, resp.StatusCode, want)
		}
	}

	if resp, _ := get(t, c, ts.URL+"/", ""); resp.StatusCode != http.StatusNotFound {
		t.Errorf("locked GET / = %d, want 404", resp.StatusCode)
	}
	if resp := postCode(t, c, ts.URL+"/", gateTestCode); resp.StatusCode != http.StatusNotFound {
		t.Errorf("locked correct-code POST = %d, want 404", resp.StatusCode)
	}
	if resp, _ := get(t, c, ts.URL+"/index.html", session); resp.StatusCode != http.StatusNotFound {
		t.Errorf("locked GET with pre-lock session = %d, want 404", resp.StatusCode)
	}
}

// TestGateRedirectPathConfined: a crafted POST target starting with "//" must
// not become a protocol-relative (open) redirect.
func TestGateRedirectPathConfined(t *testing.T) {
	ts := newGateSite(t)

	req, err := http.NewRequest(http.MethodPost, ts.URL+"//evil.example/", strings.NewReader("code="+gateTestCode))
	if err != nil {
		t.Fatal(err)
	}
	req.Header.Set("Content-Type", "application/x-www-form-urlencoded")
	resp, err := noRedirectClient().Do(req)
	if err != nil {
		t.Fatal(err)
	}
	resp.Body.Close()
	if loc := resp.Header.Get("Location"); loc != "/" {
		t.Errorf("Location = %q, want / (no protocol-relative redirect)", loc)
	}

	for _, in := range []string{"/ok", "/", "//evil", "", "http://evil"} {
		got := safeRedirectPath(in)
		if strings.HasPrefix(got, "//") || !strings.HasPrefix(got, "/") {
			t.Errorf("safeRedirectPath(%q) = %q, unsafe", in, got)
		}
	}
}

func TestNewPasscodeFormat(t *testing.T) {
	re := regexp.MustCompile(`^\d{6}$`)
	for i := 0; i < 50; i++ {
		code, err := newPasscode()
		if err != nil {
			t.Fatal(err)
		}
		if !re.MatchString(code) {
			t.Fatalf("passcode %q is not exactly 6 digits", code)
		}
	}
}

func TestNewSessionTokenFormat(t *testing.T) {
	a, err := newSessionToken()
	if err != nil {
		t.Fatal(err)
	}
	b, err := newSessionToken()
	if err != nil {
		t.Fatal(err)
	}
	re := regexp.MustCompile(`^[0-9a-f]{32}$`)
	if !re.MatchString(a) || !re.MatchString(b) {
		t.Fatalf("session tokens %q / %q are not 32 lowercase hex chars", a, b)
	}
	if a == b {
		t.Fatal("two session tokens must differ")
	}
}

// TestSingleFileHandlerServesOnlyTheFile: "/" answers with the file and its
// extension-derived Content-Type; siblings and even the file's own basename
// are 404 — the parent directory is never exposed.
func TestSingleFileHandlerServesOnlyTheFile(t *testing.T) {
	dir := t.TempDir()
	writeFile(t, filepath.Join(dir, "page.html"), "<h1>the-page</h1>")
	writeFile(t, filepath.Join(dir, "secret.txt"), "sibling-secret")

	ts := httptest.NewServer(singleFileHandler(filepath.Join(dir, "page.html")))
	defer ts.Close()
	c := noRedirectClient()

	resp, body := get(t, c, ts.URL+"/", "")
	if resp.StatusCode != http.StatusOK || !strings.Contains(body, "the-page") {
		t.Fatalf("GET / = %d, want the file", resp.StatusCode)
	}
	if ct := resp.Header.Get("Content-Type"); !strings.Contains(ct, "text/html") {
		t.Errorf("Content-Type = %q, want text/html", ct)
	}
	for _, path := range []string{"/secret.txt", "/page.html", "/..", "/sub/"} {
		if resp, body := get(t, c, ts.URL+path, ""); resp.StatusCode != http.StatusNotFound || strings.Contains(body, "sibling-secret") {
			t.Errorf("GET %s = %d, want plain 404 with no sibling content", path, resp.StatusCode)
		}
	}
}
