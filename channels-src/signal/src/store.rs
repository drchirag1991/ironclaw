//! Signal Protocol storage backed by WASM workspace.
//!
//! Implements libsignal-protocol's storage traits (IdentityKeyStore,
//! PreKeyStore, SignedPreKeyStore, SessionStore, SenderKeyStore)
//! over the WASM host's workspace_read/workspace_write functions.
//!
//! All values are serialized as base64-encoded bytes.

use async_trait::async_trait;
use base64::Engine;
use libsignal_protocol::{
    GenericSignedPreKey, IdentityKey, IdentityKeyPair, KyberPreKeyId, KyberPreKeyRecord,
    PreKeyId, PreKeyRecord, ProtocolAddress, SenderKeyRecord, SessionRecord,
    SignalProtocolError, SignedPreKeyId, SignedPreKeyRecord,
};
use uuid::Uuid;

type Result<T> = std::result::Result<T, SignalProtocolError>;

use crate::near::agent::channel_host;

const B64: base64::engine::GeneralPurpose = base64::engine::general_purpose::STANDARD;

/// Signal Protocol store backed by WASM workspace storage.
///
/// Reads/writes use the host's `workspace_read`/`workspace_write` functions,
/// which are automatically prefixed with `channels/signal/`.
///
/// Clone is cheap — the actual state lives in workspace storage, not in memory.
/// Cloning is needed because libsignal-protocol takes `&mut self` for multiple
/// store traits simultaneously, but our I/O is stateless (host function calls).
#[derive(Clone)]
pub struct WorkspaceSignalStore {
    identity_key_pair: IdentityKeyPair,
    registration_id: u32,
}

impl WorkspaceSignalStore {
    /// Create a new store with the given identity key pair and registration ID.
    pub fn new(identity_key_pair: IdentityKeyPair, registration_id: u32) -> Self {
        Self {
            identity_key_pair,
            registration_id,
        }
    }
}

// Helper functions for workspace I/O
fn ws_read(path: &str) -> Option<Vec<u8>> {
    channel_host::workspace_read(path)
        .and_then(|s| B64.decode(s).ok())
}

fn ws_write(path: &str, data: &[u8]) {
    let encoded = B64.encode(data);
    let _ = channel_host::workspace_write(path, &encoded);
}

fn ws_delete(path: &str) {
    // Write empty string to "delete" (workspace doesn't have a delete op)
    let _ = channel_host::workspace_write(path, "");
}

// ============================================================================
// IdentityKeyStore
// ============================================================================

#[async_trait(?Send)]
impl libsignal_protocol::IdentityKeyStore for WorkspaceSignalStore {
    async fn get_identity_key_pair(&self) -> Result<IdentityKeyPair> {
        Ok(self.identity_key_pair)
    }

    async fn get_local_registration_id(&self) -> Result<u32> {
        Ok(self.registration_id)
    }

    async fn save_identity(
        &mut self,
        address: &ProtocolAddress,
        identity: &IdentityKey,
    ) -> Result<bool> {
        let path = format!("state/identities/{}", address);
        let existing = ws_read(&path)
            .and_then(|bytes| IdentityKey::decode(&bytes).ok());

        ws_write(&path, &identity.serialize());

        Ok(existing.is_some_and(|old| old != *identity))
    }

    async fn is_trusted_identity(
        &self,
        address: &ProtocolAddress,
        identity: &IdentityKey,
        _direction: libsignal_protocol::Direction,
    ) -> Result<bool> {
        let path = format!("state/identities/{}", address);
        match ws_read(&path) {
            Some(bytes) => {
                let stored = IdentityKey::decode(&bytes)
                    .map_err(|_| SignalProtocolError::InvalidState("identity", "corrupt".into()))?;
                Ok(stored == *identity)
            }
            // TOFU: trust on first use
            None => Ok(true),
        }
    }

    async fn get_identity(&self, address: &ProtocolAddress) -> Result<Option<IdentityKey>> {
        let path = format!("state/identities/{}", address);
        match ws_read(&path) {
            Some(bytes) => Ok(Some(IdentityKey::decode(&bytes).map_err(|_| {
                SignalProtocolError::InvalidState("identity", "corrupt".into())
            })?)),
            None => Ok(None),
        }
    }
}

// ============================================================================
// PreKeyStore
// ============================================================================

#[async_trait(?Send)]
impl libsignal_protocol::PreKeyStore for WorkspaceSignalStore {
    async fn get_pre_key(&self, prekey_id: PreKeyId) -> Result<PreKeyRecord> {
        let path = format!("state/prekeys/{}", u32::from(prekey_id));
        let bytes = ws_read(&path).ok_or(SignalProtocolError::InvalidPreKeyId)?;
        PreKeyRecord::deserialize(&bytes)
    }

