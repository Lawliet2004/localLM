use aes_gcm::{
    aead::{Aead, AeadCore, OsRng, Payload},
    Aes256Gcm, KeyInit, Nonce,
};
use std::{io::Write, path::PathBuf, sync::Mutex};

#[derive(Clone)]
pub struct Vault {
    directory: PathBuf,
    #[cfg(test)]
    fail_writes: std::sync::Arc<std::sync::atomic::AtomicBool>,
    lock: std::sync::Arc<Mutex<()>>,
    pub refresh: std::sync::Arc<tokio::sync::Mutex<()>>,
}
impl Vault {
    pub fn new(directory: PathBuf) -> Self {
        Self {
            directory,
            #[cfg(test)]
            fail_writes: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            lock: std::sync::Arc::new(Mutex::new(())),
            refresh: std::sync::Arc::new(tokio::sync::Mutex::new(())),
        }
    }
    /// Presence is a UI fact, not authentication or decryption success. Do
    /// not unlock the OS credential store just to list local conversations.
    pub fn contains(&self, id: &str) -> Result<bool, String> {
        validate_id(id)?;
        match std::fs::metadata(self.directory.join(format!("{id}.sealed"))) {
            Ok(metadata) => Ok(metadata.is_file()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(error.to_string()),
        }
    }
    // Unit tests use isolated process-memory keys per temporary vault. They
    // still exercise authenticated encryption and never touch account secrets.
    #[cfg(test)]
    fn cipher(&self) -> Result<Aes256Gcm, String> {
        use std::{collections::HashMap, sync::OnceLock};
        static KEYS: OnceLock<Mutex<HashMap<PathBuf, Vec<u8>>>> = OnceLock::new();
        let mut keys = KEYS
            .get_or_init(|| Mutex::new(HashMap::new()))
            .lock()
            .map_err(|e| e.to_string())?;
        let key = keys
            .entry(self.directory.clone())
            .or_insert_with(|| Aes256Gcm::generate_key(OsRng).to_vec());
        Aes256Gcm::new_from_slice(key).map_err(|e| e.to_string())
    }
    #[cfg(not(test))]
    fn cipher(&self) -> Result<Aes256Gcm, String> {
        use base64::Engine as _;
        if !cfg!(any(
            target_os = "linux",
            target_os = "windows",
            target_os = "macos"
        )) {
            return Err(
                "Persistent native credential storage is not supported on this platform.".into(),
            );
        }
        let entry = keyring::Entry::new("app.locallm.desktop", "credential-encryption-key")
            .map_err(|_| "The operating-system credential store is unavailable.")?;
        let key = match entry.get_secret() {
            Ok(key) if key.len() == 32 => key,
            Ok(key) => match std::str::from_utf8(&key)
                .ok()
                .and_then(|text| base64::engine::general_purpose::STANDARD.decode(text).ok())
            {
                Some(key) if key.len() == 32 => key,
                _ => {
                    return Err("The protected credential encryption key is invalid. Clear the stored 'credential-encryption-key' entry in the operating-system credential store and try again.".into())
                }
            },
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
                    return Err("The credential encryption key is missing. Stored credentials cannot be recovered with this account's credential store.".into());
                }
                let key = Aes256Gcm::generate_key(OsRng);
                entry.set_password(&base64::engine::general_purpose::STANDARD.encode(key)).map_err(|_| {
                    "Could not protect the credential encryption key in the operating-system credential store. On Linux, start and unlock GNOME Keyring or KWallet (Secret Service)."
                })?;
                key.to_vec()
            }
            Err(_) => {
                return Err("Could not access the protected credential encryption key. On Linux, start and unlock GNOME Keyring or KWallet (Secret Service).".into())
            }
        };
        Aes256Gcm::new_from_slice(&key)
            .map_err(|_| "The protected credential encryption key is invalid.".into())
    }
    pub fn load(&self, id: &str) -> Result<Option<Vec<u8>>, String> {
        validate_id(id)?;
        let _guard = self
            .lock
            .lock()
            .map_err(|_| "Credential vault lock unavailable.")?;
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
    #[cfg(test)]
    pub(crate) fn fail_writes_for_test(&self) {
        self.fail_writes
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }
    pub fn save(&self, id: &str, secret: &[u8]) -> Result<(), String> {
        #[cfg(test)]
        if self.fail_writes.load(std::sync::atomic::Ordering::SeqCst) {
            return Err("Injected credential-store failure".into());
        }
        validate_id(id)?;
        let _guard = self
            .lock
            .lock()
            .map_err(|_| "Credential vault lock unavailable.")?;
        let bytes = seal(&self.cipher()?, id, secret)?;
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
    /// Read a credential without blocking the async worker.
    ///
    /// The keyring call is a D-Bus round trip to the platform credential
    /// store. On Linux that can block for seconds when the keyring is locked,
    /// and it must never happen on a tokio worker thread.
    pub async fn load_blocking(&self, id: &str) -> Result<Option<Vec<u8>>, String> {
        let vault = self.clone();
        let id = id.to_string();
        tokio::task::spawn_blocking(move || vault.load(&id))
            .await
            .map_err(|error| error.to_string())?
    }

    /// Store a credential without blocking the async worker.
    pub async fn save_blocking(&self, id: &str, secret: Vec<u8>) -> Result<(), String> {
        let vault = self.clone();
        let id = id.to_string();
        tokio::task::spawn_blocking(move || vault.save(&id, &secret))
            .await
            .map_err(|error| error.to_string())?
    }

    /// Remove a credential without blocking the async worker.
    pub async fn clear_blocking(&self, id: &str) -> Result<(), String> {
        let vault = self.clone();
        let id = id.to_string();
        tokio::task::spawn_blocking(move || vault.clear(&id))
            .await
            .map_err(|error| error.to_string())?
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
    #[test]
    fn credential_presence_does_not_require_unlocking_or_decrypting() {
        let directory = tempfile::tempdir().unwrap();
        let vault = super::Vault::new(directory.path().to_path_buf());
        assert!(!vault.contains("saved-key").unwrap());
        std::fs::write(
            directory.path().join("saved-key.sealed"),
            b"corrupt fixture",
        )
        .unwrap();
        assert!(vault.contains("saved-key").unwrap());
        assert!(vault.load("saved-key").is_err());
        assert!(vault.contains("../outside").is_err());
    }
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
}
