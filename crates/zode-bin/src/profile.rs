use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::vault::{self, VaultPlaintext};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ProfileMeta {
    pub id: String,
    pub name: String,
    pub peer_id: String,
    pub did: String,
    pub created_at: u64,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct ProfilesManifest {
    profiles: Vec<ProfileMeta>,
}

pub(crate) fn base_dir() -> PathBuf {
    PathBuf::from(".zode")
}

fn manifest_path(base: &Path) -> PathBuf {
    base.join("profiles.json")
}

fn validate_profile_id(id: &str) -> Result<(), ProfileError> {
    if id.is_empty()
        || id.contains('/')
        || id.contains('\\')
        || id.contains("..")
        || id.contains('\0')
    {
        return Err(ProfileError::Io(format!(
            "invalid profile id: {id:?}"
        )));
    }
    Ok(())
}

fn profile_dir(base: &Path, id: &str) -> Result<PathBuf, ProfileError> {
    validate_profile_id(id)?;
    Ok(base.join("profiles").join(id))
}

fn vault_path(base: &Path, id: &str) -> Result<PathBuf, ProfileError> {
    Ok(profile_dir(base, id)?.join("vault.enc"))
}

pub(crate) fn data_dir_for_profile(base: &Path, id: &str) -> Result<PathBuf, ProfileError> {
    Ok(profile_dir(base, id)?.join("data"))
}

pub(crate) fn settings_path_for_profile(base: &Path, id: &str) -> Result<PathBuf, ProfileError> {
    Ok(profile_dir(base, id)?.join("settings.json"))
}

pub(crate) fn global_settings_path(base: &Path) -> PathBuf {
    base.join("settings.json")
}

fn load_manifest(base: &Path) -> ProfilesManifest {
    let p = manifest_path(base);
    if !p.exists() {
        return ProfilesManifest::default();
    }
    let data = match std::fs::read_to_string(&p) {
        Ok(d) => d,
        Err(_) => return ProfilesManifest::default(),
    };
    serde_json::from_str(&data).unwrap_or_default()
}

fn save_manifest(base: &Path, manifest: &ProfilesManifest) -> Result<(), ProfileError> {
    let p = manifest_path(base);
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).map_err(|e| ProfileError::Io(e.to_string()))?;
    }
    let json =
        serde_json::to_string_pretty(manifest).map_err(|e| ProfileError::Io(e.to_string()))?;
    std::fs::write(p, json).map_err(|e| ProfileError::Io(e.to_string()))
}

pub(crate) fn list_profiles(base: &Path) -> Vec<ProfileMeta> {
    load_manifest(base).profiles
}

pub(crate) struct CreateProfileParams {
    pub name: String,
    pub peer_id: String,
    pub did: String,
    pub plaintext: VaultPlaintext,
    pub password: String,
}

pub(crate) fn create_profile(
    base: &Path,
    params: CreateProfileParams,
) -> Result<ProfileMeta, ProfileError> {
    let id = format!("{:016x}", {
        use std::time::{SystemTime, UNIX_EPOCH};
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    });

    let dir = profile_dir(base, &id)?;
    std::fs::create_dir_all(&dir).map_err(|e| ProfileError::Io(e.to_string()))?;
    std::fs::create_dir_all(data_dir_for_profile(base, &id)?)
        .map_err(|e| ProfileError::Io(e.to_string()))?;

    let vault = vault::encrypt_vault(&params.plaintext, &params.password)
        .map_err(|e| ProfileError::Vault(e.to_string()))?;
    vault::save_vault(&vault_path(base, &id)?, &vault)
        .map_err(|e| ProfileError::Vault(e.to_string()))?;

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let meta = ProfileMeta {
        id: id.clone(),
        name: params.name,
        peer_id: params.peer_id,
        did: params.did,
        created_at: now,
    };

    let mut manifest = load_manifest(base);
    manifest.profiles.push(meta.clone());
    save_manifest(base, &manifest)?;

    Ok(meta)
}

pub(crate) fn unlock_profile(
    base: &Path,
    profile_id: &str,
    password: &str,
) -> Result<VaultPlaintext, ProfileError> {
    let vp = vault_path(base, profile_id)?;
    let vault = vault::load_vault(&vp).map_err(|e| ProfileError::Vault(e.to_string()))?;
    vault::decrypt_vault(&vault, password).map_err(|e| ProfileError::Vault(e.to_string()))
}

pub(crate) fn update_vault(
    base: &Path,
    profile_id: &str,
    plaintext: &VaultPlaintext,
    password: &str,
) -> Result<(), ProfileError> {
    let vault = vault::encrypt_vault(plaintext, password)
        .map_err(|e| ProfileError::Vault(e.to_string()))?;
    vault::save_vault(&vault_path(base, profile_id)?, &vault)
        .map_err(|e| ProfileError::Vault(e.to_string()))
}

pub(crate) fn delete_profile(base: &Path, profile_id: &str) -> Result<(), ProfileError> {
    let dir = profile_dir(base, profile_id)?;
    if dir.exists() {
        std::fs::remove_dir_all(&dir).map_err(|e| ProfileError::Io(e.to_string()))?;
    }

    let mut manifest = load_manifest(base);
    manifest.profiles.retain(|p| p.id != profile_id);
    save_manifest(base, &manifest)
}

#[derive(Debug)]
pub(crate) enum ProfileError {
    Io(String),
    Vault(String),
}

impl std::fmt::Display for ProfileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProfileError::Io(e) => write!(f, "I/O error: {e}"),
            ProfileError::Vault(e) => write!(f, "vault error: {e}"),
        }
    }
}

impl std::error::Error for ProfileError {}
