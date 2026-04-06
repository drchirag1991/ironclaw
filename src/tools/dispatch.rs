//! Channel-agnostic tool dispatch with audit trail.
//!
//! `ToolDispatcher` is the universal entry point for executing tools from
//! any non-agent caller — gateway handlers, CLI commands, routine engines,
//! or other channels. It creates a fresh system job for FK integrity,
//! executes the tool, records an `ActionRecord`, and returns the result.
//!
//! This is a third entry point alongside:
//! - v1: `Worker::execute_tool()` (agent agentic loop — has its own sequence tracking)
//! - v2: `EffectBridgeAdapter::execute_action()` (engine Python orchestrator)
//!
//! All three converge on the same `ToolRegistry`. Agent-initiated tool calls
//! must go through the agent's worker (which manages action sequence numbers
//! atomically); the dispatcher is only for callers that don't have an
//! existing agent job context.

use std::sync::Arc;
use std::time::Instant;

use tracing::{debug, warn};
use uuid::Uuid;

use crate::context::{ActionRecord, JobContext};
use crate::db::Database;
use crate::tools::registry::ToolRegistry;
use crate::tools::tool::{ToolError, ToolOutput};

/// Identifies where a tool dispatch originated.
///
/// `Channel` is intentionally a `String`, not an enum — channels are
/// extensions that can appear at runtime (gateway, CLI, telegram, slack,
/// WASM channels, future custom channels). Each dispatch creates a fresh
/// system job for audit trail purposes; agent-initiated tool calls must
/// use `Worker::execute_tool()` instead, which manages sequence numbers
/// against the agent's existing job.
#[derive(Debug, Clone)]
pub enum DispatchSource {
    /// A channel-initiated operation (gateway, CLI, telegram, etc.).
    Channel(String),
    /// A routine engine operation.
    Routine { routine_id: Uuid },
    /// An internal system operation.
    System,
}

impl std::fmt::Display for DispatchSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Channel(name) => write!(f, "channel:{name}"),
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

        // Always create a fresh system job for audit trail. Each dispatch
        // becomes its own group of actions — sequence_num starts at 0 with
        // no risk of UNIQUE(job_id, sequence_num) collision.
        let source_label = source.to_string();
        let job_id = self
            .store
            .create_system_job(user_id, &source_label)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("failed to create system job: {e}")))?;

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

        // Build and persist the ActionRecord. Awaited (not spawned) so that
        // short-lived callers (CLI commands) cannot terminate before the
        // audit row is written. Persistence failures are logged but do not
        // mask the tool result, which is the more important signal.
        let action = ActionRecord::new(0, &resolved_name, params);
        let action = match &result {
            Ok(output) => {
                let raw = serde_json::to_string_pretty(&output.result).ok();
                action.succeed(raw, output.result.clone(), elapsed)
            }
            Err(e) => action.fail(e.to_string(), elapsed),
        };
        if let Err(e) = self.store.save_action(job_id, &action).await {
            warn!(
                error = %e,
                tool = %resolved_name,
                job_id = %job_id,
                "failed to persist dispatch ActionRecord"
            );
        }

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
            DispatchSource::Routine { routine_id: id }.to_string(),
            format!("routine:{id}")
        );
        assert_eq!(DispatchSource::System.to_string(), "system");
    }
}
