use std::path::Path;

use crate::{
    profile::{Entry, PackageId},
    store,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Dev,
    Strict,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    Enforce(Entry),
    Observe,
    Fail(String),
}

pub fn decide(profile_dir: &Path, package: &PackageId, os: &str, mode: Mode) -> Decision {
    match store::load(profile_dir, &package.name, &package.version) {
        Ok(Some(profile)) => {
            if let Some(entry) = profile
                .entries
                .into_iter()
                .find(|entry| entry.os.iter().any(|entry_os| entry_os == os))
            {
                Decision::Enforce(entry)
            } else if mode == Mode::Strict {
                Decision::Fail(format!(
                    "no {os} profile entry for {}@{}",
                    package.name, package.version
                ))
            } else {
                Decision::Observe
            }
        }
        Ok(None) if mode == Mode::Strict => Decision::Fail(format!(
            "no profile for {}@{}",
            package.name, package.version
        )),
        Ok(None) => Decision::Observe,
        Err(error) => Decision::Fail(error.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        profile::{Entry, PackageId, Profile},
        store,
    };

    use super::*;

    #[test]
    fn branches_over_profile_presence_os_and_mode() {
        let temp = tempfile::tempdir().unwrap();
        let package = PackageId::new("pkg", "1.0.0");

        assert_eq!(
            decide(temp.path(), &package, "darwin", Mode::Dev),
            Decision::Observe
        );
        assert!(matches!(
            decide(temp.path(), &package, "darwin", Mode::Strict),
            Decision::Fail(_)
        ));

        let mut profile = Profile::new(package.clone(), "install");
        profile.entries.push(Entry::new("linux"));
        store::save(temp.path(), &profile).unwrap();

        assert_eq!(
            decide(temp.path(), &package, "darwin", Mode::Dev),
            Decision::Observe
        );
        assert!(matches!(
            decide(temp.path(), &package, "darwin", Mode::Strict),
            Decision::Fail(_)
        ));

        profile.entries.push(Entry::new("darwin"));
        store::save(temp.path(), &profile).unwrap();
        assert!(matches!(
            decide(temp.path(), &package, "darwin", Mode::Strict),
            Decision::Enforce(_)
        ));
    }
}
