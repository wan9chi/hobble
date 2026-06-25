#![cfg(target_os = "macos")]

use std::{
    env,
    ffi::{CString, OsStr, OsString},
    fs, io,
    os::{
        raw::{c_char, c_int},
        unix::{ffi::OsStrExt, process::CommandExt},
    },
    path::{Path, PathBuf},
    process::{Child, Command},
    ptr,
};

use anyhow::{Context, Result};

#[link(name = "sandbox")]
unsafe extern "C" {
    fn sandbox_init_with_parameters(
        profile: *const c_char,
        flags: u64,
        parameters: *const *const c_char,
        errorbuf: *mut *mut c_char,
    ) -> c_int;
}

#[derive(Debug)]
pub struct SandboxBuilder {
    profile: String,
    parameters: String,
    next_parameter_id: usize,
}

impl Default for SandboxBuilder {
    fn default() -> Self {
        Self {
            profile: String::from(
                r#"(version 1)
(deny file-read* file-write*)
(import "system.sb")
"#,
            ),
            parameters: String::new(),
            next_parameter_id: 0,
        }
    }
}

impl SandboxBuilder {
    // Allow a path to be accessed by the sandboxed process.
    // If the path is a directory, all files and directories under it will be allowed.
    pub fn allow_path(&mut self, path: &OsStr) -> Result<&mut Self> {
        let path = Path::new(path);
        anyhow::ensure!(path.is_absolute(), "sandbox paths must be absolute");

        let metadata = fs::metadata(path)
            .with_context(|| format!("failed to inspect sandbox path {}", path.display()))?;
        let filter = if metadata.is_dir() {
            "subpath"
        } else {
            "literal"
        };
        let name = self.push_path_parameter("hobble_allow", path)?;

        self.profile.push_str(&format!(
            "(allow file-read* file-write* ({filter} (param \"{name}\")))\n"
        ));

        Ok(self)
    }

    pub fn spawn(mut self, mut command: Command) -> Result<Child, anyhow::Error> {
        let executable_path = resolve_program_path(&command);
        if let Some(executable_path) = executable_path {
            self.allow_command_executable(&executable_path)?;
        }

        let profile = CString::new(self.profile).context("sandbox profile contains a null byte")?;
        let parameter_strings = parameter_cstrings(&self.parameters)?;
        let mut parameter_ptrs = parameter_strings
            .iter()
            .map(|parameter| parameter.as_ptr() as usize)
            .collect::<Vec<_>>();
        parameter_ptrs.push(ptr::null::<c_char>() as usize);
        let sandbox = (profile, parameter_strings, parameter_ptrs);

        // SAFETY: The closure only uses data prepared before fork and calls the
        // platform sandbox entry point in the child before exec.
        unsafe {
            command.pre_exec(move || {
                let _keep_parameters_alive = &sandbox.1;
                apply_sandbox(&sandbox.0, &sandbox.2)
            });
        }

        command
            .spawn()
            .context("failed to spawn command with macOS sandbox")
    }

    fn allow_command_executable(&mut self, executable_path: &Path) -> Result<()> {
        let name = self.push_path_parameter("hobble_executable", executable_path)?;
        self.profile.push_str(&format!(
            "(allow process-exec* (literal (param \"{name}\")))\n"
        ));
        Ok(())
    }

    fn push_path_parameter(&mut self, prefix: &str, path: &Path) -> Result<String> {
        let name = format!("{prefix}_{}", self.next_parameter_id);
        self.next_parameter_id += 1;
        let value = path
            .to_str()
            .with_context(|| format!("sandbox path is not valid UTF-8: {}", path.display()))?
            .to_owned();
        self.push_parameter(&name, &value)?;
        Ok(name)
    }

    fn push_parameter(&mut self, name: &str, value: &str) -> Result<()> {
        anyhow::ensure!(!name.is_empty(), "sandbox parameter name must not be empty");
        anyhow::ensure!(
            !name.contains('\0'),
            "sandbox parameter name contains a null byte"
        );
        anyhow::ensure!(
            !value.contains('\0'),
            "sandbox parameter value contains a null byte"
        );

        self.parameters.push_str(name);
        self.parameters.push('\0');
        self.parameters.push_str(value);
        self.parameters.push('\0');
        Ok(())
    }
}

fn apply_sandbox(profile: &CString, parameter_ptrs: &[usize]) -> io::Result<()> {
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
        Err(io::Error::other("sandbox_init_with_parameters failed"))
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
