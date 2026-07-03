//! Exercises the sandbox semantics documented on `SandboxProfile` from
//! inside a sandboxed child process: union-of-allows, write-implies-read,
//! directory subtrees, spawn-time symlink resolution, and skipped missing
//! entries.

use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write as _},
    os::unix::fs::{PermissionsExt, symlink},
    path::{Path, PathBuf},
    process::{Child, Stdio},
    thread,
    time::Duration,
};

use hobble_command_test::command_for_fn;
use hobble_sandbox::SandboxProfile;
use tempfile::{NamedTempFile, tempdir};

use Expect::{Allowed, Denied, Missing};
use Op::{AwaitRead, Create, Exec, List, Read, Write};

#[test]
fn read_entry_grants_the_file_only() {
    let dir = tempdir().unwrap();
    let allowed = dir.path().join("allowed.txt");
    let sibling = dir.path().join("sibling.txt");
    fs::write(&allowed, b"allowed").unwrap();
    fs::write(&sibling, b"sibling").unwrap();

    assert_access(
        &profile(&[&allowed], &[]),
        &[
            (Read, &allowed, Allowed),
            (Read, &sibling, Denied),
            (Write, &allowed, Denied),
            (List, dir.path(), Denied),
        ],
    );
}

#[test]
fn read_dir_entry_grants_the_subtree() {
    let dir = tempdir().unwrap();
    let nested_dir = dir.path().join("nested");
    fs::create_dir(&nested_dir).unwrap();
    let nested = nested_dir.join("file.txt");
    fs::write(&nested, b"nested").unwrap();
    let missing = dir.path().join("missing.txt");
    let new_file = dir.path().join("new.txt");
    let mut outside = NamedTempFile::new().unwrap();
    outside.write_all(b"outside").unwrap();

    assert_access(
        &profile(&[dir.path()], &[]),
        &[
            (Read, &nested, Allowed),
            (List, dir.path(), Allowed),
            (List, &nested_dir, Allowed),
            (Read, &missing, Missing),
            (Read, outside.path(), Denied),
            (Write, &nested, Denied),
            (Create, &new_file, Denied),
        ],
    );
}

#[test]
fn write_entry_implies_read() {
    let dir = tempdir().unwrap();
    let file = dir.path().join("file.txt");
    fs::write(&file, b"content").unwrap();
    let sibling = dir.path().join("sibling.txt");

    assert_access(
        &profile(&[], &[&file]),
        &[
            (Write, &file, Allowed),
            (Read, &file, Allowed),
            (Create, &sibling, Denied),
        ],
    );
}

#[test]
fn write_dir_entry_grants_the_subtree() {
    let dir = tempdir().unwrap();
    let existing = dir.path().join("existing.txt");
    fs::write(&existing, b"existing").unwrap();
    let created = dir.path().join("created.txt");
    let outside_dir = tempdir().unwrap();
    let outside = outside_dir.path().join("outside.txt");

    assert_access(
        &profile(&[], &[dir.path()]),
        &[
            (Create, &created, Allowed),
            (Read, &created, Allowed),
            (Write, &existing, Allowed),
            (List, dir.path(), Allowed),
            (Create, &outside, Denied),
        ],
    );
}

#[test]
fn entry_created_after_spawn_stays_denied() {
    let dir = tempdir().unwrap();
    let late = dir.path().join("late.txt");

    // The entry does not exist at spawn, so it is skipped. The child waits
    // for the file to appear (metadata is unconfined) and must still be
    // denied once it does.
    let checks = [(AwaitRead, late.as_path(), Denied)];
    let child = spawn_checks(&profile(&[&late], &[]), &checks);
    fs::write(&late, b"late").unwrap();
    assert_outcomes(child, &checks);
}

#[test]
fn dangling_symlink_entry_is_skipped() {
    let dir = tempdir().unwrap();
    let dangling = dir.path().join("dangling");
    symlink(dir.path().join("missing-target"), &dangling).unwrap();
    let mut allowed = NamedTempFile::new().unwrap();
    allowed.write_all(b"allowed").unwrap();

    // Spawning must succeed, and the remaining entries must still apply.
    assert_access(
        &profile(&[&dangling, allowed.path()], &[]),
        &[(Read, allowed.path(), Allowed)],
    );
}

#[test]
fn symlink_entry_grants_link_and_target() {
    let target_dir = tempdir().unwrap();
    let target_file = target_dir.path().join("file.txt");
    fs::write(&target_file, b"target").unwrap();
    let link_dir = tempdir().unwrap();
    let link = link_dir.path().join("link");
    symlink(target_dir.path(), &link).unwrap();
    let through_link = link.join("file.txt");

    assert_access(
        &profile(&[&link], &[]),
        &[
            (Read, &through_link, Allowed),
            (Read, &target_file, Allowed),
        ],
    );
}

#[test]
fn symlink_inside_allowed_dir_stays_confined() {
    let dir = tempdir().unwrap();
    let mut outside = NamedTempFile::new().unwrap();
    outside.write_all(b"outside").unwrap();
    let escape = dir.path().join("escape");
    symlink(outside.path(), &escape).unwrap();

    assert_access(
        &profile(&[dir.path()], &[]),
        &[(Read, &escape, Denied), (Read, outside.path(), Denied)],
    );
}

