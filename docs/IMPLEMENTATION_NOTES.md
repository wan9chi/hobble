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
- **Decision:** pass the dangerous allow-all flag only when there is no lockfile
  allowlist. Skip it whenever `pnpm-lock.yaml` already carries
  `onlyBuiltDependencies`. The shim remains the gatekeeper either way because
  `script-shell` is still set to `creance`.
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

## M10 private repository side effect

- **Area affected:** publish/CI operation.
- **Gap/deviation/oversight:** M10 asks the implementer to create a private
  GitHub repository and push the branch after all local work is complete.
- **Decision:** add a CI workflow and verify the local workspace, but do not
  create the private repository implicitly. Repository creation/push is a
  persistent external side effect and should be explicitly confirmed at handoff.
- **Follow-up condition:** create the private repo, push, and watch CI when the
  user confirms that external publication should proceed.
