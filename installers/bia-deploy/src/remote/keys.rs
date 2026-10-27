//! SSH keys for people who have none: an ed25519 key made here, kept in this app's folder,
//! whose public half goes to the hosting provider when the server is ordered.
use anyhow::{anyhow, Context, Result};
use russh::keys::ssh_key::private::{Ed25519Keypair, KeypairData};
use russh::keys::ssh_key::{LineEnding, PrivateKey};
use std::path::PathBuf;

pub struct NewKey {
    /// OpenSSH private key (no passphrase: it is in this computer's app folder).
    pub private: String,
    /// `ssh-ed25519 AAAA… comment`: what the provider asks for.
    pub public: String,
    /// Where both were saved (`<path>` and `<path>.pub`).
    pub path: PathBuf,
}

pub fn generate(comment: &str) -> Result<NewKey> {
    let mut seed = [0u8; 32];
    getrandom::getrandom(&mut seed).map_err(|e| anyhow!("random: {e}"))?;
    let key = PrivateKey::new(KeypairData::from(Ed25519Keypair::from_seed(&seed)), comment)?;
    let private = key.to_openssh(LineEnding::LF)?.to_string();
    let public = key.public_key().to_openssh()?;
    let dir = crate::app_dir().context("no folder for this app's settings")?.join("keys");
    std::fs::create_dir_all(&dir)?;
    let mut path = dir.join("bia-deploy");
    let mut n = 1;
    while path.exists() {
        n += 1;
        path = dir.join(format!("bia-deploy-{n}"));
    }
    write_private(&path, private.as_bytes())?;
    std::fs::write(path.with_extension("pub"), format!("{public}\n"))?;
    Ok(NewKey { private, public, path })
}

/// The keys made earlier, newest first: (private key path, public line).
pub fn saved() -> Vec<(PathBuf, String)> {
    let Some(dir) = crate::app_dir().map(|d| d.join("keys")) else { return vec![] };
    let mut v: Vec<(std::time::SystemTime, PathBuf, String)> = std::fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().extension().is_none())
        .filter_map(|e| {
            let p = e.path();
            let public = std::fs::read_to_string(p.with_extension("pub")).ok()?.trim().to_string();
            Some((e.metadata().and_then(|m| m.modified()).ok()?, p, public))
        })
        .collect();
    v.sort_by(|a, b| b.0.cmp(&a.0));
    v.into_iter().map(|(_, p, k)| (p, k)).collect()
}

/// The public line of a private key (OpenSSH, PEM or .ppk), to check what was pasted.
pub fn public_of(pem: &str, passphrase: Option<&str>) -> Result<String> {
    let k = russh::keys::decode_secret_key(pem.trim(), passphrase.filter(|p| !p.is_empty()))?;
    Ok(k.public_key().to_openssh()?)
}

fn write_private(path: &std::path::Path, data: &[u8]) -> Result<()> {
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(path)?;
        f.write_all(data)?;
        Ok(())
    }
    #[cfg(not(unix))]
    {
        std::fs::write(path, data)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn generated_key_reads_back() {
        let dir = std::env::temp_dir().join(format!("bia-deploy-keys-{}", std::process::id()));
        std::env::set_var("BIA_DEPLOY_HOME", &dir);
        let k = super::generate("bia-deploy test").unwrap();
        assert!(k.public.starts_with("ssh-ed25519 AAAA"));
        assert!(k.public.ends_with("bia-deploy test"));
        assert_eq!(super::public_of(&k.private, None).unwrap(), k.public);
        assert_eq!(super::saved()[0].1, k.public);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
