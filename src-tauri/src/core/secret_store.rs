use argon2::{
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use base64::Engine;
use chacha20poly1305::{
    aead::{Aead, Payload},
    KeyInit, XChaCha20Poly1305, XNonce,
};
use keyring::{Entry, Error as KeyringError};
use rand::RngCore;
use sha2::{Digest, Sha256};
use std::sync::{Arc, OnceLock};
use zeroize::Zeroizing;

const KEYRING_SERVICE: &str = "com.llf.crowapi.vault";
const MASTER_KEY_ACCOUNT: &str = "master-key-v1";
pub const SECRET_VERSION: i64 = 1;

#[derive(Debug, Clone)]
pub struct EncryptedSecret {
    pub version: i64,
    pub nonce: Vec<u8>,
    pub ciphertext: Vec<u8>,
}

pub trait SecretStore: Send + Sync {
    fn encrypt(&self, context: &str, plaintext: &str) -> Result<EncryptedSecret, String>;
    fn decrypt(
        &self,
        context: &str,
        version: i64,
        nonce: &[u8],
        ciphertext: &[u8],
    ) -> Result<String, String>;
}

struct KeyMaterial(Zeroizing<[u8; 32]>);

struct KeyringSecretStore {
    key: OnceLock<Result<Arc<KeyMaterial>, String>>,
}

impl KeyringSecretStore {
    fn new() -> Self {
        Self {
            key: OnceLock::new(),
        }
    }

    fn key(&self) -> Result<Arc<KeyMaterial>, String> {
        self.key.get_or_init(load_or_create_master_key).clone()
    }
}

impl SecretStore for KeyringSecretStore {
    fn encrypt(&self, context: &str, plaintext: &str) -> Result<EncryptedSecret, String> {
        encrypt_with_key(&self.key()?.0, context, plaintext)
    }

    fn decrypt(
        &self,
        context: &str,
        version: i64,
        nonce: &[u8],
        ciphertext: &[u8],
    ) -> Result<String, String> {
        decrypt_with_key(&self.key()?.0, context, version, nonce, ciphertext)
    }
}

pub fn default_secret_store() -> Arc<dyn SecretStore> {
    static STORE: OnceLock<Arc<KeyringSecretStore>> = OnceLock::new();
    STORE
        .get_or_init(|| Arc::new(KeyringSecretStore::new()))
        .clone()
}

fn load_or_create_master_key() -> Result<Arc<KeyMaterial>, String> {
    let entry = Entry::new(KEYRING_SERVICE, MASTER_KEY_ACCOUNT)
        .map_err(|error| format!("无法打开系统密钥库: {}", error))?;
    let encoded = match entry.get_password() {
        Ok(value) => value,
        Err(KeyringError::NoEntry) => {
            let mut key = [0_u8; 32];
            rand::rng().fill_bytes(&mut key);
            let encoded = base64::engine::general_purpose::STANDARD.encode(key);
            entry
                .set_password(&encoded)
                .map_err(|error| format!("无法保存密钥库主密钥: {}", error))?;
            encoded
        }
        Err(error) => return Err(format!("无法读取密钥库主密钥: {}", error)),
    };

    let decoded = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|_| "系统密钥库中的主密钥格式无效".to_string())?;
    let key: [u8; 32] = decoded
        .try_into()
        .map_err(|_| "系统密钥库中的主密钥长度无效".to_string())?;
    Ok(Arc::new(KeyMaterial(Zeroizing::new(key))))
}

fn encrypt_with_key(
    key: &[u8; 32],
    context: &str,
    plaintext: &str,
) -> Result<EncryptedSecret, String> {
    let cipher = XChaCha20Poly1305::new_from_slice(key)
        .map_err(|_| "无法初始化密钥加密器".to_string())?;
    let mut nonce = [0_u8; 24];
    rand::rng().fill_bytes(&mut nonce);
    let ciphertext = cipher
        .encrypt(
            XNonce::from_slice(&nonce),
            Payload {
                msg: plaintext.as_bytes(),
                aad: context.as_bytes(),
            },
        )
        .map_err(|_| "加密密钥失败".to_string())?;
    Ok(EncryptedSecret {
        version: SECRET_VERSION,
        nonce: nonce.to_vec(),
        ciphertext,
    })
}

fn decrypt_with_key(
    key: &[u8; 32],
    context: &str,
    version: i64,
    nonce: &[u8],
    ciphertext: &[u8],
) -> Result<String, String> {
    if version != SECRET_VERSION || nonce.len() != 24 {
        return Err("不支持的密钥密文版本".to_string());
    }
    let cipher = XChaCha20Poly1305::new_from_slice(key)
        .map_err(|_| "无法初始化密钥解密器".to_string())?;
    let plaintext = cipher
        .decrypt(
            XNonce::from_slice(nonce),
            Payload {
                msg: ciphertext,
                aad: context.as_bytes(),
            },
        )
        .map_err(|_| "密钥密文校验失败".to_string())?;
    String::from_utf8(plaintext).map_err(|_| "密钥内容编码无效".to_string())
}

pub fn api_key_lookup(key: &str) -> String {
    hex::encode(Sha256::digest(key.as_bytes()))
}

pub fn hash_api_key(key: &str) -> Result<String, String> {
    let mut salt = [0_u8; 16];
    rand::rng().fill_bytes(&mut salt);
    let salt = SaltString::encode_b64(&salt).map_err(|error| error.to_string())?;
    Argon2::default()
        .hash_password(key.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|error| error.to_string())
}

pub fn verify_api_key(key: &str, encoded_hash: &str) -> bool {
    PasswordHash::new(encoded_hash)
        .ok()
        .is_some_and(|hash| {
            Argon2::default()
                .verify_password(key.as_bytes(), &hash)
                .is_ok()
        })
}

pub fn key_preview_parts(key: &str) -> (String, String) {
    let prefix: String = key.chars().take(12).collect();
    let chars: Vec<char> = key.chars().collect();
    let last_four = chars[chars.len().saturating_sub(4)..].iter().collect();
    (prefix, last_four)
}

#[cfg(test)]
mod tests {
    use super::{
        api_key_lookup, decrypt_with_key, encrypt_with_key, hash_api_key, verify_api_key,
    };

    #[test]
    fn encrypted_secret_is_bound_to_its_context() {
        let key = [7_u8; 32];
        let encrypted = encrypt_with_key(&key, "channel:one", "sk-secret")
            .expect("encrypt secret");

        assert_eq!(
            decrypt_with_key(
                &key,
                "channel:one",
                encrypted.version,
                &encrypted.nonce,
                &encrypted.ciphertext,
            )
            .expect("decrypt secret"),
            "sk-secret",
        );
        assert!(decrypt_with_key(
            &key,
            "channel:two",
            encrypted.version,
            &encrypted.nonce,
            &encrypted.ciphertext,
        )
        .is_err());
    }

    #[test]
    fn api_key_hash_verifies_without_storing_plaintext() {
        let encoded = hash_api_key("sk-crowapi-test").expect("hash API key");

        assert!(verify_api_key("sk-crowapi-test", &encoded));
        assert!(!verify_api_key("sk-crowapi-other", &encoded));
        assert_eq!(api_key_lookup("same"), api_key_lookup("same"));
        assert_ne!(api_key_lookup("same"), api_key_lookup("different"));
    }
}
