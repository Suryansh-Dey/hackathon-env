//! Field-level envelope encryption and keyed hashing (PLAN.md §16.2, §20.1).
//!
//! Phone numbers, address text, landmarks and document numbers are sealed
//! with AES-256-GCM under a KMS data key before they are written; lookups use
//! HMAC-SHA256 keyed hashes so the plaintext never appears in keys.

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use async_trait::async_trait;
use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use hmac::{Hmac, Mac};
use kabados_domain::user::Sealed;
use rand::RngCore;
use sha2::Sha256;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};

#[async_trait]
pub trait Sealer: Send + Sync {
    async fn seal(&self, plaintext: &str) -> anyhow::Result<Sealed>;
    async fn open(&self, sealed: &Sealed) -> anyhow::Result<String>;
}

fn aead_seal(key: &[u8; 32], plaintext: &[u8]) -> anyhow::Result<(Vec<u8>, Vec<u8>)> {
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key));
    let mut nonce = [0u8; 12];
    rand::rng().fill_bytes(&mut nonce);
    let ct = cipher.encrypt(Nonce::from_slice(&nonce), plaintext).map_err(|_| anyhow::anyhow!("encryption failed"))?;
    Ok((nonce.to_vec(), ct))
}

fn aead_open(key: &[u8], nonce: &[u8], ct: &[u8]) -> anyhow::Result<Vec<u8>> {
    if key.len() != 32 || nonce.len() != 12 {
        anyhow::bail!("bad key or nonce length");
    }
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key));
    cipher.decrypt(Nonce::from_slice(nonce), ct).map_err(|_| anyhow::anyhow!("decryption failed"))
}

/// Local/CI sealer with a static key (`v1.local.<nonce>.<ct>`). Never used in deployed envs.
pub struct LocalSealer {
    key: [u8; 32],
}

impl LocalSealer {
    pub fn new(secret: &str) -> Self {
        let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(b"kabados-local-sealer").expect("hmac");
        mac.update(secret.as_bytes());
        Self { key: mac.finalize().into_bytes().into() }
    }
}

#[async_trait]
impl Sealer for LocalSealer {
    async fn seal(&self, plaintext: &str) -> anyhow::Result<Sealed> {
        let (n, ct) = aead_seal(&self.key, plaintext.as_bytes())?;
        Ok(Sealed(format!("v1.local.{}.{}", B64.encode(n), B64.encode(ct))))
    }

    async fn open(&self, sealed: &Sealed) -> anyhow::Result<String> {
        let parts: Vec<&str> = sealed.0.split('.').collect();
        let ["v1", "local", n, ct] = parts.as_slice() else { anyhow::bail!("not a local-sealed value") };
        Ok(String::from_utf8(aead_open(&self.key, &B64.decode(n)?, &B64.decode(ct)?)?)?)
    }
}

/// KMS envelope sealer (`v1.kms.<wrapped key>.<nonce>.<ct>`). One data key is
/// generated per container and reused; unwrapped keys are cached by blob.
pub struct KmsSealer {
    client: aws_sdk_kms::Client,
    key_id: String,
    current: Mutex<Option<([u8; 32], Vec<u8>)>>,
    unwrapped: RwLock<HashMap<Vec<u8>, Vec<u8>>>,
}

impl KmsSealer {
    pub fn new(client: aws_sdk_kms::Client, key_id: impl Into<String>) -> Self {
        Self { client, key_id: key_id.into(), current: Mutex::new(None), unwrapped: RwLock::default() }
    }

    async fn data_key(&self) -> anyhow::Result<([u8; 32], Vec<u8>)> {
        let mut cur = self.current.lock().await;
        if let Some(k) = cur.as_ref() {
            return Ok(k.clone());
        }
        let out = self.client.generate_data_key().key_id(&self.key_id).key_spec(aws_sdk_kms::types::DataKeySpec::Aes256).send().await?;
        let plain = out.plaintext().ok_or_else(|| anyhow::anyhow!("no plaintext data key"))?.as_ref();
        let blob = out.ciphertext_blob().ok_or_else(|| anyhow::anyhow!("no wrapped data key"))?.as_ref().to_vec();
        let key: [u8; 32] = plain.try_into().map_err(|_| anyhow::anyhow!("unexpected data key length"))?;
        *cur = Some((key, blob.clone()));
        Ok((key, blob))
    }
}

#[async_trait]
impl Sealer for KmsSealer {
    async fn seal(&self, plaintext: &str) -> anyhow::Result<Sealed> {
        let (key, blob) = self.data_key().await?;
        let (n, ct) = aead_seal(&key, plaintext.as_bytes())?;
        Ok(Sealed(format!("v1.kms.{}.{}.{}", B64.encode(blob), B64.encode(n), B64.encode(ct))))
    }

