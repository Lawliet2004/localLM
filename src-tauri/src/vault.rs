use aes_gcm::{
    aead::{Aead, AeadCore, OsRng, Payload},
    Aes256Gcm, KeyInit, Nonce,
};
use std::{io::Write, path::PathBuf, sync::Mutex};

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
    pub fn save(&self, id: &str, secret: &[u8]) -> Result<(), String> {
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
