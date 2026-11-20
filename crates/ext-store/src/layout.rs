use crate::state::Installed;
use craft_ext_core::id::ExtensionId;
use craft_ext_core::version::Version;
use std::path::{Path, PathBuf};

pub fn release_dir(root: &Path, id: &ExtensionId, version: Version, sha256: &str) -> PathBuf {
    root.join("packages").join(id.as_str()).join(format!("{version}-{sha256}"))
}

pub fn legacy_dir(root: &Path, id: &ExtensionId, version: Version) -> PathBuf {
    root.join("packages").join(id.as_str()).join(version.to_string())
}

pub fn installed_dir(root: &Path, i: &Installed) -> PathBuf {
    let release = release_dir(root, &i.id, i.version, &i.sha256);
    if !i.sha256.is_empty() && release.is_dir() { release } else { legacy_dir(root, &i.id, i.version) }
}
