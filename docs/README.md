# Creance

Creance is a macOS proof of concept for observing and enforcing pnpm lifecycle
script behavior. It records filesystem accesses and CONNECT proxy domains during
observe mode, stores a reviewed profile under `.creance/profiles`, then runs the
same dependency lifecycle script under a deny-by-default macOS Seatbelt sandbox
in strict install mode.

The current implementation is macOS-only and targets dependency scripts executed
from pnpm's virtual store, such as `node_modules/.pnpm/<pkg>/node_modules/<pkg>`.
First-party workspace scripts are skipped for now so the profile surface stays
focused on third-party dependency install behavior.

## Prerequisites

Use the repo's checked-in tool configuration:

```sh
mise install
```

Representative output:

```text
mise node@24.17.0    installed
mise pnpm@11.8.0     installed
```

The Rust toolchain is managed by `rustup` through `rust-toolchain.toml`; do not
add Rust or Cargo to mise.

## Build and Check

1. Build the binary.

   ```sh
   cargo build -p creance
   ```

   Output:

   ```text
   Finished `dev` profile [unoptimized + debuginfo] target(s) in ...
   ```

2. Format the workspace.

   ```sh
   mise run fmt
   ```

   Output:

   ```text
   [fmt] $ cargo fmt --all
   ```

3. Run clippy.

   ```sh
   mise run lint
   ```

   Output:

   ```text
   [lint] $ cargo clippy --workspace --all-targets -- -D warnings
   Finished `dev` profile [unoptimized + debuginfo] target(s) in ...
   ```

4. Run the normal test suite.

   ```sh
   mise run test
   ```

   Output:

   ```text
   [test] $ cargo test --workspace --all-targets
   test result: ok. ... passed; 0 failed
   ```

## Observe, Review, and Enforce

This transcript uses the smallest committed fixture,
`e2e/fixtures/aspect-c`. Work from a copy so the committed profile and golden
files are not overwritten while experimenting.

1. Copy the fixture into a temporary project.

   ```sh
   repo=$(pwd)
   workdir=$(mktemp -d)
   cp -R "$repo/e2e/fixtures/aspect-c" "$workdir/aspect-c"
   cd "$workdir/aspect-c"
   rm -rf .creance node_modules .pnpm-store
   ```

   Output:

   ```text
   # no output on success
   ```

2. Observe the dependency lifecycle script.

   ```sh
   "$repo/target/debug/creance" observe --force --store-dir .pnpm-store
   ```

   Output:

   ```text
   .../node_modules/@aspect-test/c postinstall$ echo "{\"answer\":\"42*\"}" > data.json
   .../node_modules/@aspect-test/c postinstall: Done

   dependencies:
   + @aspect-test/c 2.0.0

   Done in ... using pnpm ...
   ```

3. Inspect the generated profile path.

   ```sh
   find .creance -type f | sort
   ```

   Output:

   ```text
   .creance/profiles/@aspect-test/c/2.0.0.json
   ```

4. Review the generated profile JSON.

   ```sh
   sed -n '1,120p' .creance/profiles/@aspect-test/c/2.0.0.json
   ```

   Output:

   ```json
   {
     "package": {
       "name": "@aspect-test/c",
       "version": "2.0.0"
     },
     "creance": "0.1.0",
     "lifecycle": "postinstall",
     "entries": [
       {
         "os": [
           "darwin"
         ],
         "read": [
           "${PKG_DIR}",
           "${PROJECT_ROOT}",
           "/private/var/db/timezone/tz/2026b.1.0/zoneinfo/Asia/Shanghai"
         ],
         "write": [
           "${PKG_DIR}/data.json"
         ],
         "domains": []
       }
     ]
   }
   ```

5. Generate the sandbox-surface JSON golden from the profile.

   ```sh
   "$repo/target/debug/creance" sandbox-json \
     .creance @aspect-test/c 2.0.0 darwin \
     .creance/sandbox-profiles/@aspect-test/c/2.0.0.darwin.json
   sed -n '1,120p' .creance/sandbox-profiles/@aspect-test/c/2.0.0.darwin.json
   ```

   Output:

   ```json
   {
     "schema": 1,
     "os": [
       "darwin"
     ],
     "file_read": [
       "${PKG_DIR}",
       "${PROJECT_ROOT}",
       "/private/var/db/timezone/tz/2026b.1.0/zoneinfo/Asia/Shanghai"
     ],
     "file_write": [
       "${PKG_DIR}/data.json"
     ],
     "network": {
       "mode": "proxy-only",
       "domains": []
     }
   }
   ```

6. Reinstall under strict enforcement.

   ```sh
   rm -rf node_modules .pnpm-store
   "$repo/target/debug/creance" install --strict --force --store-dir .pnpm-store
   cat node_modules/.pnpm/@aspect-test+c@2.0.0/node_modules/@aspect-test/c/data.json
   ```

   Output:

   ```text
   .../node_modules/@aspect-test/c postinstall$ echo "{\"answer\":\"42*\"}" > data.json
   .../node_modules/@aspect-test/c postinstall: Done
   {"answer":"42*"}
   ```

