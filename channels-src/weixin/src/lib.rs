wit_bindgen::generate!({
    world: "sandboxed-channel",
    path: "../../wit/channel.wit",
});

mod api;
mod auth;
mod state;
mod types;

use exports::near::agent::channel::{
    AgentResponse, ChannelConfig, Guest, PollConfig, StatusUpdate,
};
use near::agent::channel_host::{self, EmittedMessage};
use serde_json::json;

use crate::auth::TOKEN_SECRET_NAME;
use crate::state::{
    clear_session_expired, load_config, load_context_tokens, load_get_updates_buf,
    mark_session_expired, persist_config, persist_context_tokens, persist_get_updates_buf,
    session_expired,
};
use crate::types::{
    OutboundMetadata, WeixinConfig, WeixinMessage, MESSAGE_ITEM_TEXT, MESSAGE_TYPE_USER,
};

struct WeixinChannel;

impl Guest for WeixinChannel {
    fn on_start(config_json: String) -> Result<ChannelConfig, String> {
        let config = serde_json::from_str::<WeixinConfig>(&config_json)
            .map_err(|e| format!("Failed to parse Weixin config: {e}"))?;
        persist_config(&config)?;
        clear_session_expired();

        Ok(ChannelConfig {
            display_name: "Weixin".to_string(),
            http_endpoints: Vec::new(),
            poll: Some(PollConfig {
                interval_ms: config.poll_interval_ms.max(30_000),
                enabled: true,
            }),
        })
    }

    fn on_http_request(
        _req: exports::near::agent::channel::IncomingHttpRequest,
    ) -> exports::near::agent::channel::OutgoingHttpResponse {
        exports::near::agent::channel::OutgoingHttpResponse {
            status: 404,
            headers_json: "{}".to_string(),
            body: b"{\"error\":\"weixin channel does not expose webhooks\"}".to_vec(),
        }
    }

    fn on_poll() {
        if session_expired() {
            channel_host::log(
                channel_host::LogLevel::Warn,
                "Weixin session is marked expired; reconnect the channel to resume polling",
            );
            return;
        }

        if !channel_host::secret_exists(TOKEN_SECRET_NAME) {
            channel_host::log(
                channel_host::LogLevel::Warn,
                "Weixin bot token is missing; skipping poll",
            );
            return;
        }

        let config = load_config();
        let cursor = load_get_updates_buf();
        let mut context_tokens = load_context_tokens();

        match api::get_updates(&config, &cursor) {
            Ok(response) => {
                if response.errcode == Some(-14) {
                    mark_session_expired();
                    channel_host::log(
                        channel_host::LogLevel::Error,
                        "Weixin session expired; reconnect the channel",
                    );
                    return;
                }

                if response.ret.unwrap_or(0) != 0 {
                    let errmsg = response
                        .errmsg
                        .as_deref()
                        .unwrap_or("unknown Weixin polling error");
                    channel_host::log(
                        channel_host::LogLevel::Warn,
                        &format!(
                            "Weixin getUpdates returned ret={} errmsg={errmsg}",
                            response.ret.unwrap_or(-1)
                        ),
                    );
                }

                if let Some(next_cursor) = response.get_updates_buf.as_deref() {
                    if next_cursor != cursor {
                        if let Err(error) = persist_get_updates_buf(next_cursor) {
                            channel_host::log(
                                channel_host::LogLevel::Warn,
                                &format!("Failed to persist Weixin polling cursor: {error}"),
                            );
                        }
                    }
                }

                let mut context_tokens_changed = false;
                for message in response.msgs {
                    if let Some(from_user_id) = message.from_user_id.as_deref() {
                        if let Some(context_token) = message.context_token.as_deref() {
                            let changed = context_tokens
                                .insert(from_user_id.to_string(), context_token.to_string())
                                .as_deref()
                                != Some(context_token);
                            context_tokens_changed |= changed;
                        }
                    }
                    emit_incoming_message(message);
                }

                if context_tokens_changed {
                    if let Err(error) = persist_context_tokens(&context_tokens) {
                        channel_host::log(
                            channel_host::LogLevel::Warn,
                            &format!("Failed to persist Weixin context tokens: {error}"),
                        );
                    }
                }
            }
            Err(error) => {
                channel_host::log(
                    channel_host::LogLevel::Error,
                    &format!("Weixin polling failed: {error}"),
                );
            }
        }
    }

    fn on_respond(response: AgentResponse) -> Result<(), String> {
        let metadata = serde_json::from_str::<OutboundMetadata>(&response.metadata_json)
            .map_err(|e| format!("Invalid Weixin response metadata: {e}"))?;
        let config = load_config();
        let context_tokens = load_context_tokens();
        let context_token = metadata
            .context_token
            .clone()
            .or_else(|| context_tokens.get(&metadata.from_user_id).cloned());

        api::send_text_message(
            &config,
            &metadata.from_user_id,
            response.content.trim(),
            context_token.as_deref(),
        )
    }

    fn on_status(_update: StatusUpdate) {}

    fn on_broadcast(_user_id: String, _response: AgentResponse) -> Result<(), String> {
        Ok(())
    }

    fn on_shutdown() {}
}

fn emit_incoming_message(message: WeixinMessage) {
    if message.message_type != Some(MESSAGE_TYPE_USER) {
        return;
    }

    let Some(from_user_id) = message.from_user_id.as_deref() else {
        return;
    };

    let text = extract_text(&message);
    if text.trim().is_empty() {
        return;
    }

    let metadata = json!({
        "from_user_id": from_user_id,
        "to_user_id": message.to_user_id,
        "message_id": message.message_id,
        "session_id": message.session_id,
        "context_token": message.context_token,
    });

    channel_host::emit_message(&EmittedMessage {
        user_id: from_user_id.to_string(),
        user_name: None,
        content: text,
        thread_id: Some(format!("weixin:{from_user_id}")),
        metadata_json: metadata.to_string(),
        attachments: Vec::new(),
    });
}

fn extract_text(message: &WeixinMessage) -> String {
    message
        .item_list
        .iter()
        .find_map(|item| {
            if item.r#type == Some(MESSAGE_ITEM_TEXT) {
                item.text_item.as_ref().map(|item| item.text.clone())
            } else {
                None
            }
        })
        .unwrap_or_default()
}

export!(WeixinChannel);
