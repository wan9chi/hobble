#![cfg(target_os = "linux")]

use std::{
    env,
    ffi::{OsStr, OsString},
    fs, io,
    os::unix::{ffi::OsStrExt, process::CommandExt},
    path::{Path, PathBuf},
    process::{Child, Command},
};

use anyhow::{Context, Result};
use hobble_sandbox_profile::SandboxProfile;
use landlock::{
    ABI, Access, AccessFs, BitFlags, CompatLevel, Compatible, PathBeneath, PathFd, Ruleset,
    RulesetAttr, RulesetCreated, RulesetCreatedAttr, RulesetStatus,
};

pub fn spawn_with_sandbox(
    mut command: Command,
    profile: &SandboxProfile,
) -> Result<Child, anyhow::Error> {
    let executable_path = resolve_program_path(&command);
    let ruleset = build_ruleset(&profile.allowed_paths, executable_path.as_deref())?;
    let mut ruleset = Some(ruleset);

    // SAFETY: The closure applies the already-created Landlock ruleset in
    // the child before exec.
    unsafe {
        command.pre_exec(move || {
            let ruleset = ruleset
                .take()
                .ok_or_else(|| io::Error::other("Landlock ruleset already applied"))?;
            apply_ruleset(ruleset)
        });
    }

    command
        .spawn()
        .context("failed to spawn command with Linux sandbox")
}

fn build_ruleset(
    allowed_paths: &[PathBuf],
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

    for path in allowed_paths {
        anyhow::ensure!(
            path.is_absolute(),
            "sandbox paths must be absolute: {}",
            path.display()
        );
        ruleset = add_path_rule(
            ruleset,
            path,
            read_write_dir_access,
            read_write_file_access,
            true,
        )
        .with_context(|| format!("failed to allow sandbox path {}", path.display()))?;
    }

    Ok(ruleset)
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

fn resolve_program_path(command: &Command) -> Option<PathBuf> {
    let program = Path::new(command.get_program());
    if program.is_absolute() {
        return Some(program.to_path_buf());
    }

    if program.as_os_str().as_bytes().contains(&b'/') {
        let base = command
            .get_current_dir()
            .map(Path::to_path_buf)
            .or_else(|| env::current_dir().ok())?;
        return Some(base.join(program));
    }

    let path = command_path_env(command)?;
    env::split_paths(&path)
        .map(|path_dir| path_dir.join(program))
        .find(|candidate| candidate.exists())
}

fn command_path_env(command: &Command) -> Option<OsString> {
    let mut path = env::var_os("PATH");
    for (key, value) in command.get_envs() {
        if key == OsStr::new("PATH") {
            path = value.map(OsString::from);
        }
    }
    path
}
