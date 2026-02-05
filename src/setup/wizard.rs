//! Main setup wizard orchestration.
//!
//! The wizard guides users through:
//! 1. NEAR AI authentication
//! 2. Model selection
//! 3. Channel configuration

use std::sync::Arc;

use deadpool_postgres::{Config as PoolConfig, Runtime};
use secrecy::SecretString;
use tokio_postgres::NoTls;

use crate::channels::wasm::ChannelCapabilitiesFile;
use crate::llm::{SessionConfig, SessionManager};
use crate::secrets::SecretsCrypto;
use crate::settings::Settings;
use crate::setup::channels::{
    SecretsContext, setup_http, setup_telegram, setup_tunnel, setup_wasm_channel,
};
use crate::setup::prompts::{
    input, print_header, print_info, print_step, print_success, select_many, select_one,
};

/// Setup wizard error.
#[derive(Debug, thiserror::Error)]
pub enum SetupError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Authentication error: {0}")]
    Auth(String),

    #[error("Database error: {0}")]
    Database(String),

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Channel setup error: {0}")]
    Channel(String),

    #[error("User cancelled")]
    Cancelled,
}

/// Setup wizard configuration.
#[derive(Debug, Clone, Default)]
pub struct SetupConfig {
    /// Skip authentication step (use existing session).
    pub skip_auth: bool,
    /// Only reconfigure channels.
    pub channels_only: bool,
}

/// Interactive setup wizard for NEAR Agent.
pub struct SetupWizard {
    config: SetupConfig,
    settings: Settings,
    session_manager: Option<Arc<SessionManager>>,
}

impl SetupWizard {
    /// Create a new setup wizard.
    pub fn new() -> Self {
        Self {
            config: SetupConfig::default(),
            settings: Settings::load(),
            session_manager: None,
        }
    }

    /// Create a wizard with custom configuration.
    pub fn with_config(config: SetupConfig) -> Self {
        Self {
            config,
            settings: Settings::load(),
            session_manager: None,
        }
    }

    /// Set the session manager (for reusing existing auth).
    pub fn with_session(mut self, session: Arc<SessionManager>) -> Self {
        self.session_manager = Some(session);
        self
    }

    /// Run the setup wizard.
    pub async fn run(&mut self) -> Result<(), SetupError> {
        print_header("NEAR Agent Setup Wizard");

        let total_steps = if self.config.channels_only { 1 } else { 3 };
        let mut current_step = 1;

        // Step 1: Authentication (unless skipped or channels-only)
        if !self.config.channels_only && !self.config.skip_auth {
            print_step(current_step, total_steps, "NEAR AI Authentication");
            self.step_authentication().await?;
            current_step += 1;
        }

        // Step 2: Model selection (unless channels-only)
        if !self.config.channels_only {
            print_step(current_step, total_steps, "Model Selection");
            self.step_model_selection().await?;
            current_step += 1;
        }

        // Step 3: Channel configuration
        print_step(current_step, total_steps, "Channel Configuration");
        self.step_channels().await?;

        // Save settings and print summary
        self.save_and_summarize()?;

        Ok(())
    }

    /// Step 1: NEAR AI authentication.
    async fn step_authentication(&mut self) -> Result<(), SetupError> {
        // Check if we already have a session
        if let Some(ref session) = self.session_manager {
            if session.has_token().await {
                print_info("Existing session found. Validating...");
                match session.ensure_authenticated().await {
                    Ok(()) => {
                        print_success("Session valid");
                        return Ok(());
                    }
                    Err(e) => {
                        print_info(&format!("Session invalid: {}. Re-authenticating...", e));
                    }
                }
            }
        }

        // Create session manager if we don't have one
        let session = if let Some(ref s) = self.session_manager {
            Arc::clone(s)
        } else {
            let config = SessionConfig::default();
            Arc::new(SessionManager::new(config))
        };

        // Trigger authentication flow
        session
            .ensure_authenticated()
            .await
            .map_err(|e| SetupError::Auth(e.to_string()))?;

        self.session_manager = Some(session);
        Ok(())
    }

