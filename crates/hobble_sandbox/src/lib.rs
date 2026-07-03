//! Platform sandbox facade.
//!
//! Beyond the paths granted by the [`SandboxProfile`], every platform backend
//! ships a built-in default allowlist so ordinary process startup (shells,
//! dynamic loader, system libraries) works without per-profile boilerplate:
//!
//! - macOS: Apple's `system.sb` profile is imported (OS essentials such as
//!   system libraries, the dynamic loader, and the basic device nodes), the
//!   runtime roots `/bin`, `/sbin`, `/usr`, and `/opt/homebrew` are readable
//!   and executable, and path metadata (`stat`) is readable everywhere.
//! - Linux: the runtime roots `/bin`, `/sbin`, `/lib`, `/lib64`, `/usr`, and
//!   `/etc` are readable and executable, and the device nodes `/dev/null`,
//!   `/dev/zero`, `/dev/random`, and `/dev/urandom` are read-write.
//!
//! Process exec is confined to the same policy: the runtime roots, the
//! profile entries (reads imply exec), and the spawned command's resolved
//! executable, which is always granted read and exec so the command can
//! start.
//!
//! The default allowlist is a read-only baseline (plus device nodes); it
//! never grants write access to user data. Everything outside it and the
//! profile is denied.

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
