use std::{
    collections::{BTreeSet, HashSet},
    ffi::OsString,
    path::{Path, PathBuf},
    process::ExitStatus,
    str::FromStr,
};

use anyhow::{Context as _, Result};
use creance_proxy::{HostPattern, Policy, Proxy};
use creance_sandbox::{Sbpl, run_sandboxed};
use serde::{Deserialize, Serialize};
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SandboxProfileJson {
    pub schema: u32,
    pub os: Vec<String>,
    pub file_read: Vec<String>,
    pub file_write: Vec<String>,
    pub network: SandboxNetworkJson,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SandboxNetworkJson {
    pub mode: String,
    pub domains: Vec<String>,
}

pub fn sandbox_profile_json_for_entry(entry: &Entry) -> SandboxProfileJson {
    SandboxProfileJson {
        schema: 1,
        os: sorted_unique(&entry.os),
        file_read: sorted_unique(&entry.read),
        file_write: sorted_unique(&entry.write),
        network: SandboxNetworkJson {
            mode: "proxy-only".to_string(),
            domains: sorted_unique(&entry.domains),
        },
    }
}

fn resolve_paths(templater: &crate::template::Templater, templates: &[String]) -> Vec<PathBuf> {
    templater.resolve_many(templates)
}

fn sorted_unique(values: &[String]) -> Vec<String> {
    values
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

#[cfg(test)]
mod tests {
    use crate::profile::Entry;

    use super::*;

    #[test]
    fn sandbox_profile_json_is_stable_and_deduped() {
        let entry = Entry {
            os: vec!["darwin".to_string(), "darwin".to_string()],
            read: vec!["b".to_string(), "a".to_string(), "a".to_string()],
            write: vec!["w".to_string()],
            domains: vec!["z.test".to_string(), "a.test".to_string()],
        };

        assert_eq!(
            sandbox_profile_json_for_entry(&entry),
            SandboxProfileJson {
                schema: 1,
                os: vec!["darwin".to_string()],
                file_read: vec!["a".to_string(), "b".to_string()],
                file_write: vec!["w".to_string()],
                network: SandboxNetworkJson {
                    mode: "proxy-only".to_string(),
                    domains: vec!["a.test".to_string(), "z.test".to_string()],
                },
            }
        );
    }
}
