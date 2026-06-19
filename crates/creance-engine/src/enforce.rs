use std::{
    collections::HashSet,
    ffi::OsString,
    path::{Path, PathBuf},
    process::ExitStatus,
    str::FromStr,
};

use anyhow::{Context as _, Result};
use creance_proxy::{HostPattern, Policy, Proxy};
use creance_sandbox::{Sbpl, run_sandboxed};
use tokio_util::sync::CancellationToken;

use crate::{profile::Entry, template::Context};

pub async fn enforce(
    entry: &Entry,
    ctx: &Context,
    argv: &[OsString],
    env: &[(OsString, OsString)],
    cwd: &Path,
) -> Result<ExitStatus> {
    let allowed = entry
        .domains
        .iter()
        .map(|domain| HostPattern::from_str(domain))
        .collect::<Result<HashSet<_>, _>>()?;
    let (proxy, proxy_addr) = Proxy::bind_with_policy("127.0.0.1:0", Policy::Enforce(allowed))
        .await
        .context("bind enforce proxy")?;
    let cancellation = CancellationToken::new();
    let proxy_env = proxy
        .proxy_env()
        .into_iter()
        .map(|(key, value)| (OsString::from(key), OsString::from(value)))
        .collect::<Vec<_>>();
    let proxy_task = tokio::spawn(proxy.run(cancellation.clone()));

    let profile = sandbox_profile_for_entry(entry, ctx, proxy_addr.port());
    let mut sandbox_env = env.to_vec();
    sandbox_env.extend(proxy_env);
    let argv = argv.to_vec();
    let cwd = cwd.to_path_buf();
    let status =
        tokio::task::spawn_blocking(move || run_sandboxed(&profile, &argv, &sandbox_env, &cwd))
            .await
            .context("join sandbox task")??;

    cancellation.cancel();
    proxy_task.await.context("join enforce proxy")??;
    Ok(status)
}

pub fn sandbox_profile_for_entry(entry: &Entry, ctx: &Context, proxy_port: u16) -> String {
    let templater = crate::template::Templater::new(ctx.clone());
    let mut profile = Sbpl::base();
    profile.push_str(&Sbpl::allow_read(&resolve_paths(&templater, &entry.read)));
    profile.push_str(&Sbpl::allow_write(&resolve_paths(&templater, &entry.write)));
    profile.push_str(&Sbpl::allow_proxy(proxy_port));
    profile
}

fn resolve_paths(templater: &crate::template::Templater, templates: &[String]) -> Vec<PathBuf> {
    templater.resolve_many(templates)
}
