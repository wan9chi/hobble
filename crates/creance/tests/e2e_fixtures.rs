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
    run_fixture(FixtureSpec {
        fixture: "aspect-c",
        packages: &[PackageSpec {
            name: "@aspect-test/c",
            version: "2.0.0",
            expected_file: "data.json",
            expected_content: Some("{\"answer\":\"42*\"}"),
        }],
    });
}

#[test]
#[ignore = "networked pnpm fixture"]
fn tasuku_fixture_enforces_node_pty_lifecycle_scripts() {
    run_fixture(FixtureSpec {
        fixture: "tasuku",
        packages: &[PackageSpec {
            name: "node-pty",
            version: "1.2.0-beta.10",
            expected_file: "prebuilds/darwin-arm64/pty.node",
            expected_content: None,
        }],
    });
}

#[test]
#[ignore = "networked pnpm fixture"]
fn url_shortener_fixture_enforces_better_sqlite3() {
    run_fixture(FixtureSpec {
        fixture: "url-shortener",
        packages: &[PackageSpec {
            name: "better-sqlite3",
            version: "12.6.2",
            expected_file: "build/Release/better_sqlite3.node",
            expected_content: None,
        }],
    });
}

#[test]
#[ignore = "networked pnpm fixture"]
fn angular_calendar_fixture_enforces_workspace_native_scripts() {
    run_fixture(FixtureSpec {
        fixture: "angular-calendar",
        packages: &[
            PackageSpec {
                name: "@parcel/watcher",
                version: "2.5.1",
                expected_file: "scripts/build-from-source.js",
                expected_content: None,
            },
            PackageSpec {
                name: "esbuild",
                version: "0.25.9",
                expected_file: "bin/esbuild",
                expected_content: None,
            },
            PackageSpec {
                name: "lmdb",
                version: "3.4.2",
                expected_file: "native.js",
                expected_content: None,
            },
            PackageSpec {
                name: "msgpackr-extract",
                version: "3.0.3",
                expected_file: "index.js",
                expected_content: None,
            },
        ],
    });
}

#[test]
#[ignore = "networked pnpm fixture"]
fn kudos_fixture_enforces_multi_version_native_scripts() {
    run_fixture(FixtureSpec {
        fixture: "kudos",
        packages: &[
            PackageSpec {
                name: "better-sqlite3",
                version: "11.10.0",
                expected_file: "build/Release/better_sqlite3.node",
                expected_content: None,
            },
            PackageSpec {
                name: "esbuild",
                version: "0.18.20",
                expected_file: "bin/esbuild",
                expected_content: None,
            },
            PackageSpec {
                name: "esbuild",
                version: "0.19.12",
                expected_file: "bin/esbuild",
                expected_content: None,
            },
            PackageSpec {
                name: "esbuild",
                version: "0.21.5",
                expected_file: "bin/esbuild",
                expected_content: None,
            },
            PackageSpec {
                name: "esbuild",
                version: "0.27.3",
                expected_file: "bin/esbuild",
                expected_content: None,
            },
        ],
    });
}

#[test]
#[ignore = "networked pnpm fixture"]
fn kindle_ai_export_fixture_enforces_network_cache_scripts() {
    run_fixture(FixtureSpec {
        fixture: "kindle-ai-export",
        packages: &[
            PackageSpec {
                name: "esbuild",
                version: "0.25.11",
                expected_file: "bin/esbuild",
                expected_content: None,
            },
            PackageSpec {
                name: "sharp",
                version: "0.34.4",
                expected_file: "install/check.js",
                expected_content: None,
            },
            PackageSpec {
                name: "simple-git-hooks",
                version: "2.13.1",
                expected_file: "simple-git-hooks.js",
                expected_content: None,
            },
        ],
    });
}

struct FixtureSpec<'a> {
    fixture: &'a str,
    packages: &'a [PackageSpec<'a>],
}

struct PackageSpec<'a> {
    name: &'a str,
    version: &'a str,
    expected_file: &'a str,
    expected_content: Option<&'a str>,
}

fn run_fixture(spec: FixtureSpec<'_>) {
    let temp = tempfile::tempdir().unwrap();
    let fixture = temp.path().join(spec.fixture);
    copy_fixture(fixture_root(spec.fixture), &fixture);

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

    for package in spec.packages {
        assert_expected_side_effect(&fixture, package);
        assert_unprofiled_write_is_blocked(&fixture, package);
        assert_sandbox_golden_matches(&fixture, package);
    }
}

fn assert_expected_side_effect(fixture: &Path, package: &PackageSpec<'_>) {
    let path = package_dir(fixture, package).join(package.expected_file);
    assert!(path.exists(), "expected side effect {}", path.display());
    if let Some(expected) = package.expected_content {
        assert_eq!(fs::read_to_string(&path).unwrap().trim(), expected);
    }
}

fn assert_unprofiled_write_is_blocked(fixture: &Path, package: &PackageSpec<'_>) {
    let package_dir = package_dir(fixture, package);
    let status = Command::new(creance_bin())
        .arg("-c")
        .arg("echo bad > \"$INIT_CWD/pwned\"")
        .current_dir(&package_dir)
        .env("CREANCE_MODE", "enforce")
        .env("CREANCE_STRICT", "1")
        .env("CREANCE_DIR", fixture.join(".creance"))
        .env("npm_package_name", package.name)
        .env("npm_package_version", package.version)
        .env("npm_lifecycle_event", "postinstall")
        .env("PNPM_SCRIPT_SRC_DIR", &package_dir)
        .env("INIT_CWD", fixture)
        .status()
        .unwrap();
    assert!(!status.success());
    assert!(!fixture.join("pwned").exists());
}

fn assert_sandbox_golden_matches(fixture: &Path, package: &PackageSpec<'_>) {
    let profile = store::load(&fixture.join(".creance"), package.name, package.version)
        .unwrap()
        .unwrap();
    let entry = profile
        .entries
        .iter()
        .find(|entry| entry.os.iter().any(|os| os == "darwin"))
        .unwrap();
    let regenerated = sandbox_profile_json_for_entry(entry);
    let committed: SandboxProfileJson =
        serde_json::from_slice(&fs::read(sandbox_golden_path(fixture, package)).unwrap()).unwrap();
    assert_eq!(regenerated, committed);
}

fn sandbox_golden_path(fixture: &Path, package: &PackageSpec<'_>) -> PathBuf {
    fixture
        .join(".creance/sandbox-profiles")
        .join(package.name)
        .join(format!("{}.darwin.json", package.version))
}

fn package_dir(fixture: &Path, package: &PackageSpec<'_>) -> PathBuf {
    let node_modules = Path::new("node_modules").join(package.name);
    let escaped = package.name.replace('/', "+");
    let pnpm_dir = fixture.join("node_modules/.pnpm");
    for entry in fs::read_dir(&pnpm_dir).unwrap() {
        let entry = entry.unwrap();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !name.starts_with(&format!("{escaped}@{}", package.version)) {
            continue;
        }
        let candidate = entry.path().join(&node_modules);
        if candidate.is_dir() {
            return candidate;
        }
    }
    panic!(
        "could not find package dir for {}@{} under {}",
        package.name,
        package.version,
        pnpm_dir.display()
    );
}

fn fixture_root(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../e2e/fixtures")
        .join(name)
}

fn copy_fixture(src: PathBuf, dst: &Path) {
    fs::create_dir_all(dst).unwrap();
    for entry in walk(&src) {
        let rel = entry.strip_prefix(&src).unwrap();
        if rel.components().any(|component| {
            let value = component.as_os_str();
            value == "node_modules" || value == ".pnpm-store" || value == ".git"
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
