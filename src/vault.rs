//! Password encryption for investigations: XChaCha20-Poly1305 with an Argon2id key.
//! Every file of an encrypted case is stored as `<name>.qenc` = nonce (24 bytes) + ciphertext.

use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use rand::RngCore;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};

pub const LOCK_FILE: &str = ".quietlock";
pub const EXT: &str = "qenc";
const MAGIC: &[u8] = b"QNLOCK1\n";
const CHECK: &[u8] = b"quiet-note-ok";

#[derive(Clone)]
pub struct Key([u8; 32]);

impl Key {
    fn cipher(&self) -> XChaCha20Poly1305 {
        XChaCha20Poly1305::new((&self.0).into())
    }

    pub fn seal(&self, plain: &[u8]) -> Vec<u8> {
        let mut nonce = [0u8; 24];
        rand::thread_rng().fill_bytes(&mut nonce);
        let mut out = nonce.to_vec();
        out.extend(self.cipher().encrypt(XNonce::from_slice(&nonce), plain).expect("encrypt"));
        out
    }

    pub fn open(&self, data: &[u8]) -> Option<Vec<u8>> {
        if data.len() < 24 {
            return None;
        }
        self.cipher().decrypt(XNonce::from_slice(&data[..24]), &data[24..]).ok()
    }

    /// Stable file name for an entity that does not reveal the entity's name.
    pub fn hidden_name(&self, name: &str) -> String {
        let mut h = Sha256::new();
        h.update(self.0);
        h.update(name.as_bytes());
        h.finalize().iter().take(10).map(|b| format!("{b:02x}")).collect()
    }
}

fn derive(password: &str, salt: &[u8]) -> Key {
    let mut k = [0u8; 32];
    argon2::Argon2::default().hash_password_into(password.as_bytes(), salt, &mut k).expect("argon2");
    Key(k)
}

pub fn is_locked(dir: &Path) -> bool {
    dir.join(LOCK_FILE).exists()
}

/// Creates the lock file for a new password and returns the key.
pub fn create(dir: &Path, password: &str) -> Result<Key, String> {
    let mut salt = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut salt);
    let key = derive(password, &salt);
    let mut data = MAGIC.to_vec();
    data.extend(salt);
    data.extend(key.seal(CHECK));
    fs::write(dir.join(LOCK_FILE), data).map_err(|e| format!("Не удалось зашифровать: {e}"))?;
    Ok(key)
}

/// Checks the password against the lock file.
pub fn unlock(dir: &Path, password: &str) -> Result<Key, String> {
    let data = fs::read(dir.join(LOCK_FILE)).map_err(|e| format!("Не удалось прочитать дело: {e}"))?;
    if !data.starts_with(MAGIC) || data.len() < MAGIC.len() + 16 {
        return Err("Файл шифрования повреждён".into());
    }
    let salt = &data[MAGIC.len()..MAGIC.len() + 16];
    let key = derive(password, salt);
    match key.open(&data[MAGIC.len() + 16..]) {
        Some(c) if c == CHECK => Ok(key),
        _ => Err("Неверный пароль".into()),
    }
}

pub fn enc_path(p: &Path) -> PathBuf {
    let mut s = p.as_os_str().to_owned();
    s.push(".");
    s.push(EXT);
    PathBuf::from(s)
}

pub fn is_enc(p: &Path) -> bool {
    p.extension().is_some_and(|e| e == EXT)
}

/// Decrypts one stored file into a private temporary folder, for opening in another app.
pub fn to_temp(key: &Key, enc: &Path) -> Result<PathBuf, String> {
    let data = fs::read(enc).map_err(|e| e.to_string())?;
    let plain = key.open(&data).ok_or("Не удалось расшифровать файл")?;
    let dir = std::env::temp_dir().join("quiet-note-open");
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&dir, fs::Permissions::from_mode(0o700));
    }
    let name = enc.file_stem().map(|s| s.to_os_string()).unwrap_or_default();
    let out = dir.join(name);
    fs::write(&out, plain).map_err(|e| e.to_string())?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_roundtrip() {
        let dir = std::env::temp_dir().join(format!("qn-vault-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let k = create(&dir, "секрет").unwrap();
        assert!(unlock(&dir, "неверно").is_err());
        let k2 = unlock(&dir, "секрет").unwrap();
        assert_eq!(k2.open(&k.seal(b"hello")).unwrap(), b"hello");
        assert_eq!(k.hidden_name("a"), k2.hidden_name("a"));
    }
}
