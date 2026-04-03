//! Access control for Signal messages (DM and group policies).

use crate::config::SignalConfig;
use crate::near::agent::channel_host;

const CHANNEL_NAME: &str = "signal";

/// Check whether a DM sender is allowed based on the configured policy.
///
/// Returns `true` if the message should be processed.
pub fn is_dm_allowed(config: &SignalConfig, sender: &str) -> bool {
    match config.dm_policy.as_str() {
        "open" => true,
        "pairing" => {
            if is_in_list(&config.allow_from, sender) {
                return true;
            }
            // Check pairing store
            match channel_host::pairing_is_allowed(CHANNEL_NAME, sender, None) {
                Ok(true) => true,
                Ok(false) => {
                    // Create pairing request
                    let meta = serde_json::json!({ "sender": sender }).to_string();
                    match channel_host::pairing_upsert_request(CHANNEL_NAME, sender, &meta) {
                        Ok(result) => {
                            if result.created {
                                channel_host::log(
                                    channel_host::LogLevel::Info,
                                    &format!(
                                        "Pairing request created for {sender}, code: {}",
                                        result.code
                                    ),
                                );
                                // TODO: Send pairing reply message to sender
                            }
                        }
                        Err(e) => {
                            channel_host::log(
                                channel_host::LogLevel::Error,
                                &format!("Pairing upsert failed: {e}"),
                            );
                        }
                    }
                    false
                }
                Err(_) => false,
            }
        }
        // "allowlist" or anything else defaults to allowlist
        _ => is_in_list(&config.allow_from, sender),
    }
}

/// Check whether a group message is allowed.
pub fn is_group_allowed(
    config: &SignalConfig,
    group_id: &str,
    sender: &str,
) -> bool {
    match config.group_policy.as_str() {
        "disabled" => false,
        "open" => is_in_list(&config.allow_from_groups, group_id),
        // "allowlist" or default
        _ => {
            is_in_list(&config.allow_from_groups, group_id)
                && is_group_sender_allowed(config, sender)
        }
    }
}

fn is_group_sender_allowed(config: &SignalConfig, sender: &str) -> bool {
    let list = if config.group_allow_from.is_empty() {
        &config.allow_from
    } else {
        &config.group_allow_from
    };
    is_in_list(list, sender)
}

fn is_in_list(list: &[String], value: &str) -> bool {
    if list.is_empty() {
        return false;
    }
    list.iter().any(|entry| {
        entry == "*" || normalize_entry(entry) == normalize_entry(value)
    })
}

/// Strip `uuid:` prefix for comparison.
fn normalize_entry(entry: &str) -> &str {
    entry.strip_prefix("uuid:").unwrap_or(entry)
}
