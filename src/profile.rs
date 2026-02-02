use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::io;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

// --- Claude Code's own credential/config structures ---

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OAuthCredentials {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at: u64,
    pub scopes: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subscription_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rate_limit_tier: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OAuthAccount {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account_uuid: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email_address: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub organization_uuid: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub organization_role: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub organization_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub has_extra_usage_enabled: Option<bool>,
}

// --- Our profile types ---

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Profile {
    #[serde(rename = "oauth")]
    OAuth {
        credentials: OAuthCredentials,
        account: Box<OAuthAccount>,
    },
    #[serde(rename = "api_key")]
    ApiKey {
        api_key: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        label: Option<String>,
    },
}

impl Profile {
    pub fn display_email(&self) -> &str {
        match self {
            Profile::OAuth { account, .. } => {
                account.email_address.as_deref().unwrap_or("(unknown)")
            }
            Profile::ApiKey { .. } => "-",
        }
    }

    pub fn display_type(&self) -> &str {
        match self {
            Profile::OAuth { .. } => "oauth",
            Profile::ApiKey { .. } => "api_key",
        }
    }

    pub fn display_org(&self) -> &str {
        match self {
            Profile::OAuth { account, .. } => {
                account.organization_name.as_deref().unwrap_or("-")
            }
            Profile::ApiKey { .. } => "-",
        }
    }

    pub fn display_sub(&self) -> &str {
        match self {
            Profile::OAuth { credentials, .. } => {
                credentials.subscription_type.as_deref().unwrap_or("-")
            }
            Profile::ApiKey { .. } => "-",
        }
    }

    pub fn expires_at(&self) -> Option<u64> {
        match self {
            Profile::OAuth { credentials, .. } => Some(credentials.expires_at),
            Profile::ApiKey { .. } => None,
        }
    }
}

// --- State tracking ---

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct State {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_profile: Option<String>,
}

// --- Directory/path helpers ---

pub fn config_dir() -> PathBuf {
    let base = dirs_next().unwrap_or_else(|| PathBuf::from("."));
    base.join("claude-switch")
}

fn dirs_next() -> Option<PathBuf> {
    // XDG_CONFIG_HOME or ~/.config
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            home::home_dir().map(|h| h.join(".config"))
        })
}

pub fn profiles_dir() -> PathBuf {
    config_dir().join("profiles")
}

pub fn state_path() -> PathBuf {
    config_dir().join("state.json")
}

pub fn claude_config_dir() -> PathBuf {
    std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            home::home_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".claude")
        })
}

pub fn credentials_path() -> PathBuf {
    claude_config_dir().join(".credentials.json")
}

pub fn claude_json_path() -> PathBuf {
    home::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".claude.json")
}

// --- File I/O with 0600 permissions ---

fn write_secure(path: &Path, data: &[u8]) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut opts = fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true).mode(0o600);
    io::Write::write_all(&mut opts.open(path)?, data)?;
    Ok(())
}

// --- Profile CRUD ---

// To prevent directory traversal attacks we ensure the name maps to exactly one normal
// path component.
fn validate_profile_name(name: &str) -> anyhow::Result<()> {
    let path = Path::new(name);
    let mut components = path.components().peekable();
    let valid = matches!(components.peek(), Some(std::path::Component::Normal(_)))
        && components.count() == 1;
    if !valid {
        anyhow::bail!("invalid profile name: '{name}'");
    }
    Ok(())
}

fn profile_path(name: &str) -> PathBuf {
    profiles_dir().join(format!("{name}.json"))
}

pub fn save_profile(name: &str, profile: &Profile) -> anyhow::Result<()> {
    validate_profile_name(name)?;
    let data = serde_json::to_vec_pretty(profile)?;
    write_secure(&profile_path(name), &data)?;
    Ok(())
}

pub fn load_profile(name: &str) -> anyhow::Result<Profile> {
    validate_profile_name(name)?;
    let path = profile_path(name);
    let data = fs::read(&path)
        .map_err(|_| anyhow::anyhow!("profile '{}' not found", name))?;
    Ok(serde_json::from_slice(&data)?)
}

pub fn list_profiles() -> anyhow::Result<Vec<String>> {
    let dir = profiles_dir();
    if !dir.exists() {
        return Ok(vec![]);
    }
    let mut names = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("json")
            && let Some(stem) = path.file_stem().and_then(|s| s.to_str())
        {
            names.push(stem.to_string());
        }
    }
    names.sort();
    Ok(names)
}

pub fn remove_profile(name: &str) -> anyhow::Result<()> {
    validate_profile_name(name)?;
    let path = profile_path(name);
    if !path.exists() {
        anyhow::bail!("profile '{}' not found", name);
    }
    fs::remove_file(path)?;

    // Clear active state if this was the active profile
    let mut state = load_state();
    if state.active_profile.as_deref() == Some(name) {
        state.active_profile = None;
        save_state(&state)?;
    }
    Ok(())
}

// --- State CRUD ---

pub fn load_state() -> State {
    let path = state_path();
    fs::read(&path)
        .ok()
        .and_then(|data| serde_json::from_slice(&data).ok())
        .unwrap_or_default()
}

pub fn save_state(state: &State) -> anyhow::Result<()> {
    let data = serde_json::to_vec_pretty(state)?;
    write_secure(&state_path(), &data)?;
    Ok(())
}

// --- Surgical config editing ---

pub fn write_credentials(creds: &OAuthCredentials) -> anyhow::Result<()> {
    let path = credentials_path();
    let mut doc: HashMap<String, serde_json::Value> = if path.exists() {
        let data = fs::read(&path)?;
        serde_json::from_slice(&data).unwrap_or_default()
    } else {
        HashMap::new()
    };
    doc.insert(
        "claudeAiOauth".to_string(),
        serde_json::to_value(creds)?,
    );
    let data = serde_json::to_vec_pretty(&doc)?;
    write_secure(&path, &data)?;
    Ok(())
}

pub fn write_oauth_account(account: &OAuthAccount) -> anyhow::Result<()> {
    let path = claude_json_path();
    let mut doc: serde_json::Value = if path.exists() {
        let data = fs::read(&path)?;
        serde_json::from_slice(&data)?
    } else {
        serde_json::json!({})
    };
    if let Some(obj) = doc.as_object_mut() {
        obj.insert("oauthAccount".to_string(), serde_json::to_value(account)?);
    }
    let data = serde_json::to_vec_pretty(&doc)?;
    write_secure(&path, &data)?;
    Ok(())
}

/// Remove auth state (OAuth + API key) from Claude's config files so the CLI sees "not logged in."
pub fn clear_auth() -> anyhow::Result<()> {
    let creds_path = credentials_path();
    if creds_path.exists() {
        let data = fs::read(&creds_path)?;
        let mut doc: HashMap<String, serde_json::Value> =
            serde_json::from_slice(&data).unwrap_or_default();
        doc.remove("claudeAiOauth");
        let data = serde_json::to_vec_pretty(&doc)?;
        write_secure(&creds_path, &data)?;
    }

    let claude_path = claude_json_path();
    if claude_path.exists() {
        let data = fs::read(&claude_path)?;
        let mut doc: serde_json::Value = serde_json::from_slice(&data)?;
        if let Some(obj) = doc.as_object_mut() {
            obj.remove("oauthAccount");
            obj.remove("primaryApiKey");
        }
        let data = serde_json::to_vec_pretty(&doc)?;
        write_secure(&claude_path, &data)?;
    }

    Ok(())
}
