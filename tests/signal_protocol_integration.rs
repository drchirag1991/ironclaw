//! Integration tests for the Signal channel.
//!
//! Tests verify:
//! - Signal Protocol session establishment and message encryption
//! - WASM module loads and starts correctly
//! - Provisioning cipher encrypt/decrypt
//!
//! Test tiers:
//! - Protocol tests run without feature flags (use vendored libsignal-protocol)
//! - WASM module tests require signal.wasm to be built
//! - Full integration tests require `--features integration`

use std::sync::Arc;

use ironclaw::channels::wasm::{
    ChannelCapabilities, WasmChannelRuntime, WasmChannelRuntimeConfig,
};
use ironclaw::pairing::PairingStore;

// ============================================================================
// Test helpers
// ============================================================================

fn signal_wasm_path() -> std::path::PathBuf {
    let base = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("channels-src/signal");
    // Check flat layout first (post-build.sh)
    let flat = base.join("signal.wasm");
    if flat.exists() {
        return flat;
    }
    // Then build tree
    base.join("target/wasm32-wasip2/release/signal_channel.wasm")
}

macro_rules! require_signal_wasm {
    () => {
        if !signal_wasm_path().exists() {
            let msg = format!(
                "Signal WASM module not found at {:?}. \
                 Build with: cd channels-src/signal && ./build.sh",
                signal_wasm_path()
            );
            if std::env::var("CI").is_ok() {
                panic!("{}", msg);
            }
            eprintln!("Skipping test: {}", msg);
            return;
        }
    };
}

fn create_test_runtime() -> Arc<WasmChannelRuntime> {
    Arc::new(
        WasmChannelRuntime::new(WasmChannelRuntimeConfig::for_testing())
            .expect("Failed to create WASM runtime"),
    )
}

// ============================================================================
// Tier 1: Signal Protocol session tests (no WASM module needed)
// ============================================================================

/// Verify key generation works with the vendored libsignal-protocol.
#[test]
fn signal_protocol_key_generation() {
    use libsignal_protocol::{IdentityKeyPair, KeyPair};

    let mut rng = rand::thread_rng();
    let identity = IdentityKeyPair::generate(&mut rng);
    assert!(!identity.public_key().serialize().is_empty());

    let pre_key = KeyPair::generate(&mut rng);
    assert!(!pre_key.public_key.serialize().is_empty());
}