#[test]
fn exec_follows_the_read_allowlist() {
    let dir = tempdir().unwrap();
    let tool = dir.path().join("tool.sh");
    fs::write(&tool, "#!/bin/sh\nexit 0\n").unwrap();
    fs::set_permissions(&tool, fs::Permissions::from_mode(0o755)).unwrap();

    // Runtime roots are executable by default; a program elsewhere is not,
    // unless a read entry grants it (reads imply exec).
    assert_access(
        &SandboxProfile::default(),
        &[(Exec, Path::new("/bin/echo"), Allowed), (Exec, &tool, Denied)],
    );
    assert_access(&profile(&[&tool], &[]), &[(Exec, &tool, Allowed)]);
}

#[test]
fn dev_null_is_writable_by_default() {
    assert_access(
        &SandboxProfile::default(),
        &[(Write, Path::new("/dev/null"), Allowed)],
    );
}

#[test]
fn relative_entry_is_rejected() {
    let mut profile = SandboxProfile::default();
    profile.allowed_reads.push(PathBuf::from("relative/path"));

    let command = command_for_fn!(String::new(), |_script: String| {});
    let result = hobble_sandbox::spawn_with_sandbox(command, &profile);
    assert!(result.is_err());
}

/// A filesystem access performed inside the sandboxed child.
#[derive(Clone, Copy, Debug)]
enum Op {
    /// Open the path for reading.
    Read,
    /// Open the path for writing and write one byte.
    Write,
    /// Create a file at the path.
    Create,
    /// List the path as a directory.
    List,
    /// Run the path as a program.
    Exec,
    /// Wait until the path exists, then open it for reading.
    AwaitRead,
}

impl Op {
    fn wire(self) -> &'static str {
        match self {
            Op::Read => "read",
            Op::Write => "write",
            Op::Create => "create",
            Op::List => "list",
            Op::Exec => "exec",
            Op::AwaitRead => "await-read",
        }
    }
}

/// The outcome an access is expected to produce.
#[derive(Clone, Copy, Debug)]
enum Expect {
    Allowed,
    Denied,
    Missing,
}

impl Expect {
    fn wire(self) -> &'static str {
        match self {
            Expect::Allowed => "Ok",
            Expect::Denied => "PermissionDenied",
            Expect::Missing => "NotFound",
        }
    }
}

type Check<'a> = (Op, &'a Path, Expect);

fn profile(reads: &[&Path], writes: &[&Path]) -> SandboxProfile {
    SandboxProfile {
        allowed_reads: reads.iter().map(|path| path.to_path_buf()).collect(),
        allowed_writes: writes.iter().map(|path| path.to_path_buf()).collect(),
    }
}

/// Performs every check inside a sandboxed child and asserts each outcome.
fn assert_access(profile: &SandboxProfile, checks: &[Check]) {
    assert_outcomes(spawn_checks(profile, checks), checks);
}

fn spawn_checks(profile: &SandboxProfile, checks: &[Check]) -> Child {
    let script = checks
        .iter()
        .map(|(op, path, _)| format!("{}\t{}", op.wire(), path.display()))
        .collect::<Vec<_>>()
        .join("\n");
    let mut command = command_for_fn!(script, |script: String| {
        for line in script.lines() {
            let (op, path) = line.split_once('\t').unwrap();
            println!("{}", perform(op, path));
        }
    });
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    hobble_sandbox::spawn_with_sandbox(command, profile).unwrap()
}

fn assert_outcomes(child: Child, checks: &[Check]) {
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "child failed with status {} stdout: {} stderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = std::str::from_utf8(&output.stdout).unwrap();
    let outcomes = stdout.trim().lines().collect::<Vec<_>>();
    assert_eq!(
        outcomes.len(),
        checks.len(),
        "expected one outcome per check, got: {stdout}"
    );
    for ((op, path, expect), outcome) in checks.iter().zip(outcomes) {
        assert_eq!(
            outcome,
            expect.wire(),
            "{op:?} {} should be {expect:?}",
            path.display()
        );
    }
}

/// Runs in the sandboxed child: performs one wire-format operation and
/// returns `Ok`, an `io::ErrorKind` name, or `Timeout`.
fn perform(op: &str, path: &str) -> String {
    let result = match op {
        "read" => File::open(path).map(drop),
        "write" => OpenOptions::new()
            .write(true)
            .open(path)
            .and_then(|mut file| file.write_all(b"x")),
        "create" => File::create(path).map(drop),
        "list" => fs::read_dir(path).map(drop),
        "exec" => run_program(path),
        "await-read" => return await_then_read(path),
        other => panic!("unknown op: {other}"),
    };
    outcome(result)
}

fn run_program(path: &str) -> io::Result<()> {
    let status = std::process::Command::new(path)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!("exited with {status}")))
    }
}

fn await_then_read(path: &str) -> String {
    for _ in 0..200 {
        if fs::symlink_metadata(path).is_ok() {
            return outcome(File::open(path).map(drop));
        }
        thread::sleep(Duration::from_millis(25));
    }
    "Timeout".to_string()
}

fn outcome(result: io::Result<()>) -> String {
    match result {
        Ok(()) => "Ok".to_string(),
        Err(error) => format!("{:?}", error.kind()),
    }
}
