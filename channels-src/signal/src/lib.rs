//! Native Signal Protocol channel for IronClaw.
//!
//! This WASM component implements the channel interface for sending and
//! receiving Signal messages using the Signal Protocol directly — no
//! external daemon (signal-cli) required.
//!
//! # Architecture
//!
//! The WASM host manages:
//! - WebSocket connection to `wss://chat.signal.org/v1/websocket/`
//! - HTTP requests to Signal REST API (pre-keys, messages, profiles)
//! - Secrets storage (identity keys, credentials)
//! - Durable workspace (session state, pre-keys)
//!
//! This module handles:
//! - Signal Protocol encrypt/decrypt (via vendored libsignal-protocol)
//! - Protobuf encode/decode for Signal wire format
//! - Device linking provisioning flow
//! - Message routing and access control

wit_bindgen::generate!({
    world: "sandboxed-channel",
    path: "../../wit/channel.wit",
});

mod access_control;
mod config;
mod proto;
mod signal_service;
mod store;

use exports::near::agent::channel::{
    AgentResponse, ChannelConfig, Guest, IncomingHttpRequest,
    OutgoingHttpResponse, PollConfig, StatusUpdate,
};
use near::agent::channel_host;

use config::SignalConfig;

const CHANNEL_NAME: &str = "signal";

// Workspace paths for persistent state
const CONFIG_PATH: &str = "state/config";
const CREDENTIALS_PATH: &str = "state/credentials";

struct SignalChannel;

impl Guest for SignalChannel {
    fn on_start(config_json: String) -> Result<ChannelConfig, String> {
        channel_host::log(
            channel_host::LogLevel::Info,
            "Signal channel starting (native protocol)",
        );

        let config: SignalConfig = serde_json::from_str(&config_json)
            .map_err(|e| format!("Failed to parse config: {e}"))?;

        // Persist config for use in subsequent callbacks
        let config_str = serde_json::to_string(&config)
            .map_err(|e| format!("Failed to serialize config: {e}"))?;
        let _ = channel_host::workspace_write(CONFIG_PATH, &config_str);

        // Check if credentials exist
        let has_credentials = channel_host::secret_exists("signal_credentials");
        if !has_credentials {
            channel_host::log(
                channel_host::LogLevel::Warn,
                "No Signal credentials found. Run: ironclaw channel setup signal",
            );
        } else {
            channel_host::log(
                channel_host::LogLevel::Info,
                "Signal credentials found, channel ready",
            );
        }

        // Enable polling for WebSocket frame processing
        // The host manages the WebSocket connection; we process queued frames in on_poll
        Ok(ChannelConfig {
            display_name: "Signal".to_string(),
            http_endpoints: vec![],
            poll: Some(PollConfig {
                interval_ms: 5000,
                enabled: true,
            }),
        })
    }

    fn on_http_request(_req: IncomingHttpRequest) -> OutgoingHttpResponse {
        // Signal doesn't use webhooks — all communication is via WebSocket
        OutgoingHttpResponse {
            status: 404,
            headers_json: "{}".to_string(),
            body: b"Not found".to_vec(),
        }
    }

    fn on_poll() {
        // Read queued WebSocket frames from workspace
        let queue = match channel_host::workspace_read("state/gateway_event_queue_processing") {
            Some(data) if !data.is_empty() => data,
            _ => return,
        };

        // Parse the queue as JSON array of frames
        let frames: Vec<serde_json::Value> = match serde_json::from_str(&queue) {
            Ok(f) => f,
            Err(e) => {
                channel_host::log(
                    channel_host::LogLevel::Error,
                    &format!("Failed to parse WebSocket frame queue: {e}"),
                );
                return;
            }
        };

        if frames.is_empty() {
            return;
        }

        channel_host::log(
            channel_host::LogLevel::Debug,
            &format!("Processing {} WebSocket frames", frames.len()),
        );

        // Load config for access control
        let config = load_config();

        for frame in &frames {
            let frame_type = frame
                .get("type")
                .and_then(|t| t.as_str())
                .unwrap_or("text");

            match frame_type {
                "binary" => {
                    if let Some(data_b64) = frame.get("data").and_then(|d| d.as_str()) {
                        process_binary_frame(data_b64, &config);
                    }
                }
                "text" => {
                    if let Some(data) = frame.get("data").and_then(|d| d.as_str()) {
                        channel_host::log(
                            channel_host::LogLevel::Debug,
                            &format!("Ignoring text WebSocket frame ({} bytes)", data.len()),
                        );
                    }
                }
                _ => {
                    channel_host::log(
                        channel_host::LogLevel::Debug,
                        &format!("Unknown frame type: {frame_type}"),
                    );
                }
            }
        }

        // Send keepalive if needed
        send_keepalive_if_needed();
    }

