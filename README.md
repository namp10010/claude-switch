# claude-switch

Switch between multiple Claude Code accounts without re-authenticating. Keep a personal login, a work login, and an API key profile side by side — swap between them in one command.

## Install

```sh
cargo install claude-switch
```

Or grab the binary from the [latest release](https://github.com/Global-Astro-Labs/claude-switch/releases/latest) (statically linked, runs on any x86_64 Linux):

```sh
curl -fsSL https://github.com/Global-Astro-Labs/claude-switch/releases/latest/download/claude-switch-x86_64-linux -o claude-switch
chmod +x claude-switch
```

## Quick start

Save your current Claude Code session, then add a second account:

```
claude-switch import work
claude-switch add personal
```

Switch between them:

```
claude-switch use personal
claude-switch use work
```

## Usage

### `import <name>`

Snapshot the currently active Claude Code credentials as a named profile:

```
claude-switch import work
```

### `add <name>`

Launch the Claude CLI's login flow to authenticate a new account. Supports both OAuth and API key:

```
claude-switch add personal
```

### `use <name>`

Switch to a named profile. For OAuth profiles, this writes credentials directly into Claude Code's config files. Only auth-related keys are touched; everything else is left intact.

```
claude-switch use personal
```

For API key profiles, it prints the export command instead (since API keys are passed via environment variable):

```
claude-switch use dev
# prints: export ANTHROPIC_API_KEY=sk-ant-...
```

### `exec <name> -- <command>`

Run a command with a profile's credentials injected via environment variables. No config files are modified.

```
claude-switch exec work -- claude
claude-switch exec dev -- claude --print "hello"
```

Sets `CLAUDE_CODE_OAUTH_TOKEN` for OAuth profiles or `ANTHROPIC_API_KEY` for API key profiles.

### `list`

Show all profiles with the active profile, type, email, org, plan, and token expiry.

```
claude-switch list
```

### `remove <name>`

Delete a profile.

```
claude-switch remove old-account
```

## How it works

Profiles are stored in `~/.config/claude-switch/profiles/` as JSON files (mode 0600). Each profile contains either OAuth tokens (access + refresh) or an API key.

When switching OAuth profiles, `claude-switch` surgically edits two files:

- `~/.claude/.credentials.json` — replaces the `claudeAiOauth` key
- `~/.claude.json` — replaces the `oauthAccount` key

All other keys in those files are preserved. The `CLAUDE_CONFIG_DIR` environment variable is respected if set.

Expired OAuth tokens are automatically refreshed when switching or exec-ing.

## License

ISC
