package app

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"net/http"
	"net/http/httptest"
	"strings"
	"sync"
	"testing"
	"time"

	ref "github.com/1broseidon/hotline/protocol/code-link-v1/ref"
)

// fakeCodeCore models the box-facing side of the WP-CL2 relay endpoints
// (design §5.1): create / poll / finish. The client turn is driven directly by
// the test (it IS party B), which deposits attempts on `deliver` — an unbuffered
// channel that makes each poll/verify/finish cycle lockstep, so a strike test can
// feed exactly N wrong confirms.
type fakeCodeCore struct {
	mu            sync.Mutex
	createCalls   int
	conflictFirst bool // return 409 code_taken on the first create
	unavailable   bool // return 503 provider_unavailable on create (kill switch)

	// finalAborts counts session-wide final aborts (finish{final:true}); the
	// teardown must fire exactly once per dropped/expired/cancelled session.
	finalAborts int

	msgA   string
	msgACh chan string

	deliver     chan *pendingAttempt
	finishOK    chan finishRelease
	finishAbort chan struct{}

	// aid correlation (WP-CL2): handlePoll stamps each delivered attempt with a
	// fresh monotonic aid and remembers the latest as curAid; handleFinish asserts
	// the box echoes curAid on attempt-specific verdicts (ok:true / plain ok:false)
	// and carries NO aid on the final session-wide abort. Any violation is recorded
	// in aidErr (handlers can't call t.Fatal) and checked by the test.
	pollAid int
	curAid  int
	aidErr  error
}

func (fc *fakeCodeCore) recordAidErr(err error) {
	fc.mu.Lock()
	if fc.aidErr == nil {
		fc.aidErr = err
	}
	fc.mu.Unlock()
}

func (fc *fakeCodeCore) aidError() error {
	fc.mu.Lock()
	defer fc.mu.Unlock()
	return fc.aidErr
}

func (fc *fakeCodeCore) finalAbortCount() int {
	fc.mu.Lock()
	defer fc.mu.Unlock()
	return fc.finalAborts
}

type finishRelease struct {
	ConfirmA string
	N, C     string
}

func newFakeCodeCore(conflictFirst bool) *fakeCodeCore {
	return &fakeCodeCore{
		conflictFirst: conflictFirst,
		msgACh:        make(chan string, 1),
		deliver:       make(chan *pendingAttempt),
		finishOK:      make(chan finishRelease, 1),
		finishAbort:   make(chan struct{}, 1),
	}
}

func (fc *fakeCodeCore) handler() http.Handler {
	mux := http.NewServeMux()
	// The box registers the room before minting the code (design §6.1 step 2).
	mux.HandleFunc("/v1/rooms/", func(w http.ResponseWriter, r *http.Request) {
		_, _ = w.Write([]byte(`{}`))
	})
	mux.HandleFunc("/v1/link-codes/", func(w http.ResponseWriter, r *http.Request) {
		tail := strings.TrimPrefix(r.URL.Path, "/v1/link-codes/")
		switch {
		case !strings.Contains(tail, "/"): // create
			fc.handleCreate(w, r)
		case strings.HasSuffix(tail, "/poll"):
			fc.handlePoll(w, r)
		case strings.HasSuffix(tail, "/finish"):
			fc.handleFinish(w, r)
		default:
			http.Error(w, "{}", http.StatusNotFound)
		}
	})
	return mux
}

func (fc *fakeCodeCore) handleCreate(w http.ResponseWriter, r *http.Request) {
	fc.mu.Lock()
	fc.createCalls++
	n := fc.createCalls
	fc.mu.Unlock()
	if fc.conflictFirst && n == 1 {
		w.WriteHeader(http.StatusConflict)
		_, _ = w.Write([]byte(`{"code":"code_taken"}`))
		return
	}
	if fc.unavailable {
		w.WriteHeader(http.StatusServiceUnavailable)
		_, _ = w.Write([]byte(`{"code":"provider_unavailable"}`))
		return
	}
	var body struct {
		MsgA string `json:"msg_a"`
	}
	_ = json.NewDecoder(r.Body).Decode(&body)
	select {
	case fc.msgACh <- body.MsgA:
	default:
	}
	_, _ = w.Write([]byte(`{"expires_in":300}`))
}