    fn on_respond(response: AgentResponse) -> Result<(), String> {
        channel_host::log(
            channel_host::LogLevel::Debug,
            &format!(
                "Signal on_respond: message_id={}, content_len={}",
                response.message_id,
                response.content.len()
            ),
        );

        // TODO: Parse metadata for recipient, encrypt with Signal Protocol, send via HTTP
        // For now, log that we received the response
        channel_host::log(
            channel_host::LogLevel::Warn,
            "Signal message sending not yet implemented",
        );

        Ok(())
    }

    fn on_broadcast(user_id: String, response: AgentResponse) -> Result<(), String> {
        channel_host::log(
            channel_host::LogLevel::Debug,
            &format!(
                "Signal on_broadcast: user_id={}, content_len={}",
                user_id,
                response.content.len()
            ),
        );

        // TODO: Encrypt and send to user_id
        channel_host::log(
            channel_host::LogLevel::Warn,
            "Signal broadcast not yet implemented",
        );

        Ok(())
    }

    fn on_status(update: StatusUpdate) {
        // TODO: Send typing indicators, approval prompts, etc.
        let _ = update;
    }

    fn on_shutdown() {
        channel_host::log(channel_host::LogLevel::Info, "Signal channel shutting down");
    }
}

export!(SignalChannel);

// ============================================================================
// Internal helpers
// ============================================================================

fn load_config() -> Option<SignalConfig> {
    channel_host::workspace_read(CONFIG_PATH)
        .and_then(|s| serde_json::from_str(&s).ok())
}

fn process_binary_frame(data_b64: &str, config: &Option<SignalConfig>) {
    use base64::Engine;
    use prost::Message;

    let bytes = match base64::engine::general_purpose::STANDARD.decode(data_b64) {
        Ok(b) => b,
        Err(e) => {
            channel_host::log(
                channel_host::LogLevel::Error,
                &format!("Failed to decode binary frame: {e}"),
            );
            return;
        }
    };

    // Parse as Signal WebSocket message (prost-generated protobuf)
    match proto::WebSocketMessage::decode(bytes.as_slice()) {
        Ok(ws_msg) => handle_websocket_message(ws_msg, config),
        Err(e) => {
            channel_host::log(
                channel_host::LogLevel::Error,
                &format!("Failed to decode WebSocket protobuf: {e}"),
            );
        }
    }
}

fn handle_websocket_message(
    ws_msg: proto::WebSocketMessage,
    config: &Option<SignalConfig>,
) {
    if ws_msg.is_request() {
        if let Some(ref req) = ws_msg.request {
            handle_server_request(req, config);
        }
    } else if ws_msg.is_response() {
        channel_host::log(
            channel_host::LogLevel::Debug,
            &format!(
                "WebSocket response: status={}",
                ws_msg
                    .response
                    .as_ref()
                    .and_then(|r| r.status)
                    .unwrap_or(0)
            ),
        );
    }
}

fn handle_server_request(
    req: &proto::WebSocketRequestMessage,
    config: &Option<SignalConfig>,
) {
    let verb = req.verb.as_deref().unwrap_or("");
    let path = req.path.as_deref().unwrap_or("");

    channel_host::log(
        channel_host::LogLevel::Debug,
        &format!("Server request: {verb} {path}"),
    );

    // Acknowledge the request
    if let Some(id) = req.id {
        use prost::Message;
        let ack = proto::WebSocketMessage::ack(id);
        let encoded = ack.encode_to_vec();
        if let Err(e) = channel_host::websocket_send(&encoded) {
            channel_host::log(
                channel_host::LogLevel::Warn,
                &format!("Failed to send WebSocket ack: {e}"),
            );
        }
    }

    // Handle message delivery
    if req.is_signal_service_envelope() {
        if let Some(ref body) = req.body {
            handle_incoming_envelope(body, config);
        }
    }

    // Handle queue empty notification
    if req.is_queue_empty() {
        channel_host::log(
            channel_host::LogLevel::Debug,
            "Offline message queue drained",
        );
    }
}

