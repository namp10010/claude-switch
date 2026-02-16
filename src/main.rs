mod oauth;
mod profile;

use crate::oauth::RefreshError;
use crate::profile::{
    OAuthAccount, OAuthCredentials, Profile,
    claude_json_path, clear_auth, read_oauth_credentials,
    list_profiles, load_profile, load_state, remove_profile, save_profile, save_state,
    write_credentials, write_oauth_account,
};
use chrono::{DateTime, Utc};
use clap::{Parser, Subcommand};
use comfy_table::{presets, Attribute, Cell, CellAlignment, Color, ContentArrangement, Table};
use std::fs;
use std::os::unix::process::CommandExt;
use std::process::Command;

#[derive(Parser)]
#[command(name = "claude-switch", about = "Manage multiple Claude Code accounts")]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Add a new profile (logs out, launches Claude CLI's auth flow, imports the result)
    Add {
        /// Profile name
        name: String,
    },
    /// Import the currently active Claude Code credentials as a named profile
    Import {
        /// Profile name
        name: String,
    },
    /// Switch to a named profile
    Use {
        /// Profile name
        name: String,
    },
    /// List all profiles
    List,
    /// Remove a profile
    Remove {
        /// Profile name
        name: String,
    },
    /// Run a command with a profile's credentials injected via environment variables
    Exec {
        /// Profile name
        name: String,
        /// Command and arguments to run
        #[arg(trailing_var_arg = true, required = true)]
        cmd: Vec<String>,
    },
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Cmd::Add { name } => cmd_add(&name)?,
        Cmd::Import { name } => cmd_import(&name)?,
        Cmd::Use { name } => cmd_use(&name)?,
        Cmd::List => cmd_list()?,
        Cmd::Remove { name } => cmd_remove(&name)?,
        Cmd::Exec { name, cmd } => cmd_exec(&name, &cmd)?,
    }

    Ok(())
}

fn cmd_add(name: &str) -> anyhow::Result<()> {
    if profile_exists(name) {
        anyhow::bail!("profile '{name}' already exists (use 'remove' first)");
    }

    let state = load_state();
    if state.active_profile.is_none() {
        anyhow::bail!(
            "no active profile — run 'claude-switch import <name>' first to save your current session"
        );
    }

    // Clear Claude's auth so the CLI triggers its first-run login flow
    clear_auth()?;

    let status = Command::new("claude")
        .arg("/login")
        .status()?;

    if !status.success() {
        anyhow::bail!("claude exited with {status} — use 'claude-switch use <profile>' to restore your previous session");
    }

    // Import the fresh credentials that Claude's auth flow just wrote.
    // /login can produce either OAuth creds or an API key.
    let claude_path = claude_json_path();

    let oauth_creds = read_oauth_credentials();

    let api_key = fs::read(&claude_path)
        .ok()
        .and_then(|data| serde_json::from_slice::<serde_json::Value>(&data).ok())
        .and_then(|doc| doc.get("primaryApiKey")?.as_str().map(String::from));

    let profile = if let Some(oauth_value) = oauth_creds {
        let credentials: OAuthCredentials = serde_json::from_value(oauth_value)?;
        let account: OAuthAccount = fs::read(&claude_path)
            .ok()
            .and_then(|data| serde_json::from_slice::<serde_json::Value>(&data).ok())
            .and_then(|doc| doc.get("oauthAccount").cloned())
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default();
        Profile::OAuth {
            credentials,
            account: Box::new(account),
        }
    } else if let Some(key) = api_key {
        Profile::ApiKey { api_key: key, label: None }
    } else {
        anyhow::bail!("no credentials found after login — did auth complete?");
    };

    save_profile(name, &profile)?;

    let mut state = load_state();
    state.active_profile = Some(name.to_string());
    save_state(&state)?;

    match &profile {
        Profile::OAuth { account, .. } => {
            let email = account.email_address.as_deref().unwrap_or("(unknown)");
            eprintln!("Saved profile '{name}' ({email})");
        }
        Profile::ApiKey { .. } => {
            eprintln!("Saved profile '{name}' (API key)");
        }
    }

    Ok(())
}