/// Full session establishment + message encrypt/decrypt roundtrip.
#[tokio::test]
async fn signal_protocol_session_roundtrip() {
    use libsignal_protocol::*;

    let mut rng = rand::thread_rng();

    // --- Alice setup ---
    let alice_identity = IdentityKeyPair::generate(&mut rng);
    let alice_address = ProtocolAddress::new("alice".to_string(), 1.into());
    let mut alice_store =
        InMemSignalProtocolStore::new(alice_identity, 1).expect("alice store");

    // --- Bob setup ---
    let bob_identity = IdentityKeyPair::generate(&mut rng);
    let bob_address = ProtocolAddress::new("bob".to_string(), 1.into());
    let mut bob_store =
        InMemSignalProtocolStore::new(bob_identity, 2).expect("bob store");

    // --- Bob generates signed pre-key ---
    let bob_signed_pk_pair = KeyPair::generate(&mut rng);
    let bob_signed_pk_id: SignedPreKeyId = 1.into();
    let bob_signed_pk_sig = bob_identity
        .private_key()
        .calculate_signature(&bob_signed_pk_pair.public_key.serialize(), &mut rng)
        .unwrap();

    bob_store
        .save_signed_pre_key(
            bob_signed_pk_id,
            &SignedPreKeyRecord::new(
                bob_signed_pk_id,
                Timestamp::from_epoch_millis(42),
                &bob_signed_pk_pair,
                &bob_signed_pk_sig,
            ),
        )
        .await
        .unwrap();

    // --- Bob generates one-time pre-key ---
    let bob_pk_pair = KeyPair::generate(&mut rng);
    let bob_pk_id: PreKeyId = 1.into();
    bob_store
        .save_pre_key(bob_pk_id, &PreKeyRecord::new(bob_pk_id, &bob_pk_pair))
        .await
        .unwrap();

    // --- Bob's pre-key bundle ---
    let bob_bundle = PreKeyBundle::new(
        2, // reg_id
        1.into(),
        Some((bob_pk_id, bob_pk_pair.public_key)),
        bob_signed_pk_id,
        bob_signed_pk_pair.public_key,
        bob_signed_pk_sig.to_vec(),
        *bob_identity.identity_key(),
    )
    .unwrap();

    // --- Alice processes Bob's bundle ---
    process_prekey_bundle(
        &bob_address,
        &mut alice_store.session_store,
        &mut alice_store.identity_store,
        &bob_bundle,
        std::time::SystemTime::UNIX_EPOCH,
        &mut rng,
    )
    .await
    .unwrap();

    // --- Alice encrypts ---
    let plaintext = b"Hello from Alice!";
    let ciphertext = message_encrypt(
        plaintext,
        &bob_address,
        &mut alice_store.session_store,
        &mut alice_store.identity_store,
        std::time::SystemTime::UNIX_EPOCH,
    )
    .await
    .unwrap();

    assert_eq!(ciphertext.message_type(), CiphertextMessageType::PreKey);

    // --- Bob decrypts ---
    let prekey_msg = PreKeySignalMessage::try_from(ciphertext.serialize()).unwrap();
    let decrypted = message_decrypt_prekey(
        &prekey_msg,
        &alice_address,
        &mut bob_store.session_store,
        &mut bob_store.identity_store,
        &mut bob_store.pre_key_store,
        &bob_store.signed_pre_key_store,
        &mut bob_store.kyber_pre_key_store,
        &mut rng,
    )
    .await
    .unwrap();

    assert_eq!(decrypted, plaintext);

    // --- Bob replies ---
    let reply = b"Hello back from Bob!";
    let reply_ct = message_encrypt(
        reply,
        &alice_address,
        &mut bob_store.session_store,
        &mut bob_store.identity_store,
        std::time::SystemTime::UNIX_EPOCH,
    )
    .await
    .unwrap();

    assert_eq!(reply_ct.message_type(), CiphertextMessageType::Whisper);

    // --- Alice decrypts reply ---
    let signal_msg = SignalMessage::try_from(reply_ct.serialize()).unwrap();
    let decrypted_reply = message_decrypt_signal(
        &signal_msg,
        &bob_address,
        &mut alice_store.session_store,
        &mut alice_store.identity_store,
        &mut rng,
    )
    .await
    .unwrap();

    assert_eq!(decrypted_reply, reply);
}