    /// Step 2: Model selection.
    async fn step_model_selection(&mut self) -> Result<(), SetupError> {
        // Show current model if already configured
        if let Some(ref current) = self.settings.selected_model {
            print_info(&format!("Current model: {}", current));
            println!();

            let options = ["Keep current model", "Change model"];
            let choice = select_one("What would you like to do?", &options)?;

            if choice == 0 {
                print_success(&format!("Keeping {}", current));
                return Ok(());
            }
        }

        // Try to fetch available models
        let models = if let Some(ref session) = self.session_manager {
            self.fetch_available_models(session).await
        } else {
            vec![]
        };

        // Default models if we couldn't fetch
        let default_models = [
            (
                "fireworks::accounts/fireworks/models/llama4-maverick-instruct-basic",
                "Llama 4 Maverick (default, fast)",
            ),
            (
                "anthropic::claude-sonnet-4-20250514",
                "Claude Sonnet 4 (best quality)",
            ),
            ("openai::gpt-4o", "GPT-4o"),
        ];

        println!("Available models:");
        println!();

        let options: Vec<&str> = if models.is_empty() {
            default_models.iter().map(|(_, desc)| *desc).collect()
        } else {
            models.iter().map(|m| m.as_str()).collect()
        };

        // Add custom option
        let mut all_options = options.clone();
        all_options.push("Custom model ID");

        let choice = select_one("Select a model:", &all_options)?;

        let selected_model = if choice == all_options.len() - 1 {
            // Custom model
            input("Enter model ID")?
        } else if models.is_empty() {
            default_models[choice].0.to_string()
        } else {
            models[choice].clone()
        };

        self.settings.selected_model = Some(selected_model.clone());
        print_success(&format!("Selected {}", selected_model));

        Ok(())
    }

    /// Fetch available models from the API.
    async fn fetch_available_models(&self, session: &Arc<SessionManager>) -> Vec<String> {
        // Create a temporary LLM provider to fetch models
        use crate::config::LlmConfig;
        use crate::llm::create_llm_provider;

        // Read base URL from env, fallback to cloud-api.near.ai
        let base_url = std::env::var("NEARAI_BASE_URL")
            .unwrap_or_else(|_| "https://cloud-api.near.ai".to_string());
        let auth_base_url = std::env::var("NEARAI_AUTH_URL")
            .unwrap_or_else(|_| "https://private.near.ai".to_string());

        let config = LlmConfig {
            nearai: crate::config::NearAiConfig {
                model: "dummy".to_string(), // Not used for listing
                base_url,
                auth_base_url,
                session_path: crate::llm::session::default_session_path(),
                api_mode: crate::config::NearAiApiMode::Responses,
                api_key: None,
            },
        };

        match create_llm_provider(&config, Arc::clone(session)) {
            Ok(provider) => match provider.list_models().await {
                Ok(models) => models,
                Err(e) => {
                    print_info(&format!("Could not fetch models: {}. Using defaults.", e));
                    vec![]
                }
            },
            Err(e) => {
                print_info(&format!(
                    "Could not initialize provider: {}. Using defaults.",
                    e
                ));
                vec![]
            }
        }
    }

    /// Initialize secrets context for channel setup.
    async fn init_secrets_context(&self) -> Result<SecretsContext, SetupError> {
        // Get DATABASE_URL
        let database_url = std::env::var("DATABASE_URL").map_err(|_| {
            SetupError::Config(
                "DATABASE_URL not set. Please set it in .env or environment.".to_string(),
            )
        })?;

        // Get or generate SECRETS_MASTER_KEY
        let master_key = match std::env::var("SECRETS_MASTER_KEY") {
            Ok(key) => {
                if key.len() < 32 {
                    return Err(SetupError::Config(
                        "SECRETS_MASTER_KEY must be at least 32 characters".to_string(),
                    ));
                }
                key
            }
            Err(_) => {
                // Generate a new master key
                print_info("SECRETS_MASTER_KEY not set. Generating a new one...");
                let key = generate_master_key();
                print_info(&format!(
                    "Generated master key. Add to your .env file:\nSECRETS_MASTER_KEY={}",
                    key
                ));
                key
            }
        };

        // Create database pool
        let mut cfg = PoolConfig::new();
        cfg.url = Some(database_url);
        cfg.pool = Some(deadpool_postgres::PoolConfig {
            max_size: 5,
            ..Default::default()
        });

        let pool = cfg
            .create_pool(Some(Runtime::Tokio1), NoTls)
            .map_err(|e| SetupError::Database(format!("Failed to create pool: {}", e)))?;

        // Test connection
        let _ = pool
            .get()
            .await
            .map_err(|e| SetupError::Database(format!("Failed to connect to database: {}", e)))?;

        print_success("Connected to database");

        // Create crypto
        let crypto = SecretsCrypto::new(SecretString::from(master_key))
            .map_err(|e| SetupError::Config(format!("Invalid master key: {}", e)))?;

        Ok(SecretsContext::new(pool, Arc::new(crypto), "default"))
    }

