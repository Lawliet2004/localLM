use aes_gcm::{
    aead::{Aead, AeadCore, OsRng, Payload},
    Aes256Gcm, KeyInit, Nonce,
};
use serde::{Deserialize, Serialize};
use std::{io::Write, path::PathBuf, sync::Mutex};

/// Metadata kept alongside each stored credential. A `created_at` of `0`
/// means the secret predates records and its age is unknown.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CredentialMeta {
    pub created_at: i64,
    pub expires_at: Option<i64>,
    pub last_verified_at: Option<i64>,
}
#[derive(Serialize, Deserialize)]
struct CredentialRecord {
    secret: String,
    meta: CredentialMeta,
}

pub struct Vault {
    directory: PathBuf,
    lock: Mutex<()>,
    pub refresh: std::sync::Arc<tokio::sync::Mutex<()>>,
}
impl Vault {
    pub fn new(directory: PathBuf) -> Self {
        Self {
            directory,
            lock: Mutex::new(()),
            refresh: std::sync::Arc::new(tokio::sync::Mutex::new(())),
        }
    }
    fn cipher(&self) -> Result<Aes256Gcm, String> {
        let entry = keyring::Entry::new("app.locallm.desktop", "credential-encryption-key")
            .map_err(|_| "Windows Credential Manager is unavailable.")?;
        let key = match entry.get_secret() {
            Ok(key) => key,
            Err(keyring::Error::NoEntry) => {
                if self.directory.exists()
                    && std::fs::read_dir(&self.directory)
                        .map_err(|error| error.to_string())?
                        .any(|item| {
                            item.is_ok_and(|item| {
                                item.path().extension().is_some_and(|ext| ext == "sealed")
                            })
                        })
                {
                    return Err("The credential encryption key is missing. Stored credentials cannot be recovered with this Windows account.".into());
                }
                let key = Aes256Gcm::generate_key(OsRng);
                entry.set_secret(&key).map_err(|_| {
                    "Could not protect the credential encryption key in Windows Credential Manager."
                })?;
                key.to_vec()
            }
            Err(_) => {
                return Err("Could not access the protected credential encryption key.".into())
            }
        };
        Aes256Gcm::new_from_slice(&key)
            .map_err(|_| "The protected credential encryption key is invalid.".into())
    }
    /// Loads the stored secret. Credential records unwrap transparently, so
    /// callers always receive bare secret bytes.
    /// The app database sits next to the vault directory.
    pub fn db_path(&self) -> std::path::PathBuf {
        self.directory
            .parent()
            .unwrap_or(&self.directory)
            .join("locallm.sqlite")
    }
    pub fn load(&self, id: &str) -> Result<Option<Vec<u8>>, String> {
        validate_id(id)?;
        let _guard = self
            .lock
            .lock()
            .map_err(|_| "Credential vault lock unavailable.")?;
        Ok(self.load_sealed(id)?.map(|bytes| unwrap_record(bytes).0))
    }
    /// Loads the stored secret with its record metadata. Secrets written
    /// before records existed return `CredentialMeta::default()`.
    pub fn load_record(&self, id: &str) -> Result<Option<(Vec<u8>, CredentialMeta)>, String> {
        validate_id(id)?;
        let _guard = self
            .lock
            .lock()
            .map_err(|_| "Credential vault lock unavailable.")?;
        Ok(self.load_sealed(id)?.map(unwrap_record))
    }
    fn load_sealed(&self, id: &str) -> Result<Option<Vec<u8>>, String> {
        let path = self.directory.join(format!("{id}.sealed"));
        let metadata = match std::fs::metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.to_string()),
        };
        if metadata.len() > 131_101 {
            return Err("Stored credential exceeds its size limit.".into());
        }
        let bytes = std::fs::read(path).map_err(|error| error.to_string())?;
        Ok(Some(unseal(&self.cipher()?, id, &bytes)?))
    }
    pub fn save(&self, id: &str, secret: &[u8]) -> Result<(), String> {
        self.save_with_expiry(id, secret, None)
    }
    /// Stores `secret` as a fresh credential record: `created_at` is stamped
    /// now and `expires_at` carries the caller's expiry, if any. Saving over
    /// an existing id replaces the record entirely (rotation semantics).
    pub fn save_with_expiry(
        &self,
        id: &str,
        secret: &[u8],
        expires_at: Option<i64>,
    ) -> Result<(), String> {
        validate_id(id)?;
        let plaintext = wrap_record(
            secret,
            CredentialMeta {
                created_at: crate::store::now() / 1000,
                expires_at,
                last_verified_at: None,
            },
        )?;
        let _guard = self
            .lock
            .lock()
            .map_err(|_| "Credential vault lock unavailable.")?;
        self.save_sealed(id, &plaintext)
    }
    /// Stamps `last_verified_at` on the stored record, upgrading a legacy bare
    /// secret to a record in place. No-op when nothing is stored under `id`.
    pub fn mark_verified(&self, id: &str) -> Result<(), String> {
        validate_id(id)?;
        let _guard = self
            .lock
            .lock()
            .map_err(|_| "Credential vault lock unavailable.")?;
        let Some(bytes) = self.load_sealed(id)? else {
            return Ok(());
        };
        let (secret, mut meta) = unwrap_record(bytes);
        meta.last_verified_at = Some(crate::store::now() / 1000);
        self.save_sealed(id, &wrap_record(&secret, meta)?)
    }
    fn save_sealed(&self, id: &str, plaintext: &[u8]) -> Result<(), String> {
        let bytes = seal(&self.cipher()?, id, plaintext)?;
        std::fs::create_dir_all(&self.directory).map_err(|error| error.to_string())?;
        let temporary = self.directory.join(format!("{}.tmp", uuid::Uuid::new_v4()));
        let result = (|| {
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)
                .map_err(|error| error.to_string())?;
            file.write_all(&bytes).map_err(|error| error.to_string())?;
            file.sync_all().map_err(|error| error.to_string())?;
            drop(file);
            std::fs::rename(&temporary, self.directory.join(format!("{id}.sealed")))
                .map_err(|error| error.to_string())
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(temporary);
        }
        result
    }
    pub fn clear(&self, id: &str) -> Result<(), String> {
        validate_id(id)?;
        let _guard = self
            .lock
            .lock()
            .map_err(|_| "Credential vault lock unavailable.")?;
        match std::fs::remove_file(self.directory.join(format!("{id}.sealed"))) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.to_string()),
        }
    }
}
fn validate_id(id: &str) -> Result<(), String> {
    if id.is_empty()
        || id.len() > 120
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    {
        return Err("Invalid credential identifier.".into());
    }
    Ok(())
}
fn wrap_record(secret: &[u8], meta: CredentialMeta) -> Result<Vec<u8>, String> {
    let secret = std::str::from_utf8(secret)
        .map_err(|_| "Credential payloads must be UTF-8 text.".to_string())?;
    serde_json::to_vec(&CredentialRecord {
        secret: secret.to_string(),
        meta,
    })
    .map_err(|error| format!("Could not encode credential record: {error}"))
}
fn unwrap_record(bytes: Vec<u8>) -> (Vec<u8>, CredentialMeta) {
    match serde_json::from_slice::<CredentialRecord>(&bytes) {
        Ok(record) => (record.secret.into_bytes(), record.meta),
        Err(_) => (bytes, CredentialMeta::default()),
    }
}
fn seal(cipher: &Aes256Gcm, id: &str, secret: &[u8]) -> Result<Vec<u8>, String> {
    if secret.len() > 131_072 {
        return Err("Credential exceeds the 128 KiB limit.".into());
    }
    let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
    let encrypted = cipher
        .encrypt(
            &nonce,
            Payload {
                msg: secret,
                aad: id.as_bytes(),
            },
        )
        .map_err(|_| "Credential encryption failed.")?;
    let mut bytes = vec![1];
    bytes.extend_from_slice(&nonce);
    bytes.extend(encrypted);
    Ok(bytes)
}
fn unseal(cipher: &Aes256Gcm, id: &str, bytes: &[u8]) -> Result<Vec<u8>, String> {
    if bytes.len() < 29 || bytes[0] != 1 {
        return Err("Invalid encrypted credential format.".into());
    }
    cipher
        .decrypt(
            Nonce::from_slice(&bytes[1..13]),
            Payload {
                msg: &bytes[13..],
                aad: id.as_bytes(),
            },
        )
        .map_err(|_| {
            "Credential verification failed. The file may be damaged or belong to another account."
                .into()
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use aes_gcm::{aead::OsRng, Aes256Gcm, KeyInit};
    #[test]
    fn large_credentials_round_trip_without_plaintext_at_rest() {
        let cipher = Aes256Gcm::new(&Aes256Gcm::generate_key(OsRng));
        let secret = "long-oauth-token-".repeat(400);
        let sealed = seal(&cipher, "oauth-linear", secret.as_bytes()).unwrap();
        assert!(!sealed.windows(16).any(|part| part == b"long-oauth-token-"));
        assert_eq!(
            unseal(&cipher, "oauth-linear", &sealed).unwrap(),
            secret.as_bytes()
        );
    }
    #[test]
    fn rejects_tampering_wrong_connector_and_invalid_filenames() {
        let cipher = Aes256Gcm::new(&Aes256Gcm::generate_key(OsRng));
        let mut sealed = seal(&cipher, "token-github", b"secret").unwrap();
        assert!(unseal(&cipher, "token-stripe", &sealed).is_err());
        sealed[15] ^= 1;
        assert!(unseal(&cipher, "token-github", &sealed).is_err());
        for name in ["../escape", "", "token\\file", "token:stream"] {
            assert!(validate_id(name).is_err());
        }
        assert!(validate_id("oauth-linear").is_ok());
    }
    #[test]
    fn record_metadata_round_trip_and_verified_stamp() {
        let temp = tempfile::tempdir().unwrap();
        let vault = Vault::new(temp.path().join("vault"));
        vault.save("oauth-linear", b"raw-token").unwrap();
        let (secret, meta) = vault.load_record("oauth-linear").unwrap().unwrap();
        assert_eq!(secret, b"raw-token");
        assert!(meta.created_at > 0);
        assert_eq!(meta.expires_at, None);
        assert_eq!(meta.last_verified_at, None);
        assert_eq!(vault.load("oauth-linear").unwrap().unwrap(), b"raw-token");
        vault.mark_verified("oauth-linear").unwrap();
        let meta = vault.load_record("oauth-linear").unwrap().unwrap().1;
        assert!(meta.last_verified_at.unwrap() > 0);
        assert_eq!(vault.load("oauth-linear").unwrap().unwrap(), b"raw-token");
        vault.mark_verified("missing-id").unwrap();
        assert!(vault.load_record("missing-id").unwrap().is_none());
    }
    #[test]
    fn rotation_replaces_record_and_refreshes_metadata() {
        let temp = tempfile::tempdir().unwrap();
        let vault = Vault::new(temp.path().join("vault"));
        vault
            .save_with_expiry("subscription-x", b"token-one", Some(111))
            .unwrap();
        vault.mark_verified("subscription-x").unwrap();
        let (_, before) = vault.load_record("subscription-x").unwrap().unwrap();
        vault
            .save_with_expiry("subscription-x", b"token-two", Some(222))
            .unwrap();
        let (secret, after) = vault.load_record("subscription-x").unwrap().unwrap();
        assert_eq!(secret, b"token-two");
        assert_eq!(after.expires_at, Some(222));
        assert_eq!(after.last_verified_at, None);
        assert!(after.created_at >= before.created_at);
    }
    #[test]
    fn legacy_bare_secret_reads_with_empty_meta() {
        let temp = tempfile::tempdir().unwrap();
        let directory = temp.path().join("vault");
        std::fs::create_dir_all(&directory).unwrap();
        let vault = Vault::new(directory.clone());
        let cipher = vault.cipher().unwrap();
        std::fs::write(
            directory.join("legacy-id.sealed"),
            seal(&cipher, "legacy-id", b"plain-secret").unwrap(),
        )
        .unwrap();
        assert_eq!(vault.load("legacy-id").unwrap().unwrap(), b"plain-secret");
        let (secret, meta) = vault.load_record("legacy-id").unwrap().unwrap();
        assert_eq!(secret, b"plain-secret");
        assert_eq!(meta, CredentialMeta::default());
        vault.mark_verified("legacy-id").unwrap();
        let (secret, meta) = vault.load_record("legacy-id").unwrap().unwrap();
        assert_eq!(secret, b"plain-secret");
        assert!(meta.last_verified_at.unwrap() > 0);
    }
}
