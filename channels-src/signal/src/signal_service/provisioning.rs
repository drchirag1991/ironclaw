//! Device provisioning (linking) for Signal.
//!
//! Adapted from libsignal-service-rs provisioning cipher.
//! The provisioning flow links this device as a secondary device
//! to an existing Signal account.
//!
//! Flow:
//! 1. Generate ephemeral keypair
//! 2. Open provisioning WebSocket (unauthenticated)
//! 3. Receive provisioning address from server
//! 4. Display QR code URL: sgnl://linkdevice?uuid={addr}&pub_key={key}
//! 5. Primary device scans QR, encrypts ProvisionMessage with our public key
//! 6. Server delivers encrypted ProvisionEnvelope via WebSocket
//! 7. We decrypt to get identity keys, ACI, PNI, phone number
//! 8. Generate pre-keys, call PUT /v1/devices/link to finalize

use std::fmt;

use aes::cipher::block_padding::Pkcs7;
use aes::cipher::{BlockDecryptMut, BlockEncryptMut, KeyIvInit};
use aes::Aes256;
use hmac::{Hmac, Mac};
use libsignal_protocol::{KeyPair, PublicKey};
use prost::Message;
use rand::{CryptoRng, Rng};
use sha2::Sha256;

use crate::proto::{ProvisionEnvelope, ProvisionMessage};

// Crypto constants
const CIPHER_KEY_SIZE: usize = 32;
const IV_LENGTH: usize = 16;
const IV_OFFSET: usize = 1; // After VERSION byte
const VERSION: u8 = 1;

/// Errors during device provisioning.
#[derive(Debug)]
pub enum ProvisioningError {
    BadVersionNumber,
    MismatchedMac,
    EncryptOnlyProvisioningCipher,
    InvalidPublicKey(String),
    InvalidPrivateKey(String),
    AesPaddingError,
    ProtobufDecode(prost::DecodeError),
}

impl fmt::Display for ProvisioningError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadVersionNumber => write!(f, "bad version number"),
            Self::MismatchedMac => write!(f, "MAC verification failed"),
            Self::EncryptOnlyProvisioningCipher => {
                write!(f, "cannot decrypt with encrypt-only cipher")
            }
            Self::InvalidPublicKey(e) => write!(f, "invalid public key: {e}"),
            Self::InvalidPrivateKey(e) => write!(f, "invalid private key: {e}"),
            Self::AesPaddingError => write!(f, "AES padding error"),
            Self::ProtobufDecode(e) => write!(f, "protobuf decode error: {e}"),
        }
    }
}

impl From<prost::DecodeError> for ProvisioningError {
    fn from(e: prost::DecodeError) -> Self {
        Self::ProtobufDecode(e)
    }
}

enum CipherMode {
    DecryptAndEncrypt(KeyPair),
    EncryptOnly(PublicKey),
}

impl fmt::Debug for CipherMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CipherMode::DecryptAndEncrypt(kp) => f
                .debug_tuple("DecryptAndEncrypt")
                .field(&kp.public_key)
                .finish(),
            CipherMode::EncryptOnly(pk) => {
                f.debug_tuple("EncryptOnly").field(&pk).finish()
            }
        }
    }
}

impl CipherMode {
    fn public(&self) -> &PublicKey {
        match self {
            CipherMode::DecryptAndEncrypt(pair) => &pair.public_key,
            CipherMode::EncryptOnly(pub_key) => pub_key,
        }
    }
}

/// Cipher for encrypting/decrypting provisioning messages.
///
/// Used during the device linking flow. The secondary device generates
/// a keypair, shares the public key via QR code, and the primary device
/// encrypts the provisioning message with it.
#[derive(Debug)]
pub struct ProvisioningCipher {
    key_material: CipherMode,
}

impl ProvisioningCipher {
    /// Create a cipher that can only encrypt (for the primary device side).
    pub fn from_public(key: PublicKey) -> Self {
        Self {
            key_material: CipherMode::EncryptOnly(key),
        }
    }

    /// Create a cipher that can decrypt (for the secondary/linking device).
    pub fn from_key_pair(key_pair: KeyPair) -> Self {
        Self {
            key_material: CipherMode::DecryptAndEncrypt(key_pair),
        }
    }

    /// Get the public key for sharing via QR code.
    pub fn public_key(&self) -> &PublicKey {
        self.key_material.public()
    }

