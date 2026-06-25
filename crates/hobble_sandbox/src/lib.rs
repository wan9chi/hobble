use std::{
    ffi::OsStr,
    process::{Child, Command},
};

#[cfg(target_os = "macos")]
use hobble_sandbox_macos as hobble_sandbox_impl;

#[cfg(target_os = "linux")]
use hobble_sandbox_linux as hobble_sandbox_impl;

#[derive(Default, Debug)]
pub struct SandboxBuilder(hobble_sandbox_impl::SandboxBuilder);

impl SandboxBuilder {
    // Allow a path to be accessed by the sandboxed process.
    // If the path is a directory, all files and directories under it will be allowed.
    pub fn allow_path(&mut self, path: &OsStr) -> Result<&mut Self, anyhow::Error> {
        self.0.allow_path(path)?;
        Ok(self)
    }

    pub fn spawn(self, command: Command) -> Result<Child, anyhow::Error> {
        self.0.spawn(command)
    }
}
