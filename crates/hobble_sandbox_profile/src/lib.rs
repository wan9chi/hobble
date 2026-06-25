use std::path::PathBuf;

#[derive(Default, Debug, Clone)]
pub struct SandboxProfile {
    pub allowed_paths: Vec<PathBuf>,
}