    /// Step 3: Channel configuration.
    async fn step_channels(&mut self) -> Result<(), SetupError> {
        // First, configure tunnel (shared across all channels that need webhooks)
        match setup_tunnel() {
            Ok(Some(url)) => {
                self.settings.tunnel.public_url = Some(url);
            }
            Ok(None) => {
                self.settings.tunnel.public_url = None;
            }
            Err(e) => {
                print_info(&format!("Tunnel setup skipped: {}", e));
            }
        }
        println!();

        // Discover available WASM channels
        let channels_dir = dirs::home_dir()
            .unwrap_or_default()
            .join(".near-agent/channels");

        let discovered_channels = discover_wasm_channels(&channels_dir).await;

        // Build options list dynamically
        let mut options: Vec<(String, bool)> = vec![
            ("CLI/TUI (always enabled)".to_string(), true),
            (
                "HTTP webhook".to_string(),
                self.settings.channels.http_enabled,
            ),
        ];

        // Add discovered WASM channels
        for (name, _) in &discovered_channels {
            let is_enabled = self.settings.channels.wasm_channels.contains(name);
            let display_name = format!("{} (WASM)", capitalize_first(name));
            options.push((display_name, is_enabled));
        }

        let options_refs: Vec<(&str, bool)> =
            options.iter().map(|(s, b)| (s.as_str(), *b)).collect();

        let selected = select_many("Which channels do you want to enable?", &options_refs)?;

        // Determine if we need secrets context
        let needs_secrets = selected.iter().any(|&i| i >= 1);
        let secrets = if needs_secrets {
            Some(self.init_secrets_context().await?)
        } else {
            None
        };

        // HTTP is index 1
        if selected.contains(&1) {
            println!();
            if let Some(ref ctx) = secrets {
                let result = setup_http(ctx).await.map_err(SetupError::Channel)?;
                self.settings.channels.http_enabled = result.enabled;
                self.settings.channels.http_port = Some(result.port);
            }
        } else {
            self.settings.channels.http_enabled = false;
        }

        // Process WASM channels (index 2 and above)
        let mut enabled_wasm_channels = Vec::new();
        for (idx, (channel_name, cap_file)) in discovered_channels.iter().enumerate() {
            let option_idx = idx + 2; // Offset for CLI and HTTP

            if selected.contains(&option_idx) {
                println!();
                if let Some(ref ctx) = secrets {
                    // Use setup schema from capabilities if available
                    let result = if !cap_file.setup.required_secrets.is_empty() {
                        setup_wasm_channel(ctx, channel_name, &cap_file.setup)
                            .await
                            .map_err(SetupError::Channel)?
                    } else {
                        // Fall back to legacy Telegram setup for backwards compatibility
                        if channel_name == "telegram" {
                            let telegram_result =
                                setup_telegram(ctx).await.map_err(SetupError::Channel)?;
                            crate::setup::channels::WasmChannelSetupResult {
                                enabled: telegram_result.enabled,
                                channel_name: "telegram".to_string(),
                            }
                        } else {
                            print_info(&format!(
                                "No setup configuration found for {}",
                                channel_name
                            ));
                            crate::setup::channels::WasmChannelSetupResult {
                                enabled: true,
                                channel_name: channel_name.to_string(),
                            }
                        }
                    };

                    if result.enabled {
                        enabled_wasm_channels.push(result.channel_name);
                    }
                }
            }
        }
        self.settings.channels.wasm_channels = enabled_wasm_channels;

        Ok(())
    }

