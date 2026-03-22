//! Engine v2 router — handles user messages via the engine when enabled.

use std::sync::Arc;

use tracing::{debug, info};

use ironclaw_engine::{
    Capability, CapabilityRegistry, LeaseManager, PolicyEngine, Project, Store,
    ThreadConfig, ThreadManager, ThreadOutcome, ThreadType,
};

use crate::agent::Agent;
use crate::bridge::effect_adapter::EffectBridgeAdapter;
use crate::bridge::llm_adapter::LlmBridgeAdapter;
use crate::bridge::store_adapter::InMemoryStore;
use crate::channels::IncomingMessage;
use crate::error::Error;

/// Check if the engine v2 is enabled via `ENGINE_V2=true` environment variable.
pub fn is_engine_v2_enabled() -> bool {
    std::env::var("ENGINE_V2")
        .map(|v| v == "true" || v == "1")
        .unwrap_or(false)
}

/// Handle a user message through the engine v2 pipeline.
///
/// This is the engine-based equivalent of `Agent::process_user_input()`.
/// It builds bridge adapters from the agent's existing dependencies,
/// creates an engine `ThreadManager`, spawns a thread, and waits for
/// the result.
pub async fn handle_with_engine(
    agent: &Agent,
    message: &IncomingMessage,
    content: &str,
) -> Result<Option<String>, Error> {
    info!(
        user_id = %message.user_id,
        channel = %message.channel,
        "engine v2: handling message"
    );

    // Build bridge adapters from agent's existing dependencies
    let llm_adapter = Arc::new(LlmBridgeAdapter::new(
        agent.llm().clone(),
        Some(agent.cheap_llm().clone()),
    ));

    let effect_adapter = Arc::new(EffectBridgeAdapter::new(
        agent.tools().clone(),
        agent.safety().clone(),
    ));

    let store = Arc::new(InMemoryStore::new());

    // Build capability registry from available tools
    let mut capabilities = CapabilityRegistry::new();
    let tool_defs = agent.tools().tool_definitions().await;
    if !tool_defs.is_empty() {
        capabilities.register(Capability {
            name: "tools".into(),
            description: "Available tools".into(),
            actions: tool_defs
                .into_iter()
                .map(|td| ironclaw_engine::ActionDef {
                    name: td.name,
                    description: td.description,
                    parameters_schema: td.parameters,
                    effects: vec![],
                    requires_approval: false,
                })
                .collect(),
            knowledge: vec![],
            policies: vec![],
        });
    }

    let leases = Arc::new(LeaseManager::new());
    let policy = Arc::new(PolicyEngine::new());

    // Create thread manager
    let thread_manager = Arc::new(ThreadManager::new(
        llm_adapter,
        effect_adapter,
        store.clone(),
        Arc::new(capabilities),
        leases,
        policy,
    ));

    // Create a default project for this session
    let project = Project::new(
        format!("{}:{}", message.channel, message.user_id),
        "Auto-created project",
    );
    let project_id = project.id;
    store.save_project(&project).await.map_err(|e| {
        crate::error::Error::from(crate::error::JobError::ContextError {
            id: uuid::Uuid::nil(),
            reason: format!("engine v2 store error: {e}"),
        })
    })?;

    // Spawn a thread for this message
    let config = ThreadConfig::default();
    let thread_id = thread_manager
        .spawn_thread(
            content,
            ThreadType::Foreground,
            project_id,
            config,
            None,
            &message.user_id,
        )
        .await
        .map_err(|e| {
            crate::error::Error::from(crate::error::JobError::ContextError {
                id: uuid::Uuid::nil(),
                reason: format!("engine v2 spawn error: {e}"),
            })
        })?;

    debug!(thread_id = %thread_id, "engine v2: thread spawned, waiting for completion");

    // Wait for the thread to complete
    let outcome = thread_manager.join_thread(thread_id).await.map_err(|e| {
        crate::error::Error::from(crate::error::JobError::ContextError {
            id: uuid::Uuid::nil(),
            reason: format!("engine v2 join error: {e}"),
        })
    })?;

    // Convert outcome to response
    match outcome {
        ThreadOutcome::Completed { response } => {
            debug!(thread_id = %thread_id, "engine v2: thread completed");
            Ok(response)
        }
        ThreadOutcome::Stopped => {
            debug!(thread_id = %thread_id, "engine v2: thread stopped");
            Ok(Some("Thread was stopped.".into()))
        }
        ThreadOutcome::MaxIterations => {
            debug!(thread_id = %thread_id, "engine v2: max iterations");
            Ok(Some("Reached maximum iterations without completing.".into()))
        }
        ThreadOutcome::Failed { error } => {
            debug!(thread_id = %thread_id, error = %error, "engine v2: thread failed");
            Ok(Some(format!("Error: {error}")))
        }
        ThreadOutcome::NeedApproval {
            action_name,
            call_id: _,
            parameters: _,
        } => {
            // Phase 6: approval flow not yet wired — return as message
            debug!(thread_id = %thread_id, action = %action_name, "engine v2: approval needed (not yet supported)");
            Ok(Some(format!(
                "Action '{action_name}' requires approval (engine v2 approval flow not yet implemented)"
            )))
        }
    }
}