func (fc *fakeCodeCore) handlePoll(w http.ResponseWriter, r *http.Request) {
	select {
	case p := <-fc.deliver:
		fc.mu.Lock()
		fc.pollAid++
		p.Aid = fc.pollAid // relay-assigned per-attempt id the box must echo.
		fc.curAid = p.Aid
		fc.mu.Unlock()
		_ = json.NewEncoder(w).Encode(map[string]any{"pending": p})
	case <-r.Context().Done():
		_ = json.NewEncoder(w).Encode(map[string]any{"pending": nil})
	case <-time.After(150 * time.Millisecond):
		// Safety park cap: a real relay returns {pending:null} on its own timeout;
		// mirror that so no handler blocks past the client's poll budget.
		_ = json.NewEncoder(w).Encode(map[string]any{"pending": nil})
	}
}

func (fc *fakeCodeCore) handleFinish(w http.ResponseWriter, r *http.Request) {
	var body struct {
		OK       bool   `json:"ok"`
		Final    bool   `json:"final"`
		Aid      *int   `json:"aid"` // pointer so we can tell "absent" from 0.
		ConfirmA string `json:"confirm_a"`
		Payload  struct {
			N string `json:"n"`
			C string `json:"c"`
		} `json:"payload"`
	}
	_ = json.NewDecoder(r.Body).Decode(&body)
	fc.mu.Lock()
	want := fc.curAid
	fc.mu.Unlock()
	switch {
	case body.OK:
		if body.Aid == nil || *body.Aid != want {
			fc.recordAidErr(fmt.Errorf("ok:true finish must echo aid %d, got %v", want, body.Aid))
		}
		fc.finishOK <- finishRelease{ConfirmA: body.ConfirmA, N: body.Payload.N, C: body.Payload.C}
	case body.Final:
		if body.Aid != nil {
			fc.recordAidErr(fmt.Errorf("final abort must carry no aid, got %d", *body.Aid))
		}
		fc.mu.Lock()
		fc.finalAborts++
		fc.mu.Unlock()
		select {
		case fc.finishAbort <- struct{}{}:
		default:
		}
	default: // plain ok:false — attempt-specific mismatch verdict, aid required.
		if body.Aid == nil || *body.Aid != want {
			fc.recordAidErr(fmt.Errorf("ok:false reject must echo aid %d, got %v", want, body.Aid))
		}
	}
	_, _ = w.Write([]byte(`{}`))
}

// codeFixture is a known code so the test (party B) shares the PRS the box picks.
const codeFixture = "3YPJ24B8K7QM"

// clientReply plays party B for one attempt: normalize the shared code, respond
// to msg_a, and return the state + wire values. tamper=true corrupts confirm_b to
// simulate a wrong code (a burned strike).
func clientReply(t *testing.T, baseURL, msgA string, tamper bool) (*ref.ClientAwaiting, *pendingAttempt) {
	t.Helper()
	prs, err := ref.NormalizeCode(codeFixture)
	if err != nil {
		t.Fatal(err)
	}
	ci, err := ref.BuildCIFromURL(baseURL)
	if err != nil {
		t.Fatal(err)
	}
	seed := make([]byte, ref.UniformScalarBytes)
	seed[0] = 0x11 // any nonzero seed; interop is determined by the scalar value.
	cs, msgB, confirmB, err := ref.NewClientResponder([]byte(prs), ci, ref.Channel(prs), msgA, seed)
	if err != nil {
		t.Fatalf("client responder: %v", err)
	}
	if tamper {
		// Flip a character so confirm_b fails to verify under the box's real K_cb.
		b := []byte(confirmB)
		if b[0] == 'A' {
			b[0] = 'B'
		} else {
			b[0] = 'A'
		}
		confirmB = string(b)
	}
	return cs, &pendingAttempt{MsgB: msgB, ConfirmB: confirmB}
}

func testLink() Link {
	secret, _ := randomBase64URL(32)
	return Link{
		Room:     "room-abcd1234",
		Name:     "pi",
		Secret:   secret,
		URI:      PairingURIMode("wss://relay.example", "room-abcd1234", secret, "pi", true),
		Envelope: true,
	}
}

