//! NEAR Agent - Main entry point.

use std::sync::Arc;

use clap::Parser;
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

use near_agent::{
    agent::{Agent, AgentDeps},
    channels::{
        AppEvent, ChannelManager, HttpChannel, ReplChannel, TuiChannel,
        wasm::{
            RegisteredEndpoint, SharedWasmChannel, WasmChannelLoader, WasmChannelRouter,
            WasmChannelRuntime, WasmChannelRuntimeConfig, WasmChannelServer,
        },
    },
    cli::{Cli, Command, run_tool_command},
    config::Config,
    history::Store,
    llm::{SessionConfig, create_llm_provider, create_session_manager},
    safety::SafetyLayer,
    secrets::{PostgresSecretsStore, SecretsCrypto, SecretsStore},
    settings::Settings,
    setup::{SetupConfig, SetupWizard},
    tools::{
        ToolRegistry,
        wasm::{WasmToolLoader, WasmToolRuntime},
    },
    workspace::{EmbeddingProvider, NearAiEmbeddings, OpenAiEmbeddings, Workspace},
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    // Handle non-agent commands first (they don't need TUI/full setup)
    match &cli.command {
        Some(Command::Tool(tool_cmd)) => {
            // Simple logging for CLI commands
            tracing_subscriber::fmt()
                .with_env_filter(
                    EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn")),
                )
                .init();

            return run_tool_command(tool_cmd.clone()).await;
        }
        Some(Command::Setup {
            skip_auth,
            channels_only,
        }) => {
            // Load .env before running setup wizard
            let _ = dotenvy::dotenv();

            // Run setup wizard
            let config = SetupConfig {
                skip_auth: *skip_auth,
                channels_only: *channels_only,
            };
            let mut wizard = SetupWizard::with_config(config);
            wizard.run().await?;
            return Ok(());
        }
        None | Some(Command::Run) => {
            // Continue to run agent
        }
    }

    // Load .env if present
    let _ = dotenvy::dotenv();

    // First-run detection: if setup hasn't been completed and user didn't skip it,
    // automatically run the setup wizard
    if !cli.no_setup {
        let settings = Settings::load();
        let session_path = near_agent::llm::session::default_session_path();

        if !settings.setup_completed && !session_path.exists() {
            println!("First run detected. Starting setup wizard...");
            println!();
            let mut wizard = SetupWizard::new();
            wizard.run().await?;
        }
    }

    // Load configuration (after potential setup)
    let config = Config::from_env()?;

    // Initialize session manager and authenticate BEFORE TUI setup
    // This allows the auth menu to display cleanly without TUI interference
    let session_config = SessionConfig {
        auth_base_url: config.llm.nearai.auth_base_url.clone(),
        session_path: config.llm.nearai.session_path.clone(),
        ..Default::default()
    };
    let session = create_session_manager(session_config).await;

    // Ensure we're authenticated before proceeding (may trigger login flow)
    // This happens before TUI so the menu displays correctly
    session.ensure_authenticated().await?;

    // Initialize tracing and channels based on mode
    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("near_agent=info,tower_http=debug"));

    // Determine which mode to use: REPL, single message, or TUI
    let use_repl = cli.repl || cli.message.is_some();

    // Create appropriate channel based on mode
    let (tui_channel, tui_event_sender, repl_channel) = if use_repl {
        // REPL mode - use simple stdin/stdout
        tracing_subscriber::registry()
            .with(env_filter)
            .with(tracing_subscriber::fmt::layer().with_target(false))
            .init();

        let repl = if let Some(ref msg) = cli.message {
            ReplChannel::with_message(msg.clone())
        } else {
            ReplChannel::new()
        };

        (None, None, Some(repl))
    } else if config.channels.cli.enabled {
        // TUI mode
        let channel = TuiChannel::new();
        let log_writer = channel.log_writer();
        let event_sender = channel.event_sender();

        tracing_subscriber::registry()
            .with(env_filter)
            .with(
                tracing_subscriber::fmt::layer()
                    .with_writer(log_writer)
                    .without_time()
                    .with_target(false)
                    .with_level(true),
            )
            .init();

        (Some(channel), Some(event_sender), None)
    } else {
        // No CLI - just logging
        tracing_subscriber::registry()
            .with(env_filter)
            .with(tracing_subscriber::fmt::layer().with_target(false))
            .init();

        (None, None, None)
    };

    tracing::info!("Starting NEAR Agent...");
    tracing::info!("Loaded configuration for agent: {}", config.agent.name);
    tracing::info!("NEAR AI session authenticated");

    // Initialize database store (optional for testing)
    let store = if cli.no_db {
        tracing::warn!("Running without database connection");
        None
    } else {
        let store = Store::new(&config.database).await?;
        store.run_migrations().await?;
        tracing::info!("Database connected and migrations applied");
        Some(Arc::new(store))
    };

    // Initialize LLM provider (clone session so we can reuse it for embeddings)
    let llm = create_llm_provider(&config.llm, session.clone())?;
    tracing::info!("LLM provider initialized: {}", llm.model_name());

    // Fetch available models and send to TUI (async, non-blocking)
    if let Some(ref event_tx) = tui_event_sender {
        let llm_for_models = llm.clone();
        let event_tx = event_tx.clone();
        tokio::spawn(async move {
            match llm_for_models.list_models().await {
                Ok(models) if !models.is_empty() => {
                    let _ = event_tx.send(AppEvent::AvailableModels(models)).await;
                }
                Ok(_) => {
                    let _ = event_tx
                        .send(AppEvent::ErrorMessage(
                            "No models available from API".into(),
                        ))
                        .await;
                }
                Err(e) => {
                    let _ = event_tx
                        .send(AppEvent::ErrorMessage(format!(
                            "Failed to fetch models: {}",
                            e
                        )))
                        .await;
                }
            }
        });
    }

    // Initialize safety layer
    let safety = Arc::new(SafetyLayer::new(&config.safety));
    tracing::info!("Safety layer initialized");

    // Initialize tool registry
    let tools = Arc::new(ToolRegistry::new());
    tools.register_builtin_tools();
    tracing::info!("Registered {} built-in tools", tools.count());

    // Create embeddings provider if configured
    let embeddings: Option<Arc<dyn EmbeddingProvider>> = if config.embeddings.enabled {
        match config.embeddings.provider.as_str() {
            "nearai" => {
                tracing::info!(
                    "Embeddings enabled via NEAR AI (model: {})",
                    config.embeddings.model
                );
                Some(Arc::new(
                    NearAiEmbeddings::new(&config.llm.nearai.base_url, session.clone())
                        .with_model(&config.embeddings.model, 1536),
                ))
            }
            _ => {
                // Default to OpenAI for unknown providers
                if let Some(api_key) = config.embeddings.openai_api_key() {
                    tracing::info!(
                        "Embeddings enabled via OpenAI (model: {})",
                        config.embeddings.model
                    );
                    Some(Arc::new(OpenAiEmbeddings::with_model(
                        api_key,
                        &config.embeddings.model,
                        match config.embeddings.model.as_str() {
                            "text-embedding-3-large" => 3072,
                            _ => 1536, // text-embedding-3-small and ada-002
                        },
                    )))
                } else {
                    tracing::warn!("Embeddings configured but OPENAI_API_KEY not set");
                    None
                }
            }
        }
    } else {
        tracing::info!("Embeddings disabled (set OPENAI_API_KEY or EMBEDDING_ENABLED=true)");
        None
    };

    // Register memory tools if database is available
    if let Some(ref store) = store {
        let mut workspace = Workspace::new("default", store.pool());
        if let Some(ref emb) = embeddings {
            workspace = workspace.with_embeddings(emb.clone());
        }
        let workspace = Arc::new(workspace);
        tools.register_memory_tools(workspace);
    }

    // Register builder tool if enabled
    if config.builder.enabled {
        tools
            .register_builder_tool(
                llm.clone(),
                safety.clone(),
                Some(config.builder.to_builder_config()),
            )
            .await;
        tracing::info!("Builder mode enabled");
    }

    // Load installed WASM tools
    if config.wasm.enabled && config.wasm.tools_dir.exists() {
        match WasmToolRuntime::new(config.wasm.to_runtime_config()) {
            Ok(runtime) => {
                let runtime = Arc::new(runtime);
                let loader = WasmToolLoader::new(Arc::clone(&runtime), Arc::clone(&tools));

                match loader.load_from_dir(&config.wasm.tools_dir).await {
                    Ok(results) => {
                        if !results.loaded.is_empty() {
                            tracing::info!(
                                "Loaded {} WASM tools from {}",
                                results.loaded.len(),
                                config.wasm.tools_dir.display()
                            );
                        }
                        for (path, err) in &results.errors {
                            tracing::warn!("Failed to load WASM tool {}: {}", path.display(), err);
                        }
                    }
                    Err(e) => {
                        tracing::warn!("Failed to scan WASM tools directory: {}", e);
                    }
                }
            }
            Err(e) => {
                tracing::warn!("Failed to initialize WASM runtime: {}", e);
            }
        }
    }
    tracing::info!(
        "Tool registry initialized with {} total tools",
        tools.count()
    );

    // Initialize channel manager
    let mut channels = ChannelManager::new();

    // Add REPL channel if in REPL mode
    if let Some(repl) = repl_channel {
        channels.add(Box::new(repl));
        if cli.message.is_some() {
            tracing::info!("Single message mode");
        } else {
            tracing::info!("REPL mode enabled");
        }
    }
    // Add TUI channel if CLI is enabled (already created for logging hookup)
    else if let Some(tui) = tui_channel {
        channels.add(Box::new(tui));
        tracing::info!("TUI channel enabled");
    }

    // Add HTTP channel if configured and not CLI-only mode
    if !cli.cli_only && !use_repl {
        if let Some(ref http_config) = config.channels.http {
            channels.add(Box::new(HttpChannel::new(http_config.clone())));
            tracing::info!(
                "HTTP channel enabled on {}:{}",
                http_config.host,
                http_config.port
            );
        }
    }

    // Create secrets store if master key is configured (needed for Telegram webhook registration)
    let secrets_store: Option<Arc<dyn SecretsStore + Send + Sync>> =
        if let (Some(store), Some(master_key)) = (&store, config.secrets.master_key()) {
            match SecretsCrypto::new(master_key.clone()) {
                Ok(crypto) => Some(Arc::new(PostgresSecretsStore::new(
                    store.pool(),
                    Arc::new(crypto),
                ))),
                Err(e) => {
                    tracing::warn!("Failed to initialize secrets crypto: {}", e);
                    None
                }
            }
        } else {
            None
        };

    // Load WASM channels if enabled
    if config.channels.wasm_channels_enabled && config.channels.wasm_channels_dir.exists() {
        match WasmChannelRuntime::new(WasmChannelRuntimeConfig::default()) {
            Ok(runtime) => {
                let runtime = Arc::new(runtime);
                let loader = WasmChannelLoader::new(Arc::clone(&runtime));

                match loader
                    .load_from_dir(&config.channels.wasm_channels_dir)
                    .await
                {
                    Ok(results) => {
                        // Create router for WASM channel webhooks
                        let wasm_router = Arc::new(WasmChannelRouter::new());
                        let mut has_webhook_channels = false;

                        for channel in results.loaded {
                            let channel_name = channel.channel_name().to_string();
                            tracing::info!("Loaded WASM channel: {}", channel_name);

                            // Get webhook secret for this channel from secrets store
                            let webhook_secret = if let Some(ref secrets) = secrets_store {
                                let secret_name = format!("{}_webhook_secret", channel_name);
                                secrets
                                    .get_decrypted("default", &secret_name)
                                    .await
                                    .ok()
                                    .map(|s| s.expose().to_string())
                            } else {
                                None
                            };

                            // Register channel with router for webhook handling
                            // Use known webhook path based on channel name
                            let webhook_path = format!("/webhook/{}", channel_name);
                            let endpoints = vec![RegisteredEndpoint {
                                channel_name: channel_name.clone(),
                                path: webhook_path.clone(),
                                methods: vec!["POST".to_string()],
                                require_secret: webhook_secret.is_some(),
                            }];

                            let channel_arc = Arc::new(channel);

                            // Clone webhook_secret before moving it to register()
                            // We need it later for Telegram API registration
                            let webhook_secret_for_telegram = webhook_secret.clone();

                            tracing::info!(
                                channel = %channel_name,
                                has_webhook_secret = webhook_secret.is_some(),
                                "Registering channel with router"
                            );

                            wasm_router
                                .register(Arc::clone(&channel_arc), endpoints, webhook_secret)
                                .await;
                            has_webhook_channels = true;

                            // Set up Telegram channel credentials and optionally register webhook
                            if channel_name == "telegram" {
                                if let Some(ref secrets) = secrets_store {
                                    // Inject bot token for HTTP request URL substitution
                                    // This is needed for both webhook and polling modes
                                    match inject_telegram_credentials(
                                        &channel_arc,
                                        secrets.as_ref(),
                                    )
                                    .await
                                    {
                                        Ok(()) => {
                                            tracing::debug!("Telegram bot token injected");
                                        }
                                        Err(e) => {
                                            tracing::error!(
                                                "Failed to inject Telegram credentials: {}",
                                                e
                                            );
                                            tracing::warn!(
                                                "Telegram channel may not be able to send responses"
                                            );
                                        }
                                    }

                                    // Register webhook if tunnel URL is configured
                                    // Use the SAME webhook_secret that the router expects (from secrets store)
                                    if let Some(ref tunnel_url) = config.tunnel.public_url {
                                        match register_telegram_webhook(
                                            &channel_arc,
                                            tunnel_url,
                                            webhook_secret_for_telegram.as_deref(),
                                        )
                                        .await
                                        {
                                            Ok(()) => {
                                                tracing::info!(
                                                    "Telegram webhook registered at {}/webhook/telegram",
                                                    tunnel_url
                                                );
                                            }
                                            Err(e) => {
                                                tracing::error!(
                                                    "Failed to register Telegram webhook: {}",
                                                    e
                                                );
                                                tracing::warn!(
                                                    "Telegram will fall back to polling mode"
                                                );
                                            }
                                        }
                                    }
                                } else {
                                    tracing::warn!(
                                        "Telegram channel loaded but secrets store not available"
                                    );
                                    tracing::warn!(
                                        "Set SECRETS_MASTER_KEY to enable Telegram bot token injection"
                                    );
                                }
                            }

                            // Wrap in SharedWasmChannel for ChannelManager
                            // Both the router and ChannelManager share the same underlying channel
                            channels.add(Box::new(SharedWasmChannel::new(channel_arc)));
                        }

                        // Start WASM channel webhook server if we have channels with webhooks
                        if has_webhook_channels && config.tunnel.public_url.is_some() {
                            let server = WasmChannelServer::new(wasm_router);
                            let addr = std::net::SocketAddr::from(([0, 0, 0, 0], 8080));
                            match server.start(addr).await {
                                Ok(_handle) => {
                                    tracing::info!(
                                        "WASM channel webhook server started on {}",
                                        addr
                                    );
                                }
                                Err(e) => {
                                    tracing::error!(
                                        "Failed to start WASM channel webhook server: {}",
                                        e
                                    );
                                }
                            }
                        }

                        for (path, err) in &results.errors {
                            tracing::warn!(
                                "Failed to load WASM channel {}: {}",
                                path.display(),
                                err
                            );
                        }
                    }
                    Err(e) => {
                        tracing::warn!("Failed to scan WASM channels directory: {}", e);
                    }
                }
            }
            Err(e) => {
                tracing::warn!("Failed to initialize WASM channel runtime: {}", e);
            }
        }
    }

    // Create workspace for agent (shared with memory tools)
    let workspace = store.as_ref().map(|s| {
        let mut ws = Workspace::new("default", s.pool());
        if let Some(ref emb) = embeddings {
            ws = ws.with_embeddings(emb.clone());
        }
        Arc::new(ws)
    });

    // Backfill embeddings if we just enabled the provider
    if let (Some(ws), Some(_)) = (&workspace, &embeddings) {
        match ws.backfill_embeddings().await {
            Ok(count) if count > 0 => {
                tracing::info!("Backfilled embeddings for {} chunks", count);
            }
            Ok(_) => {}
            Err(e) => {
                tracing::warn!("Failed to backfill embeddings: {}", e);
            }
        }
    }

    // Create and run the agent
    let deps = AgentDeps {
        store,
        llm,
        safety,
        tools,
        workspace,
    };
    let agent = Agent::new(
        config.agent.clone(),
        deps,
        channels,
        Some(config.heartbeat.clone()),
    );

    tracing::info!("Agent initialized, starting main loop...");

    // Run the agent (blocks until shutdown)
    agent.run().await?;

    tracing::info!("Agent shutdown complete");
    Ok(())
}

