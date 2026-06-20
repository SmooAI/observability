package observability

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net/http"
	"net/url"
	"strings"
	"sync"
	"time"
)

// TokenProvider is an OAuth2 client_credentials token provider — a direct port
// of the TS auth/token-provider.ts. Consulted at request time by the
// auth-injecting OTLP exporter, so there's no header snapshot / expiry drift.
// Caches in memory until refreshWindow before expiry, then refreshes;
// concurrent callers during a refresh share one in-flight request.
//
// Server contract:
//
//	POST {authUrl}/token
//	Content-Type: application/x-www-form-urlencoded
//	grant_type=client_credentials&provider=client_credentials&client_id=...&client_secret=...
type TokenProvider struct {
	authURL       string
	clientID      string
	clientSecret  string
	refreshWindow time.Duration
	client        *http.Client
	now           func() time.Time

	mu          sync.Mutex
	accessToken string
	expiresAt   time.Time
	inflight    *inflightToken
}

type inflightToken struct {
	done  chan struct{}
	token string
	err   error
}

// TokenProviderOptions configures a TokenProvider.
type TokenProviderOptions struct {
	// AuthURL is the OAuth issuer base URL, e.g. https://auth.smoo.ai.
	AuthURL string
	// ClientID is the M2M client id.
	ClientID string
	// ClientSecret is the M2M client secret (sk_...).
	ClientSecret string
	// RefreshWindow is how long before expiry to proactively refresh.
	// Defaults to 60s, matching the TS SDK.
	RefreshWindow time.Duration
	// HTTPClient overrides the default client (test seam).
	HTTPClient *http.Client
	// Now overrides the time source (test seam).
	Now func() time.Time
}

// NewTokenProvider validates options and returns a provider.
func NewTokenProvider(opts TokenProviderOptions) (*TokenProvider, error) {
	if opts.AuthURL == "" {
		return nil, errors.New("observability: TokenProvider requires AuthURL")
	}
	if opts.ClientID == "" {
		return nil, errors.New("observability: TokenProvider requires ClientID")
	}
	if opts.ClientSecret == "" {
		return nil, errors.New("observability: TokenProvider requires ClientSecret")
	}
	window := opts.RefreshWindow
	if window <= 0 {
		window = 60 * time.Second
	}
	client := opts.HTTPClient
	if client == nil {
		client = &http.Client{Timeout: 10 * time.Second}
	}
	now := opts.Now
	if now == nil {
		now = time.Now
	}
	return &TokenProvider{
		authURL:       strings.TrimRight(opts.AuthURL, "/"),
		clientID:      opts.ClientID,
		clientSecret:  opts.ClientSecret,
		refreshWindow: window,
		client:        client,
		now:           now,
	}, nil
}

// AccessToken returns a valid token, refreshing if the cached value is missing,
// expired, or within RefreshWindow of expiry. Concurrent callers during a
// refresh share one in-flight request.
func (p *TokenProvider) AccessToken(ctx context.Context) (string, error) {
	p.mu.Lock()
	if !p.shouldRefreshLocked() {
		tok := p.accessToken
		p.mu.Unlock()
		return tok, nil
	}
	if p.inflight != nil {
		fl := p.inflight
		p.mu.Unlock()
		select {
		case <-fl.done:
			return fl.token, fl.err
		case <-ctx.Done():
			return "", ctx.Err()
		}
	}
	fl := &inflightToken{done: make(chan struct{})}
	p.inflight = fl
	p.mu.Unlock()

	token, err := p.refresh(ctx)

	p.mu.Lock()
	fl.token, fl.err = token, err
	close(fl.done)
	p.inflight = nil
	p.mu.Unlock()
	return token, err
}

// Invalidate drops the cached token so the next AccessToken re-mints. Called by
// the exporter's 401 retry path.
func (p *TokenProvider) Invalidate() {
	p.mu.Lock()
	defer p.mu.Unlock()
	p.accessToken = ""
	p.expiresAt = time.Time{}
}

func (p *TokenProvider) shouldRefreshLocked() bool {
	if p.accessToken == "" {
		return true
	}
	return !p.now().Before(p.expiresAt.Add(-p.refreshWindow))
}

func (p *TokenProvider) refresh(ctx context.Context) (string, error) {
	form := url.Values{
		"grant_type":    {"client_credentials"},
		"provider":      {"client_credentials"},
		"client_id":     {p.clientID},
		"client_secret": {p.clientSecret},
	}
	req, err := http.NewRequestWithContext(ctx, http.MethodPost, p.authURL+"/token", strings.NewReader(form.Encode()))
	if err != nil {
		return "", err
	}
	req.Header.Set("Content-Type", "application/x-www-form-urlencoded")

	resp, err := p.client.Do(req)
	if err != nil {
		return "", err
	}
	defer resp.Body.Close()

	if resp.StatusCode != http.StatusOK {
		body, _ := io.ReadAll(io.LimitReader(resp.Body, 1024))
		return "", fmt.Errorf("observability: OAuth token exchange failed: HTTP %d %s", resp.StatusCode, string(body))
	}

	var parsed struct {
		AccessToken string `json:"access_token"`
		ExpiresIn   int    `json:"expires_in"`
	}
	if err := json.NewDecoder(resp.Body).Decode(&parsed); err != nil {
		return "", fmt.Errorf("observability: OAuth token response decode failed: %w", err)
	}
	if parsed.AccessToken == "" {
		return "", errors.New("observability: OAuth token endpoint returned no access_token")
	}
	expiresIn := parsed.ExpiresIn
	if expiresIn <= 0 {
		expiresIn = 3600
	}

	p.mu.Lock()
	p.accessToken = parsed.AccessToken
	p.expiresAt = p.now().Add(time.Duration(expiresIn) * time.Second)
	p.mu.Unlock()
	return parsed.AccessToken, nil
}