    /// Save settings and print summary.
    fn save_and_summarize(&mut self) -> Result<(), SetupError> {
        self.settings.setup_completed = true;

        self.settings.save().map_err(|e| {
            SetupError::Io(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("Failed to save settings: {}", e),
            ))
        })?;

        println!();
        print_success("Configuration saved to ~/.near-agent/");
        println!();

        // Print summary
        println!("Configuration Summary:");
        println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

        if let Some(ref model) = self.settings.selected_model {
            println!("  Model: {}", model);
        }

        if let Some(ref tunnel_url) = self.settings.tunnel.public_url {
            println!("  Tunnel: {}", tunnel_url);
        }

        println!("  Channels:");
        println!("    - CLI/TUI: enabled");

        if self.settings.channels.http_enabled {
            let port = self.settings.channels.http_port.unwrap_or(8080);
            println!("    - HTTP: enabled (port {})", port);
        }

        for channel_name in &self.settings.channels.wasm_channels {
            let mode = if self.settings.tunnel.public_url.is_some() {
                "webhook"
            } else {
                "polling"
            };
            println!(
                "    - {}: enabled ({})",
                capitalize_first(channel_name),
                mode
            );
        }

        println!();
        println!("To start the agent, run:");
        println!("  near-agent");
        println!();

        Ok(())
    }
}

/// Generate a random 32-byte master key as hex string.
fn generate_master_key() -> String {
    use rand::RngCore;
    let mut rng = rand::thread_rng();
    let mut bytes = [0u8; 32];
    rng.fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

impl Default for SetupWizard {
    fn default() -> Self {
        Self::new()
    }
}

/// Discover WASM channels in a directory.
///
/// Returns a list of (channel_name, capabilities_file) pairs.
async fn discover_wasm_channels(dir: &std::path::Path) -> Vec<(String, ChannelCapabilitiesFile)> {
    let mut channels = Vec::new();

    if !dir.is_dir() {
        return channels;
    }

    let mut entries = match tokio::fs::read_dir(dir).await {
        Ok(e) => e,
        Err(_) => return channels,
    };

    while let Ok(Some(entry)) = entries.next_entry().await {
        let path = entry.path();

        // Look for .capabilities.json files
        let extension = path.file_name().and_then(|n| n.to_str()).unwrap_or("");

        if !extension.ends_with(".capabilities.json") {
            continue;
        }

        // Extract channel name
        let name = extension.trim_end_matches(".capabilities.json").to_string();
        if name.is_empty() {
            continue;
        }

        // Check if corresponding .wasm file exists
        let wasm_path = dir.join(format!("{}.wasm", name));
        if !wasm_path.exists() {
            continue;
        }

        // Parse capabilities file
        match tokio::fs::read(&path).await {
            Ok(bytes) => match ChannelCapabilitiesFile::from_bytes(&bytes) {
                Ok(cap_file) => {
                    channels.push((name, cap_file));
                }
                Err(e) => {
                    tracing::warn!(
                        path = %path.display(),
                        error = %e,
                        "Failed to parse channel capabilities file"
                    );
                }
            },
            Err(e) => {
                tracing::warn!(
                    path = %path.display(),
                    error = %e,
                    "Failed to read channel capabilities file"
                );
            }
        }
    }

    // Sort by name for consistent ordering
    channels.sort_by(|a, b| a.0.cmp(&b.0));
    channels
}

/// Capitalize the first letter of a string.
fn capitalize_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(first) => first.to_uppercase().chain(chars).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wizard_creation() {
        let wizard = SetupWizard::new();
        assert!(!wizard.config.skip_auth);
        assert!(!wizard.config.channels_only);
    }

    #[test]
    fn test_wizard_with_config() {
        let config = SetupConfig {
            skip_auth: true,
            channels_only: false,
        };
        let wizard = SetupWizard::with_config(config);
        assert!(wizard.config.skip_auth);
    }

    #[test]
    fn test_generate_master_key() {
        let key = generate_master_key();
        assert_eq!(key.len(), 64); // 32 bytes = 64 hex chars
    }
}
