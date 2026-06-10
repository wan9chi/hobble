#![cfg(target_os = "macos")]

//! macOS sandbox profile generation and execution.

use std::{
    ffi::{OsStr, OsString},
    fmt::Write as _,
    os::unix::process::CommandExt,
    path::{Path, PathBuf},
    process::{Command, ExitStatus},
};

#[derive(Debug, thiserror::Error)]
pub enum SandboxError {
    #[error("sandbox profile is empty")]
    EmptyProfile,
    #[error("argv is empty")]
    EmptyArgv,
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

pub struct Sbpl;

impl Sbpl {
    pub fn base() -> String {
        let mut profile = String::new();
        profile.push_str("(version 1)\n");
        profile.push_str("(deny default)\n");
        profile.push_str("(allow process-exec)\n");
        profile.push_str("(allow process-fork)\n");
        profile.push_str("(allow process-info* (target same-sandbox))\n");
        profile.push_str("(allow signal (target same-sandbox))\n");
        profile.push_str("(allow sysctl-read)\n");
        profile.push_str("(allow mach-lookup)\n");
        profile.push_str("(allow ipc-posix-shm*)\n");
        profile.push_str("(allow iokit-open)\n");
        profile.push_str("(allow file-read-metadata)\n");
        profile.push_str("(allow file-map-executable)\n");
        profile.push_str("(allow file-read* (literal \"/\"))\n");
        profile.push_str(&read_rule(runtime_read_roots().iter()));
        profile.push_str(&read_rule(device_roots().iter()));
        profile.push_str(&write_rule(device_roots().iter()));
        profile
    }

    pub fn allow_write(paths: &[PathBuf]) -> String {
        let paths = canonicalize_paths(paths);
        write_rule(paths.iter())
    }

    pub fn allow_read(paths: &[PathBuf]) -> String {
        let paths = canonicalize_paths(paths);
        read_rule(paths.iter())
    }

    pub fn allow_proxy(port: u16) -> String {
        format!("(allow network-outbound (remote ip \"localhost:{port}\"))\n")
    }
}

pub fn canonicalize_rule_path(path: impl AsRef<Path>) -> PathBuf {
    let path = path.as_ref();
    if let Ok(canonical) = path.canonicalize() {
        return canonical;
    }

    normalize_known_symlink_root(path)
}

pub fn run_sandboxed(
    profile: &str,
    argv: &[OsString],
    env: &[(OsString, OsString)],
    cwd: &Path,
) -> Result<ExitStatus, SandboxError> {
    if profile.trim().is_empty() {
        return Err(SandboxError::EmptyProfile);
    }
    if argv.is_empty() {
        return Err(SandboxError::EmptyArgv);
    }

    let profile_file = tempfile::NamedTempFile::new()?;
    std::fs::write(profile_file.path(), profile)?;

    let mut command = Command::new("/usr/bin/sandbox-exec");
    command
        .arg("-f")
        .arg(profile_file.path())
        .arg("--")
        .arg(&argv[0])
        .args(&argv[1..])
        .current_dir(cwd)
        .envs(env.iter().cloned());

    // Put sandbox-exec and descendants in their own process group so background
    // children cannot survive the gatekeeper process.
    unsafe {
        command.pre_exec(|| {
            if libc::setpgid(0, 0) == 0 {
                Ok(())
            } else {
                Err(std::io::Error::last_os_error())
            }
        });
    }

    let mut child = command.spawn()?;
    let process_group = child.id() as i32;
    let status = child.wait()?;

    // The group may already be gone. ESRCH is the expected benign case.
    unsafe {
        libc::kill(-process_group, libc::SIGKILL);
    }

    Ok(status)
}

fn canonicalize_paths(paths: &[PathBuf]) -> Vec<PathBuf> {
    paths.iter().map(canonicalize_rule_path).collect()
}

fn read_rule<'a>(paths: impl Iterator<Item = &'a PathBuf>) -> String {
    path_rule("file-read*", paths)
}

fn write_rule<'a>(paths: impl Iterator<Item = &'a PathBuf>) -> String {
    path_rule("file-write*", paths)
}

fn path_rule<'a>(operation: &str, paths: impl Iterator<Item = &'a PathBuf>) -> String {
    let mut paths = paths.peekable();
    if paths.peek().is_none() {
        return String::new();
    }

    let mut rule = format!("(allow {operation}");
    for path in paths {
        let escaped = escape_sbpl(path.as_os_str());
        let _ = write!(rule, " (literal \"{escaped}\") (subpath \"{escaped}\")");
    }
    rule.push_str(")\n");
    rule
}

