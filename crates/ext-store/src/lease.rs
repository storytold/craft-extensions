//! Leases let a reader keep using an installed release while the store replaces or removes it.
//!
//! A lease is a small json file under `leases/<id>/<version>/` that the holder keeps locked with a
//! file lock. The operating system releases the lock when the holder dies, so a crashed process
//! never keeps a release on disk forever: the next `is_leased` or `cleanup` sweep sees the unlocked
//! file as dead and removes it.

use crate::state::Installed;
use crate::store::{StoreError, unique_name};
use craft_ext_core::id::ExtensionId;
use craft_ext_core::version::Version;
use serde::{Deserialize, Serialize};
use std::fs::{self, File, TryLockError};
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Serialize, Deserialize)]
struct LeaseRecord {
    pid: u32,
    id: ExtensionId,
    version: Version,
    created_at: u64,
}

/// Holds one lease. Dropping it releases the lease file at once.
pub struct LeaseGuard {
    file: File,
    path: PathBuf,
}

impl Drop for LeaseGuard {
    fn drop(&mut self) {
        let _ = self.file.unlock();
        let _ = fs::remove_file(&self.path);
    }
}

pub(crate) fn acquire(root: &Path, id: &ExtensionId, version: Version) -> Result<LeaseGuard, StoreError> {
    let dir = root.join("leases").join(id.as_str()).join(version.to_string());
    fs::create_dir_all(&dir).map_err(|_| StoreError::Io)?;
    let path = dir.join(format!("{}.json", unique_name()));
    let record = LeaseRecord {
        pid: std::process::id(),
        id: id.clone(),
        version,
        created_at: SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0),
    };
    let bytes = serde_json::to_vec(&record).map_err(|_| StoreError::Io)?;
    let mut file = fs::OpenOptions::new().create(true).truncate(true).write(true).open(&path).map_err(|_| StoreError::Io)?;
    file.write_all(&bytes).map_err(|_| StoreError::Io)?;
    file.try_lock().map_err(|_| StoreError::Io)?;
    Ok(LeaseGuard { file, path })
}

/// True while at least one live lease file exists for that release. Dead lease files (their holder
/// is gone) are removed on the way, so they cannot pile up.
pub(crate) fn held(root: &Path, id: &ExtensionId, version: Version) -> bool {
    let dir = root.join("leases").join(id.as_str()).join(version.to_string());
    let Ok(entries) = fs::read_dir(&dir) else { return false };
    for entry in entries.flatten() {
        let Ok(file) = fs::OpenOptions::new().read(true).write(true).open(entry.path()) else { continue };
        match file.try_lock() {
            Err(TryLockError::WouldBlock) => return true,
            Ok(()) => {
                let _ = file.unlock();
                let _ = fs::remove_file(entry.path());
            }
            Err(TryLockError::Error(_)) => return true,
        }
    }
    false
}

/// Removes lease files whose holder is gone, and release directories that are neither the installed
/// version nor protected by a live lease.
pub(crate) fn cleanup(root: &Path, installed: &[Installed]) -> Result<(), StoreError> {
    let leases = root.join("leases");
    let mut live: Vec<(String, String)> = Vec::new();
    if let Ok(ids) = fs::read_dir(&leases) {
        for id_dir in ids.flatten() {
            let id_name = id_dir.file_name().to_string_lossy().into_owned();
            if let Ok(vers) = fs::read_dir(id_dir.path()) {
                for ver_dir in vers.flatten() {
                    let ver_name = ver_dir.file_name().to_string_lossy().into_owned();
                    let mut any_live = false;
                    let mut unknown = false;
                    if let Ok(files) = fs::read_dir(ver_dir.path()) {
                        for lease_file in files.flatten() {
                            let Ok(file) = fs::OpenOptions::new().read(true).write(true).open(lease_file.path()) else {
                                unknown = true;
                                continue;
                            };
                            match file.try_lock() {
                                Err(TryLockError::WouldBlock) => any_live = true,
                                Ok(()) => {
                                    let _ = file.unlock();
                                    let _ = fs::remove_file(lease_file.path());
                                }
                                Err(TryLockError::Error(_)) => unknown = true,
                            }
                        }
                    }
                    if any_live || unknown {
                        live.push((id_name.clone(), ver_name.clone()));
                    }
                    let _ = fs::remove_dir(ver_dir.path());
                }
            }
        }
    }
    if let Ok(ids) = fs::read_dir(&leases) {
        for id_dir in ids.flatten() {
            let _ = fs::remove_dir(id_dir.path());
        }
    }
    let _ = fs::remove_dir(&leases);
    if let Ok(packages) = fs::read_dir(root.join("packages")) {
        for package in packages.flatten() {
            let id_name = package.file_name().to_string_lossy().into_owned();
            let installed_version = installed.iter().find(|i| i.id.as_str() == id_name).map(|i| i.version.to_string());
            if let Ok(vers) = fs::read_dir(package.path()) {
                for ver_dir in vers.flatten() {
                    let ver_name = ver_dir.file_name().to_string_lossy().into_owned();
                    if installed_version.as_deref() == Some(ver_name.as_str()) {
                        continue;
                    }
                    if live.iter().any(|(i, v)| i == &id_name && v == &ver_name) {
                        continue;
                    }
                    let _ = fs::remove_dir_all(ver_dir.path());
                }
            }
            let _ = fs::remove_dir(package.path());
        }
    }
    Ok(())
}