fn cmd_import(name: &str) -> anyhow::Result<()> {
    if profile_exists(name) {
        anyhow::bail!("profile '{name}' already exists (use 'remove' first)");
    }

    let claude_path = claude_json_path();

    let oauth_creds = read_oauth_credentials();

    let api_key = fs::read(&claude_path)
        .ok()
        .and_then(|data| serde_json::from_slice::<serde_json::Value>(&data).ok())
        .and_then(|doc| doc.get("primaryApiKey")?.as_str().map(String::from));

    let profile = if let Some(oauth_value) = oauth_creds {
        let credentials: OAuthCredentials = serde_json::from_value(oauth_value)?;
        let account: OAuthAccount = fs::read(&claude_path)
            .ok()
            .and_then(|data| serde_json::from_slice::<serde_json::Value>(&data).ok())
            .and_then(|doc| doc.get("oauthAccount").cloned())
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default();
        Profile::OAuth {
            credentials,
            account: Box::new(account),
        }
    } else if let Some(key) = api_key {
        Profile::ApiKey { api_key: key, label: None }
    } else {
        anyhow::bail!("no credentials found — is Claude Code logged in?");
    };

    save_profile(name, &profile)?;

    let mut state = load_state();
    state.active_profile = Some(name.to_string());
    save_state(&state)?;

    match &profile {
        Profile::OAuth { account, .. } => {
            let email = account.email_address.as_deref().unwrap_or("(unknown)");
            let sub = profile.display_sub();
            eprintln!("Imported current session as '{name}' ({email}, {sub})");
        }
        Profile::ApiKey { .. } => {
            eprintln!("Imported current session as '{name}' (API key)");
        }
    }

    Ok(())
}

fn cmd_use(name: &str) -> anyhow::Result<()> {
    let profile = load_profile(name)?;

    match profile {
        Profile::OAuth {
            mut credentials,
            account,
        } => {
            // Refresh if expired
            if oauth::is_expired(&credentials) {
                eprintln!("Token expired, refreshing...");
                match oauth::refresh_token(&credentials) {
                    Ok(refreshed_creds) => {
                        credentials = refreshed_creds;
                        // Save updated tokens back to the profile
                        save_profile(name, &Profile::OAuth {
                            credentials: credentials.clone(),
                            account: account.clone(),
                        })?;
                    }
                    Err(RefreshError::InvalidGrant) => {
                        // Refresh token is invalid, trigger re-authentication
                        let new_profile = reauthenticate_profile(name)?;
                        if let Profile::OAuth { credentials: new_creds, account: new_account } = new_profile {
                            credentials = new_creds;
                            // Update the account info as well
                            write_credentials(&credentials)?;
                            write_oauth_account(&new_account)?;
                            
                            let mut state = load_state();
                            state.active_profile = Some(name.to_string());
                            save_state(&state)?;
                            
                            eprintln!("Switched to '{name}' (re-authenticated)");
                            return Ok(());
                        } else {
                            anyhow::bail!("re-authentication resulted in non-OAuth profile");
                        }
                    }
                    Err(RefreshError::Other(e)) => {
                        return Err(e);
                    }
                }
            }

            write_credentials(&credentials)?;
            write_oauth_account(&account)?;

            let mut state = load_state();
            state.active_profile = Some(name.to_string());
            save_state(&state)?;

            eprintln!("Switched to '{name}'");
        }
        Profile::ApiKey { ref api_key, .. } => {
            let mut state = load_state();
            state.active_profile = Some(name.to_string());
            save_state(&state)?;

            eprintln!("API key profiles can't be written to Claude's config files.");
            eprintln!("Use one of these instead:\n");
            eprintln!("  export ANTHROPIC_API_KEY={api_key}");
            eprintln!("  claude-switch exec {name} -- claude");
        }
    }

    Ok(())
}

fn cmd_list() -> anyhow::Result<()> {
    let names = list_profiles()?;
    if names.is_empty() {
        eprintln!("No profiles. Use 'claude-switch add <name>' or 'claude-switch import <name>' to create one.");
        return Ok(());
    }

    let state = load_state();

    let mut table = Table::new();
    table
        .load_preset(presets::UTF8_FULL_CONDENSED)
        .set_content_arrangement(ContentArrangement::Dynamic)
        .set_header(vec![
            Cell::new("").set_alignment(CellAlignment::Center),
            Cell::new("NAME").add_attribute(Attribute::Bold),
            Cell::new("TYPE").add_attribute(Attribute::Bold),
            Cell::new("EMAIL").add_attribute(Attribute::Bold),
            Cell::new("ORG").add_attribute(Attribute::Bold),
            Cell::new("PLAN").add_attribute(Attribute::Bold),
            Cell::new("EXPIRES").add_attribute(Attribute::Bold),
        ]);

    for name in &names {
        let is_active = state.active_profile.as_deref() == Some(name.as_str());

        match load_profile(name) {
            Ok(profile) => {
                let expiry = profile
                    .expires_at()
                    .map(|ts| {
                        DateTime::<Utc>::from_timestamp_millis(ts as i64)
                            .map(|dt| dt.format("%Y-%m-%d %H:%M UTC").to_string())
                            .unwrap_or_else(|| "invalid".to_string())
                    })
                    .unwrap_or_else(|| "-".to_string());

                let active_cell = if is_active {
                    Cell::new("*").fg(Color::Green).add_attribute(Attribute::Bold)
                } else {
                    Cell::new("")
                };

                let name_cell = if is_active {
                    Cell::new(name).fg(Color::Green).add_attribute(Attribute::Bold)
                } else {
                    Cell::new(name)
                };

                table.add_row(vec![
                    active_cell,
                    name_cell,
                    Cell::new(profile.display_type()),
                    Cell::new(profile.display_email()),
                    Cell::new(profile.display_org()),
                    Cell::new(profile.display_sub()),
                    Cell::new(expiry),
                ]);
            }
            Err(_) => {
                table.add_row(vec![
                    Cell::new(if is_active { "*" } else { "" }),
                    Cell::new(name),
                    Cell::new("error").fg(Color::Red),
                    Cell::new("-"),
                    Cell::new("-"),
                    Cell::new("-"),
                    Cell::new("-"),
                ]);
            }
        }
    }

    println!("{table}");
    Ok(())
}

