#![cfg(target_os = "linux")]

use std::{
    fs, io,
    os::unix::process::CommandExt,
    path::Path,
    process::{Child, Command},
};

use anyhow::{Context, Result};
use hobble_sandbox_profile::{
    ResolvedEntry, SandboxProfile, resolve_entries, resolve_program_path,
};
use landlock::{
    ABI, Access, AccessFs, BitFlags, CompatLevel, Compatible, PathBeneath, PathFd, Ruleset,
    RulesetAttr, RulesetCreated, RulesetCreatedAttr, RulesetStatus,
};

pub fn spawn_with_sandbox(
    mut command: Command,
    profile: &SandboxProfile,
) -> Result<Child, anyhow::Error> {
    let executable_path = resolve_program_path(&command);
    let ruleset = build_ruleset(profile, executable_path.as_deref())?;
    let mut ruleset = Some(ruleset);

    // SAFETY: The closure applies the already-created Landlock ruleset in
    // the child before exec.
    unsafe {
        command.pre_exec(move || {
            let ruleset = ruleset
                .take()
                .ok_or_else(|| io::Error::other("Landlock ruleset already applied"))?;
            let ruleset = grant_proc_self(ruleset)?;
            apply_ruleset(ruleset)
        });
    }

    command
        .spawn()
        .context("failed to spawn command with Linux sandbox")
}

fn build_ruleset(
    profile: &SandboxProfile,
    executable_path: Option<&Path>,
) -> Result<RulesetCreated> {
    let abi = ABI::V1;
    let read_exec_dir_access = AccessFs::from_read(abi);
    let read_exec_file_access = AccessFs::from_file(abi) & read_exec_dir_access;
    let read_write_dir_access = AccessFs::from_all(abi);
    let read_write_file_access = AccessFs::from_file(abi);

    let mut ruleset = Ruleset::default()
        .set_compatibility(CompatLevel::HardRequirement)
        .handle_access(AccessFs::from_all(abi))?
        .create()?;

    for path in runtime_roots() {
        ruleset = add_path_rule(
            ruleset,
            Path::new(path),
            read_exec_dir_access,
            read_exec_file_access,
            false,
        )?;
    }

    for path in device_nodes() {
        ruleset = add_path_rule(
            ruleset,
            Path::new(path),
            read_write_dir_access,
            read_write_file_access,
            false,
        )?;
    }

    // Global, non-per-process `/proc` info files. Per-process access is
    // handled separately: the child's own `/proc/self` is granted in
    // `grant_proc_self` (see below), and other processes' directories stay
    // denied so their `environ`/`cmdline` are unreadable.
    for path in proc_info_files() {
        ruleset = add_path_rule(
            ruleset,
            Path::new(path),
            read_exec_dir_access,
            read_exec_file_access,
            false,
        )?;
    }

    if let Some(executable_path) = executable_path {
        ruleset = add_path_rule(
            ruleset,
            executable_path,
            read_exec_dir_access,
            read_exec_file_access,
            true,
        )
        .with_context(|| {
            format!(
                "failed to allow command executable {}",
                executable_path.display()
            )
        })?;
    }

    for entry in resolve_entries(&profile.allowed_reads)? {
        ruleset = add_entry_rule(ruleset, &entry, read_exec_dir_access, read_exec_file_access)?;
    }

    for entry in resolve_entries(&profile.allowed_writes)? {
        ruleset = add_entry_rule(ruleset, &entry, read_write_dir_access, read_write_file_access)?;
    }

    Ok(ruleset)
}

// Landlock rules attach to inodes, so opening the literal path (which follows
// symlinks) covers the resolved spelling as well.
fn add_entry_rule(
    ruleset: RulesetCreated,
    entry: &ResolvedEntry,
    dir_access: BitFlags<AccessFs>,
    file_access: BitFlags<AccessFs>,
) -> Result<RulesetCreated> {
    let access = if entry.is_dir { dir_access } else { file_access };
    let path_fd = PathFd::new(&entry.literal)
        .with_context(|| format!("failed to allow sandbox path {}", entry.literal.display()))?;
    Ok(ruleset.add_rule(PathBeneath::new(path_fd, access))?)
}

fn add_path_rule(
    ruleset: RulesetCreated,
    path: &Path,
    dir_access: BitFlags<AccessFs>,
    file_access: BitFlags<AccessFs>,
    required: bool,
) -> Result<RulesetCreated> {
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if !required && error.kind() == io::ErrorKind::NotFound => return Ok(ruleset),
        Err(error) => return Err(error).with_context(|| path.display().to_string()),
    };

    let access = if metadata.is_dir() {
        dir_access
    } else {
        file_access
    };
    let path_fd = PathFd::new(path)?;
    Ok(ruleset.add_rule(PathBeneath::new(path_fd, access))?)
}

/// Grants the calling process read access to its own `/proc/self` subtree.
///
/// Runs in the child after fork, where `/proc/self` resolves to the child's
/// own per-process directory. `/proc/self` cannot be granted from the parent
/// because Landlock pins the resolved inode, which would be the parent's
/// directory. Processes the child later spawns get their own per-process
/// directory that this frozen ruleset does not cover, so they cannot read
/// their `/proc/self`.
fn grant_proc_self(ruleset: RulesetCreated) -> io::Result<RulesetCreated> {
    let access = AccessFs::from_read(ABI::V1);
    let Ok(path_fd) = PathFd::new("/proc/self") else {
        // `/proc` is not mounted; nothing to grant.
        return Ok(ruleset);
    };
    ruleset
        .add_rule(PathBeneath::new(path_fd, access))
        .map_err(|error| io::Error::other(error.to_string()))
}

fn apply_ruleset(ruleset: RulesetCreated) -> io::Result<()> {
    let status = ruleset
        .restrict_self()
        .map_err(|error| io::Error::other(error.to_string()))?;

    if status.ruleset != RulesetStatus::FullyEnforced {
        return Err(io::Error::other(format!(
            "Landlock ruleset was not fully enforced: {:?}",
            status.ruleset
        )));
    }

    if !status.no_new_privs {
        return Err(io::Error::other("Landlock did not set no_new_privs"));
    }

    Ok(())
}

fn runtime_roots() -> &'static [&'static str] {
    &["/bin", "/sbin", "/lib", "/lib64", "/usr", "/etc"]
}

fn device_nodes() -> &'static [&'static str] {
    &["/dev/null", "/dev/zero", "/dev/random", "/dev/urandom"]
}

fn proc_info_files() -> &'static [&'static str] {
    &[
        "/proc/cpuinfo",
        "/proc/stat",
        "/proc/meminfo",
        "/proc/loadavg",
        "/proc/uptime",
    ]
}
