package main

import (
	"encoding/json"
	"fmt"
	"os"
	"os/exec"
	"strings"
	"syscall"
	"text/tabwriter"
	"time"
)

const usage = `Manage multiple Claude Code accounts

Usage: claude-switch <command> [arguments]

Commands:
  add <name>              Add a new profile (logs out, launches auth flow, imports result)
  import <name>           Import currently active Claude Code credentials as a named profile
  use <name>              Switch to a named profile
  list                    List all profiles
  remove <name>           Remove a profile
  exec <name> -- <cmd>    Run a command with a profile's credentials injected
`

func main() {
	if len(os.Args) < 2 {
		fmt.Fprint(os.Stderr, usage)
		os.Exit(1)
	}

	var err error
	switch os.Args[1] {
	case "add":
		err = requireName("add", cmdAdd)
	case "import":
		err = requireName("import", cmdImport)
	case "use":
		err = requireName("use", cmdUse)
	case "list":
		err = cmdList()
	case "remove":
		err = requireName("remove", cmdRemove)
	case "exec":
		err = cmdExec()
	case "-h", "--help", "help":
		fmt.Fprint(os.Stderr, usage)
		os.Exit(0)
	default:
		fmt.Fprintf(os.Stderr, "unknown command: %s\n\n%s", os.Args[1], usage)
		os.Exit(1)
	}

	if err != nil {
		fmt.Fprintf(os.Stderr, "error: %v\n", err)
		os.Exit(1)
	}
}

func requireName(cmd string, fn func(string) error) error {
	if len(os.Args) < 3 {
		return fmt.Errorf("%s requires a profile name", cmd)
	}
	return fn(os.Args[2])
}

func cmdAdd(name string) error {
	if profileExists(name) {
		return fmt.Errorf("profile '%s' already exists (use 'remove' first)", name)
	}

	// Clear Claude's auth so the CLI triggers its first-run login flow
	if err := clearAuth(); err != nil {
		return err
	}

	cmd := exec.Command("claude", "/login")
	cmd.Stdin = os.Stdin
	cmd.Stdout = os.Stdout
	cmd.Stderr = os.Stderr
	if err := cmd.Run(); err != nil {
		return fmt.Errorf("claude exited with error — use 'claude-switch use <profile>' to restore your previous session: %w", err)
	}

	profile, err := importCurrentCredentials()
	if err != nil {
		return err
	}

	if err := saveProfile(name, profile); err != nil {
		return err
	}

	state := loadState()
	state.ActiveProfile = &name
	if err := saveState(&state); err != nil {
		return err
	}

	printProfileSaved("Saved", name, profile)
	return nil
}

func cmdImport(name string) error {
	if profileExists(name) {
		return fmt.Errorf("profile '%s' already exists (use 'remove' first)", name)
	}

	profile, err := importCurrentCredentials()
	if err != nil {
		return fmt.Errorf("no credentials found — is Claude Code logged in?")
	}

	if err := saveProfile(name, profile); err != nil {
		return err
	}

	state := loadState()
	state.ActiveProfile = &name
	if err := saveState(&state); err != nil {
		return err
	}

	if profile.Type == "oauth" {
		email := profile.DisplayEmail()
		sub := profile.DisplaySub()
		fmt.Fprintf(os.Stderr, "Imported current session as '%s' (%s, %s)\n", name, email, sub)
	} else {
		fmt.Fprintf(os.Stderr, "Imported current session as '%s' (API key)\n", name)
	}
	return nil
}

