use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::Command,
};

use creance_engine::{SandboxProfileJson, sandbox_profile_json_for_entry, store};

#[test]
#[ignore = "networked pnpm fixture"]
fn aspect_fixture_installs_strict_and_blocks_unprofiled_write() {
    let temp = tempfile::tempdir().unwrap();
    let fixture = temp.path().join("aspect-c");
    copy_fixture(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../e2e/fixtures/aspect-c"),
        &fixture,
    );

    let status = Command::new(creance_bin())
        .arg("install")
        .arg("--strict")
        .arg("--force")
        .arg("--store-dir")
        .arg(".pnpm-store")
        .current_dir(&fixture)
        .status()
        .unwrap();
    assert!(status.success());

    let package_dir =
        fixture.join("node_modules/.pnpm/@aspect-test+c@2.0.0/node_modules/@aspect-test/c");
    assert_eq!(
        fs::read_to_string(package_dir.join("data.json"))
            .unwrap()
            .trim(),
        "{\"answer\":\"42*\"}"
    );

    let status = Command::new(creance_bin())
        .arg("-c")
        .arg("echo bad > \"$INIT_CWD/pwned\"")
        .current_dir(&package_dir)
        .env("CREANCE_MODE", "enforce")
        .env("CREANCE_STRICT", "1")
        .env("CREANCE_DIR", fixture.join(".creance"))
        .env("npm_package_name", "@aspect-test/c")
        .env("npm_package_version", "2.0.0")
        .env("npm_lifecycle_event", "postinstall")
        .env("PNPM_SCRIPT_SRC_DIR", &package_dir)
        .env("INIT_CWD", &fixture)
        .status()
        .unwrap();
    assert!(!status.success());
    assert!(!fixture.join("pwned").exists());

    let profile = store::load(&fixture.join(".creance"), "@aspect-test/c", "2.0.0")
        .unwrap()
        .unwrap();
    let entry = profile
        .entries
        .iter()
        .find(|entry| entry.os.iter().any(|os| os == "darwin"))
        .unwrap();
    let regenerated = sandbox_profile_json_for_entry(entry);
    let committed: SandboxProfileJson = serde_json::from_slice(
        &fs::read(fixture.join(".creance/sandbox-profiles/@aspect-test/c/2.0.0.darwin.json"))
            .unwrap(),
    )
    .unwrap();
    assert_eq!(regenerated, committed);
}

fn copy_fixture(src: PathBuf, dst: &Path) {
    fs::create_dir_all(dst).unwrap();
    for entry in walk(&src) {
        let rel = entry.strip_prefix(&src).unwrap();
        if rel.components().any(|component| {
            let value = component.as_os_str();
            value == "node_modules" || value == ".pnpm-store"
        }) {
            continue;
        }
        let target = dst.join(rel);
        if entry.is_dir() {
            fs::create_dir_all(&target).unwrap();
        } else {
            fs::create_dir_all(target.parent().unwrap()).unwrap();
            fs::copy(&entry, &target).unwrap();
            let mode = fs::metadata(&entry).unwrap().permissions().mode();
            let mut permissions = fs::metadata(&target).unwrap().permissions();
            permissions.set_mode(mode);
            fs::set_permissions(&target, permissions).unwrap();
        }
    }
}

fn walk(root: &Path) -> Vec<PathBuf> {
    let mut out = vec![root.to_path_buf()];
    let mut idx = 0;
    while idx < out.len() {
        let path = out[idx].clone();
        idx += 1;
        if path.is_dir() {
            for entry in fs::read_dir(path).unwrap() {
                out.push(entry.unwrap().path());
            }
        }
    }
    out
}

fn creance_bin() -> &'static str {
    env!("CARGO_BIN_EXE_creance")
}
