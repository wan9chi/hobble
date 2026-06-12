use std::collections::BTreeSet;

use crate::{
    observe::Observation,
    profile::Entry,
    template::{Context, generalize},
};

pub fn synthesize(observation: &Observation, os: impl Into<String>, ctx: &Context) -> Entry {
    let domains = observation
        .domains
        .iter()
        .map(|domain| domain.host().to_string())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();

    Entry {
        os: vec![os.into()],
        read: generalize(&observation.reads, ctx),
        write: generalize(&observation.writes, ctx),
        domains,
    }
}

#[cfg(test)]
mod tests {
    use std::{path::PathBuf, process::Command};

    use creance_proxy::HostPort;

    use super::*;

    #[test]
    fn synthesizes_deterministic_entry() {
        let temp = tempfile::tempdir().unwrap();
        let ctx = Context::from_roots(
            temp.path().join("pkg"),
            temp.path().join("project"),
            temp.path().join("store"),
            temp.path().join("home"),
            temp.path().join("cache"),
            temp.path().join("run"),
        );
        std::fs::create_dir_all(&ctx.run_tmp).unwrap();
        let out = ctx.run_tmp.join("out");
        std::fs::write(&out, "").unwrap();

        let status = Command::new("/usr/bin/true").status().unwrap();
        let observation = Observation {
            status,
            reads: vec![PathBuf::from("/usr/lib/libSystem.B.dylib")],
            writes: vec![out],
            domains: vec![
                HostPort::new("z.test", 443),
                HostPort::new("a.test", 443),
                HostPort::new("a.test", 443),
            ],
        };

        let entry = synthesize(&observation, "darwin", &ctx);
        assert_eq!(entry.os, vec!["darwin"]);
        assert_eq!(entry.write, vec!["${RUN_TMP}/**"]);
        assert!(entry.read.is_empty());
        assert_eq!(entry.domains, vec!["a.test", "z.test"]);
    }
}
