//! Channel-specific setup flows.
//!
//! Each channel (Telegram, HTTP, etc.) has its own setup function that:
//! 1. Displays setup instructions
//! 2. Collects configuration (tokens, ports, etc.)
//! 3. Validates the configuration
//! 4. Saves secrets securely

use std::io;
use std::path::PathBuf;

use reqwest::Client;
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};

use crate::setup::prompts::{
    confirm, optional_input, print_error, print_info, print_success, secret_input,
};

/// Result of Telegram setup.
#[derive(Debug, Clone)]
pub struct TelegramSetupResult {
    pub enabled: bool,
    pub bot_username: Option<String>,
}

/// Telegram Bot API response for getMe.
#[derive(Debug, Deserialize)]
struct TelegramGetMeResponse {
    ok: bool,
    result: Option<TelegramUser>,
}

#[derive(Debug, Deserialize)]
struct TelegramUser {
    username: Option<String>,
    #[allow(dead_code)]
    first_name: String,
}

/// Set up Telegram bot channel.
///
/// Guides the user through:
/// 1. Creating a bot with @BotFather
/// 2. Entering the bot token
/// 3. Validating the token
/// 4. Saving the token to secrets
pub async fn setup_telegram() -> io::Result<TelegramSetupResult> {
    println!("Telegram Setup:");
    println!();
    print_info("To create a Telegram bot:");
    print_info("1. Open Telegram and message @BotFather");
    print_info("2. Send /newbot and follow the prompts");
    print_info("3. Copy the bot token (looks like 123456:ABC-DEF...)");
    println!();

    let token = secret_input("Bot token (from @BotFather)")?;

    // Validate the token
    print_info("Validating bot token...");

    match validate_telegram_token(&token).await {
        Ok(username) => {
            print_success(&format!(
                "Bot validated: @{}",
                username.as_deref().unwrap_or("unknown")
            ));

            // Save to secrets file
            if let Err(e) = save_channel_secret("telegram_bot_token", &token) {
                print_error(&format!("Failed to save token: {}", e));
                return Err(io::Error::new(io::ErrorKind::Other, e.to_string()));
            }

            print_success("Token saved to ~/.near-agent/secrets/telegram_bot_token");

            Ok(TelegramSetupResult {
                enabled: true,
                bot_username: username,
            })
        }
        Err(e) => {
            print_error(&format!("Token validation failed: {}", e));

            if confirm("Try again?", true)? {
                // Recursive retry
                Box::pin(setup_telegram()).await
            } else {
                Ok(TelegramSetupResult {
                    enabled: false,
                    bot_username: None,
                })
            }
        }
    }
}

/// Validate a Telegram bot token by calling the getMe API.
///
/// Returns the bot's username if valid.
pub async fn validate_telegram_token(token: &SecretString) -> Result<Option<String>, String> {
    let client = Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {}", e))?;

    let url = format!(
        "https://api.telegram.org/bot{}/getMe",
        token.expose_secret()
    );

    let response = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("Request failed: {}", e))?;

    if !response.status().is_success() {
        return Err(format!("API returned status {}", response.status()));
    }

    let body: TelegramGetMeResponse = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse response: {}", e))?;

    if body.ok {
        Ok(body.result.and_then(|u| u.username))
    } else {
        Err("Telegram API returned error".to_string())
    }
}

/// Result of HTTP webhook setup.
#[derive(Debug, Clone)]
pub struct HttpSetupResult {
    pub enabled: bool,
    pub port: u16,
    pub host: String,
}

/// Set up HTTP webhook channel.
pub fn setup_http() -> io::Result<HttpSetupResult> {
    println!("HTTP Webhook Setup:");
    println!();
    print_info("The HTTP webhook allows external services to send messages to the agent.");
    println!();

    let port_str = optional_input("Port", Some("default: 8080"))?;
    let port: u16 =
        port_str.as_deref().unwrap_or("8080").parse().map_err(|e| {
            io::Error::new(io::ErrorKind::InvalidInput, format!("Invalid port: {}", e))
        })?;

    if port < 1024 {
        print_info("Note: Ports below 1024 may require root privileges");
    }

    let host =
        optional_input("Host", Some("default: 0.0.0.0"))?.unwrap_or_else(|| "0.0.0.0".to_string());

    // Generate a webhook secret
    if confirm("Generate a webhook secret for authentication?", true)? {
        let secret = generate_webhook_secret();
        save_channel_secret("http_webhook_secret", &SecretString::from(secret.clone()))?;
        print_success("Webhook secret generated and saved");
        print_info(&format!(
            "Secret: {} (store this for your webhook clients)",
            secret
        ));
    }

    print_success(&format!("HTTP webhook will listen on {}:{}", host, port));

    Ok(HttpSetupResult {
        enabled: true,
        port,
        host,
    })
}

/// Generate a random webhook secret.
fn generate_webhook_secret() -> String {
    use rand::RngCore;
    let mut rng = rand::thread_rng();
    let mut bytes = [0u8; 32];
    rng.fill_bytes(&mut bytes);
    // Encode as hex manually (avoid adding hex crate dependency)
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

/// Get the secrets directory path.
pub fn secrets_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".near-agent")
        .join("secrets")
}

/// Save a channel secret to the secrets directory.
///
/// Secrets are stored as individual files with restricted permissions.
pub fn save_channel_secret(name: &str, value: &SecretString) -> io::Result<()> {
    let dir = secrets_dir();
    std::fs::create_dir_all(&dir)?;

    let path = dir.join(name);

    // Write the secret
    std::fs::write(&path, value.expose_secret())?;

    // Set restrictive permissions on Unix
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&path)?.permissions();
        perms.set_mode(0o600); // Owner read/write only
        std::fs::set_permissions(&path, perms)?;
    }

    Ok(())
}

/// Load a channel secret from the secrets directory.
#[allow(dead_code)]
pub fn load_channel_secret(name: &str) -> io::Result<Option<SecretString>> {
    let path = secrets_dir().join(name);

    if !path.exists() {
        return Ok(None);
    }

    let contents = std::fs::read_to_string(&path)?;
    Ok(Some(SecretString::from(contents.trim().to_string())))
}

/// Check if a channel secret exists.
#[allow(dead_code)]
pub fn has_channel_secret(name: &str) -> bool {
    secrets_dir().join(name).exists()
}

/// Delete a channel secret.
#[allow(dead_code)]
pub fn delete_channel_secret(name: &str) -> io::Result<bool> {
    let path = secrets_dir().join(name);
    if path.exists() {
        std::fs::remove_file(&path)?;
        Ok(true)
    } else {
        Ok(false)
    }
}

/// Channel secrets configuration (persisted to settings).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ChannelSecretsConfig {
    /// Whether Telegram has a saved token.
    pub telegram_configured: bool,
    /// Whether HTTP webhook has a saved secret.
    pub http_configured: bool,
}

impl ChannelSecretsConfig {
    /// Load from the secrets directory.
    #[allow(dead_code)]
    pub fn from_secrets_dir() -> Self {
        Self {
            telegram_configured: has_channel_secret("telegram_bot_token"),
            http_configured: has_channel_secret("http_webhook_secret"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_webhook_secret() {
        let secret = generate_webhook_secret();
        assert_eq!(secret.len(), 64); // 32 bytes = 64 hex chars
    }
}
