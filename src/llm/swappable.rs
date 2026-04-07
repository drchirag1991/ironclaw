use async_trait::async_trait;
use rust_decimal::Decimal;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::llm::error::LlmError;
use crate::llm::provider::{
    CompletionRequest, CompletionResponse, LlmProvider, ModelMetadata, ToolCompletionRequest,
    ToolCompletionResponse,
};

/// A thread-safe, hot-swappable LLM provider wrapper.
///
/// This allowing the active LLM backend to be swapped at runtime (e.g., during
/// a configuration hot-reload via SIGHUP) without recreating the agent or
/// dropping its dependencies.
pub struct SwappableLlmProvider {
    inner: RwLock<Arc<dyn LlmProvider>>,
}

impl SwappableLlmProvider {
    /// Create a new swappable provider.
    pub fn new(provider: Arc<dyn LlmProvider>) -> Self {
        Self {
            inner: RwLock::new(provider),
        }
    }

    /// Swap the current provider for a new one.
    pub async fn swap(&self, new_provider: Arc<dyn LlmProvider>) {
        let mut lock = self.inner.write().await;
        *lock = new_provider;
    }

    /// Get a clone of the current inner provider.
    pub async fn get_inner(&self) -> Arc<dyn LlmProvider> {
        self.inner.read().await.clone()
    }
}

#[async_trait]
impl LlmProvider for SwappableLlmProvider {
    fn model_name(&self) -> &str {
        // Warning: this is suboptimal as we can't return &str from a locked value easily.
        // But most code uses model_name() for logging. 
        // We'll use a hack or return effective_model_name().
        "swappable-llm"
    }

    fn active_model_name(&self) -> String {
        // Can't block here easily if we want to follow the trait, but we can use try_read or just assume it's fast.
        futures::executor::block_on(async {
            self.inner.read().await.active_model_name()
        })
    }

    fn cost_per_token(&self) -> (Decimal, Decimal) {
        futures::executor::block_on(async {
            self.inner.read().await.cost_per_token()
        })
    }

    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        self.inner.read().await.complete(request).await
    }

    async fn complete_with_tools(
        &self,
        request: ToolCompletionRequest,
    ) -> Result<ToolCompletionResponse, LlmError> {
        self.inner.read().await.complete_with_tools(request).await
    }

    async fn list_models(&self) -> Result<Vec<String>, LlmError> {
        self.inner.read().await.list_models().await
    }

    async fn model_metadata(&self) -> Result<ModelMetadata, LlmError> {
        self.inner.read().await.model_metadata().await
    }

    fn effective_model_name(&self, requested_model: Option<&str>) -> String {
        futures::executor::block_on(async {
            self.inner.read().await.effective_model_name(requested_model)
        })
    }
    
    fn calculate_cost(&self, input_tokens: u32, output_tokens: u32) -> Decimal {
        futures::executor::block_on(async {
            self.inner.read().await.calculate_cost(input_tokens, output_tokens)
        })
    }
}
