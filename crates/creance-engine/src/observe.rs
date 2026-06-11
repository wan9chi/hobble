use std::{
    collections::BTreeSet,
    ffi::OsString,
    path::{Path, PathBuf},
    process::ExitStatus,
};

use anyhow::{Context as _, Result};
use creance_proxy::{HostPort, Policy, Proxy, Recorder};
use fspy::AccessMode;
use tokio_util::sync::CancellationToken;

#[derive(Debug)]
pub struct Observation {
    pub status: ExitStatus,
    pub reads: Vec<PathBuf>,
    pub writes: Vec<PathBuf>,
    pub domains: Vec<HostPort>,
}

pub async fn observe_fs(
    argv: &[OsString],
    env: &[(OsString, OsString)],
    cwd: &Path,
) -> Result<Observation> {
    anyhow::ensure!(!argv.is_empty(), "argv is empty");

    let mut command = fspy::Command::new(&argv[0]);
    command
        .args(&argv[1..])
        .envs(env.iter().cloned())
        .current_dir(cwd);

    let child = command
        .spawn(CancellationToken::new())
        .await
        .context("spawn fspy command")?;
    let termination = child.wait_handle.await.context("wait for fspy command")?;

    let mut reads = BTreeSet::new();
    let mut writes = BTreeSet::new();
    for access in termination.path_accesses.iter() {
        let Some(path) = access.path.strip_path_prefix(Path::new("/"), |result| {
            result.ok().map(|relative| Path::new("/").join(relative))
        }) else {
            continue;
        };
        if access.mode.contains(AccessMode::WRITE) {
            writes.insert(path.clone());
        }
        if access
            .mode
            .intersects(AccessMode::READ | AccessMode::READ_DIR)
        {
            reads.insert(path);
        }
    }

    Ok(Observation {
        status: termination.status,
        reads: reads.into_iter().collect(),
        writes: writes.into_iter().collect(),
        domains: Vec::new(),
    })
}

pub async fn observe(
    argv: &[OsString],
    env: &[(OsString, OsString)],
    cwd: &Path,
) -> Result<Observation> {
    let recorder = Recorder::default();
    let (proxy, _) = Proxy::bind_with_policy("127.0.0.1:0", Policy::ObserveLog(recorder.clone()))
        .await
        .context("bind observe proxy")?;
    let proxy_env = proxy
        .proxy_env()
        .into_iter()
        .map(|(key, value)| (OsString::from(key), OsString::from(value)));
    let cancellation = CancellationToken::new();
    let proxy_task = tokio::spawn(proxy.run(cancellation.clone()));

    let mut observed_env = env.to_vec();
    observed_env.extend(proxy_env);

    let observation = observe_fs(argv, &observed_env, cwd).await;
    cancellation.cancel();
    proxy_task.await.context("join observe proxy")??;

    let domains = recorder.snapshot().await;
    if let Ok(observation) = &observation
        && observation.status.success()
        && domains.is_empty()
    {
        tracing::warn!(
            "observed command exited successfully without proxy connections; it may not honor proxy environment variables"
        );
    }

    let mut observation = observation.context("observe filesystem")?;
    observation.domains = domains;
    Ok(observation)
}
