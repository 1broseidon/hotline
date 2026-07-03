package opencode

import (
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"testing"
)

// TestResolveSessionPinned returns the pinned session without hitting the
// server.
func TestResolveSessionPinned(t *testing.T) {
	c := NewClient("http://127.0.0.1:1", "") // unreachable on purpose
	got, err := c.ResolveSession(context.Background(), "ses_pinned")
	if err != nil {
		t.Fatalf("ResolveSession: %v", err)
	}
	if got != "ses_pinned" {
		t.Fatalf("got %q, want ses_pinned", got)
	}
}

// TestResolveSessionMostRecent picks the session with the latest activity from
// GET /session.
func TestResolveSessionMostRecent(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path != "/session" || r.Method != http.MethodGet {
			w.WriteHeader(http.StatusNotFound)
			return
		}
		_, _ = w.Write([]byte(`[
			{"id":"ses_old","time":{"created":100,"updated":100}},
			{"id":"ses_new","time":{"created":100,"updated":900}},
			{"id":"ses_mid","time":{"created":100,"updated":500}}
		]`))
	}))
	defer srv.Close()

	c := NewClient(srv.URL, "")
	got, err := c.ResolveSession(context.Background(), "")
	if err != nil {
		t.Fatalf("ResolveSession: %v", err)
	}
	if got != "ses_new" {
		t.Fatalf("got %q, want ses_new (latest updated)", got)
	}
}

// TestResolveSessionEmpty errors clearly when the server has no sessions.
func TestResolveSessionEmpty(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		_, _ = w.Write([]byte(`[]`))
	}))
	defer srv.Close()
	c := NewClient(srv.URL, "")
	if _, err := c.ResolveSession(context.Background(), ""); err == nil {
		t.Fatal("expected error for empty session list")
	}
}

// TestPromptAsyncPayload asserts the path and JSON body of an inbound push, and
// that an EMPTY response body (the sst/opencode#2168 caveat) is not an error —
// results come from SSE, never this call.
func TestPromptAsyncPayload(t *testing.T) {
	var gotPath, gotCT string
	var gotBody promptRequest
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		gotPath = r.URL.Path
		gotCT = r.Header.Get("Content-Type")
		_ = json.NewDecoder(r.Body).Decode(&gotBody)
		// Deliberately return an empty 200 body (the known-empty-response build).
		w.WriteHeader(http.StatusOK)
	}))
	defer srv.Close()

	c := NewClient(srv.URL, "")
	if err := c.PromptAsync(context.Background(), "ses_42", "hello from telegram"); err != nil {
		t.Fatalf("PromptAsync: %v", err)
	}
	if gotPath != "/session/ses_42/prompt_async" {
		t.Fatalf("path %q", gotPath)
	}
	if gotCT != "application/json" {
		t.Fatalf("content-type %q", gotCT)
	}
	if len(gotBody.Parts) != 1 || gotBody.Parts[0].Type != "text" || gotBody.Parts[0].Text != "hello from telegram" {
		t.Fatalf("body %+v", gotBody)
	}
}

// TestAnswerPermissionPayload asserts the permissions endpoint path and body for
// both allow and deny.
func TestAnswerPermissionPayload(t *testing.T) {
	cases := []struct {
		allow    bool
		wantResp string
	}{
		{true, "once"},
		{false, "reject"},
	}
	for _, tc := range cases {
		var gotPath string
		var gotBody permissionAnswer
		srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
			gotPath = r.URL.Path
			_ = json.NewDecoder(r.Body).Decode(&gotBody)
			w.WriteHeader(http.StatusOK)
		}))
		c := NewClient(srv.URL, "")
		if err := c.AnswerPermission(context.Background(), "ses_9", "perm_abc", tc.allow); err != nil {
			t.Fatalf("AnswerPermission: %v", err)
		}
		if gotPath != "/session/ses_9/permissions/perm_abc" {
			t.Fatalf("path %q", gotPath)
		}
		if gotBody.Response != tc.wantResp {
			t.Fatalf("allow=%v response %q, want %q", tc.allow, gotBody.Response, tc.wantResp)
		}
		srv.Close()
	}
}

// TestBasicAuthHeader confirms the basic-auth password is sent when configured,
// and omitted when not.
func TestBasicAuthHeader(t *testing.T) {
	for _, password := range []string{"", "s3cret"} {
		var sawAuth bool
		var gotPass string
		srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
			_, pass, ok := r.BasicAuth()
			sawAuth = ok
			gotPass = pass
			_, _ = w.Write([]byte(`[]`))
		}))
		c := NewClient(srv.URL, password)
		_, _ = c.ResolveSession(context.Background(), "") // ignore empty-list error
		if password == "" {
			if sawAuth {
				t.Fatalf("no password set but Authorization header present")
			}
		} else {
			if !sawAuth || gotPass != password {
				t.Fatalf("basic auth: ok=%v pass=%q, want pass=%q", sawAuth, gotPass, password)
			}
		}
		srv.Close()
	}
}