fn runtime_read_roots() -> Vec<PathBuf> {
    let mut roots = [
        "/usr",
        "/System",
        "/Library",
        "/bin",
        "/sbin",
        "/private/etc",
        "/dev/fd",
    ]
    .into_iter()
    .map(PathBuf::from)
    .map(canonicalize_rule_path)
    .collect::<Vec<_>>();

    roots.extend(node_runtime_dirs());

    roots.sort();
    roots.dedup();
    roots
}

fn device_roots() -> Vec<PathBuf> {
    ["/dev/null", "/dev/zero", "/dev/random", "/dev/urandom"]
        .into_iter()
        .map(PathBuf::from)
        .collect()
}

fn node_runtime_dirs() -> Vec<PathBuf> {
    let Some(node) = node_binary_path() else {
        return Vec::new();
    };
    let node = canonicalize_rule_path(node);
    let mut roots = Vec::new();
    if let Some(bin_dir) = node.parent() {
        roots.push(bin_dir.to_path_buf());
        if let Some(install_root) = bin_dir.parent() {
            roots.push(install_root.to_path_buf());
        }
    }
    roots
}

fn node_binary_path() -> Option<PathBuf> {
    let output = Command::new("mise")
        .arg("which")
        .arg("node")
        .output()
        .ok()
        .filter(|output| output.status.success())
        .or_else(|| {
            Command::new("/bin/sh")
                .arg("-lc")
                .arg("command -v node")
                .output()
                .ok()
                .filter(|output| output.status.success())
        })?;

    let node = String::from_utf8(output.stdout).ok()?;
    Some(PathBuf::from(node.trim()))
}

fn normalize_known_symlink_root(path: &Path) -> PathBuf {
    let path_string = path.to_string_lossy();
    if path_string == "/tmp" {
        return PathBuf::from("/private/tmp");
    }
    if let Some(rest) = path_string.strip_prefix("/tmp/") {
        return PathBuf::from("/private/tmp").join(rest);
    }
    if path_string == "/var" {
        return PathBuf::from("/private/var");
    }
    if let Some(rest) = path_string.strip_prefix("/var/") {
        return PathBuf::from("/private/var").join(rest);
    }
    path.to_path_buf()
}

fn escape_sbpl(path: &OsStr) -> String {
    path.to_string_lossy()
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
}

#[cfg(test)]
mod tests {
    use std::{
        ffi::OsString,
        io::{Read, Write},
        net::TcpListener,
        path::PathBuf,
        thread,
        time::Duration,
    };

    use super::*;

    #[test]
    fn base_contains_expected_deny_and_allow_blocks() {
        let base = Sbpl::base();
        assert!(base.contains("(version 1)"));
        assert!(base.contains("(deny default)"));
        assert!(base.contains("(allow process-exec)"));
        assert!(base.contains("(allow file-read*"));
        assert!(base.contains("(subpath \"/usr\")"));
        assert!(base.contains("(subpath \"/System\")"));
        assert!(base.contains("(subpath \"/Library\")"));
    }

    #[test]
    fn base_boots_shell_and_node() {
        let profile = Sbpl::base();

        let shell = run_sandboxed(
            &profile,
            &argv(["/bin/sh", "-c", "echo ok"]),
            &[],
            Path::new("/"),
        )
        .unwrap();
        assert!(shell.success());

        let node = node_binary_path().unwrap();
        let node_status = run_sandboxed(
            &profile,
            &[
                node.into_os_string(),
                OsString::from("-e"),
                OsString::from("process.exit(0)"),
            ],
            &[],
            Path::new("/"),
        )
        .unwrap();
        assert!(node_status.success());
    }

    #[test]
    fn write_rules_allow_only_configured_roots() {
        let temp = tempfile::tempdir().unwrap();
        let ok = temp.path().join("ok");
        let denied = temp.path().join("denied");
        std::fs::create_dir_all(&ok).unwrap();
        std::fs::create_dir_all(&denied).unwrap();

        let profile = Sbpl::base() + &Sbpl::allow_write(std::slice::from_ref(&ok));
        let allowed = run_sandboxed(
            &profile,
            &argv(["/bin/sh", "-c", "echo x > \"$OK/a\""]),
            &[(OsString::from("OK"), ok.as_os_str().to_os_string())],
            temp.path(),
        )
        .unwrap();
        assert!(allowed.success());
        assert_eq!(std::fs::read_to_string(ok.join("a")).unwrap().trim(), "x");

        let denied_status = run_sandboxed(
            &profile,
            &argv(["/bin/sh", "-c", "echo y > \"$DENIED/b\""]),
            &[(OsString::from("DENIED"), denied.as_os_str().to_os_string())],
            temp.path(),
        )
        .unwrap();
        assert!(!denied_status.success());
        assert!(!denied.join("b").exists());
    }

