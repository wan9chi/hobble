#![cfg(target_os = "macos")]

use std::{
    ffi::{CStr, CString},
    io,
    os::{
        raw::{c_char, c_int},
        unix::process::CommandExt,
    },
    path::{Path, PathBuf},
    process::{Child, Command},
    ptr,
};

use anyhow::{Context, Result};
use hobble_sandbox_profile::{SandboxProfile, resolve_entries, resolve_program_path};

#[link(name = "sandbox")]
unsafe extern "C" {
    fn sandbox_init_with_parameters(
        profile: *const c_char,
        flags: u64,
        parameters: *const *const c_char,
        errorbuf: *mut *mut c_char,
    ) -> c_int;

    fn sandbox_free_error(errorbuf: *mut c_char);
}

pub fn spawn_with_sandbox(
    mut command: Command,
    profile: &SandboxProfile,
) -> Result<Child, anyhow::Error> {
    let executable_path = resolve_program_path(&command);
    let (profile, parameter_strings, parameter_ptrs) =
        compile_profile(profile, executable_path.as_deref())?;
    let sandbox = (profile, parameter_strings, parameter_ptrs);

    // SAFETY: The closure only uses data prepared before fork and calls the
    // platform sandbox entry point in the child before exec.
    unsafe {
        command.pre_exec(move || {
            let _keep_parameters_alive = &sandbox.1;
            apply_sandbox(&sandbox.0, &sandbox.2).map_err(io::Error::other)
        });
    }

    command
        .spawn()
        .context("failed to spawn command with macOS sandbox")
}

fn compile_profile(
    profile: &SandboxProfile,
    executable_path: Option<&Path>,
) -> Result<(CString, Vec<CString>, Vec<usize>)> {
    // `system.sb` supplies the OS baseline (dyld, /System, /usr/lib,
    // /dev/null and friends); the runtime roots mirror the Linux backend so
    // shells and system tools work without profile entries.
    // `(deny process-exec*)` also catches `process-fork`, which creates no
    // new code image, so fork is re-allowed explicitly.
    let mut profile_source = String::from(
        r#"(version 1)
(deny process-exec* file-read* file-write*)
(import "system.sb")
(allow process-fork)
(allow file-read-metadata)
(allow file-read* process-exec* (subpath "/bin") (subpath "/sbin") (subpath "/usr") (subpath "/opt/homebrew"))
"#,
    );
    let mut parameters = String::new();
    let mut next_parameter_id = 0;

    if let Some(executable_path) = executable_path {
        let name = push_path_parameter(
            &mut parameters,
            &mut next_parameter_id,
            "hobble_exec",
            executable_path,
        )?;
        profile_source.push_str(&format!(
            "(allow file-read* process-exec* (literal (param \"{name}\")))\n"
        ));
    }

    push_entry_rules(
        &mut profile_source,
        &mut parameters,
        &mut next_parameter_id,
        "hobble_read",
        "file-read* process-exec*",
        &profile.allowed_reads,
    )?;
    push_entry_rules(
        &mut profile_source,
        &mut parameters,
        &mut next_parameter_id,
        "hobble_write",
        "file-read* file-write* process-exec*",
        &profile.allowed_writes,
    )?;

    let profile = CString::new(profile_source).context("sandbox profile contains a null byte")?;
    let parameter_strings = parameter_cstrings(&parameters)?;
    let mut parameter_ptrs = parameter_strings
        .iter()
        .map(|parameter| parameter.as_ptr() as usize)
        .collect::<Vec<_>>();
    parameter_ptrs.push(ptr::null::<c_char>() as usize);

    Ok((profile, parameter_strings, parameter_ptrs))
}

fn push_path_parameter(
    parameters: &mut String,
    next_parameter_id: &mut usize,
    prefix: &str,
    path: &Path,
) -> Result<String> {
    anyhow::ensure!(
        path.is_absolute(),
        "sandbox paths must be absolute: {}",
        path.display()
    );
    let name = format!("{prefix}_{}", *next_parameter_id);
    *next_parameter_id += 1;
    let value = path
        .to_str()
        .with_context(|| format!("sandbox path is not valid UTF-8: {}", path.display()))?
        .to_owned();
    push_parameter(parameters, &name, &value)?;
    Ok(name)
}

fn push_entry_rules(
    profile_source: &mut String,
    parameters: &mut String,
    next_parameter_id: &mut usize,
    prefix: &str,
    operations: &str,
    entries: &[PathBuf],
) -> Result<()> {
    for entry in resolve_entries(entries)? {
        for path in entry.paths() {
            let name = push_path_parameter(parameters, next_parameter_id, prefix, path)?;
            profile_source
                .push_str(&format!("(allow {operations} (literal (param \"{name}\")))\n"));
            if entry.is_dir {
                profile_source
                    .push_str(&format!("(allow {operations} (subpath (param \"{name}\")))\n"));
            }
        }
    }
    Ok(())
}

fn push_parameter(parameters: &mut String, name: &str, value: &str) -> Result<()> {
    anyhow::ensure!(!name.is_empty(), "sandbox parameter name must not be empty");
    anyhow::ensure!(
        !name.contains('\0'),
        "sandbox parameter name contains a null byte"
    );
    anyhow::ensure!(
        !value.contains('\0'),
        "sandbox parameter value contains a null byte"
    );

    parameters.push_str(name);
    parameters.push('\0');
    parameters.push_str(value);
    parameters.push('\0');
    Ok(())
}

fn apply_sandbox(profile: &CString, parameter_ptrs: &[usize]) -> Result<()> {
    let mut errorbuf = ptr::null_mut();
    let result = unsafe {
        sandbox_init_with_parameters(
            profile.as_ptr(),
            0,
            parameter_ptrs.as_ptr().cast::<*const c_char>(),
            &mut errorbuf,
        )
    };

    if result == 0 {
        Ok(())
    } else {
        let message = if errorbuf.is_null() {
            "sandbox_init_with_parameters failed".to_string()
        } else {
            let message = unsafe { CStr::from_ptr(errorbuf) }
                .to_string_lossy()
                .into_owned();
            unsafe { sandbox_free_error(errorbuf) };
            format!("sandbox_init_with_parameters failed: {message}")
        };
        anyhow::bail!(message)
    }
}

fn parameter_cstrings(parameters: &str) -> Result<Vec<CString>> {
    if parameters.is_empty() {
        return Ok(Vec::new());
    }

    let parameters = parameters
        .strip_suffix('\0')
        .context("sandbox parameter buffer is missing a null terminator")?;
    let mut cstrings = Vec::new();
    for parameter in parameters.split('\0') {
        anyhow::ensure!(
            !parameter.is_empty(),
            "sandbox parameter buffer contains an empty segment"
        );
        cstrings.push(CString::new(parameter).context("sandbox parameter contains a null byte")?);
    }
    anyhow::ensure!(
        cstrings.len() % 2 == 0,
        "sandbox parameter buffer must contain name/value pairs"
    );
    Ok(cstrings)
}
