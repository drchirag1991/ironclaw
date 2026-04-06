//! Channel-agnostic tool dispatch with audit trail.
//!
//! `ToolDispatcher` is the universal entry point for executing tools from
//! any caller — gateway handlers, CLI commands, routine engines, or other
//! channels. It creates a system job for FK integrity, executes the tool,
//! records an `ActionRecord`, and returns the result.
//!
//! This is a third entry point alongside:
//! - v1: `Worker::execute_tool()` (agent agentic loop)
//! - v2: `EffectBridgeAdapter::execute_action()` (engine Python orchestrator)
//!
//! All three converge on the same `ToolRegistry`.

use std::sync::Arc;
use std::time::Instant;

use tracing::debug;
use uuid::Uuid;

use crate::context::{ActionRecord, JobContext};
use crate::db::Database;
use crate::tools::registry::ToolRegistry;
use crate::tools::tool::{ToolError, ToolOutput};

/// Identifies where a tool dispatch originated.
///
/// `Channel` is intentionally a `String`, not an enum — channels are
/// extensions that can appear at runtime (gateway, CLI, telegram, slack,
/// WASM channels, future custom channels).
#[derive(Debug, Clone)]
pub enum DispatchSource {
    /// A channel-initiated operation (gateway, CLI, telegram, etc.).
    Channel(String),
    /// An agent job reusing its existing job_id for the audit trail.
    Agent { job_id: Uuid },
    /// A routine engine operation.
    Routine { routine_id: Uuid },
    /// An internal system operation.
    System,
}

impl std::fmt::Display for DispatchSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Channel(name) => write!(f, "channel:{name}"),
            Self::Agent { job_id } => write!(f, "agent:{job_id}"),
            Self::Routine { routine_id } => write!(f, "routine:{routine_id}"),
            Self::System => write!(f, "system"),
        }
    }
}

/// Channel-agnostic tool dispatcher with audit trail.
///
/// Wraps `ToolRegistry` + `Database` to provide a single dispatch function
/// that any caller can use to execute tools with proper `ActionRecord` persistence.
pub struct ToolDispatcher {
    registry: Arc<ToolRegistry>,
    store: Arc<dyn Database>,
}

impl ToolDispatcher {
    /// Create a new dispatcher.
    pub fn new(registry: Arc<ToolRegistry>, store: Arc<dyn Database>) -> Self {
        Self { registry, store }
    }

    /// Execute a tool by name with the given parameters.
    ///
    /// 1. Resolves the tool from the registry
    /// 2. Creates or reuses a job_id for FK integrity
    /// 3. Builds a minimal `JobContext`
    /// 4. Calls `Tool::execute()`
    /// 5. Persists an `ActionRecord`
    /// 6. Returns the `ToolOutput`
    ///
    /// Approval checks are skipped — channel-initiated operations are
    /// user-confirmed by definition.
    pub async fn dispatch(
        &self,
        tool_name: &str,
        params: serde_json::Value,
        user_id: &str,
        source: DispatchSource,
    ) -> Result<ToolOutput, ToolError> {
        let (resolved_name, tool) =
            self.registry.get_resolved(tool_name).await.ok_or_else(|| {
                ToolError::ExecutionFailed(format!("tool not found: {tool_name}"))
            })?;

        // Resolve or create job_id for the audit trail.
        let job_id = match &source {
            DispatchSource::Agent { job_id } => *job_id,
            other => {
                let source_label = other.to_string();
                self.store
                    .create_system_job(user_id, &source_label)
                    .await
                    .map_err(|e| {
                        ToolError::ExecutionFailed(format!("failed to create system job: {e}"))
                    })?
            }
        };

        let ctx = JobContext::system(user_id, job_id);
        let start = Instant::now();

        debug!(
            tool = %resolved_name,
            source = %source,
            user_id = %user_id,
            "dispatching tool"
        );

        let result = tool.execute(params.clone(), &ctx).await;
        let elapsed = start.elapsed();

        // Build and persist the ActionRecord (fire-and-forget).
        let action = ActionRecord::new(0, &resolved_name, params);
        let action = match &result {
            Ok(output) => {
                let raw = serde_json::to_string_pretty(&output.result).ok();
                action.succeed(raw, output.result.clone(), elapsed)
            }
            Err(e) => action.fail(e.to_string(), elapsed),
        };

        let store = Arc::clone(&self.store);
        tokio::spawn(async move {
            if let Err(e) = store.save_action(job_id, &action).await {
                debug!(error = %e, "failed to persist dispatch ActionRecord");
            }
        });

        result
    }

    /// Access the underlying tool registry.
    pub fn registry(&self) -> &Arc<ToolRegistry> {
        &self.registry
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dispatch_source_display() {
        assert_eq!(
            DispatchSource::Channel("gateway".into()).to_string(),
            "channel:gateway"
        );
        let id = Uuid::nil();
        assert_eq!(
            DispatchSource::Agent { job_id: id }.to_string(),
            format!("agent:{id}")
        );
        assert_eq!(DispatchSource::System.to_string(), "system");
    }
}
