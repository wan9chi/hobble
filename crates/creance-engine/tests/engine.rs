use std::{ffi::OsString, io::Read, net::TcpListener, path::Path, thread, time::Duration};

use creance_engine::{
    Context, Entry, enforce, observe, observe_fs, sandbox_profile_for_entry, synthesize,
};

#[tokio::test]
async fn fspy_captures_shell_grandchild_reads_and_writes() {
    let temp = tempfile::tempdir().unwrap();
    let out = temp.path().join("out");
    let observation = observe_fs(
        &argv([
            "/bin/sh",
            "-c",
            "cat /etc/hosts >/dev/null; echo x > \"$OUT\"",
        ]),
        &[(OsString::from("OUT"), out.as_os_str().to_os_string())],
        temp.path(),
    )
    .await
    .unwrap();

    assert!(observation.status.success());
    assert!(
        observation
            .reads
            .iter()
            .any(|path| path == Path::new("/etc/hosts"))
    );
    assert!(observation.writes.iter().any(|path| path == &out));
}

#[tokio::test]
async fn observe_records_proxy_domains_and_filesystem_writes() {
    let temp = tempfile::tempdir().unwrap();
    let out = temp.path().join("probe-out");
    let observation = observe(
        &[connect_probe(), out.as_os_str().to_os_string()],
        &std::env::vars_os().collect::<Vec<_>>(),
        temp.path(),
    )
    .await
    .unwrap();

    assert!(observation.status.success());
    assert!(out.exists());
    assert!(observation.writes.iter().any(|path| path == &out));
    assert!(
        observation
            .domains
            .iter()
            .any(|domain| domain.host() == "recorded.test")
    );
}

#[tokio::test]
async fn enforce_allows_profiled_behavior_and_blocks_tampering() {
    let temp = tempfile::tempdir().unwrap();
    let ctx = test_context(temp.path());
    let (port, server) = spawn_tcp_server();
    let out = ctx.run_tmp.join("probe-out");

    let mut entry = Entry::new("darwin");
    entry.read = vec![
        "${PKG_DIR}/**".to_string(),
        "${PROJECT_ROOT}/target/**".to_string(),
    ];
    entry.write = vec!["${RUN_TMP}/**".to_string()];
    entry.domains = vec!["localhost".to_string()];

    let status = enforce(
        &entry,
        &ctx,
        &[
            connect_probe(),
            out.as_os_str().to_os_string(),
            OsString::from(port.to_string()),
        ],
        &std::env::vars_os().collect::<Vec<_>>(),
        &ctx.pkg_dir,
    )
    .await
    .unwrap();
    assert!(status.success());
    assert!(out.exists());
    server.join().unwrap();

    let denied = temp.path().join("denied");
    std::fs::create_dir_all(&denied).unwrap();
    let status = enforce(
        &entry,
        &ctx,
        &argv(["/bin/sh", "-c", "echo no > \"$DENIED/out\""]),
        &[(OsString::from("DENIED"), denied.as_os_str().to_os_string())],
        &ctx.pkg_dir,
    )
    .await
    .unwrap();
    assert!(!status.success());
    assert!(!denied.join("out").exists());

    let sandbox = sandbox_profile_for_entry(&entry, &ctx, 12345);
    assert!(sandbox.contains("localhost:12345"));
}

#[tokio::test]
async fn observe_synthesize_enforce_loop_for_shell_command() {
    let temp = tempfile::tempdir().unwrap();
    let ctx = test_context(temp.path());
    let out = ctx.run_tmp.join("loop-out");
    let command = "cat /etc/hosts >/dev/null; echo ok > \"$OUT\"";
    let env = vec![(OsString::from("OUT"), out.as_os_str().to_os_string())];

    let observation = observe_fs(&argv(["/bin/sh", "-c", command]), &env, &ctx.pkg_dir)
        .await
        .unwrap();
    assert!(observation.status.success());

    let entry = synthesize(&observation, "darwin", &ctx);
    let status = enforce(
        &entry,
        &ctx,
        &argv(["/bin/sh", "-c", command]),
        &env,
        &ctx.pkg_dir,
    )
    .await
    .unwrap();
    assert!(status.success());

    let tamper = temp.path().join("tamper");
    std::fs::create_dir_all(&tamper).unwrap();
    let status = enforce(
        &entry,
        &ctx,
        &argv(["/bin/sh", "-c", "echo bad > \"$TAMPER/out\""]),
        &[(OsString::from("TAMPER"), tamper.as_os_str().to_os_string())],
        &ctx.pkg_dir,
    )
    .await
    .unwrap();
    assert!(!status.success());
}

fn test_context(root: &Path) -> Context {
    let project = std::env::current_dir().unwrap();
    let pkg_dir = root.join("pkg");
    let run_tmp = root.join("run");
    for dir in [&pkg_dir, &run_tmp] {
        std::fs::create_dir_all(dir).unwrap();
    }
    Context::from_roots(
        pkg_dir,
        project,
        root.join("store"),
        root.join("home"),
        root.join("home/Library/Caches"),
        run_tmp,
    )
}

fn connect_probe() -> OsString {
    OsString::from(env!("CARGO_BIN_EXE_connect-probe"))
}

fn argv<const N: usize>(items: [&str; N]) -> Vec<OsString> {
    items.into_iter().map(OsString::from).collect()
}

fn spawn_tcp_server() -> (u16, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
        let mut buf = [0; 1];
        let _ = stream.read(&mut buf);
    });
    (port, handle)
}