func TestCodeLinkFakeCoreHappyPath(t *testing.T) {
	fc := newFakeCodeCore(false)
	ts := httptest.NewServer(fc.handler())
	defer ts.Close()

	link := testLink()
	var buf syncBuffer
	done := make(chan error, 1)
	go func() {
		done <- runNewLinkCode(context.Background(), t.TempDir(), ts.URL, link, &buf,
			codeLinkOpts{forceCode: codeFixture, pollWait: 300 * time.Millisecond, httpClient: ts.Client()})
	}()

	msgA := <-fc.msgACh
	cs, pend := clientReply(t, ts.URL, msgA, false)
	fc.deliver <- pend // box polls this, verifies, finishes ok

	rel := <-fc.finishOK
	uri, err := cs.Finish(rel.ConfirmA, rel.N, rel.C)
	if err != nil {
		t.Fatalf("client Finish (confirm_a + payload decrypt): %v", err)
	}
	var got struct {
		URI string `json:"uri"`
	}
	if err := json.Unmarshal(uri, &got); err != nil {
		t.Fatalf("payload JSON: %v", err)
	}
	if got.URI != link.URI {
		t.Fatalf("delivered URI mismatch:\n got %q\nwant %q", got.URI, link.URI)
	}

	if err := <-done; err != nil {
		t.Fatalf("box flow: %v", err)
	}
	if !strings.Contains(buf.String(), "claimed") {
		t.Fatalf("box output missing claimed line: %q", buf.String())
	}
	if !strings.Contains(buf.String(), "3YPJ-24B8-K7QM") {
		t.Fatalf("box did not print the human code: %q", buf.String())
	}
	if err := fc.aidError(); err != nil {
		t.Fatalf("aid round-trip: %v", err)
	}
}

func TestCodeLinkThreeStrikesAbortsAndInvalidates(t *testing.T) {
	fc := newFakeCodeCore(false)
	ts := httptest.NewServer(fc.handler())
	defer ts.Close()

	link := testLink()
	var buf syncBuffer
	done := make(chan error, 1)
	go func() {
		done <- runNewLinkCode(context.Background(), t.TempDir(), ts.URL, link, &buf,
			codeLinkOpts{forceCode: codeFixture, pollWait: 300 * time.Millisecond, httpClient: ts.Client()})
	}()

	msgA := <-fc.msgACh
	// Feed three wrong confirms; each send unblocks only once the box has polled
	// it out, so this is strictly lockstep with the box's strike counter.
	for i := 0; i < 3; i++ {
		_, pend := clientReply(t, ts.URL, msgA, true)
		fc.deliver <- pend
	}

	select {
	case <-fc.finishAbort:
	case <-time.After(3 * time.Second):
		t.Fatal("box never sent the final abort (code invalidation)")
	}

	if err := <-done; err != ErrCodeStrikeout {
		t.Fatalf("want ErrCodeStrikeout, got %v", err)
	}
	if !strings.Contains(buf.String(), "too many wrong attempts") {
		t.Fatalf("missing strikeout message: %q", buf.String())
	}
	// The two mismatch verdicts (strikes 1-2) must each echo their polled aid; the
	// third strike is the session-wide final abort and must carry NO aid.
	if err := fc.aidError(); err != nil {
		t.Fatalf("aid round-trip: %v", err)
	}
}

func TestCodeLinkCreateRetriesOn409(t *testing.T) {
	fc := newFakeCodeCore(true) // first create 409 code_taken, then succeeds
	ts := httptest.NewServer(fc.handler())
	defer ts.Close()

	link := testLink()
	var buf syncBuffer
	done := make(chan error, 1)
	go func() {
		done <- runNewLinkCode(context.Background(), t.TempDir(), ts.URL, link, &buf,
			// No forceCode: a 409 must regenerate a fresh code and retry.
			codeLinkOpts{pollWait: 200 * time.Millisecond, ttl: 2 * time.Second, httpClient: ts.Client()})
	}()

	<-fc.msgACh // create eventually succeeded despite the first 409
	err := <-done
	if err != ErrCodeExpired {
		t.Fatalf("want ErrCodeExpired after TTL, got %v", err)
	}
	fc.mu.Lock()
	calls := fc.createCalls
	fc.mu.Unlock()
	if calls < 2 {
		t.Fatalf("expected a regenerate-and-retry create, got %d create calls", calls)
	}
}

