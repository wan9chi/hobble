# Creance Implementation Notes

This ledger records implementation decisions that close gaps or deviations from
`docs/DESIGN.md` and `docs/PLAN.md`.

## Workspace bootstrap

- **Area affected:** workspace/tooling.
- **Gap/deviation/oversight:** the repository only contained docs and a
  `mise.toml` with Node and pnpm pins.
- **Decision:** add the Rust workspace, crate skeletons, `rust-toolchain.toml`,
  and mise tasks without adding Rust or Cargo to mise. This follows the global
  tool-management rule and keeps Rust under rustup.
- **Follow-up condition:** retire this note when M0 is committed or when the
  bootstrap details are captured in a changelog.

## macOS base SBPL boot allowances

- **Area affected:** `creance-sandbox` base profile.
- **Gap/deviation/oversight:** `docs/DESIGN.md` names the broad classes of
  mandatory macOS allowances but does not call out that `/` itself must be
  readable as a literal root directory for `/bin/sh` to boot under
  `(deny default)`.
- **Decision:** add `(allow file-read* (literal "/"))` while keeping package and
  temp-tree reads out of the base profile. Tests that need package/cwd access
  add explicit read/write rules.
- **Follow-up condition:** keep unless a later macOS version no longer requires
  the literal root allowance.

## Node runtime discovery

- **Area affected:** `creance-sandbox` runtime read roots.
- **Gap/deviation/oversight:** `command -v node` may resolve to a local wrapper
  rather than the mise-pinned Node binary from `mise.toml`.
- **Decision:** prefer `mise which node` and allow both its `bin` directory and
  install root as runtime read roots, falling back to `command -v node` when mise
  is unavailable.
- **Follow-up condition:** keep while mise is the source of pinned Node/pnpm
  tooling.

## Nightly Cargo for local fspy

- **Area affected:** Rust toolchain.
- **Gap/deviation/oversight:** the local `../vite-task/crates/fspy` dependency
  uses artifact dependencies for preload libraries, which require Cargo
  `-Z bindeps`.
- **Decision:** align Creance with `../vite-task/rust-toolchain.toml` by using
  `nightly-2026-06-07` and enabling `[unstable] bindeps = true` in local Cargo
  config. Rust remains managed only by rustup and `rust-toolchain.toml`.
  Development started against the local checkout, then moved to the public
  upstream revision
  `voidzero-dev/vite-task@8daa9bb72faa89b745cb58c087b416b15d3bddc5` for CI
  portability.
- **Follow-up condition:** revisit before final publish/CI if the upstream
  `fspy` branch no longer requires nightly Cargo.

## pnpm 11 build allowlist conflict

- **Area affected:** `creance observe` / `creance install` pnpm runner.
- **Gap/deviation/oversight:** the plan assumed passing
  `--config.dangerously-allow-all-builds=true` is always accepted. pnpm 11
  rejects that flag when `pnpm-lock.yaml` already contains
  `onlyBuiltDependencies`.
- **Decision:** pass the dangerous allow-all flag only when there is no
  allowlist in `pnpm-lock.yaml`, `pnpm-workspace.yaml`, or root `package.json`.
  Skip it whenever those files already carry `onlyBuiltDependencies`. The shim
  remains the gatekeeper either way because `script-shell` is still set to
  `creance`.
- **Follow-up condition:** keep until pnpm's build approval model changes or
  Creance grows explicit lockfile parsing for build-script policy.

## pnpm child concurrency flag

- **Area affected:** `creance observe` / `creance install` pnpm runner.
- **Gap/deviation/oversight:** the plan used
  `--config.child-concurrency=...`, but pnpm 11 accepts the direct install
  option `--child-concurrency=...` and can fail after lifecycle execution when
  the config form is used.
- **Decision:** pass `--child-concurrency=5` directly.
- **Follow-up condition:** keep unless pnpm removes or renames the install
  option.

## Multi-lifecycle profile merging

- **Area affected:** profile store.
- **Gap/deviation/oversight:** `store::upsert_entry` originally replaced the
  entry for a matching OS. Real packages such as `node-pty` run multiple
  lifecycle scripts for the same package/version, and replacing would lose
  permissions observed from earlier stages.
- **Decision:** merge read, write, and domain allowlists for matching OS entries
  while still keeping one flat OS-tagged entry per package/version.
- **Follow-up condition:** revisit if profiles become lifecycle-event-specific
  instead of package/version-specific.

## First-party workspace script skipping

- **Area affected:** pnpm shim.
- **Gap/deviation/oversight:** workspace fixtures can run first-party root or
  importer scripts such as `prepare`/`rebuild`; these are not the third-party
  dependency lifecycle scripts Creance is trying to constrain.
- **Decision:** when `PNPM_SCRIPT_SRC_DIR` is outside `node_modules/.pnpm`, skip
  the command successfully instead of observing/enforcing or writing a profile.
  This keeps fixtures focused on dependency lifecycle scripts and avoids
  requiring non-install-input first-party helper files. Dependency scripts under
  pnpm's virtual store remain gated.
- **Follow-up condition:** revisit when Creance grows explicit first-party
  policy controls.

## Native prebuild cache nondeterminism

- **Area affected:** `better-sqlite3` fixture profiles.
- **Gap/deviation/oversight:** `prebuild-install` behavior depends on the
  user's npm cache state. One observe run recorded writes under `${HOME}/.npm`,
  while a later run with a warm cache did not, but strict replay still needed
  write/access permission for that cache path.
- **Decision:** keep `${HOME}/.npm/**` in the URL Shortener
  `better-sqlite3@12.6.2` fixture profile as a stable native prebuild cache
  allowance. The Kudos `better-sqlite3@11.10.0` fixture intentionally exercises
  the stricter fallback path where an npm-cache access denial causes
  `prebuild-install` to fall back to the profiled local `node-gyp` build.
- **Follow-up condition:** replace this manual fixture allowance when observe
  grows deterministic cache-root defaults for native prebuild tools.

## Fixture-local git hook state

- **Area affected:** e2e fixture harness and Kindle AI Export fixture.
- **Gap/deviation/oversight:** `simple-git-hooks@2.13.1` creates
  `.git/hooks/pre-commit` during postinstall when the fixture has a local
  `.git` directory. That directory is generated install state, not one of the
  vendored install inputs.
- **Decision:** exclude `.git` directories when copying fixtures into temp e2e
  workspaces, and clean generated `.git` directories from fixture sources before
  commit. The committed profile still records the dependency script's write
  surface as `${PROJECT_ROOT}/.git/**`.
- **Follow-up condition:** revisit if Creance adds a first-class policy for
  dependency scripts that try to write VCS metadata.

## M10 private repository publication

- **Area affected:** publish/CI operation.
- **Gap/deviation/oversight:** pushing `.github/workflows/ci.yml` over the
  GitHub CLI-created HTTPS remote failed because the OAuth token did not have
  the `workflow` scope.
- **Decision:** create the private repository `wan9chi/creance`, switch the
  remote to SSH, push `main`, and watch the required macOS CI job. The push CI
  run `27913241473` completed green.
- **Follow-up condition:** none for M10's required local/push CI path. Optional
  ignored e2e remains available through `workflow_dispatch` with `e2e=true`.