    async fn save_pre_key(&mut self, prekey_id: PreKeyId, record: &PreKeyRecord) -> Result<()> {
        let path = format!("state/prekeys/{}", u32::from(prekey_id));
        ws_write(&path, &record.serialize()?);
        Ok(())
    }

    async fn remove_pre_key(&mut self, prekey_id: PreKeyId) -> Result<()> {
        let path = format!("state/prekeys/{}", u32::from(prekey_id));
        ws_delete(&path);
        Ok(())
    }
}

// ============================================================================
// SignedPreKeyStore
// ============================================================================

#[async_trait(?Send)]
impl libsignal_protocol::SignedPreKeyStore for WorkspaceSignalStore {
    async fn get_signed_pre_key(
        &self,
        signed_prekey_id: SignedPreKeyId,
    ) -> Result<SignedPreKeyRecord> {
        let path = format!("state/signed_prekeys/{}", u32::from(signed_prekey_id));
        let bytes = ws_read(&path).ok_or(SignalProtocolError::InvalidSignedPreKeyId)?;
        SignedPreKeyRecord::deserialize(&bytes)
    }

    async fn save_signed_pre_key(
        &mut self,
        signed_prekey_id: SignedPreKeyId,
        record: &SignedPreKeyRecord,
    ) -> Result<()> {
        let path = format!("state/signed_prekeys/{}", u32::from(signed_prekey_id));
        ws_write(&path, &record.serialize()?);
        Ok(())
    }
}

// ============================================================================
// KyberPreKeyStore
// ============================================================================

#[async_trait(?Send)]
impl libsignal_protocol::KyberPreKeyStore for WorkspaceSignalStore {
    async fn get_kyber_pre_key(
        &self,
        kyber_prekey_id: KyberPreKeyId,
    ) -> Result<KyberPreKeyRecord> {
        let path = format!("state/kyber_prekeys/{}", u32::from(kyber_prekey_id));
        let bytes = ws_read(&path).ok_or(SignalProtocolError::InvalidKyberPreKeyId)?;
        KyberPreKeyRecord::deserialize(&bytes)
    }

    async fn save_kyber_pre_key(
        &mut self,
        kyber_prekey_id: KyberPreKeyId,
        record: &KyberPreKeyRecord,
    ) -> Result<()> {
        let path = format!("state/kyber_prekeys/{}", u32::from(kyber_prekey_id));
        ws_write(&path, &record.serialize()?);
        Ok(())
    }

    async fn mark_kyber_pre_key_used(
        &mut self,
        _kyber_prekey_id: KyberPreKeyId,
    ) -> Result<()> {
        // Mark as used — for last-resort keys this is a no-op,
        // for one-time keys we could remove them here.
        Ok(())
    }
}

// ============================================================================
// SessionStore
// ============================================================================

#[async_trait(?Send)]
impl libsignal_protocol::SessionStore for WorkspaceSignalStore {
    async fn load_session(&self, address: &ProtocolAddress) -> Result<Option<SessionRecord>> {
        let path = format!("state/sessions/{}", address);
        match ws_read(&path) {
            Some(bytes) if !bytes.is_empty() => Ok(Some(SessionRecord::deserialize(&bytes)?)),
            _ => Ok(None),
        }
    }

    async fn store_session(
        &mut self,
        address: &ProtocolAddress,
        record: &SessionRecord,
    ) -> Result<()> {
        let path = format!("state/sessions/{}", address);
        ws_write(&path, &record.serialize()?);
        Ok(())
    }
}

// ============================================================================
// SenderKeyStore
// ============================================================================

#[async_trait(?Send)]
impl libsignal_protocol::SenderKeyStore for WorkspaceSignalStore {
    async fn store_sender_key(
        &mut self,
        sender: &ProtocolAddress,
        distribution_id: Uuid,
        record: &SenderKeyRecord,
    ) -> Result<()> {
        let path = format!("state/sender_keys/{}/{}", sender, distribution_id);
        ws_write(&path, &record.serialize()?);
        Ok(())
    }

    async fn load_sender_key(
        &mut self,
        sender: &ProtocolAddress,
        distribution_id: Uuid,
    ) -> Result<Option<SenderKeyRecord>> {
        let path = format!("state/sender_keys/{}/{}", sender, distribution_id);
        match ws_read(&path) {
            Some(bytes) if !bytes.is_empty() => {
                Ok(Some(SenderKeyRecord::deserialize(&bytes)?))
            }
            _ => Ok(None),
        }
    }
}
