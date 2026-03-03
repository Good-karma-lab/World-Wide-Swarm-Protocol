//! Persistent Ed25519 identity key management.
//!
//! Key is stored at ~/.config/wws-connector/<agent_name>.key as 32 raw seed bytes.
//! BIP-39 mnemonic (24 words) printed to stdout on first generation — never again.
//! File permissions set to 0600 (owner read/write only) on Unix.

use std::path::{Path, PathBuf};
use ed25519_dalek::SigningKey;

/// Load or generate an Ed25519 signing key from the given path.
///
/// If the key file exists, load it. If not, generate a new key, print
/// the BIP-39 mnemonic to stdout, and save the key file with mode 0600.
pub fn load_or_generate_key(key_path: &Path) -> anyhow::Result<SigningKey> {
    if key_path.exists() {
        load_key(key_path)
    } else {
        generate_and_save_key(key_path)
    }
}

/// Load an Ed25519 signing key from a 32-byte seed file.
pub fn load_key(key_path: &Path) -> anyhow::Result<SigningKey> {
    let bytes = std::fs::read(key_path)?;
    if bytes.len() != 32 {
        anyhow::bail!("Invalid key file: expected 32 bytes, got {}", bytes.len());
    }
    let seed: [u8; 32] = bytes.try_into().unwrap();
    Ok(SigningKey::from_bytes(&seed))
}

/// Generate a new Ed25519 key, print BIP-39 mnemonic, save to file with 0600 perms.
pub fn generate_and_save_key(key_path: &Path) -> anyhow::Result<SigningKey> {
    use rand::RngCore;

    // Generate 32 bytes of entropy (= Ed25519 seed)
    let mut seed = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut seed);

    let signing_key = SigningKey::from_bytes(&seed);

    // Generate BIP-39 mnemonic from the seed
    let mnemonic = bip39::Mnemonic::from_entropy(&seed)
        .map_err(|e| anyhow::anyhow!("BIP-39 error: {}", e))?;

    // Print mnemonic — shown only once
    println!();
    println!("╔══════════════════════════════════════════════════════════════╗");
    println!("║  WWS Identity Mnemonic — write this down, keep it offline   ║");
    println!("╠══════════════════════════════════════════════════════════════╣");
    println!("║                                                              ║");
    let words = mnemonic.to_string();
    // Print in 6-word rows
    let word_list: Vec<&str> = words.split_whitespace().collect();
    for chunk in word_list.chunks(6) {
        println!("║  {:<60}  ║", chunk.join(" "));
    }
    println!("║                                                              ║");
    println!("╠══════════════════════════════════════════════════════════════╣");
    println!("║  WARNING: Anyone with these words can control your identity ║");
    println!("║  WARNING: This is shown ONCE. It cannot be recovered.       ║");
    println!("╚══════════════════════════════════════════════════════════════╝");
    println!();

    // Save key file
    if let Some(parent) = key_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(key_path, &seed)?;

    // Set file permissions to 0600 on Unix
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o600);
        std::fs::set_permissions(key_path, perms)?;
    }

    tracing::info!(
        key_path = %key_path.display(),
        pubkey = %hex::encode(signing_key.verifying_key().as_bytes()),
        "Generated new Ed25519 identity key"
    );

    Ok(signing_key)
}

/// Compute the default key file path: ~/.config/wws-connector/<agent_name>.key
pub fn default_key_path(agent_name: &str) -> PathBuf {
    let base = dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("wws-connector");
    base.join(format!("{}.key", agent_name))
}

/// Derive a recovery verifying key from the primary seed.
/// Recovery key = Ed25519 key derived from SHA256("recovery:" + primary_seed).
pub fn recovery_pubkey(primary_seed: &[u8; 32]) -> ed25519_dalek::VerifyingKey {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(b"recovery:");
    hasher.update(primary_seed);
    let recovery_seed: [u8; 32] = hasher.finalize().into();
    SigningKey::from_bytes(&recovery_seed).verifying_key()
}

/// Hash of the recovery pubkey for DHT commitment (stored locally for now).
pub fn recovery_pubkey_hash(primary_seed: &[u8; 32]) -> String {
    use sha2::{Digest, Sha256};
    let rpk = recovery_pubkey(primary_seed);
    let mut hasher = Sha256::new();
    hasher.update(rpk.as_bytes());
    hex::encode(hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load_save_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.key");

        let key1 = generate_and_save_key(&path).unwrap();
        let key2 = load_key(&path).unwrap();

        assert_eq!(
            key1.verifying_key().as_bytes(),
            key2.verifying_key().as_bytes()
        );
    }

    #[test]
    fn test_recovery_pubkey_is_deterministic() {
        let seed = [42u8; 32];
        let rpk1 = recovery_pubkey(&seed);
        let rpk2 = recovery_pubkey(&seed);
        assert_eq!(rpk1.as_bytes(), rpk2.as_bytes());
    }

    #[test]
    fn test_recovery_pubkey_differs_from_primary() {
        let seed = [42u8; 32];
        let primary = SigningKey::from_bytes(&seed).verifying_key();
        let recovery = recovery_pubkey(&seed);
        assert_ne!(primary.as_bytes(), recovery.as_bytes());
    }
}
