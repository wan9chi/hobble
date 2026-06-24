#![cfg(target_os = "linux")]

use std::{ffi::OsStr, process::{Child, Command}};

#[derive(Default, Debug)]
pub struct SandboxBuilder {

}

impl SandboxBuilder {
    // Allow a path to be accessed by the sandboxed process.
    // If the path is a directory, all files and directories under it will be allowed.
    // # Panics
    // Panics if the path is not absolute.
    pub fn allow_path(&mut self, path: &OsStr) -> &mut Self {
        self
    }


    pub fn spawn(self, command: Command) -> Result<Child, anyhow::Error> {
        todo!()
    }
}
