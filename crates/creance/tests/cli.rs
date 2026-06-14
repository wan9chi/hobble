use std::{ffi::OsStr, fs, os::unix::fs::PermissionsExt, path::Path, process::Command};

#[test]
fn shim_observes_saves_enforces_and_blocks_tampering() {
    let temp = tempfile::tempdir().unwrap();
    let pkg = temp.path().join("pkg");
    let creance_dir = temp.path().join(".creance");
    let outside = temp.path().join("outside");
    fs::create_dir_all(&pkg).unwrap();
    fs::create_dir_all(&outside).unwrap();

    let status = shim_command(&pkg, &creance_dir)
        .arg("-c")
        .arg("echo hi; echo x > \"$PWD/out\"")
        .env("CREANCE_MODE", "observe")
        .status()
        .unwrap();
    assert!(status.success());
    assert!(pkg.join("out").exists());
    assert!(creance_dir.join("profiles/pkg/1.0.0.json").exists());

    let status = shim_command(&pkg, &creance_dir)
        .arg("-c")
        .arg("echo hi; echo x > \"$PWD/out\"")
        .env("CREANCE_MODE", "enforce")
        .env("CREANCE_STRICT", "1")
        .status()
        .unwrap();
    assert!(status.success());

    let status = shim_command(&pkg, &creance_dir)
        .arg("-c")
        .arg("echo bad > \"$OUTSIDE/pwned\"")
        .env("CREANCE_MODE", "enforce")
        .env("CREANCE_STRICT", "1")
        .env("OUTSIDE", &outside)
        .status()
        .unwrap();
    assert!(!status.success());
    assert!(!outside.join("pwned").exists());
}

#[test]
fn observe_command_wires_pnpm_with_script_shell_and_env() {
    let temp = tempfile::tempdir().unwrap();
    let fake = install_fake_pnpm(temp.path());

    let status = Command::new(creance_bin())
        .arg("observe")
        .arg("--offline")
        .env(
            "PATH",
            format!("{}:{}", fake.display(), std::env::var("PATH").unwrap()),
        )
        .env("PNPM_ARGV_OUT", temp.path().join("argv"))
        .env("PNPM_ENV_OUT", temp.path().join("env"))
        .current_dir(temp.path())
        .status()
        .unwrap();
    assert!(status.success());

    let argv = fs::read_to_string(temp.path().join("argv")).unwrap();
    assert!(argv.contains("install\n"));
    assert!(argv.contains("--config.dangerously-allow-all-builds=true\n"));
    assert!(argv.contains("--child-concurrency=5\n"));
    assert!(argv.contains("--offline\n"));
    assert!(argv.contains("--config.script-shell="));

    let env = fs::read_to_string(temp.path().join("env")).unwrap();
    assert!(env.contains("CREANCE_MODE=observe\n"));
    let creance_dir = temp.path().canonicalize().unwrap().join(".creance");
    assert!(env.contains(&format!("CREANCE_DIR={}\n", creance_dir.display())));
}

#[test]
fn install_command_wires_enforce_mode_and_strict_flag() {
    let temp = tempfile::tempdir().unwrap();
    let fake = install_fake_pnpm(temp.path());

    let status = Command::new(creance_bin())
        .arg("install")
        .arg("--frozen-lockfile")
        .env(
            "PATH",
            format!("{}:{}", fake.display(), std::env::var("PATH").unwrap()),
        )
        .env("PNPM_ARGV_OUT", temp.path().join("argv-normal"))
        .env("PNPM_ENV_OUT", temp.path().join("env-normal"))
        .current_dir(temp.path())
        .status()
        .unwrap();
    assert!(status.success());
    let env = fs::read_to_string(temp.path().join("env-normal")).unwrap();
    assert!(env.contains("CREANCE_MODE=enforce\n"));
    assert!(!env.contains("CREANCE_STRICT=1\n"));
    let argv = fs::read_to_string(temp.path().join("argv-normal")).unwrap();
    assert!(argv.contains("--frozen-lockfile\n"));

    let status = Command::new(creance_bin())
        .arg("install")
        .arg("--strict")
        .arg("--offline")
        .env(
            "PATH",
            format!("{}:{}", fake.display(), std::env::var("PATH").unwrap()),
        )
        .env("PNPM_ARGV_OUT", temp.path().join("argv-strict"))
        .env("PNPM_ENV_OUT", temp.path().join("env-strict"))
        .current_dir(temp.path())
        .status()
        .unwrap();
    assert!(status.success());
    let env = fs::read_to_string(temp.path().join("env-strict")).unwrap();
    assert!(env.contains("CREANCE_MODE=enforce\n"));
    assert!(env.contains("CREANCE_STRICT=1\n"));
    let argv = fs::read_to_string(temp.path().join("argv-strict")).unwrap();
    assert!(argv.contains("--offline\n"));
    assert!(!argv.contains("--strict\n"));
}

fn shim_command(pkg: &Path, creance_dir: &Path) -> Command {
    let mut command = Command::new(creance_bin());
    command
        .current_dir(pkg)
        .env("CREANCE_DIR", creance_dir)
        .env("npm_package_name", "pkg")
        .env("npm_package_version", "1.0.0")
        .env("npm_lifecycle_event", "postinstall")
        .env("PNPM_SCRIPT_SRC_DIR", pkg)
        .env("INIT_CWD", pkg);
    command
}

fn install_fake_pnpm(root: &Path) -> std::path::PathBuf {
    let dir = root.join("fake-bin");
    fs::create_dir_all(&dir).unwrap();
    let pnpm = dir.join("pnpm");
    fs::write(
        &pnpm,
        "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$PNPM_ARGV_OUT\"\nenv > \"$PNPM_ENV_OUT\"\n",
    )
    .unwrap();
    let mut permissions = fs::metadata(&pnpm).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&pnpm, permissions).unwrap();
    dir
}

fn creance_bin() -> &'static OsStr {
    OsStr::new(env!("CARGO_BIN_EXE_creance"))
}
