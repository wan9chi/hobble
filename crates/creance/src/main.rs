use std::{
    env,
    ffi::OsString,
    path::{Path, PathBuf},
    process::{Command, ExitCode, ExitStatus},
};

use anyhow::{Context as _, Result, anyhow};
use creance_engine::{
    Context, Decision, Mode, PackageId, Profile, decide, enforce, observe, store, synthesize,
};

#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(code) => ExitCode::from(code),
        Err(error) => {
            eprintln!("creance: {error:#}");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<u8> {
    let mut args = env::args_os().skip(1).collect::<Vec<_>>();
    if args.first().is_some_and(|arg| arg == "-c") {
        if args.len() != 2 {
            return Err(anyhow!("usage: creance -c <cmd>"));
        }
        return run_shim(args.remove(1)).await;
    }

    let Some(command) = args
        .first()
        .and_then(|arg| arg.to_str())
        .map(str::to_string)
    else {
        println!("creance {}", env!("CARGO_PKG_VERSION"));
        return Ok(0);
    };
    args.remove(0);

    match command.as_str() {
        "observe" => run_pnpm(PnpmMode::Observe, false, args),
        "install" => {
            let (strict, pnpm_args) = split_strict(args);
            run_pnpm(PnpmMode::Enforce, strict, pnpm_args)
        }
        "--version" | "-V" => {
            println!("creance {}", env!("CARGO_PKG_VERSION"));
            Ok(0)
        }
        other => Err(anyhow!("unknown command {other:?}")),
    }
}

async fn run_shim(command: OsString) -> Result<u8> {
    let package = package_from_env()?;
    let creance_dir = PathBuf::from(env::var_os("CREANCE_DIR").context("CREANCE_DIR is not set")?);
    let ctx = context_from_env(&package)?;
    let cwd = env::current_dir().context("read current directory")?;
    if !is_dependency_script(&ctx.pkg_dir) {
        return Ok(0);
    }

    let argv = vec![OsString::from("/bin/sh"), OsString::from("-c"), command];
    let env = env::vars_os().collect::<Vec<_>>();
    let lifecycle = env::var("npm_lifecycle_event").unwrap_or_else(|_| "install".to_string());
    let mode = env::var("CREANCE_MODE").unwrap_or_else(|_| "enforce".to_string());

    match mode.as_str() {
        "observe" => {
            observe_and_save(&argv, &env, &cwd, &ctx, &creance_dir, package, lifecycle).await
        }
        "enforce" => {
            let decision_mode = if env_truthy("CREANCE_STRICT") {
                Mode::Strict
            } else {
                Mode::Dev
            };
            match decide(&creance_dir, &package, current_os(), decision_mode) {
                Decision::Enforce(entry) => {
                    let status = enforce(&entry, &ctx, &argv, &env, &cwd).await?;
                    Ok(status_code(status))
                }
                Decision::Observe => {
                    observe_and_save(&argv, &env, &cwd, &ctx, &creance_dir, package, lifecycle)
                        .await
                }
                Decision::Fail(reason) => Err(anyhow!(reason)),
            }
        }
        other => Err(anyhow!("unsupported CREANCE_MODE {other:?}")),
    }
}

fn is_dependency_script(pkg_dir: &Path) -> bool {
    pkg_dir
        .components()
        .any(|component| component.as_os_str() == ".pnpm")
        && pkg_dir
            .components()
            .any(|component| component.as_os_str() == "node_modules")
}

async fn observe_and_save(
    argv: &[OsString],
    env: &[(OsString, OsString)],
    cwd: &Path,
    ctx: &Context,
    creance_dir: &Path,
    package: PackageId,
    lifecycle: String,
) -> Result<u8> {
    let observation = observe(argv, env, cwd).await?;
    let entry = synthesize(&observation, current_os(), ctx);
    let mut profile = store::load(creance_dir, &package.name, &package.version)?
        .unwrap_or_else(|| Profile::new(package, lifecycle));
    store::upsert_entry(&mut profile, entry);
    store::save(creance_dir, &profile)?;
    Ok(status_code(observation.status))
}

#[derive(Debug, Clone, Copy)]
enum PnpmMode {
    Observe,
    Enforce,
}

fn run_pnpm(mode: PnpmMode, strict: bool, user_args: Vec<OsString>) -> Result<u8> {
    let cwd = env::current_dir().context("read current directory")?;
    let creance_dir = cwd.join(".creance");
    let self_path = env::current_exe().context("resolve creance executable")?;

    let mut command = Command::new("pnpm");
    command
        .arg("install")
        .arg(format!("--config.script-shell={}", self_path.display()))
        .arg("--child-concurrency=5")
        .env(
            "CREANCE_MODE",
            match mode {
                PnpmMode::Observe => "observe",
                PnpmMode::Enforce => "enforce",
            },
        )
        .env("CREANCE_DIR", creance_dir);

    if should_pass_allow_all_builds(&cwd) {
        command.arg("--config.dangerously-allow-all-builds=true");
    }

    command.args(user_args);

    if strict {
        command.env("CREANCE_STRICT", "1");
    }

    let status = command.status().context("run pnpm install")?;
    Ok(status_code(status))
}

fn should_pass_allow_all_builds(cwd: &Path) -> bool {
    !["pnpm-lock.yaml", "pnpm-workspace.yaml", "package.json"]
        .into_iter()
        .any(|file| {
            std::fs::read_to_string(cwd.join(file))
                .is_ok_and(|contents| contents.contains("onlyBuiltDependencies"))
        })
}

fn split_strict(args: Vec<OsString>) -> (bool, Vec<OsString>) {
    let mut strict = false;
    let mut out = Vec::new();
    for arg in args {
        if arg == "--strict" {
            strict = true;
        } else {
            out.push(arg);
        }
    }
    (strict, out)
}

fn package_from_env() -> Result<PackageId> {
    let name = env::var("npm_package_name").context("npm_package_name is not set")?;
    let version = env::var("npm_package_version").context("npm_package_version is not set")?;
    Ok(PackageId::new(name, version))
}

fn context_from_env(package: &PackageId) -> Result<Context> {
    let cwd = env::current_dir().context("read current directory")?;
    let pkg_dir = env::var_os("PNPM_SCRIPT_SRC_DIR")
        .map(PathBuf::from)
        .unwrap_or(cwd);
    let project_root = env::var_os("INIT_CWD")
        .map(PathBuf::from)
        .unwrap_or_else(|| pkg_dir.clone());
    let home = env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| project_root.clone());
    let cache = env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join("Library/Caches"));
    let store = env::var_os("CREANCE_STORE")
        .map(PathBuf::from)
        .unwrap_or_else(|| project_root.join(".pnpm-store"));
    let run_tmp = env::var_os("CREANCE_RUN_TMP")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            env::temp_dir()
                .join("creance")
                .join(safe_package_dir(package))
        });
    std::fs::create_dir_all(&run_tmp).with_context(|| format!("create {}", run_tmp.display()))?;

    Ok(Context::from_roots(
        pkg_dir,
        project_root,
        store,
        home,
        cache,
        run_tmp,
    ))
}

fn safe_package_dir(package: &PackageId) -> String {
    format!("{}-{}", package.name, package.version)
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '.' || ch == '-' {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

fn env_truthy(key: &str) -> bool {
    env::var(key).is_ok_and(|value| matches!(value.as_str(), "1" | "true" | "yes"))
}

fn current_os() -> &'static str {
    #[cfg(target_os = "macos")]
    {
        "darwin"
    }
    #[cfg(not(target_os = "macos"))]
    {
        std::env::consts::OS
    }
}

fn status_code(status: ExitStatus) -> u8 {
    status
        .code()
        .and_then(|code| u8::try_from(code).ok())
        .unwrap_or(1)
}