fn cmd_remove(name: &str) -> anyhow::Result<()> {
    remove_profile(name)?;
    eprintln!("Removed profile '{name}'");
    Ok(())
}

fn cmd_exec(name: &str, cmd: &[String]) -> anyhow::Result<()> {
    if cmd.is_empty() {
        anyhow::bail!("no command specified");
    }

    let profile = load_profile(name)?;

    match profile {
        Profile::OAuth { mut credentials, account } => {
            if oauth::is_expired(&credentials) {
                eprintln!("Token expired, refreshing...");
                match oauth::refresh_token(&credentials) {
                    Ok(refreshed_creds) => {
                        credentials = refreshed_creds;
                        save_profile(name, &Profile::OAuth {
                            credentials: credentials.clone(),
                            account,
                        })?;
                    }
                    Err(RefreshError::InvalidGrant) => {
                        // Refresh token is invalid, trigger re-authentication
                        let new_profile = reauthenticate_profile(name)?;
                        if let Profile::OAuth { credentials: new_creds, account: _new_account } = new_profile {
                            credentials = new_creds;
                            let err = Command::new(&cmd[0])
                                .args(&cmd[1..])
                                .env("CLAUDE_CODE_OAUTH_TOKEN", &credentials.access_token)
                                .exec();
                            anyhow::bail!("exec failed: {err}");
                        } else {
                            anyhow::bail!("re-authentication resulted in non-OAuth profile");
                        }
                    }
                    Err(RefreshError::Other(e)) => {
                        return Err(e);
                    }
                }
            }

            let err = Command::new(&cmd[0])
                .args(&cmd[1..])
                .env("CLAUDE_CODE_OAUTH_TOKEN", &credentials.access_token)
                .exec();
            // exec() only returns on error
            anyhow::bail!("exec failed: {err}");
        }
        Profile::ApiKey { ref api_key, .. } => {
            let err = Command::new(&cmd[0])
                .args(&cmd[1..])
                .env("ANTHROPIC_API_KEY", api_key)
                .exec();
            anyhow::bail!("exec failed: {err}");
        }
    }
}

fn profile_exists(name: &str) -> bool {
    load_profile(name).is_ok()
}

fn reauthenticate_profile(name: &str) -> anyhow::Result<Profile> {
    eprintln!("Refresh token expired for profile '{name}'. Please re-authenticate...");
    
    // Clear Claude's auth so the CLI triggers its first-run login flow
    clear_auth()?;

    let status = Command::new("claude")
        .arg("/login")
        .status()?;

    if !status.success() {
        anyhow::bail!("claude exited with {status} — re-authentication failed");
    }

    // Import the fresh credentials that Claude's auth flow just wrote.
    let claude_path = claude_json_path();

    let oauth_creds = read_oauth_credentials();

    let api_key = fs::read(&claude_path)
        .ok()
        .and_then(|data| serde_json::from_slice::<serde_json::Value>(&data).ok())
        .and_then(|doc| doc.get("primaryApiKey")?.as_str().map(String::from));

    let profile = if let Some(oauth_value) = oauth_creds {
        let credentials: OAuthCredentials = serde_json::from_value(oauth_value)?;
        let account: OAuthAccount = fs::read(&claude_path)
            .ok()
            .and_then(|data| serde_json::from_slice::<serde_json::Value>(&data).ok())
            .and_then(|doc| doc.get("oauthAccount").cloned())
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default();
        Profile::OAuth {
            credentials,
            account: Box::new(account),
        }
    } else if let Some(key) = api_key {
        Profile::ApiKey { api_key: key, label: None }
    } else {
        anyhow::bail!("no credentials found after login — did auth complete?");
    };

    // Save the updated profile
    save_profile(name, &profile)?;
    
    match &profile {
        Profile::OAuth { account, .. } => {
            let email = account.email_address.as_deref().unwrap_or("(unknown)");
            eprintln!("Profile '{name}' re-authenticated ({email})");
        }
        Profile::ApiKey { .. } => {
            eprintln!("Profile '{name}' re-authenticated (API key)");
        }
    }

    Ok(profile)
}
