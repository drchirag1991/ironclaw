//! Signal channel configuration.

use serde::{Deserialize, Serialize};

/// Configuration injected by host via `on_start(config_json)`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignalConfig {
    /// Users allowed to interact with the bot in DMs.
    /// E.164 phone numbers, UUIDs, or `*` for everyone.
    #[serde(default)]
    pub allow_from: Vec<String>,

    /// Groups allowed to interact with the bot.
    /// Group IDs or `*` for all groups.
    #[serde(default)]
    pub allow_from_groups: Vec<String>,

    /// DM policy: "open", "allowlist", or "pairing".
    #[serde(default = "default_dm_policy")]
    pub dm_policy: String,

    /// Group policy: "disabled", "allowlist", or "open".
    #[serde(default = "default_group_policy")]
    pub group_policy: String,

    /// Allow list for group message senders.
    #[serde(default)]
    pub group_allow_from: Vec<String>,

    /// Skip story messages.
    #[serde(default = "default_true")]
    pub ignore_stories: bool,
}

fn default_dm_policy() -> String {
    "pairing".to_string()
}

fn default_group_policy() -> String {
    "allowlist".to_string()
}

fn default_true() -> bool {
    true
}
