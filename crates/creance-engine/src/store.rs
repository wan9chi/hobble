use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

use anyhow::{Context as _, Result};

use crate::profile::{Entry, PackageId, Profile};

pub fn profile_path(dir: &Path, package: &PackageId) -> PathBuf {
    dir.join("profiles")
        .join(&package.name)
        .join(format!("{}.json", package.version))
}

pub fn load(dir: &Path, name: &str, version: &str) -> Result<Option<Profile>> {
    let package = PackageId::new(name, version);
    let path = profile_path(dir, &package);
    if !path.exists() {
        return Ok(None);
    }
    let bytes = std::fs::read(&path).with_context(|| format!("read {}", path.display()))?;
    serde_json::from_slice(&bytes)
        .with_context(|| format!("parse {}", path.display()))
        .map(Some)
}

pub fn upsert_entry(profile: &mut Profile, entry: Entry) {
    if let Some(existing) = profile
        .entries
        .iter_mut()
        .find(|existing| existing.os == entry.os)
    {
        existing.read = merge_sorted(&existing.read, &entry.read);
        existing.write = merge_sorted(&existing.write, &entry.write);
        existing.domains = merge_sorted(&existing.domains, &entry.domains);
    } else {
        profile.entries.push(entry);
    }
    profile
        .entries
        .sort_by(|left, right| left.os.cmp(&right.os));
}

fn merge_sorted(left: &[String], right: &[String]) -> Vec<String> {
    left.iter()
        .chain(right)
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

pub fn save(dir: &Path, profile: &Profile) -> Result<PathBuf> {
    let path = profile_path(dir, &profile.package);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create profile dir {}", parent.display()))?;
    }
    let json = serde_json::to_vec_pretty(profile)?;
    std::fs::write(&path, json).with_context(|| format!("write {}", path.display()))?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use crate::profile::{Entry, PackageId, Profile};

    use super::*;

    #[test]
    fn save_load_and_upsert_entries_by_os() {
        let temp = tempfile::tempdir().unwrap();
        let mut profile = Profile::new(PackageId::new("pkg", "1.0.0"), "postinstall");

        let mut darwin = Entry::new("darwin");
        darwin.read = vec!["${PKG_DIR}/**".to_string()];
        upsert_entry(&mut profile, darwin.clone());

        let linux = Entry::new("linux");
        upsert_entry(&mut profile, linux);
        assert_eq!(profile.entries.len(), 2);

        darwin.read = vec!["${PROJECT_ROOT}/**".to_string()];
        upsert_entry(&mut profile, darwin.clone());
        assert_eq!(profile.entries.len(), 2);
        assert_eq!(
            profile.entries[0].read,
            vec!["${PKG_DIR}/**", "${PROJECT_ROOT}/**"]
        );

        save(temp.path(), &profile).unwrap();
        assert_eq!(load(temp.path(), "pkg", "1.0.0").unwrap(), Some(profile));
    }
}