func cmdUse(name string) error {
	profile, err := loadProfile(name)
	if err != nil {
		return err
	}

	if profile.Type == "oauth" {
		if isExpired(profile.Credentials) {
			fmt.Fprintln(os.Stderr, "Token expired, refreshing...")
			refreshed, err := refreshToken(profile.Credentials)
			if err != nil {
				if re, ok := err.(*RefreshError); ok && re.Kind == refreshInvalidGrant {
					newProfile, err := reauthenticateProfile(name)
					if err != nil {
						return err
					}
					if newProfile.Type != "oauth" {
						return fmt.Errorf("re-authentication resulted in non-OAuth profile")
					}
					if err := writeCredentials(newProfile.Credentials); err != nil {
						return err
					}
					if err := writeKeychainCredentials(newProfile.Credentials); err != nil {
						return err
					}
					if err := writeOAuthAccount(newProfile.Account); err != nil {
						return err
					}
					state := loadState()
					state.ActiveProfile = &name
					if err := saveState(&state); err != nil {
						return err
					}
					fmt.Fprintf(os.Stderr, "Switched to '%s' (re-authenticated)\n", name)
					return nil
				}
				return err
			}
			profile.Credentials = refreshed
			if err := saveProfile(name, profile); err != nil {
				return err
			}
		}

		if err := writeCredentials(profile.Credentials); err != nil {
			return err
		}
		if err := writeKeychainCredentials(profile.Credentials); err != nil {
			return err
		}
		if err := writeOAuthAccount(profile.Account); err != nil {
			return err
		}

		state := loadState()
		state.ActiveProfile = &name
		if err := saveState(&state); err != nil {
			return err
		}

		fmt.Fprintf(os.Stderr, "Switched to '%s'\n", name)
	} else {
		state := loadState()
		state.ActiveProfile = &name
		if err := saveState(&state); err != nil {
			return err
		}

		fmt.Fprintln(os.Stderr, "API key profiles can't be written to Claude's config files.")
		fmt.Fprintln(os.Stderr, "Use one of these instead:")
		fmt.Fprintln(os.Stderr)
		fmt.Fprintf(os.Stderr, "  export ANTHROPIC_API_KEY=%s\n", profile.ApiKey)
		fmt.Fprintf(os.Stderr, "  claude-switch exec %s -- claude\n", name)
	}

	return nil
}

// ANSI colour helpers
const (
	ansiReset = "\033[0m"
	ansiBold  = "\033[1m"
	ansiGreen = "\033[32m"
	ansiRed   = "\033[31m"
)

func cmdList() error {
	names, err := listProfiles()
	if err != nil {
		return err
	}
	if len(names) == 0 {
		fmt.Fprintln(os.Stderr, "No profiles. Use 'claude-switch add <name>' or 'claude-switch import <name>' to create one.")
		return nil
	}

	state := loadState()

	w := tabwriter.NewWriter(os.Stdout, 0, 0, 2, ' ', 0)
	fmt.Fprintf(w, "%s\t%s\t%s\t%s\t%s\t%s\t%s\n",
		ansiBold+" "+ansiReset,
		ansiBold+"NAME"+ansiReset,
		ansiBold+"TYPE"+ansiReset,
		ansiBold+"EMAIL"+ansiReset,
		ansiBold+"ORG"+ansiReset,
		ansiBold+"PLAN"+ansiReset,
		ansiBold+"EXPIRES"+ansiReset)

	for _, name := range names {
		isActive := state.ActiveProfile != nil && *state.ActiveProfile == name

		profile, err := loadProfile(name)
		if err != nil {
			active := " "
			if isActive {
				active = "*"
			}
			fmt.Fprintf(w, "%s\t%s\t%s%s%s\t%s\t%s\t%s\t%s\n",
				active, name, ansiRed, "error", ansiReset, "-", "-", "-", "-")
			continue
		}

		expiry := "-"
		if ts := profile.ExpiresAt(); ts != nil {
			t := time.UnixMilli(int64(*ts)).UTC()
			expiry = t.Format("2006-01-02 15:04 UTC")
		}

		if isActive {
			fmt.Fprintf(w, "%s*%s\t%s%s%s\t%s\t%s\t%s\t%s\t%s\n",
				ansiGreen+ansiBold, ansiReset,
				ansiGreen+ansiBold, name, ansiReset,
				profile.DisplayType(),
				profile.DisplayEmail(),
				profile.DisplayOrg(),
				profile.DisplaySub(),
				expiry)
		} else {
			fmt.Fprintf(w, " \t%s\t%s\t%s\t%s\t%s\t%s\n",
				name,
				profile.DisplayType(),
				profile.DisplayEmail(),
				profile.DisplayOrg(),
				profile.DisplaySub(),
				expiry)
		}
	}

	w.Flush()
	return nil
}

func cmdRemove(name string) error {
	if err := removeProfile(name); err != nil {
		return err
	}
	fmt.Fprintf(os.Stderr, "Removed profile '%s'\n", name)
	return nil
}