fn handle_incoming_envelope(
    envelope_bytes: &[u8],
    config: &Option<SignalConfig>,
) {
    use prost::Message;

    // Decode the Envelope protobuf
    let envelope = match proto::Envelope::decode(envelope_bytes) {
        Ok(e) => e,
        Err(e) => {
            channel_host::log(
                channel_host::LogLevel::Error,
                &format!("Failed to decode Signal envelope: {e}"),
            );
            return;
        }
    };

    // Skip stories if configured
    if let Some(ref cfg) = config {
        if cfg.ignore_stories && envelope.is_story() {
            channel_host::log(channel_host::LogLevel::Debug, "Dropping story message");
            return;
        }
    }

    // Skip receipts (server delivery receipts)
    if envelope.is_receipt() {
        channel_host::log(channel_host::LogLevel::Debug, "Received server delivery receipt");
        return;
    }

    let source_service_id = envelope.source_service_id.as_deref().unwrap_or("unknown");
    let source_device = envelope.source_device.unwrap_or(0);
    let timestamp = envelope.timestamp.unwrap_or(0);

    channel_host::log(
        channel_host::LogLevel::Info,
        &format!(
            "Received envelope: type={:?}, from={}.{}, ts={}",
            envelope.r#type(),
            source_service_id,
            source_device,
            timestamp,
        ),
    );

    // Decrypt the envelope content
    // The encrypted content is in envelope.content
    let content_bytes = match envelope.content {
        Some(ref bytes) if !bytes.is_empty() => bytes.clone(),
        _ => {
            channel_host::log(
                channel_host::LogLevel::Debug,
                "Envelope has no content, skipping",
            );
            return;
        }
    };

    // Load or create the signal store
    let store = match load_signal_store() {
        Some(s) => s,
        None => {
            channel_host::log(
                channel_host::LogLevel::Error,
                "No Signal credentials configured, cannot decrypt",
            );
            return;
        }
    };

    // Determine message type and decrypt
    let decrypted = decrypt_envelope_content(
        &envelope,
        &content_bytes,
        store,
    );

    match decrypted {
        Ok(plaintext) => {
            process_decrypted_content(&plaintext, source_service_id, timestamp, config);
        }
        Err(e) => {
            channel_host::log(
                channel_host::LogLevel::Error,
                &format!("Failed to decrypt envelope: {e}"),
            );
        }
    }
}

fn load_signal_store() -> Option<store::WorkspaceSignalStore> {
    // Load identity key pair from workspace
    let identity_bytes = {
        use base64::Engine;
        let b64 = channel_host::workspace_read("state/identity_key_pair")?;
        base64::engine::general_purpose::STANDARD.decode(b64).ok()?
    };

    let identity_key_pair = libsignal_protocol::IdentityKeyPair::try_from(identity_bytes.as_slice()).ok()?;

    let registration_id = channel_host::workspace_read("state/registration_id")
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(0);

    Some(store::WorkspaceSignalStore::new(identity_key_pair, registration_id))
}

fn decrypt_envelope_content(
    envelope: &proto::Envelope,
    content_bytes: &[u8],
    mut store: store::WorkspaceSignalStore,
) -> Result<Vec<u8>, String> {
    use libsignal_protocol::{
        CiphertextMessage, CiphertextMessageType, PreKeySignalMessage, ProtocolAddress,
        SignalMessage,
    };

    let source = envelope.source_service_id.as_deref().unwrap_or("unknown");
    let device_id = envelope.source_device.unwrap_or(1);
    let remote_address = ProtocolAddress::new(source.to_string(), device_id.into());

    let mut rng = rand::thread_rng();

    // Determine the ciphertext type from the envelope type
    let envelope_type = envelope.r#type();

    // Use futures executor to run async decryption synchronously
    // (WASM callbacks are synchronous)
    let result = match envelope_type {
        proto::signalservice::envelope::Type::PrekeyBundle => {
            let prekey_msg = PreKeySignalMessage::try_from(content_bytes)
                .map_err(|e| format!("Invalid PreKey message: {e}"))?;
            // Clone store for each &mut parameter (store is stateless — all state in workspace)
            let mut session_store = store.clone();
            let mut identity_store = store.clone();
            let mut prekey_store = store.clone();
            let signed_prekey_store = store.clone();
            let mut kyber_store = store;
            futures_lite_block_on(libsignal_protocol::message_decrypt_prekey(
                &prekey_msg,
                &remote_address,
                &mut session_store,
                &mut identity_store,
                &mut prekey_store,
                &signed_prekey_store,
                &mut kyber_store,
                &mut rng,
            ))
        }
        proto::signalservice::envelope::Type::Ciphertext => {
            let signal_msg = SignalMessage::try_from(content_bytes)
                .map_err(|e| format!("Invalid Signal message: {e}"))?;
            let mut session_store = store.clone();
            let mut identity_store = store;
            futures_lite_block_on(libsignal_protocol::message_decrypt_signal(
                &signal_msg,
                &remote_address,
                &mut session_store,
                &mut identity_store,
                &mut rng,
            ))
        }
        other => {
            return Err(format!("Unsupported envelope type: {other:?}"));
        }
    };

    result.map_err(|e| format!("Decryption failed: {e}"))
}