    #[test]
    fn read_rules_allow_only_configured_roots() {
        let temp = tempfile::tempdir().unwrap();
        let public = temp.path().join("pub");
        let denied = temp.path().join("denied");
        std::fs::create_dir_all(&public).unwrap();
        std::fs::create_dir_all(&denied).unwrap();
        std::fs::write(public.join("x"), "ok").unwrap();
        std::fs::write(denied.join("secret"), "no").unwrap();

        let profile = Sbpl::base() + &Sbpl::allow_read(std::slice::from_ref(&public));
        let allowed = run_sandboxed(
            &profile,
            &argv(["/bin/sh", "-c", "cat \"$PUB/x\" >/dev/null"]),
            &[(OsString::from("PUB"), public.as_os_str().to_os_string())],
            temp.path(),
        )
        .unwrap();
        assert!(allowed.success());

        let denied_status = run_sandboxed(
            &profile,
            &argv(["/bin/sh", "-c", "cat \"$DENIED/secret\" >/dev/null"]),
            &[(OsString::from("DENIED"), denied.as_os_str().to_os_string())],
            temp.path(),
        )
        .unwrap();
        assert!(!denied_status.success());
    }

    #[test]
    fn egress_rule_allows_only_proxy_port() {
        let (port, server) = spawn_http_server();
        let temp = tempfile::tempdir().unwrap();
        let profile = Sbpl::base() + &Sbpl::allow_proxy(port);

        let allowed = run_sandboxed(
            &profile,
            &argv([
                "/usr/bin/curl",
                "--silent",
                "--fail",
                &format!("http://127.0.0.1:{port}/"),
            ]),
            &[],
            temp.path(),
        )
        .unwrap();
        assert!(allowed.success());
        server.join().unwrap();

        let denied = run_sandboxed(
            &profile,
            &argv([
                "/usr/bin/curl",
                "--silent",
                "--max-time",
                "2",
                "https://example.com/",
            ]),
            &[],
            temp.path(),
        )
        .unwrap();
        assert!(!denied.success());
    }

    #[test]
    fn canonicalizes_tmp_rules_before_emitting() {
        let root = PathBuf::from("/tmp/creance-sandbox-canonical");
        let private_root = PathBuf::from("/private/tmp/creance-sandbox-canonical");
        let _ = std::fs::remove_dir_all(&private_root);
        std::fs::create_dir_all(&root).unwrap();

        let temp = tempfile::tempdir().unwrap();
        let canonical_profile = Sbpl::base() + &Sbpl::allow_write(std::slice::from_ref(&root));
        let allowed = run_sandboxed(
            &canonical_profile,
            &argv([
                "/bin/sh",
                "-c",
                "echo ok > /tmp/creance-sandbox-canonical/x",
            ]),
            &[],
            temp.path(),
        )
        .unwrap();
        assert!(allowed.success());

        let raw_profile = Sbpl::base() + &path_rule("file-write*", [&root].into_iter());
        let denied = run_sandboxed(
            &raw_profile,
            &argv([
                "/bin/sh",
                "-c",
                "echo no > /tmp/creance-sandbox-canonical/y",
            ]),
            &[],
            temp.path(),
        )
        .unwrap();
        assert!(!denied.success());
    }

    #[test]
    fn run_sandboxed_kills_background_process_group() {
        let temp = tempfile::tempdir().unwrap();
        let pid_file = temp.path().join("sleep.pid");
        let profile = Sbpl::base() + &Sbpl::allow_write(&[temp.path().to_path_buf()]);
        let status = run_sandboxed(
            &profile,
            &argv(["/bin/sh", "-c", "sleep 30 & echo $! > \"$PID_FILE\""]),
            &[(
                OsString::from("PID_FILE"),
                pid_file.as_os_str().to_os_string(),
            )],
            Path::new("/"),
        )
        .unwrap();
        assert!(status.success());

        let pid = std::fs::read_to_string(&pid_file)
            .unwrap()
            .trim()
            .parse::<i32>()
            .unwrap();
        for _ in 0..50 {
            if !process_exists(pid) {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("background process {pid} survived sandbox teardown");
    }

    fn argv<const N: usize>(items: [&str; N]) -> Vec<OsString> {
        items.into_iter().map(OsString::from).collect()
    }

    fn spawn_http_server() -> (u16, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = [0; 1024];
            let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
            let _ = stream.read(&mut buf);
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok")
                .unwrap();
        });
        (port, handle)
    }

    fn process_exists(pid: i32) -> bool {
        unsafe {
            libc::kill(pid, 0) == 0
                || std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
        }
    }
}
