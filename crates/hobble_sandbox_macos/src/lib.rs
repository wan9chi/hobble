#![cfg(target_os = "macos")]

use std::{
    env,
    ffi::{CString, OsStr, OsString},
    io,
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

#[derive(Default, Debug)]
pub struct SandboxBuilder {
    allowed_paths: Vec<PathBuf>,
}

impl SandboxBuilder {
    // Allow a path to be accessed by the sandboxed process.
    // If the path is a directory, all files and directories under it will be allowed.
    // # Panics
    // Panics if the path is not absolute.
    pub fn allow_path(&mut self, path: &OsStr) -> &mut Self {
        let path = Path::new(path);
        assert!(path.is_absolute(), "sandbox paths must be absolute");
        self.allowed_paths.push(path.to_path_buf());
        self
    }

    pub fn spawn(self, mut command: Command) -> Result<Child, anyhow::Error> {
        let executable_path = resolve_program_path(&command);
        let sandbox = PreparedSandbox::new(&self.allowed_paths, executable_path.as_deref())?;

        // SAFETY: The closure only uses data prepared before fork and calls the
        // platform sandbox entry point in the child before exec.
        unsafe {
            command.pre_exec(move || sandbox.apply());
        }

        command
            .spawn()
            .context("failed to spawn command with macOS sandbox")
    }
}

#[derive(Debug)]
struct PreparedSandbox {
    profile: CString,
    // Owns the name/value C strings referenced by `parameter_ptrs`. These are
    // passed to sandbox_init_with_parameters, not through the child environment.
    _parameter_strings: Vec<CString>,
    parameter_ptrs: Vec<usize>,
}

impl PreparedSandbox {
    fn new(allowed_paths: &[PathBuf], executable_path: Option<&Path>) -> Result<Self> {
        let mut profile = String::from(
            r#"(version 1)
(allow default)
(deny file-read* file-write*)
(allow file-read-metadata)
(import "system.sb")
"#,
        );

        let mut parameters = Vec::new();

        if let Some(home) = env::var_os("HOME") {
            let home = PathBuf::from(home);
            push_parameter(
                &mut parameters,
                "hobble_home_text_encoding",
                &home.join(".CFUserTextEncoding"),
            )?;
            push_parameter(
                &mut parameters,
                "hobble_home_preferences",
                &home.join("Library").join("Preferences"),
            )?;
            profile.push_str(
                r#"
(allow file-read*
    (literal (param "hobble_home_text_encoding"))
    (subpath (param "hobble_home_preferences")))
"#,
            );
        }

        if let Some(executable_path) = executable_path {
            let name = "hobble_executable";
            push_parameter(&mut parameters, name, executable_path)?;
            profile.push_str(
                r#"
(allow process-exec*
    (literal (param "hobble_executable")))
"#,
            );

            if let Some(parent) = executable_path.parent() {
                let name = "hobble_executable_dir";
                push_parameter(&mut parameters, name, parent)?;
                profile.push_str(
                    r#"
(allow file-read* file-map-executable
    (subpath (param "hobble_executable_dir")))
"#,
                );
            } else {
                profile.push_str(
                    r#"
(allow file-read* file-map-executable
    (literal (param "hobble_executable")))
"#,
                );
            }
        }

        for (idx, path) in allowed_paths.iter().enumerate() {
            let name = format!("hobble_allow_{idx}");
            push_parameter(&mut parameters, &name, path)?;

            let filter = if path.is_dir() { "subpath" } else { "literal" };
            profile.push_str(&format!(
                r#"
(allow file-read* file-write*
    ({filter} (param "{name}")))
"#
            ));
        }

        let profile = CString::new(profile).context("sandbox profile contains a null byte")?;
        let mut parameter_ptrs = parameters
            .iter()
            .map(|parameter| parameter.as_ptr() as usize)
            .collect::<Vec<_>>();
        parameter_ptrs.push(ptr::null::<c_char>() as usize);

        Ok(Self {
            profile,
            _parameter_strings: parameters,
            parameter_ptrs,
        })
    }

    fn apply(&self) -> io::Result<()> {
        let mut errorbuf = ptr::null_mut();
        let result = unsafe {
            sandbox_init_with_parameters(
                self.profile.as_ptr(),
                0,
                self.parameter_ptrs.as_ptr().cast::<*const c_char>(),
                &mut errorbuf,
            )
        };

        if result == 0 {
            Ok(())
        } else {
            Err(io::Error::other("sandbox_init_with_parameters failed"))
        }
    }
}

fn push_parameter(parameters: &mut Vec<CString>, name: &str, path: &Path) -> Result<()> {
    parameters.push(CString::new(name).context("sandbox parameter name contains a null byte")?);
    parameters.push(
        CString::new(path.as_os_str().as_bytes())
            .with_context(|| format!("sandbox path contains a null byte: {}", path.display()))?,
    );
    Ok(())
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
