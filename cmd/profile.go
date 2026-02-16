package main

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"strings"
)

// --- Claude Code's own credential/config structures ---

type OAuthCredentials struct {
	AccessToken      string   `json:"accessToken"`
	RefreshToken     string   `json:"refreshToken"`
	ExpiresAt        uint64   `json:"expiresAt"`
	Scopes           []string `json:"scopes"`
	SubscriptionType *string  `json:"subscriptionType,omitempty"`
	RateLimitTier    *string  `json:"rateLimitTier,omitempty"`
}

type OAuthAccount struct {
	AccountUUID          *string `json:"accountUuid,omitempty"`
	EmailAddress         *string `json:"emailAddress,omitempty"`
	OrganizationUUID     *string `json:"organizationUuid,omitempty"`
	DisplayName          *string `json:"displayName,omitempty"`
	OrganizationRole     *string `json:"organizationRole,omitempty"`
	OrganizationName     *string `json:"organizationName,omitempty"`
	HasExtraUsageEnabled *bool   `json:"hasExtraUsageEnabled,omitempty"`
}

// --- Profile (tagged union via "type" field) ---

type Profile struct {
	Type        string            `json:"type"`
	Credentials *OAuthCredentials `json:"credentials,omitempty"`
	Account     *OAuthAccount     `json:"account,omitempty"`
	ApiKey      string            `json:"api_key,omitempty"`
	Label       *string           `json:"label,omitempty"`
}

func (p *Profile) DisplayEmail() string {
	if p.Type == "oauth" && p.Account != nil && p.Account.EmailAddress != nil {
		return *p.Account.EmailAddress
	}
	if p.Type == "oauth" {
		return "(unknown)"
	}
	return "-"
}

func (p *Profile) DisplayType() string {
	return p.Type
}

func (p *Profile) DisplayOrg() string {
	if p.Type == "oauth" && p.Account != nil && p.Account.OrganizationName != nil {
		return *p.Account.OrganizationName
	}
	return "-"
}

func (p *Profile) DisplaySub() string {
	if p.Type == "oauth" && p.Credentials != nil && p.Credentials.SubscriptionType != nil {
		return *p.Credentials.SubscriptionType
	}
	return "-"
}

func (p *Profile) ExpiresAt() *uint64 {
	if p.Type == "oauth" && p.Credentials != nil {
		return &p.Credentials.ExpiresAt
	}
	return nil
}

// --- State tracking ---

type State struct {
	ActiveProfile *string `json:"active_profile,omitempty"`
}

// --- Directory/path helpers ---

func configDir() string {
	if xdg := os.Getenv("XDG_CONFIG_HOME"); xdg != "" {
		return filepath.Join(xdg, "claude-switch")
	}
	home, err := os.UserHomeDir()
	if err != nil {
		return filepath.Join(".", "claude-switch")
	}
	return filepath.Join(home, ".config", "claude-switch")
}

func profilesDir() string {
	return filepath.Join(configDir(), "profiles")
}

func statePath() string {
	return filepath.Join(configDir(), "state.json")
}

func claudeConfigDir() string {
	if dir := os.Getenv("CLAUDE_CONFIG_DIR"); dir != "" {
		return dir
	}
	home, err := os.UserHomeDir()
	if err != nil {
		return filepath.Join(".", ".claude")
	}
	return filepath.Join(home, ".claude")
}

func credentialsPath() string {
	return filepath.Join(claudeConfigDir(), ".credentials.json")
}

func claudeJSONPath() string {
	home, err := os.UserHomeDir()
	if err != nil {
		return filepath.Join(".", ".claude.json")
	}
	return filepath.Join(home, ".claude.json")
}

// --- Credential reading (flat-file with macOS keychain fallback) ---

func readOAuthCredentials() json.RawMessage {
	data, err := os.ReadFile(credentialsPath())
	if err == nil {
		var doc map[string]json.RawMessage
		if json.Unmarshal(data, &doc) == nil {
			if raw, ok := doc["claudeAiOauth"]; ok {
				return raw
			}
		}
	}

	// Fallback: macOS keychain
	return readKeychainCredentials()
}

// --- File I/O with 0600 permissions ---

func writeSecure(path string, data []byte) error {
	dir := filepath.Dir(path)
	if err := os.MkdirAll(dir, 0o755); err != nil {
		return err
	}
	return os.WriteFile(path, data, 0o600)
}

// --- Profile name validation ---

func validateProfileName(name string) error {
	if name == "" || strings.Contains(name, "/") || strings.Contains(name, "\\") ||
		name == "." || name == ".." || strings.Contains(name, string(os.PathSeparator)) {
		return fmt.Errorf("invalid profile name: '%s'", name)
	}
	// Ensure it maps to exactly one normal path component
	cleaned := filepath.Clean(name)
	if cleaned != name || filepath.Base(name) != name {
		return fmt.Errorf("invalid profile name: '%s'", name)
	}
	return nil
}

