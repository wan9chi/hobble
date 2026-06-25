use std::process::{Child, Command};

#[cfg(target_os = "macos")]
use hobble_sandbox_macos as hobble_sandbox_impl;

#[cfg(target_os = "linux")]
use hobble_sandbox_linux as hobble_sandbox_impl;

pub use hobble_sandbox_profile::SandboxProfile;

pub fn spawn_with_sandbox(
    command: Command,
    profile: &SandboxProfile,
) -> Result<Child, anyhow::Error> {
    hobble_sandbox_impl::spawn_with_sandbox(command, profile)
}
