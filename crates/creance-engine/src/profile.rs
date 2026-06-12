use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackageId {
    pub name: String,
    pub version: String,
}

impl PackageId {
    pub fn new(name: impl Into<String>, version: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            version: version.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Profile {
    pub package: PackageId,
    pub creance: String,
    pub lifecycle: String,
    pub entries: Vec<Entry>,
}

impl Profile {
    pub fn new(package: PackageId, lifecycle: impl Into<String>) -> Self {
        Self {
            package,
            creance: env!("CARGO_PKG_VERSION").to_string(),
            lifecycle: lifecycle.into(),
            entries: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub os: Vec<String>,
    pub read: Vec<String>,
    pub write: Vec<String>,
    pub domains: Vec<String>,
}

impl Entry {
    pub fn new(os: impl Into<String>) -> Self {
        Self {
            os: vec![os.into()],
            read: Vec::new(),
            write: Vec::new(),
            domains: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_round_trips_with_stable_shape() {
        let mut profile = Profile::new(PackageId::new("esbuild", "0.21.5"), "postinstall");
        profile.entries.push(Entry {
            os: vec!["darwin".to_string()],
            read: vec!["${PKG_DIR}/**".to_string()],
            write: vec!["${PKG_DIR}/bin/**".to_string()],
            domains: vec!["registry.npmjs.org".to_string()],
        });

        let json = serde_json::to_string_pretty(&profile).unwrap();
        assert!(json.contains("\"package\""));
        assert!(json.contains("\"entries\""));
        assert_eq!(serde_json::from_str::<Profile>(&json).unwrap(), profile);
    }
}