    /// Encrypt a provisioning message.
    pub fn encrypt<R: Rng + CryptoRng>(
        &self,
        csprng: &mut R,
        msg: ProvisionMessage,
    ) -> Result<ProvisionEnvelope, ProvisioningError> {
        let msg = msg.encode_to_vec();

        let our_key_pair = KeyPair::generate(csprng);
        let agreement = our_key_pair
            .calculate_agreement(self.public_key())
            .map_err(|e| ProvisioningError::InvalidPublicKey(e.to_string()))?;

        let mut shared_secrets = [0u8; 64];
        hkdf::Hkdf::<Sha256>::new(None, &agreement)
            .expand(b"TextSecure Provisioning Message", &mut shared_secrets)
            .expect("valid output length");

        let aes_key = &shared_secrets[..32];
        let mac_key = &shared_secrets[32..];
        let mut iv = [0u8; IV_LENGTH];
        csprng.fill(&mut iv);

        let cipher =
            cbc::Encryptor::<Aes256>::new(aes_key.into(), &iv.into());
        let ciphertext = cipher.encrypt_padded_vec_mut::<Pkcs7>(&msg);

        let mut mac =
            Hmac::<Sha256>::new_from_slice(mac_key).expect("HMAC accepts any key size");
        mac.update(&[VERSION]);
        mac.update(&iv);
        mac.update(&ciphertext);
        let mac = mac.finalize().into_bytes();

        let body: Vec<u8> = std::iter::once(VERSION)
            .chain(iv)
            .chain(ciphertext)
            .chain(mac)
            .collect();

        Ok(ProvisionEnvelope {
            public_key: Some(our_key_pair.public_key.serialize().to_vec()),
            body: Some(body),
        })
    }

    /// Decrypt a provisioning envelope received from the primary device.
    pub fn decrypt(
        &self,
        envelope: ProvisionEnvelope,
    ) -> Result<ProvisionMessage, ProvisioningError> {
        let key_pair = match self.key_material {
            CipherMode::DecryptAndEncrypt(ref kp) => kp,
            CipherMode::EncryptOnly(_) => {
                return Err(ProvisioningError::EncryptOnlyProvisioningCipher);
            }
        };

        let public_key_bytes = envelope
            .public_key
            .ok_or_else(|| ProvisioningError::InvalidPublicKey("missing".to_string()))?;
        let master_ephemeral = PublicKey::deserialize(&public_key_bytes)
            .map_err(|e| ProvisioningError::InvalidPublicKey(e.to_string()))?;

        let body = envelope
            .body
            .ok_or(ProvisioningError::BadVersionNumber)?;

        if body.is_empty() || body[0] != VERSION {
            return Err(ProvisioningError::BadVersionNumber);
        }

        let iv = &body[IV_OFFSET..(IV_LENGTH + IV_OFFSET)];
        let mac = &body[(body.len() - 32)..];
        let cipher_text = &body[(IV_LENGTH + 1)..(body.len() - CIPHER_KEY_SIZE)];
        let iv_and_cipher_text = &body[..(body.len() - CIPHER_KEY_SIZE)];

        // Derive shared secret
        let agreement = key_pair
            .calculate_agreement(&master_ephemeral)
            .map_err(|e| ProvisioningError::InvalidPrivateKey(e.to_string()))?;

        let mut shared_secrets = [0u8; 64];
        hkdf::Hkdf::<Sha256>::new(None, &agreement)
            .expand(b"TextSecure Provisioning Message", &mut shared_secrets)
            .expect("valid output length");

        let aes_key = &shared_secrets[..32];
        let mac_key = &shared_secrets[32..];

        // Verify MAC
        let mut verifier =
            Hmac::<Sha256>::new_from_slice(mac_key).expect("HMAC accepts any key size");
        verifier.update(iv_and_cipher_text);
        let our_mac = verifier.finalize().into_bytes();
        if our_mac[..32] != *mac {
            return Err(ProvisioningError::MismatchedMac);
        }

        // Decrypt
        let cipher =
            cbc::Decryptor::<Aes256>::new(aes_key.into(), iv.into());
        let plaintext = cipher
            .decrypt_padded_vec_mut::<Pkcs7>(cipher_text)
            .map_err(|_| ProvisioningError::AesPaddingError)?;

        Ok(ProvisionMessage::decode(plaintext.as_slice())?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encrypt_decrypt_roundtrip() {
        let mut rng = rand::thread_rng();
        let key_pair = KeyPair::generate(&mut rng);
        let decrypt_cipher = ProvisioningCipher::from_key_pair(key_pair);
        let encrypt_cipher =
            ProvisioningCipher::from_public(*decrypt_cipher.public_key());

        let msg = ProvisionMessage {
            aci_identity_key_public: Some(vec![1, 2, 3]),
            aci_identity_key_private: Some(vec![4, 5, 6]),
            number: Some("+15551234567".to_string()),
            provisioning_code: Some("abc123".to_string()),
            ..Default::default()
        };

        let encrypted = encrypt_cipher.encrypt(&mut rng, msg.clone()).unwrap();
        let decrypted = decrypt_cipher.decrypt(encrypted).unwrap();

        assert_eq!(decrypted.aci_identity_key_public, msg.aci_identity_key_public);
        assert_eq!(decrypted.number, msg.number);
        assert_eq!(decrypted.provisioning_code, msg.provisioning_code);
    }

    #[test]
    fn encrypt_only_cipher_cannot_decrypt() {
        let mut rng = rand::thread_rng();
        let key_pair = KeyPair::generate(&mut rng);
        let encrypt_cipher =
            ProvisioningCipher::from_public(key_pair.public_key);

        let msg = ProvisionMessage::default();
        let encrypted = encrypt_cipher.encrypt(&mut rng, msg).unwrap();

        assert!(matches!(
            encrypt_cipher.decrypt(encrypted),
            Err(ProvisioningError::EncryptOnlyProvisioningCipher)
        ));
    }
}
