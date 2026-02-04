//! Main setup wizard orchestration.
//!
//! The wizard guides users through:
//! 1. NEAR AI authentication
//! 2. Model selection
//! 3. Channel configuration

use std::sync::Arc;

use crate::llm::{SessionConfig, SessionManager};
use crate::settings::Settings;
use crate::setup::channels::{setup_http, setup_telegram};
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

    /// Step 3: Channel configuration.
    async fn step_channels(&mut self) -> Result<(), SetupError> {
        let options = [
            ("CLI/TUI (always enabled)", true),
            ("HTTP webhook", self.settings.channels.http_enabled),
            ("Telegram", self.settings.channels.telegram_enabled),
        ];

        let selected = select_many("Which channels do you want to enable?", &options)?;

        // HTTP is index 1
        if selected.contains(&1) {
            println!();
            let result = setup_http()?;
            self.settings.channels.http_enabled = result.enabled;
            self.settings.channels.http_port = Some(result.port);
        } else {
            self.settings.channels.http_enabled = false;
        }

        // Telegram is index 2
        if selected.contains(&2) {
            println!();
            let result = setup_telegram().await?;
            self.settings.channels.telegram_enabled = result.enabled;
        } else {
            self.settings.channels.telegram_enabled = false;
        }

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

        println!("  Channels:");
        println!("    - CLI/TUI: enabled");

        if self.settings.channels.http_enabled {
            let port = self.settings.channels.http_port.unwrap_or(8080);
            println!("    - HTTP: enabled (port {})", port);
        }

        if self.settings.channels.telegram_enabled {
            println!("    - Telegram: enabled");
        }

        println!();
        println!("To start the agent, run:");
        println!("  near-agent");
        println!();

        Ok(())
    }
}

impl Default for SetupWizard {
    fn default() -> Self {
        Self::new()
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
}