func profilePath(name string) string {
	return filepath.Join(profilesDir(), name+".json")
}

// --- Profile CRUD ---

func saveProfile(name string, profile *Profile) error {
	if err := validateProfileName(name); err != nil {
		return err
	}
	data, err := json.MarshalIndent(profile, "", "  ")
	if err != nil {
		return err
	}
	return writeSecure(profilePath(name), data)
}

func loadProfile(name string) (*Profile, error) {
	if err := validateProfileName(name); err != nil {
		return nil, err
	}
	data, err := os.ReadFile(profilePath(name))
	if err != nil {
		return nil, fmt.Errorf("profile '%s' not found", name)
	}
	var profile Profile
	if err := json.Unmarshal(data, &profile); err != nil {
		return nil, err
	}
	return &profile, nil
}

func listProfiles() ([]string, error) {
	dir := profilesDir()
	entries, err := os.ReadDir(dir)
	if err != nil {
		if os.IsNotExist(err) {
			return nil, nil
		}
		return nil, err
	}
	var names []string
	for _, e := range entries {
		if e.IsDir() {
			continue
		}
		name := e.Name()
		if filepath.Ext(name) == ".json" {
			names = append(names, strings.TrimSuffix(name, ".json"))
		}
	}
	sort.Strings(names)
	return names, nil
}

func removeProfile(name string) error {
	if err := validateProfileName(name); err != nil {
		return err
	}
	path := profilePath(name)
	if _, err := os.Stat(path); os.IsNotExist(err) {
		return fmt.Errorf("profile '%s' not found", name)
	}
	if err := os.Remove(path); err != nil {
		return err
	}

	// Clear active state if this was the active profile
	state := loadState()
	if state.ActiveProfile != nil && *state.ActiveProfile == name {
		state.ActiveProfile = nil
		return saveState(&state)
	}
	return nil
}

// --- State CRUD ---

func loadState() State {
	data, err := os.ReadFile(statePath())
	if err != nil {
		return State{}
	}
	var state State
	if json.Unmarshal(data, &state) != nil {
		return State{}
	}
	return state
}

func saveState(state *State) error {
	data, err := json.MarshalIndent(state, "", "  ")
	if err != nil {
		return err
	}
	return writeSecure(statePath(), data)
}

// --- Surgical config editing ---

func writeCredentials(creds *OAuthCredentials) error {
	path := credentialsPath()
	var doc map[string]json.RawMessage

	data, err := os.ReadFile(path)
	if err == nil {
		if json.Unmarshal(data, &doc) != nil {
			doc = make(map[string]json.RawMessage)
		}
	} else {
		doc = make(map[string]json.RawMessage)
	}

	credsJSON, err := json.Marshal(creds)
	if err != nil {
		return err
	}
	doc["claudeAiOauth"] = credsJSON

	out, err := json.MarshalIndent(doc, "", "  ")
	if err != nil {
		return err
	}
	return writeSecure(path, out)
}

func writeOAuthAccount(account *OAuthAccount) error {
	path := claudeJSONPath()
	var doc map[string]json.RawMessage

	data, err := os.ReadFile(path)
	if err == nil {
		if json.Unmarshal(data, &doc) != nil {
			doc = make(map[string]json.RawMessage)
		}
	} else {
		doc = make(map[string]json.RawMessage)
	}

	accountJSON, err := json.Marshal(account)
	if err != nil {
		return err
	}
	doc["oauthAccount"] = accountJSON

	out, err := json.MarshalIndent(doc, "", "  ")
	if err != nil {
		return err
	}
	return writeSecure(path, out)
}

func clearAuth() error {
	credsPath := credentialsPath()
	if _, err := os.Stat(credsPath); err == nil {
		data, err := os.ReadFile(credsPath)
		if err != nil {
			return err
		}
		var doc map[string]json.RawMessage
		if json.Unmarshal(data, &doc) != nil {
			doc = make(map[string]json.RawMessage)
		}
		delete(doc, "claudeAiOauth")
		out, err := json.MarshalIndent(doc, "", "  ")
		if err != nil {
			return err
		}
		if err := writeSecure(credsPath, out); err != nil {
			return err
		}
	}

	claudePath := claudeJSONPath()
	if _, err := os.Stat(claudePath); err == nil {
		data, err := os.ReadFile(claudePath)
		if err != nil {
			return err
		}
		var doc map[string]json.RawMessage
		if json.Unmarshal(data, &doc) != nil {
			doc = make(map[string]json.RawMessage)
		}
		delete(doc, "oauthAccount")
		delete(doc, "primaryApiKey")
		out, err := json.MarshalIndent(doc, "", "  ")
		if err != nil {
			return err
		}
		if err := writeSecure(claudePath, out); err != nil {
			return err
		}
	}

	return nil
}