7. Confirm an unprofiled write is blocked.

   ```sh
   pkg_dir="$PWD/node_modules/.pnpm/@aspect-test+c@2.0.0/node_modules/@aspect-test/c"
   set +e
   CREANCE_MODE=enforce \
     CREANCE_STRICT=1 \
     CREANCE_DIR="$PWD/.creance" \
     npm_package_name="@aspect-test/c" \
     npm_package_version="2.0.0" \
     npm_lifecycle_event=postinstall \
     PNPM_SCRIPT_SRC_DIR="$pkg_dir" \
     INIT_CWD="$PWD" \
     "$repo/target/debug/creance" -c 'echo bad > "$INIT_CWD/pwned"'
   echo "exit=$?"
   set -e
   test ! -e pwned
   ```

   Output:

   ```text
   /bin/sh: .../aspect-c/pwned: Operation not permitted
   exit=1
   ```

## Generated Profile JSON

Profile files live at:

```text
.creance/profiles/<package-name>/<version>.json
```

They are the reviewed policy source that should be committed with a fixture or a
real project. Important fields:

- `package`: the npm package name and version this profile applies to.
- `creance`: the Creance version that wrote the profile.
- `lifecycle`: the lifecycle event that created the profile. Permissions are
  stored per package/version; when one package runs multiple lifecycle scripts,
  Creance merges the observed allowlists into the same OS entry.
- `entries`: OS-specific policy entries. The current PoC writes `darwin`
  entries.
- `read`: filesystem paths the script may read.
- `write`: filesystem paths the script may write.
- `domains`: CONNECT proxy hostnames the script may contact during strict
  enforcement.

Path entries may be absolute macOS paths or template variables:

- `${PKG_DIR}`: the package directory under pnpm's virtual store.
- `${PROJECT_ROOT}`: the install root, normally the directory where
  `creance observe` or `creance install` was run.
- `${STORE}`: the pnpm store path passed with `--store-dir`, or the default
  project-local store.
- `${HOME}`: the user's home directory.
- `${CACHE}`: the user's cache directory.
- `${RUN_TMP}`: Creance's per-package temporary runtime directory.

Entries ending in `/**` are subtree allowances. For example,
`${PKG_DIR}/build/**` means the package script may read or write anything below
its own `build` directory but not arbitrary files in the project root.

## Sandbox-Surface JSON

The files under `.creance/sandbox-profiles/.../*.darwin.json` are generated
goldens used by tests and reviews. They are not a separate policy language; they
are the enforce surface derived from a profile entry:

- `file_read`: the profile `read` list after selecting the OS entry.
- `file_write`: the profile `write` list after selecting the OS entry.
- `network.mode`: currently `proxy-only`; direct network egress is denied by the
  macOS sandbox, and allowed hosts must go through Creance's CONNECT proxy.
- `network.domains`: the profile `domains` list.

Regenerate one with:

```sh
target/debug/creance sandbox-json \
  e2e/fixtures/aspect-c/.creance @aspect-test/c 2.0.0 darwin
```

Output:

```json
{
  "schema": 1,
  "os": [
    "darwin"
  ],
  "file_read": [
    "${PKG_DIR}",
    "${PROJECT_ROOT}",
    "/private/var/db/timezone/tz/2026b.1.0/zoneinfo/Asia/Shanghai"
  ],
  "file_write": [
    "${PKG_DIR}/data.json"
  ],
  "network": {
    "mode": "proxy-only",
    "domains": []
  }
}
```

## Fixture Matrix

Run all ignored networked fixtures:

```sh
mise run e2e
```

Output:

```text
[e2e] $ cargo test --workspace --all-targets -- --ignored
running 6 tests
test aspect_fixture_installs_strict_and_blocks_unprofiled_write ... ok
test tasuku_fixture_enforces_node_pty_lifecycle_scripts ... ok
test url_shortener_fixture_enforces_better_sqlite3 ... ok
test angular_calendar_fixture_enforces_workspace_native_scripts ... ok
test kudos_fixture_enforces_multi_version_native_scripts ... ok
test kindle_ai_export_fixture_enforces_network_cache_scripts ... ok

test result: ok. 6 passed; 0 failed
```

The fixture suite covers:

- Aspect: small deterministic postinstall write.
- URL Shortener: `better-sqlite3@12.6.2` native prebuild/cache behavior.
- Tasuku: `node-pty@1.2.0-beta.10` install and postinstall lifecycle behavior.
- Angular Calendar: multiple native workspace build dependencies.
- Kudos: `better-sqlite3@11.10.0` plus four distinct `esbuild` versions.
- Kindle AI Export: `sharp@0.34.4`, `esbuild@0.25.11`, and
  `simple-git-hooks@2.13.1`.

## CI

Push and pull-request CI run the macOS Rust job:

```text
mise run fmt
mise run lint
mise run test
```

The ignored e2e fixture matrix is available through GitHub Actions
`workflow_dispatch` with `e2e=true`.
