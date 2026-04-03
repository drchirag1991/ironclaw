//! Signal Protocol protobuf types.
//!
//! Generated from .proto files by prost-build, plus helper methods.

// Include generated protobuf types from build.rs
pub mod signalservice {
    include!(concat!(env!("OUT_DIR"), "/signalservice.rs"));
}

// Re-export commonly used types at module level
pub use signalservice::{
    Content, DataMessage, Envelope, ReceiptMessage, SyncMessage, TypingMessage,
};
pub use signalservice::{ProvisionEnvelope, ProvisionMessage};
pub use signalservice::{
    WebSocketMessage, WebSocketRequestMessage, WebSocketResponseMessage,
};

// ============================================================================
// WebSocket message helpers
// ============================================================================

impl WebSocketRequestMessage {
    /// Check if this is an incoming message delivery.
    pub fn is_signal_service_envelope(&self) -> bool {
        self.verb.as_deref() == Some("PUT")
            && self.path.as_deref() == Some("/api/v1/message")
    }

    /// Check if the server is signaling the offline queue is empty.
    pub fn is_queue_empty(&self) -> bool {
        self.verb.as_deref() == Some("PUT")
            && self.path.as_deref() == Some("/api/v1/queue/empty")
    }
}

impl WebSocketMessage {
    /// Create an acknowledgment response for a server request.
    pub fn ack(request_id: u64) -> Self {
        Self {
            r#type: Some(
                signalservice::web_socket_message::Type::Response.into(),
            ),
            request: None,
            response: Some(WebSocketResponseMessage {
                id: Some(request_id),
                status: Some(200),
                message: Some("OK".to_string()),
                headers: vec![],
                body: None,
            }),
        }
    }

    /// Create a keepalive request.
    pub fn keepalive(request_id: u64) -> Self {
        Self {
            r#type: Some(
                signalservice::web_socket_message::Type::Request.into(),
            ),
            request: Some(WebSocketRequestMessage {
                verb: Some("GET".to_string()),
                path: Some("/v1/keepalive".to_string()),
                body: None,
                headers: vec![],
                id: Some(request_id),
            }),
            response: None,
        }
    }

    /// Check if this is a request message.
    pub fn is_request(&self) -> bool {
        self.r#type
            == Some(signalservice::web_socket_message::Type::Request.into())
    }

    /// Check if this is a response message.
    pub fn is_response(&self) -> bool {
        self.r#type
            == Some(signalservice::web_socket_message::Type::Response.into())
    }
}

// ============================================================================
// Envelope helpers
// ============================================================================

impl Envelope {
    pub fn is_prekey_signal_message(&self) -> bool {
        self.r#type() == signalservice::envelope::Type::PrekeyBundle
    }

    pub fn is_signal_message(&self) -> bool {
        self.r#type() == signalservice::envelope::Type::Ciphertext
    }

    pub fn is_unidentified_sender(&self) -> bool {
        self.r#type() == signalservice::envelope::Type::UnidentifiedSender
    }

    pub fn is_receipt(&self) -> bool {
        self.r#type() == signalservice::envelope::Type::ServerDeliveryReceipt
    }

    pub fn is_urgent(&self) -> bool {
        self.urgent.unwrap_or(true)
    }

    pub fn is_story(&self) -> bool {
        self.story.unwrap_or(false)
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use prost::Message;

    #[test]
    fn websocket_ack_roundtrip() {
        let ack = WebSocketMessage::ack(42);
        let encoded = ack.encode_to_vec();
        let decoded = WebSocketMessage::decode(encoded.as_slice()).unwrap();
        assert!(decoded.is_response());
        let resp = decoded.response.unwrap();
        assert_eq!(resp.id, Some(42));
        assert_eq!(resp.status, Some(200));
    }

    #[test]
    fn websocket_keepalive_roundtrip() {
        let ka = WebSocketMessage::keepalive(123);
        let encoded = ka.encode_to_vec();
        let decoded = WebSocketMessage::decode(encoded.as_slice()).unwrap();
        assert!(decoded.is_request());
        let req = decoded.request.unwrap();
        assert_eq!(req.verb.as_deref(), Some("GET"));
        assert_eq!(req.path.as_deref(), Some("/v1/keepalive"));
        assert_eq!(req.id, Some(123));
    }

    #[test]
    fn envelope_type_checks() {
        let mut env = Envelope::default();
        env.set_type(signalservice::envelope::Type::PrekeyBundle);
        assert!(env.is_prekey_signal_message());
        assert!(!env.is_signal_message());

        env.set_type(signalservice::envelope::Type::Ciphertext);
        assert!(env.is_signal_message());

        env.set_type(signalservice::envelope::Type::UnidentifiedSender);
        assert!(env.is_unidentified_sender());
    }
}
