use std::{
    env,
    ffi::{OsStr, OsString},
    fs, io,
    os::unix::ffi::OsStrExt,
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{Context, Result};

/// Filesystem policy for a sandboxed process.
///
/// Both lists are pure allowlists over a default-deny baseline: an operation
/// is permitted only if some entry grants it. Grants are unioned; entry
/// order, duplicates, and nesting are irrelevant. There are no deny rules,
/// so one entry can never narrow another (e.g. no read-only carve-out inside
/// a writable subtree).
///
/// # Entry semantics
///
/// Entries are literal paths, not patterns; globs are not supported. Every
/// entry must be absolute. An entry that does not exist at spawn time
/// (including a dangling symlink) is skipped: because grants are resolved
/// once at spawn, the path stays denied even if it is created later during
/// the run. To grant a path that does not exist yet, create it before
/// spawning.
///
/// Each entry is resolved once at spawn time, before the sandboxed process
/// runs:
///
/// - Symlinks are fully resolved: the entry itself, chains of links, and
///   symlinked ancestors. Both the entry as written and its resolved path
///   are granted.
/// - The resolved path is then classified: a directory grants itself and its
///   entire subtree; a regular file grants that file only.
///
/// Symlinks encountered inside an allowed subtree at runtime do not extend
/// access: following one is subject to whatever the policy grants for its
/// target. Likewise, retargeting a symlink after spawn does not change what
/// was granted.
///
/// # Limits
///
/// - Path metadata (existence, `stat`, `readlink`) is not confined; the
///   policy governs reading content, listing directories, and writing.
/// - Read access also permits executing files.
#[derive(Default, Debug, Clone)]
pub struct SandboxProfile {
    /// Paths the sandboxed process may read.
    pub allowed_reads: Vec<PathBuf>,
    /// Paths the sandboxed process may read and write (create, modify,
    /// rename, and delete). Write access always includes read access.
    pub allowed_writes: Vec<PathBuf>,
}

/// A profile entry resolved at spawn time, following the semantics described
/// on [`SandboxProfile`].
#[derive(Debug, Clone)]
pub struct ResolvedEntry {
    /// The entry as written in the profile.
    pub literal: PathBuf,
    /// The fully resolved path, when it differs from the literal spelling.
    pub canonical: Option<PathBuf>,
    /// Whether the entry resolves to a directory and grants its subtree.
    pub is_dir: bool,
}

impl ResolvedEntry {
    /// The spellings this entry grants: the literal path, plus the resolved
    /// path when it differs.
    pub fn paths(&self) -> impl Iterator<Item = &Path> {
        std::iter::once(self.literal.as_path()).chain(self.canonical.as_deref())
    }
}

/// Resolves profile entries once at spawn time so every platform backend
/// applies the same rules: entries must be absolute, entries missing at
/// spawn (including dangling symlinks) are skipped, symlinks are fully
/// resolved so both spellings can be granted, and the directory/file
/// classification uses the resolved target.
pub fn resolve_entries(entries: &[PathBuf]) -> Result<Vec<ResolvedEntry>> {
    let mut resolved = Vec::with_capacity(entries.len());
    for entry in entries {
        anyhow::ensure!(
            entry.is_absolute(),
            "sandbox paths must be absolute: {}",
            entry.display()
        );
        let metadata = match fs::metadata(entry) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(error).with_context(|| {
                    format!("failed to resolve sandbox path {}", entry.display())
                });
            }
        };
        let canonical = entry
            .canonicalize()
            .ok()
            .filter(|canonical| canonical != entry);
        resolved.push(ResolvedEntry {
            literal: entry.clone(),
            canonical,
            is_dir: metadata.is_dir(),
        });
    }
    Ok(resolved)
}

/// Predicts the executable path a spawned command will resolve to, so
/// backends can grant it read and exec access. Mirrors `execvp`: absolute
/// paths as-is, paths containing a separator relative to the working
/// directory, bare names searched in `PATH` (honoring a `PATH` override on
/// the command).
pub fn resolve_program_path(command: &Command) -> Option<PathBuf> {
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