// TestCodeLinkLatePollDoesNotReleasePayload is the BLOCKER 1 guard: a valid
// pending attempt that arrives AFTER the box-enforced deadline must never be
// verified or sealed. The box must post the final abort and return ErrCodeExpired,
// and the payload must never be released.
func TestCodeLinkLatePollDoesNotReleasePayload(t *testing.T) {
	fc := newFakeCodeCore(false)
	ts := httptest.NewServer(fc.handler())
	defer ts.Close()

	// Reproduce the real window: the box is parked in its first long-poll when the
	// TTL crosses, then that in-flight poll returns a VALID attempt after expiry.
	// A short TTL (80ms) that lapses inside the fake's poll park (150ms) does this
	// with real time — no clock hook needed.
	link := testLink()
	var buf syncBuffer
	done := make(chan error, 1)
	go func() {
		done <- runNewLinkCode(context.Background(), t.TempDir(), ts.URL, link, &buf,
			codeLinkOpts{forceCode: codeFixture, ttl: 80 * time.Millisecond, pollWait: 5 * time.Second, httpClient: ts.Client()})
	}()

	msgA := <-fc.msgACh
	_, pend := clientReply(t, ts.URL, msgA, false) // a VALID attempt
	time.Sleep(120 * time.Millisecond)             // let the 80ms deadline pass first
	fc.deliver <- pend                             // in-flight poll returns it AFTER expiry

	select {
	case <-fc.finishAbort:
	case <-time.After(3 * time.Second):
		t.Fatal("box never sent the final abort after a late poll")
	}
	if err := <-done; err != ErrCodeExpired {
		t.Fatalf("want ErrCodeExpired, got %v", err)
	}
	// The payload must NOT have been released to the client.
	select {
	case <-fc.finishOK:
		t.Fatal("late poll released the payload — TTL bypass")
	default:
	}
	if got := fc.finalAbortCount(); got != 1 {
		t.Fatalf("want exactly one final abort, got %d", got)
	}
}

// TestCodeLinkCancelFiresFinalAbortOnce is the BLOCKER 2 guard: cancelling the
// context after the code is minted (the Ctrl-C path) must fire the detached final
// abort exactly once so the code is invalidated on the relay.
func TestCodeLinkCancelFiresFinalAbortOnce(t *testing.T) {
	fc := newFakeCodeCore(false)
	ts := httptest.NewServer(fc.handler())
	defer ts.Close()

	ctx, cancel := context.WithCancel(context.Background())
	link := testLink()
	var buf syncBuffer
	done := make(chan error, 1)
	go func() {
		done <- runNewLinkCode(ctx, t.TempDir(), ts.URL, link, &buf,
			codeLinkOpts{forceCode: codeFixture, pollWait: 300 * time.Millisecond, ttl: 30 * time.Second, httpClient: ts.Client()})
	}()

	<-fc.msgACh // create observed
	// Wait until the box has finished minting and entered the poll loop (it prints
	// this line right before arming the guaranteed-once teardown), so cancel lands
	// AFTER the code is live — the exact Ctrl-C-mid-link window BLOCKER 2 targets.
	waitFor(t, &buf, "waiting for the code")
	cancel() // simulate Ctrl-C mid-link

	select {
	case <-fc.finishAbort:
	case <-time.After(3 * time.Second):
		t.Fatal("cancel did not fire the final abort (code left claimable)")
	}
	if err := <-done; !errors.Is(err, context.Canceled) {
		t.Fatalf("want context.Canceled, got %v", err)
	}
	// Give any stray second abort a moment to (wrongly) arrive.
	time.Sleep(50 * time.Millisecond)
	if got := fc.finalAbortCount(); got != 1 {
		t.Fatalf("want exactly one final abort on cancel, got %d", got)
	}
}

