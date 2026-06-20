package observability

import (
	"context"
	"net/http"
	"net/http/httptest"
	"sync/atomic"
	"testing"
	"time"
)

func tokenServer(t *testing.T, mints *int32) *httptest.Server {
	return httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		atomic.AddInt32(mints, 1)
		if err := r.ParseForm(); err != nil {
			t.Errorf("parse form: %v", err)
		}
		if r.FormValue("grant_type") != "client_credentials" || r.FormValue("provider") != "client_credentials" {
			t.Errorf("bad grant params: %v", r.Form)
		}
		w.Header().Set("content-type", "application/json")
		_, _ = w.Write([]byte(`{"access_token":"tok-abc","expires_in":3600}`))
	}))
}

func TestTokenProviderValidation(t *testing.T) {
	if _, err := NewTokenProvider(TokenProviderOptions{}); err == nil {
		t.Error("missing AuthURL should error")
	}
	if _, err := NewTokenProvider(TokenProviderOptions{AuthURL: "x"}); err == nil {
		t.Error("missing ClientID should error")
	}
	if _, err := NewTokenProvider(TokenProviderOptions{AuthURL: "x", ClientID: "c"}); err == nil {
		t.Error("missing ClientSecret should error")
	}
}

func TestTokenProviderCaches(t *testing.T) {
	var mints int32
	srv := tokenServer(t, &mints)
	defer srv.Close()

	tp, err := NewTokenProvider(TokenProviderOptions{AuthURL: srv.URL, ClientID: "c", ClientSecret: "s"})
	if err != nil {
		t.Fatal(err)
	}
	for i := 0; i < 5; i++ {
		tok, err := tp.AccessToken(context.Background())
		if err != nil || tok != "tok-abc" {
			t.Fatalf("token err=%v tok=%q", err, tok)
		}
	}
	if atomic.LoadInt32(&mints) != 1 {
		t.Errorf("expected 1 mint (cached), got %d", mints)
	}
}

func TestTokenProviderRefreshesNearExpiry(t *testing.T) {
	var mints int32
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		atomic.AddInt32(&mints, 1)
		_, _ = w.Write([]byte(`{"access_token":"t","expires_in":100}`))
	}))
	defer srv.Close()

	now := time.Now()
	tp, _ := NewTokenProvider(TokenProviderOptions{
		AuthURL: srv.URL, ClientID: "c", ClientSecret: "s",
		RefreshWindow: 60 * time.Second,
		Now:           func() time.Time { return now },
	})
	if _, err := tp.AccessToken(context.Background()); err != nil {
		t.Fatal(err)
	}
	// Jump to within the refresh window (100s token, 60s window → refresh at 40s).
	now = now.Add(50 * time.Second)
	if _, err := tp.AccessToken(context.Background()); err != nil {
		t.Fatal(err)
	}
	if atomic.LoadInt32(&mints) != 2 {
		t.Errorf("expected re-mint near expiry, got %d mints", mints)
	}
}

func TestTokenProviderInvalidate(t *testing.T) {
	var mints int32
	srv := tokenServer(t, &mints)
	defer srv.Close()
	tp, _ := NewTokenProvider(TokenProviderOptions{AuthURL: srv.URL, ClientID: "c", ClientSecret: "s"})
	_, _ = tp.AccessToken(context.Background())
	tp.Invalidate()
	_, _ = tp.AccessToken(context.Background())
	if atomic.LoadInt32(&mints) != 2 {
		t.Errorf("invalidate should force re-mint, got %d", mints)
	}
}

func TestTokenProviderErrorStatus(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.WriteHeader(http.StatusUnauthorized)
		_, _ = w.Write([]byte("nope"))
	}))
	defer srv.Close()
	tp, _ := NewTokenProvider(TokenProviderOptions{AuthURL: srv.URL, ClientID: "c", ClientSecret: "s"})
	if _, err := tp.AccessToken(context.Background()); err == nil {
		t.Error("expected error on 401 token response")
	}
}
