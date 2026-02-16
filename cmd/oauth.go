package main

import (
	"bytes"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"time"
)

const (
	clientID = "9d1c250a-e61b-44d9-88ed-5944d1962f5e"
	tokenURL = "https://platform.claude.com/v1/oauth/token"
	scopes   = "user:profile user:inference user:sessions:claude_code user:mcp_servers"
)

type refreshErrorKind int

const (
	refreshInvalidGrant refreshErrorKind = iota
	refreshOther
)

type RefreshError struct {
	Kind    refreshErrorKind
	Message string
}

func (e *RefreshError) Error() string {
	return e.Message
}

func refreshToken(creds *OAuthCredentials) (*OAuthCredentials, error) {
	reqBody, err := json.Marshal(map[string]string{
		"grant_type":    "refresh_token",
		"refresh_token": creds.RefreshToken,
		"client_id":     clientID,
		"scope":         scopes,
	})
	if err != nil {
		return nil, fmt.Errorf("failed to marshal request: %w", err)
	}

	req, err := http.NewRequest("POST", tokenURL, bytes.NewReader(reqBody))
	if err != nil {
		return nil, fmt.Errorf("HTTP request setup failed: %w", err)
	}
	req.Header.Set("Content-Type", "application/json")
	req.Header.Set("anthropic-beta", "oauth-2025-04-20")

	resp, err := http.DefaultClient.Do(req)
	if err != nil {
		return nil, fmt.Errorf("HTTP request failed: %w", err)
	}
	defer resp.Body.Close()

	body, err := io.ReadAll(resp.Body)
	if err != nil {
		return nil, fmt.Errorf("failed to read response: %w", err)
	}

	if resp.StatusCode < 200 || resp.StatusCode >= 300 {
		bodyStr := string(body)
		if bytes.Contains(body, []byte("invalid_grant")) {
			return nil, &RefreshError{Kind: refreshInvalidGrant, Message: "invalid_grant"}
		}
		return nil, &RefreshError{
			Kind:    refreshOther,
			Message: fmt.Sprintf("token refresh failed (%d): %s", resp.StatusCode, bodyStr),
		}
	}

	var result map[string]any
	if err := json.Unmarshal(body, &result); err != nil {
		return nil, fmt.Errorf("failed to parse JSON response: %w", err)
	}

	accessToken, ok := result["access_token"].(string)
	if !ok {
		return nil, fmt.Errorf("missing access_token in refresh response")
	}

	newRefreshToken := creds.RefreshToken
	if rt, ok := result["refresh_token"].(string); ok {
		newRefreshToken = rt
	}

	expiresIn := uint64(3600)
	if ei, ok := result["expires_in"].(float64); ok {
		expiresIn = uint64(ei)
	}
	expiresAt := nowMs() + expiresIn*1000

	return &OAuthCredentials{
		AccessToken:      accessToken,
		RefreshToken:     newRefreshToken,
		ExpiresAt:        expiresAt,
		Scopes:           creds.Scopes,
		SubscriptionType: creds.SubscriptionType,
		RateLimitTier:    creds.RateLimitTier,
	}, nil
}

func isExpired(creds *OAuthCredentials) bool {
	// Consider expired if within 5 minutes of expiry
	bufferMs := uint64(5 * 60 * 1000)
	return nowMs()+bufferMs >= creds.ExpiresAt
}

func nowMs() uint64 {
	return uint64(time.Now().UnixMilli())
}