// TestCodeLinkStrikeoutUnderCancelFiresDetachedAbort is the cancel-after-pending
// strikeout guard: Ctrl-C cancels the caller ctx with the 3rd (strikeout) attempt
// already polled and in hand, then verify fails. The final abort MUST still reach
// the relay exactly once via the detached deferred teardown (not the cancelled
// caller ctx), so the code is invalidated and never left claimable. Neither
// TestCodeLinkCancelFiresFinalAbortOnce (cancels while polling, no pending attempt)
// nor TestCodeLinkThreeStrikesAbortsAndInvalidates (never cancels) covers this.
func TestCodeLinkStrikeoutUnderCancelFiresDetachedAbort(t *testing.T) {
	fc := newFakeCodeCore(false)
	ts := httptest.NewServer(fc.handler())
	defer ts.Close()

	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	link := testLink()
	var buf syncBuffer
	done := make(chan error, 1)

	polls := 0
	opts := codeLinkOpts{
		forceCode:  codeFixture,
		pollWait:   300 * time.Millisecond,
		ttl:        30 * time.Second,
		httpClient: ts.Client(),
		afterPoll: func() {
			polls++
			if polls == 3 {
				// The 3rd (strikeout) attempt is now in hand; kill the caller ctx
				// before it is verified. This is the exact cancel-after-pending window:
				// a cancelled ctx must NOT suppress the strikeout teardown.
				cancel()
			}
		},
	}
	go func() {
		done <- runNewLinkCode(ctx, t.TempDir(), ts.URL, link, &buf, opts)
	}()

	msgA := <-fc.msgACh
	// Three wrong confirms, lockstep with the box's strike counter.
	for i := 0; i < 3; i++ {
		_, pend := clientReply(t, ts.URL, msgA, true)
		fc.deliver <- pend
	}

	select {
	case <-fc.finishAbort:
	case <-time.After(3 * time.Second):
		t.Fatal("strikeout under a cancelled ctx never fired the detached final abort (code left claimable)")
	}
	if err := <-done; err != ErrCodeStrikeout {
		t.Fatalf("want ErrCodeStrikeout, got %v", err)
	}
	if ctx.Err() == nil {
		t.Fatal("test setup bug: caller ctx should be cancelled by afterPoll")
	}
	// The payload must never have been released to the client.
	select {
	case <-fc.finishOK:
		t.Fatal("strikeout under cancel released the payload")
	default:
	}
	// Exactly one final abort must reach the relay despite the cancelled caller ctx
	// — no suppression, no double-abort. Give any stray second abort a moment.
	time.Sleep(50 * time.Millisecond)
	if got := fc.finalAbortCount(); got != 1 {
		t.Fatalf("want exactly one final abort under cancel, got %d", got)
	}
	if !strings.Contains(buf.String(), "too many wrong attempts") {
		t.Fatalf("missing strikeout message: %q", buf.String())
	}
	if err := fc.aidError(); err != nil {
		t.Fatalf("aid round-trip: %v", err)
	}
}

// TestCodeLinkCreate503FallsBack is the SHOULD-FIX 4 guard: a create-time 503
// provider_unavailable surfaces as ErrCodesUnavailable so the CLI can fall back to
// the pair URI for the already-minted room.
func TestCodeLinkCreate503FallsBack(t *testing.T) {
	fc := newFakeCodeCore(false)
	fc.unavailable = true
	ts := httptest.NewServer(fc.handler())
	defer ts.Close()

	link := testLink()
	var buf syncBuffer
	err := runNewLinkCode(context.Background(), t.TempDir(), ts.URL, link, &buf,
		codeLinkOpts{forceCode: codeFixture, pollWait: 200 * time.Millisecond, httpClient: ts.Client()})
	if !errors.Is(err, ErrCodesUnavailable) {
		t.Fatalf("want ErrCodesUnavailable, got %v", err)
	}
	// No session was ever created, so nothing to abort.
	if got := fc.finalAbortCount(); got != 0 {
		t.Fatalf("503 fallback must not post a final abort, got %d", got)
	}
}

func TestGenerateCodeRoundTripsThroughNormalization(t *testing.T) {
	for i := 0; i < 200; i++ {
		norm, disp, err := generateCode("")
		if err != nil {
			t.Fatal(err)
		}
		if len(norm) != 12 {
			t.Fatalf("normalized code length = %d", len(norm))
		}
		back, err := ref.NormalizeCode(disp)
		if err != nil {
			t.Fatalf("display %q failed normalization: %v", disp, err)
		}
		if back != norm {
			t.Fatalf("round-trip mismatch: display %q -> %q, want %q", disp, back, norm)
		}
		if ref.Channel(norm) != norm[:4] {
			t.Fatalf("channel is not the first group")
		}
	}
}

// waitFor blocks until the box goroutine has written substr to buf (or fails the
// test after a short deadline), so the test can sequence on box-side progress.
func waitFor(t *testing.T, buf *syncBuffer, substr string) {
	t.Helper()
	deadline := time.Now().Add(2 * time.Second)
	for time.Now().Before(deadline) {
		if strings.Contains(buf.String(), substr) {
			return
		}
		time.Sleep(5 * time.Millisecond)
	}
	t.Fatalf("box never printed %q; output so far: %q", substr, buf.String())
}

// syncBuffer is a tiny concurrency-safe buffer: the box goroutine writes while the
// test reads it.
type syncBuffer struct {
	mu sync.Mutex
	b  strings.Builder
}

func (s *syncBuffer) Write(p []byte) (int, error) {
	s.mu.Lock()
	defer s.mu.Unlock()
	return s.b.Write(p)
}

func (s *syncBuffer) String() string {
	s.mu.Lock()
	defer s.mu.Unlock()
	return s.b.String()
}
