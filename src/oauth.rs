use crate::profile::OAuthCredentials;

#[derive(Debug)]
pub enum RefreshError {
    InvalidGrant,
    Other(anyhow::Error),
}

impl From<anyhow::Error> for RefreshError {
    fn from(err: anyhow::Error) -> Self {
        RefreshError::Other(err)
    }
}

const CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";
const TOKEN_URL: &str = "https://platform.claude.com/v1/oauth/token";
const SCOPES: &str = "user:profile user:inference user:sessions:claude_code user:mcp_servers";

pub fn refresh_token(creds: &OAuthCredentials) -> Result<OAuthCredentials, RefreshError> {
    let resp = minreq::post(TOKEN_URL)
        .with_header("anthropic-beta", "oauth-2025-04-20")
        .with_json(&serde_json::json!({
            "grant_type": "refresh_token",
            "refresh_token": creds.refresh_token,
            "client_id": CLIENT_ID,
            "scope": SCOPES,
        }))
        .map_err(|e| RefreshError::Other(anyhow::anyhow!("HTTP request setup failed: {}", e)))?
        .send()
        .map_err(|e| RefreshError::Other(anyhow::anyhow!("HTTP request failed: {}", e)))?;

    if resp.status_code < 200 || resp.status_code >= 300 {
        let status = resp.status_code;
        let body = resp.as_str().unwrap_or_default();
        
        // Check for invalid_grant error specifically
        if body.contains("invalid_grant") {
            return Err(RefreshError::InvalidGrant);
        }
        
        return Err(RefreshError::Other(anyhow::anyhow!("token refresh failed ({}): {}", status, body)));
    }

    let body: serde_json::Value = resp.json()
        .map_err(|e| RefreshError::Other(anyhow::anyhow!("failed to parse JSON response: {}", e)))?;
    let access_token = body["access_token"]
        .as_str()
        .ok_or_else(|| RefreshError::Other(anyhow::anyhow!("missing access_token in refresh response")))?
        .to_string();
    let refresh_token = body["refresh_token"]
        .as_str()
        .map(String::from)
        .unwrap_or_else(|| creds.refresh_token.clone());
    let expires_in = body["expires_in"].as_u64().unwrap_or(3600);
    let expires_at = now_ms() + expires_in * 1000;

    Ok(OAuthCredentials {
        access_token,
        refresh_token,
        expires_at,
        scopes: creds.scopes.clone(),
        subscription_type: creds.subscription_type.clone(),
        rate_limit_tier: creds.rate_limit_tier.clone(),
    })
}

pub fn is_expired(creds: &OAuthCredentials) -> bool {
    // Consider expired if within 5 minutes of expiry
    let buffer_ms = 5 * 60 * 1000;
    now_ms() + buffer_ms >= creds.expires_at
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}