func cmdExec() error {
	if len(os.Args) < 3 {
		return fmt.Errorf("exec requires a profile name")
	}
	name := os.Args[2]

	// Find the command args (everything after --)
	cmdArgs := os.Args[3:]
	// Strip leading "--" if present
	if len(cmdArgs) > 0 && cmdArgs[0] == "--" {
		cmdArgs = cmdArgs[1:]
	}
	if len(cmdArgs) == 0 {
		return fmt.Errorf("no command specified")
	}

	profile, err := loadProfile(name)
	if err != nil {
		return err
	}

	if profile.Type == "oauth" {
		if isExpired(profile.Credentials) {
			fmt.Fprintln(os.Stderr, "Token expired, refreshing...")
			refreshed, rerr := refreshToken(profile.Credentials)
			if rerr != nil {
				if re, ok := rerr.(*RefreshError); ok && re.Kind == refreshInvalidGrant {
					newProfile, err := reauthenticateProfile(name)
					if err != nil {
						return err
					}
					if newProfile.Type != "oauth" {
						return fmt.Errorf("re-authentication resulted in non-OAuth profile")
					}
					return execWithEnv(cmdArgs, "CLAUDE_CODE_OAUTH_TOKEN", newProfile.Credentials.AccessToken)
				}
				return rerr
			}
			profile.Credentials = refreshed
			if err := saveProfile(name, profile); err != nil {
				return err
			}
		}
		return execWithEnv(cmdArgs, "CLAUDE_CODE_OAUTH_TOKEN", profile.Credentials.AccessToken)
	}

	// API key profile
	return execWithEnv(cmdArgs, "ANTHROPIC_API_KEY", profile.ApiKey)
}

func execWithEnv(args []string, envKey, envVal string) error {
	binary, err := exec.LookPath(args[0])
	if err != nil {
		return fmt.Errorf("exec failed: %w", err)
	}
	env := append(os.Environ(), envKey+"="+envVal)
	return syscall.Exec(binary, args, env)
}

// --- Helpers ---

func profileExists(name string) bool {
	_, err := loadProfile(name)
	return err == nil
}

func importCurrentCredentials() (*Profile, error) {
	claudePath := claudeJSONPath()

	oauthRaw := readOAuthCredentials()

	// Try API key
	var apiKey string
	data, err := os.ReadFile(claudePath)
	if err == nil {
		var doc map[string]json.RawMessage
		if json.Unmarshal(data, &doc) == nil {
			if raw, ok := doc["primaryApiKey"]; ok {
				var key string
				if json.Unmarshal(raw, &key) == nil {
					apiKey = key
				}
			}
		}
	}

	if oauthRaw != nil {
		var creds OAuthCredentials
		if err := json.Unmarshal(oauthRaw, &creds); err != nil {
			return nil, fmt.Errorf("failed to parse OAuth credentials: %w", err)
		}

		var account json.RawMessage
		data, err := os.ReadFile(claudePath)
		if err == nil {
			var doc map[string]json.RawMessage
			if json.Unmarshal(data, &doc) == nil {
				account = doc["oauthAccount"]
			}
		}

		return &Profile{
			Type:        "oauth",
			Credentials: &creds,
			Account:     account,
		}, nil
	}

	if apiKey != "" {
		return &Profile{
			Type:   "api_key",
			ApiKey: apiKey,
		}, nil
	}

	return nil, fmt.Errorf("no credentials found")
}

func reauthenticateProfile(name string) (*Profile, error) {
	fmt.Fprintf(os.Stderr, "Refresh token expired for profile '%s'. Please re-authenticate...\n", name)

	if err := clearAuth(); err != nil {
		return nil, err
	}

	cmd := exec.Command("claude", "/login")
	cmd.Stdin = os.Stdin
	cmd.Stdout = os.Stdout
	cmd.Stderr = os.Stderr
	if err := cmd.Run(); err != nil {
		return nil, fmt.Errorf("claude exited with error — re-authentication failed: %w", err)
	}

	profile, err := importCurrentCredentials()
	if err != nil {
		return nil, fmt.Errorf("no credentials found after login — did auth complete?")
	}

	if err := saveProfile(name, profile); err != nil {
		return nil, err
	}

	printProfileSaved("re-authenticated", name, profile)
	return profile, nil
}

func printProfileSaved(action, name string, profile *Profile) {
	label := strings.ToUpper(action[:1]) + action[1:]
	if profile.Type == "oauth" {
		email := profile.DisplayEmail()
		fmt.Fprintf(os.Stderr, "%s profile '%s' (%s)\n", label, name, email)
	} else {
		fmt.Fprintf(os.Stderr, "%s profile '%s' (API key)\n", label, name)
	}
}