/// Block on a future synchronously.
///
/// WASM channel callbacks are synchronous (called from spawn_blocking),
/// but libsignal-protocol's decrypt functions are async. Since the async
/// operations in our store are actually synchronous (workspace_read/write),
/// we can safely block on them.
fn futures_lite_block_on<F: std::future::Future<Output = T>, T>(f: F) -> T {
    // For WASM: the futures are trivially ready since our store operations
    // are synchronous underneath the async trait. We use a simple poll loop.
    use std::future::Future;
    use std::pin::Pin;
    use std::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};

    // Create a no-op waker (we never actually need to wake)
    fn noop_raw_waker() -> RawWaker {
        fn no_op(_: *const ()) {}
        fn clone(_: *const ()) -> RawWaker { noop_raw_waker() }
        RawWaker::new(
            std::ptr::null(),
            &RawWakerVTable::new(clone, no_op, no_op, no_op),
        )
    }
    let waker = unsafe { Waker::from_raw(noop_raw_waker()) };
    let mut cx = Context::from_waker(&waker);

    let mut f = Pin::from(Box::new(f));
    loop {
        match f.as_mut().poll(&mut cx) {
            Poll::Ready(val) => return val,
            Poll::Pending => {
                // This should never happen with our synchronous store
                panic!("Signal Protocol store returned Pending — store operations must be synchronous")
            }
        }
    }
}

fn process_decrypted_content(
    plaintext: &[u8],
    source_service_id: &str,
    timestamp: u64,
    config: &Option<SignalConfig>,
) {
    use prost::Message;

    // Decode the Content protobuf
    let content = match proto::Content::decode(plaintext) {
        Ok(c) => c,
        Err(e) => {
            channel_host::log(
                channel_host::LogLevel::Error,
                &format!("Failed to decode Content protobuf: {e}"),
            );
            return;
        }
    };

    // Handle DataMessage (regular text messages)
    if let Some(ref data_msg) = content.data_message {
        if let Some(ref body) = data_msg.body {
            if !body.is_empty() {
                // Apply access control
                if let Some(ref cfg) = config {
                    // Check group context
                    if let Some(ref _group) = data_msg.group_v2 {
                        // TODO: Extract group ID, check group policy
                        channel_host::log(
                            channel_host::LogLevel::Debug,
                            "Group message received (group policy not yet implemented)",
                        );
                    } else if !access_control::is_dm_allowed(cfg, source_service_id) {
                        channel_host::log(
                            channel_host::LogLevel::Debug,
                            &format!("DM from {source_service_id} not allowed by policy"),
                        );
                        return;
                    }
                }

                // Build metadata for response routing
                let metadata = serde_json::json!({
                    "signal_sender": source_service_id,
                    "signal_timestamp": timestamp,
                })
                .to_string();

                // Emit the message to the agent
                channel_host::emit_message(&channel_host::EmittedMessage {
                    user_id: source_service_id.to_string(),
                    user_name: None, // TODO: look up profile name
                    content: body.clone(),
                    thread_id: Some(source_service_id.to_string()),
                    metadata_json: metadata,
                    attachments: vec![],
                });

                channel_host::log(
                    channel_host::LogLevel::Info,
                    &format!(
                        "Emitted message from {}: {} chars",
                        source_service_id,
                        body.len()
                    ),
                );
            }
        }
    }

    // Handle TypingMessage
    if content.typing_message.is_some() {
        channel_host::log(
            channel_host::LogLevel::Debug,
            &format!("Typing indicator from {source_service_id}"),
        );
    }

    // Handle ReceiptMessage
    if content.receipt_message.is_some() {
        channel_host::log(
            channel_host::LogLevel::Debug,
            &format!("Receipt from {source_service_id}"),
        );
    }
}

fn send_keepalive_if_needed() {
    use prost::Message;

    let now_ms = channel_host::now_millis();
    let last_keepalive = channel_host::workspace_read("state/last_keepalive")
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0);

    // Send keepalive every 55 seconds
    if now_ms.saturating_sub(last_keepalive) >= 55_000 {
        let keepalive = proto::WebSocketMessage::keepalive(now_ms);
        let encoded = keepalive.encode_to_vec();
        if let Err(e) = channel_host::websocket_send(&encoded) {
            channel_host::log(
                channel_host::LogLevel::Debug,
                &format!("Keepalive send failed (may not be connected): {e}"),
            );
            return;
        }

        let _ = channel_host::workspace_write(
            "state/last_keepalive",
            &now_ms.to_string(),
        );
    }
}