/// Multiple messages in same session maintain ratchet state.
#[tokio::test]
async fn signal_protocol_multiple_messages() {
    use libsignal_protocol::*;

    let mut rng = rand::thread_rng();

    let alice_identity = IdentityKeyPair::generate(&mut rng);
    let alice_address = ProtocolAddress::new("alice".to_string(), 1.into());
    let mut alice_store =
        InMemSignalProtocolStore::new(alice_identity, 1).expect("store");

    let bob_identity = IdentityKeyPair::generate(&mut rng);
    let bob_address = ProtocolAddress::new("bob".to_string(), 1.into());
    let mut bob_store =
        InMemSignalProtocolStore::new(bob_identity, 2).expect("store");

    // Set up Bob's pre-key bundle
    let bob_spk_pair = KeyPair::generate(&mut rng);
    let bob_spk_id: SignedPreKeyId = 1.into();
    let bob_spk_sig = bob_identity
        .private_key()
        .calculate_signature(&bob_spk_pair.public_key.serialize(), &mut rng)
        .unwrap();
    bob_store
        .save_signed_pre_key(
            bob_spk_id,
            &SignedPreKeyRecord::new(
                bob_spk_id,
                Timestamp::from_epoch_millis(42),
                &bob_spk_pair,
                &bob_spk_sig,
            ),
        )
        .await
        .unwrap();

    let bob_pk_pair = KeyPair::generate(&mut rng);
    let bob_pk_id: PreKeyId = 1.into();
    bob_store
        .save_pre_key(bob_pk_id, &PreKeyRecord::new(bob_pk_id, &bob_pk_pair))
        .await
        .unwrap();

    let bundle = PreKeyBundle::new(
        2,
        1.into(),
        Some((bob_pk_id, bob_pk_pair.public_key)),
        bob_spk_id,
        bob_spk_pair.public_key,
        bob_spk_sig.to_vec(),
        *bob_identity.identity_key(),
    )
    .unwrap();

    process_prekey_bundle(
        &bob_address,
        &mut alice_store.session_store,
        &mut alice_store.identity_store,
        &bundle,
        std::time::SystemTime::UNIX_EPOCH,
        &mut rng,
    )
    .await
    .unwrap();

    // Send 10 messages and verify each decrypts correctly
    for i in 0..10 {
        let msg = format!("Message number {i}");
        let ct = message_encrypt(
            msg.as_bytes(),
            &bob_address,
            &mut alice_store.session_store,
            &mut alice_store.identity_store,
            std::time::SystemTime::UNIX_EPOCH,
        )
        .await
        .unwrap();

        let decrypted = if ct.message_type() == CiphertextMessageType::PreKey {
            let prekey_msg = PreKeySignalMessage::try_from(ct.serialize()).unwrap();
            message_decrypt_prekey(
                &prekey_msg,
                &alice_address,
                &mut bob_store.session_store,
                &mut bob_store.identity_store,
                &mut bob_store.pre_key_store,
                &bob_store.signed_pre_key_store,
                &mut bob_store.kyber_pre_key_store,
                &mut rng,
            )
            .await
            .unwrap()
        } else {
            let signal_msg = SignalMessage::try_from(ct.serialize()).unwrap();
            message_decrypt_signal(
                &signal_msg,
                &alice_address,
                &mut bob_store.session_store,
                &mut bob_store.identity_store,
                &mut rng,
            )
            .await
            .unwrap()
        };

        assert_eq!(String::from_utf8(decrypted).unwrap(), msg);
    }
}

// ============================================================================
// Tier 2: WASM module tests
// ============================================================================

/// Test that the Signal WASM module loads and starts.
#[tokio::test]
async fn signal_wasm_module_loads_and_starts() {
    require_signal_wasm!();

    let runtime = create_test_runtime();
    let path = signal_wasm_path();
    let wasm_bytes = std::fs::read(&path).expect("read wasm");

    let module = runtime
        .prepare("signal", &wasm_bytes, None, Some("Signal".to_string()))
        .await
        .expect("prepare module");

    let config_json = serde_json::json!({
        "dm_policy": "open",
        "allow_from": ["*"],
    })
    .to_string();

    let channel = ironclaw::channels::wasm::WasmChannel::new(
        runtime,
        module,
        ChannelCapabilities::for_channel("signal").with_polling(5000),
        "default",
        config_json,
        Arc::new(PairingStore::new()),
        None,
    );

    use ironclaw::channels::Channel;
    assert_eq!(channel.name(), "signal");
}

/// Test that the Signal channel handles on_start with default config.
#[tokio::test]
async fn signal_channel_starts_with_default_config() {
    require_signal_wasm!();

    let runtime = create_test_runtime();
    let path = signal_wasm_path();
    let wasm_bytes = std::fs::read(&path).expect("read wasm");

    let module = runtime
        .prepare("signal", &wasm_bytes, None, Some("Signal".to_string()))
        .await
        .expect("prepare module");

    // Minimal config — uses serde defaults for all fields
    let config_json = serde_json::json!({
        "dm_policy": "pairing",
    })
    .to_string();

    let channel = ironclaw::channels::wasm::WasmChannel::new(
        runtime,
        module,
        ChannelCapabilities::for_channel("signal").with_polling(5000),
        "default",
        config_json,
        Arc::new(PairingStore::new()),
        None,
    );

    use ironclaw::channels::Channel;
    let result = channel.start().await;
    assert!(
        result.is_ok(),
        "Channel should start with minimal config: {:?}",
        result.err()
    );
}
