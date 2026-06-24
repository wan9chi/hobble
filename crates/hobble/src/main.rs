//! `hobble` — a pure-Rust sandbox for npm/pnpm lifecycle scripts.
//!
//! This is the skeleton entry point. It dispatches the two roles the binary
//! plays (see `docs/DESIGN.md` §3) and leaves the engine wiring unimplemented:
//!
//! * **entry mode** — `hobble observe` / `hobble install`, invoked by the user;
//!   drives pnpm with `--config.script-shell=$(self)`.
//! * **shim mode** — `hobble -c "<cmd>"`, invoked by pnpm once per dependency
//!   lifecycle script; reads `CREANCE_*` + `npm_*` env to decide
//!   observe | enforce | refuse (fail-closed).

use std::{env, process::ExitCode};

use anyhow::{Result, anyhow};

#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(code) => ExitCode::from(code),
        Err(error) => {
            eprintln!("hobble: {error:#}");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<u8> {
    let mut args = env::args_os().skip(1).collect::<Vec<_>>();

    // shim mode: pnpm calls `<script-shell> -c "<cmd>"` per lifecycle script.
    if args.first().is_some_and(|arg| arg == "-c") {
        if args.len() != 2 {
            return Err(anyhow!("usage: hobble -c <cmd>"));
        }
        let cmd = args.remove(1);
        return run_shim(cmd.to_string_lossy().into_owned()).await;
    }

    // entry mode.
    match args.first().and_then(|a| a.to_str()) {
        Some("observe") => run_observe().await,
        Some("install") => run_install().await,
        Some(other) => Err(anyhow!("unknown command: {other}")),
        None => Err(anyhow!("usage: hobble <observe|install>")),
    }
}

/// Learn profiles: run pnpm unsandboxed + instrumented, write `.creance/`.
async fn run_observe() -> Result<u8> {
    todo!("drive pnpm in observe mode (DESIGN.md §3.2)")
}

/// Enforce: drop-in for `pnpm install`, deny-by-default sandbox per profile.
async fn run_install() -> Result<u8> {
    todo!("drive pnpm in enforce mode (DESIGN.md §3.3)")
}

/// Per-script shim: observe | enforce | refuse, fail-closed.
async fn run_shim(_cmd: String) -> Result<u8> {
    todo!("dispatch on CREANCE_* + npm_* env (DESIGN.md §2.4)")
}