/// Inject Telegram bot token into the channel's credentials.
///
/// This allows the WASM channel to use `{TELEGRAM_BOT_TOKEN}` in HTTP URLs
/// without ever seeing the actual token value. Required for both webhook
/// and polling modes to send responses.
async fn inject_telegram_credentials(
    channel: &Arc<near_agent::channels::wasm::WasmChannel>,
    secrets: &dyn SecretsStore,
) -> anyhow::Result<()> {
    tracing::info!("Injecting Telegram bot token into channel credentials");

    // Get bot token from secrets
    let decrypted = secrets
        .get_decrypted("default", "telegram_bot_token")
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "Failed to get telegram_bot_token from secrets");
            anyhow::anyhow!("Failed to get Telegram bot token: {}", e)
        })?;

    let bot_token = decrypted.expose();
    let token_len = bot_token.len();

    // Inject the token into the channel's credentials for URL substitution
    channel
        .set_credential("TELEGRAM_BOT_TOKEN", bot_token.to_string())
        .await;

    // Verify injection
    let creds = channel.get_credentials().await;
    tracing::info!(
        token_length = token_len,
        has_token = creds.contains_key("TELEGRAM_BOT_TOKEN"),
        credential_count = creds.len(),
        "Telegram bot token injected successfully"
    );

    Ok(())
}

/// Register Telegram webhook for instant message delivery.
///
/// Calls the Telegram setWebhook API. Assumes credentials have already been
/// injected via `inject_telegram_credentials` (gets token from channel).
async fn register_telegram_webhook(
    channel: &Arc<near_agent::channels::wasm::WasmChannel>,
    tunnel_url: &str,
    webhook_secret: Option<&str>,
) -> anyhow::Result<()> {
    // Get the bot token via the public getter
    let credentials = channel.get_credentials().await;
    let bot_token = credentials.get("TELEGRAM_BOT_TOKEN").ok_or_else(|| {
        anyhow::anyhow!("Bot token not injected - call inject_telegram_credentials first")
    })?;

    // Register the webhook with Telegram API
    channel
        .register_telegram_webhook(tunnel_url, bot_token, webhook_secret)
        .await
        .map_err(|e| anyhow::anyhow!("Webhook registration failed: {}", e))?;

    Ok(())
}