    async fn open(&self, sealed: &Sealed) -> anyhow::Result<String> {
        let parts: Vec<&str> = sealed.0.split('.').collect();
        let ["v1", "kms", blob, n, ct] = parts.as_slice() else { anyhow::bail!("not a kms-sealed value") };
        let blob = B64.decode(blob)?;
        let cached = self.unwrapped.read().await.get(&blob).cloned();
        let key = match cached {
            Some(k) => k,
            None => {
                let out = self.client.decrypt().ciphertext_blob(aws_sdk_kms::primitives::Blob::new(blob.clone())).send().await?;
                let k = out.plaintext().ok_or_else(|| anyhow::anyhow!("kms decrypt returned no key"))?.as_ref().to_vec();
                self.unwrapped.write().await.insert(blob, k.clone());
                k
            }
        };
        Ok(String::from_utf8(aead_open(&key, &B64.decode(n)?, &B64.decode(ct)?)?)?)
    }
}

/// Keyed hashing for lookups (phone numbers, handover OTPs).
#[derive(Clone)]
pub struct Hasher {
    pepper: Arc<Vec<u8>>,
}

impl Hasher {
    pub fn new(pepper: impl AsRef<[u8]>) -> Self {
        Self { pepper: Arc::new(pepper.as_ref().to_vec()) }
    }

    pub fn hash(&self, purpose: &str, value: &str) -> String {
        let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(&self.pepper).expect("hmac accepts any key length");
        mac.update(purpose.as_bytes());
        mac.update(b"\0");
        mac.update(value.as_bytes());
        hex::encode(&mac.finalize().into_bytes()[..20])
    }

    pub fn phone(&self, e164: &str) -> String {
        self.hash("phone", e164)
    }

    /// Constant-time comparison of a value against a stored hash.
    pub fn verify(&self, purpose: &str, value: &str, expected: &str) -> bool {
        let got = self.hash(purpose, value);
        got.len() == expected.len() && got.bytes().zip(expected.bytes()).fold(0u8, |acc, (a, b)| acc | (a ^ b)) == 0
    }
}

/// HMAC-SHA256 hex signature (Razorpay webhook scheme, signed receipts).
pub fn hmac_sha256_hex(secret: &[u8], body: &[u8]) -> String {
    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(secret).expect("hmac");
    mac.update(body);
    hex::encode(mac.finalize().into_bytes())
}

/// Constant-time check of an HMAC-SHA256 hex signature.
pub fn verify_hmac_sha256_hex(secret: &[u8], body: &[u8], signature_hex: &str) -> bool {
    let Ok(sig) = hex::decode(signature_hex.trim()) else { return false };
    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(secret).expect("hmac");
    mac.update(body);
    mac.verify_slice(&sig).is_ok()
}

/// Random numeric code of `digits` length (handover OTP).
pub fn numeric_code(digits: u32) -> String {
    let n = rand::rng().next_u64() % 10u64.pow(digits);
    format!("{n:0width$}", width = digits as usize)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn local_sealer_round_trip_and_tamper_detection() {
        let s = LocalSealer::new("dev");
        let sealed = s.seal("12, MG Road, Delhi").await.unwrap();
        assert!(!sealed.0.contains("MG Road"));
        assert_eq!(s.open(&sealed).await.unwrap(), "12, MG Road, Delhi");
        let mut bad = sealed.0.clone();
        bad.pop();
        bad.push('A');
        assert!(s.open(&Sealed(bad)).await.is_err());
        assert!(LocalSealer::new("other").open(&sealed).await.is_err());
    }

    #[test]
    fn keyed_hashes() {
        let h = Hasher::new("pepper");
        assert_eq!(h.phone("+919876543210"), h.phone("+919876543210"));
        assert_ne!(h.phone("+919876543210"), Hasher::new("other").phone("+919876543210"));
        assert_ne!(h.hash("otp", "1234"), h.hash("phone", "1234"));
        let stored = h.hash("otp", "4821");
        assert!(h.verify("otp", "4821", &stored));
        assert!(!h.verify("otp", "4820", &stored));
    }

    #[test]
    fn webhook_signatures() {
        let sig = hmac_sha256_hex(b"whsec", b"{\"event\":\"payment.captured\"}");
        assert!(verify_hmac_sha256_hex(b"whsec", b"{\"event\":\"payment.captured\"}", &sig));
        assert!(!verify_hmac_sha256_hex(b"whsec", b"{\"event\":\"payment.failed\"}", &sig));
        assert!(!verify_hmac_sha256_hex(b"whsec", b"x", "zz"));
        assert_eq!(numeric_code(6).len(), 6);
    }
}
