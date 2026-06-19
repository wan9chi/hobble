# Creance — Incremental Implementation Plan (macOS PoC)

This plan builds the design in [DESIGN.md](DESIGN.md) as a sequence of **small,
self-contained, individually verifiable commits**. Scope is **macOS only** so we
can build, test, and get feedback locally and fast.

## Current status

Implemented locally through M7, plus C8.1's networked Aspect fixture with a
committed profile, committed sandbox-surface JSON golden, and ignored e2e test.
A macOS CI workflow is present. `fspy` is pinned to
`voidzero-dev/vite-task@8daa9bb72faa89b745cb58c087b416b15d3bddc5`, so the
workspace no longer depends on a sibling checkout.

The private GitHub repository exists at
`https://github.com/wan9chi/creance`, `main` is pushed, and the required macOS
CI job is green.

Remaining plan work is the broader fixture matrix (C8.2-C8.7) and the
`examples/native-build-script` README transcript work (M9).

## Ground rules

- **macOS only.** All sandbox/proxy/observe code targets macOS. Where a crate
  won't compile elsewhere, gate with `#![cfg(target_os = "macos")]`; tests use
  `#[cfg(target_os = "macos")]`. No Linux code in this phase.
- **Commit scopes are adjustable.** Listed commits are checkpoints, not sacred
  boundaries. Merge obvious scaffolding into the first real behavior that needs
  it; split a commit when it mixes independent behaviors, fixture generation, or
  CLI wiring. Each commit should have one reviewable reason to exist.
- **Green commits, with a skeleton exception:** behavioral commits pass
  `cargo build && cargo test && cargo clippy -- -D warnings`. Skeleton commits
  that only add crate layout, API shells, or task definitions may be untested,
  but still build/fmt/lint where applicable. Do not add obvious tests whose only
  value is proving that a skeleton exists.
- **No untracked unfinished behavior.** Non-skeleton code is either exercised by
  a focused unit/integration/e2e test or reachable through a tested CLI path.
  Temporary tests for temporary/unfinished code are allowed only when tracked in
  `IMPLEMENTATION_NOTES.md` with why they are temporary and the condition for
  removing or promoting them. Keep that note current until each temporary test is
  completed or removed.
- **Autonomous execution.** Do not stop to ask implementation questions. Use best
  judgment, keep moving, and record design gaps, oversights, deviations, and the
  chosen decision in `IMPLEMENTATION_NOTES.md`.
- **Commit messages:** Conventional Commits (`feat(proxy): …`, `test(sandbox): …`,
  `chore: …`). Body: what + why + how-verified.
- **Test layers:**
  - *unit* — pure logic (parsing, templating, profile (de)serialization).
  - *integration* (`<crate>/tests/`) — real `sandbox-exec`, real fspy, real local
    TCP servers. Fast, deterministic, offline.
  - *e2e* (`e2e/`) — real `pnpm install` against vendored fixtures; needs network;
    marked `#[ignore]` and run via `cargo test -- --ignored` / a mise task.
- **Toolchain:** Rust is managed by `rust-toolchain.toml` and `rustup` only; do
  not add `rust` or `cargo` to `mise*.toml`. Pin Node, pnpm, and any other
  non-Rust developer tools in `mise.toml` so e2e fixtures, examples, and README
  transcripts run against known versions.
- **fspy dependency:** during development, use and modify the local
  `../vite-task` checkout when that is faster. Keep any required `fspy` changes
  isolated so they can be pushed to a new branch in
  `https://github.com/voidzero-dev/vite-task`. Before final CI/publish work,
  point Creance at that remote branch/revision instead of the local path and
  record the branch + upstreaming decision in `IMPLEMENTATION_NOTES.md`.

## Workspace layout (target end state)

```
creance/                      # cargo workspace
  crates/
    creance-proxy/            # tokio CONNECT proxy (observe-log + enforce-allowlist)
    creance-sandbox/          # SBPL generation + sandbox-exec runner (macOS)
    creance-engine/           # observe / synthesize / enforce; profile model
    creance/                  # the `creance` binary (entry + `-c` shim)
  e2e/
    fixtures/<name>/          # vendored pnpm install-inputs + committed .creance/
    harness.rs
  examples/
    native-build-script/      # documented example project + generated outputs
  docs/
    README.md
    DESIGN.md
    PLAN.md
    IMPLEMENTATION_NOTES.md
```

Profiles for the e2e fixtures are **committed** under each fixture's
`.creance/profiles/...`; the generated sandbox profile JSON is committed beside
them. Both serve as golden artifacts (§M8).

---

## M0 — Scaffold

**C0.1 — workspace skeleton + dev tasks**
- Adds: workspace `Cargo.toml`, `crates/creance` bin with minimal version output,
  `rust-toolchain.toml`, `mise.toml`, and `docs/IMPLEMENTATION_NOTES.md`.
  `mise.toml` pins Node, pnpm, and non-Rust helper tools, and defines tasks. Do
  not list Rust or Cargo in mise.
- Test: no dedicated CLI test for `--version`; this is skeleton plumbing.
- Accept: `cargo build` works; `mise run fmt`, `mise run lint`, and
  `mise run test` invoke the expected local commands.

---

## M1 — CONNECT proxy (`creance-proxy`)

All commits here test against an in-process local TCP server + an in-process
client; no external network, no TLS (the tunnel is byte-passthrough).

**C1.1 — proxy crate + CONNECT parser**
- Adds: `creance-proxy`, `Proxy::bind("127.0.0.1:0") -> (Proxy, SocketAddr)`,
  cancellable accept-loop skeleton, `HostPort`, and
  `parse_connect(&mut BufReader) -> Result<HostPort, ProxyError>` for
  `CONNECT host:port HTTP/1.1\r\n` + headers up to the blank line.
- Test: parser unit table — valid `host:443`, IPv6 literal, missing port,
  non-`CONNECT` verb, and truncated input. No separate bind/close smoke test.
- Accept: proxy crate builds, parser returns typed results, listener shuts down
  through `CancellationToken`.

**C1.2 — tunnel CONNECT + reject cleartext**
- Adds: on valid CONNECT, dial target, write
  `HTTP/1.1 200 Connection Established\r\n\r\n`, and bidirectional-copy until
  EOF. Any non-CONNECT request returns `405 Method Not Allowed`, logs, and
  closes; cleartext HTTP remains intentionally unsupported (DESIGN §2.3).
- Test: spawn a local TCP echo server as "target"; client sends CONNECT then
  bytes and observes echo through the proxy. Send `GET http://x/ HTTP/1.1` and
  assert `405` + closed.
- Accept: full-duplex copy works, both halves close, no tunnel attempted for
  non-CONNECT.

**C1.3 — host policy + observe recording**
- Adds: `Policy { Enforce(HashSet<HostPattern>), ObserveLog }`; `HostPattern`
  exact + `*.suffix`; enforce deny returns `403 Forbidden` before dial.
  `ObserveLog` allows every host and records deduped `Vec<HostPort>` via a shared
  `Recorder`.
- Test: allowlist `{localhost}`; CONNECT to it tunnels, CONNECT to
  `denied.test` returns 403 without dialing. Two ObserveLog CONNECTs record both
  hosts once in stable order.
- Accept: enforce and observe behaviors share the same proxy path.

**C1.4 — per-instance proxy env**
- Adds: `Proxy::proxy_env() -> [(String,String)]` → `HTTPS_PROXY`/`https_proxy`/`HTTP_PROXY`/`http_proxy` = `http://127.0.0.1:<port>`.
- Test: a Rust client built from those env values reaches a target through the proxy (reuse C1.3 harness, but read the proxy URL from the produced env).
- Accept: env strings parse + work.

---

## M2 — macOS sandbox (`creance-sandbox`)

Every commit runs real `sandbox-exec` and asserts behavior. These are fast and
deterministic on macOS (validated in DESIGN §9).

**C2.1 — SBPL base block + `(deny default)`**
- Adds: `Sbpl::base() -> String` = `(version 1)(deny default)` + the mandatory allowances (process-exec/fork, process-info same-sandbox, mach-lookup names, sysctl-read `hw.*`/`kern.*`, `ipc-posix-shm*`, IOKit, `/dev/null|zero|random|urandom`) + read on runtime roots (`/usr`,`/System`,`/Library`, node dir).
- Test: snapshot the string; **functional**: `sandbox-exec -p <base> /bin/sh -c 'echo ok'` exits 0; `node -e 'process.exit(0)'` exits 0 (proves base is sufficient to boot the runtime).
- Accept: runtime boots under base.

**C2.2 — write rules (deny-by-default writes)**
- Adds: `Sbpl::allow_write(&[PathBuf])` → `(allow file-write* (subpath …))`.
- Test: profile = base + allow `<tmpdir>/ok`; `sandbox-exec` running a write to `<tmpdir>/ok/a` succeeds, write to `<tmpdir>/denied/b` fails with EPERM.
- Accept: matches DESIGN §9 result.

**C2.3 — read rules (deny-by-default reads)**
- Adds: `Sbpl::allow_read(&[PathBuf])` → `(allow file-read* (subpath …))` on top of base.
- Test: profile allows read of `<tmpdir>/pub` only (+ base roots); reading `<tmpdir>/pub/x` ok, reading `$HOME/.ssh/known_hosts` (create a temp stand-in under a denied dir) fails; `/bin/sh` still runs (runtime roots).
- Accept: secrets-style path blocked, runtime intact.

**C2.4 — egress rule (only the proxy)**
- Adds: `Sbpl::allow_proxy(port)` → `(allow network-outbound (remote ip "localhost:<port>"))`; everything else denied by `(deny default)`.
- Test: start a local TCP server on `port`; `sandbox-exec` + `curl http://127.0.0.1:<port>/` → connects; `curl --max-time 5 https://example.com` → fails (blocked). (DESIGN §9.)
- Accept: only the proxy port reachable.

**C2.5 — path canonicalization**
- Adds: canonicalize rule inputs (`realpath`, `/tmp`→`/private/tmp`, `/var`→`/private/var`) before emitting subpaths.
- Test: allow_write(`/tmp/foo`) then write `/tmp/foo/x` succeeds (because emitted as `/private/tmp/foo`); a deliberate non-canonical control (raw `/tmp/foo`) is shown to fail — proving canonicalization is required.
- Accept: symlinked roots resolve.

**C2.6 — `run_sandboxed`**
- Adds: `run_sandboxed(profile, argv, env, cwd) -> ExitStatus` writing the profile to a temp `.sb` and exec'ing `sandbox-exec -f`.
- Test: compose base+write+read+egress; run a small script doing one allowed + one denied op; assert exit + side effects.
- Accept: single entrypoint usable by the engine.

---

## M3 — Observation (`creance-engine`, observe half)

**C3.1 — fspy FS capture**
- Adds: an `fspy` dependency. During development this may be a local path to
  `../vite-task/crates/fspy` so `fspy` fixes can be made quickly. Before the
  final publish/CI step, push those `fspy` fixes to a new branch in
  `https://github.com/voidzero-dev/vite-task` and switch this dependency to that
  remote branch/revision, e.g.
  `fspy = { git = "https://github.com/voidzero-dev/vite-task", package = "fspy", rev = "<pinned-sha>" }`;
  `observe_fs(argv, env, cwd) -> {status, reads: Vec<PathAccess>, writes: Vec<PathAccess>}`
  via `fspy::Command` + `wait_handle`.
- Test: run `/bin/sh -c 'cat /etc/hosts >/dev/null; echo x > "$OUT"'` (OUT in a tempdir); assert `/etc/hosts` ∈ reads and OUT ∈ writes — **verifying fspy captures grandchildren through `/bin/sh`** (the macOS DYLD-through-shell case; confirmed in the spike).
- Accept: reads/writes captured with correct modes.

**C3.2 — observe network via the proxy (log mode)**
- Adds: `observe(argv, env, cwd) -> Observation { status, reads, writes, domains }`: starts an `ObserveLog` proxy, injects its `proxy_env()`, runs the command under fspy, returns fspy FS + proxy domains.
- Test: the observed command is a tiny in-repo helper bin (`tests/bin/connect-probe`) that reads the `HTTPS_PROXY` env and issues a CONNECT to `recorded.test:443` through it (no real TLS), then writes a file. Assert `domains` contains `recorded.test` and `writes` contains the file. (Avoids external network; exercises the full observe wiring.)
- Accept: one pass yields FS + domains.

**C3.3 — note proxy-honoring limitation**
- Adds: doc-comment + a `tracing::warn!` when the observed process makes 0 proxy connections but exits 0 (heuristic that it may have bypassed the proxy). No behavior change.
- Test: helper that does no network → warning emitted (capture logs); helper that uses proxy → no warning.
- Accept: observability of the known gap (DESIGN §8).

---

## M4 — Profile model + synthesis (`creance-engine`)

**C4.1 — profile types + serde**
- Adds: `Profile { package, creance, lifecycle, entries: Vec<Entry> }`, `Entry { os: Vec<String>, read, write, domains }` (serde). Matches DESIGN §4.2 (flat entries, each tagged with `os`).
- Test: round-trip (serialize→parse→equal) + snapshot of the esbuild example shape.
- Accept: stable JSON shape.

**C4.2 — path templating (both directions)**
- Adds: `Templater` from a `Context { pkg_dir, project_root, store, home, cache, run_tmp }`: `templatize(abs) -> "${PKG_DIR}/…"` and `resolve(tmpl) -> abs`.
- Test: unit — each variable templatizes and resolves back; longest-prefix wins; non-matching path passes through.
- Accept: round-trip stable.

**C4.3 — read/write generalization to roots**
- Adds: `generalize(paths, ctx) -> Vec<String>`: collapse many paths under a template root to `${ROOT}/**`; drop paths already covered by the built-in runtime roots.
- Test: unit — 50 files under pkg_dir → `${PKG_DIR}/**`; `/usr/lib/...` reads dropped; a stray `${HOME}/.cache/x` kept as `${CACHE}/x/**` or `${HOME}/.cache/x/**` per rule.
- Accept: small, stable rule sets.

**C4.4 — synthesize an entry**
- Adds: `synthesize(observation, os, ctx) -> Entry` = generalize(writes) + generalize(reads) + sorted/deduped domains, tagged with `os`.
- Test: unit from a synthetic `Observation`.
- Accept: deterministic entry.

**C4.5 — profile store (load/merge/save)**
- Adds: `store::load(dir, name, version)`, `store::upsert_entry(profile, entry)` (replace the entry whose `os` matches; else append), `store::save` to `.creance/profiles/<name>/<version>.json`.
- Test: upsert darwin entry, then a linux entry → two entries coexist; re-upsert darwin → replaced not duplicated.
- Accept: per-OS entries managed correctly.

---

## M5 — Enforcement (`creance-engine`, enforce half)

**C5.1 — enforce a known entry**
- Adds: `enforce(entry, ctx, argv) -> ExitStatus`: resolve templates → `Sbpl` (base + allow read/write + allow proxy) + start an `Enforce(entry.domains)` proxy → `run_sandboxed`.
- Test: synthetic entry allowing write to `${RUN_TMP}` + domain `ok.test`; run a helper that (a) writes allowed file → succeeds, (b) writes outside → blocked, (c) CONNECTs `ok.test` → ok, (d) CONNECTs `evil.test` → 403.
- Accept: all four outcomes as expected in one run.

**C5.2 — process-group teardown**
- Adds: run the child in its own process group; on exit/timeout kill the group.
- Test: child backgrounds a `sleep 30`; after enforce returns, assert the sleeper PID is gone within a deadline.
- Accept: no survivors.

---

## M6 — Engine API + decision

**C6.1 — top-level engine API**
- Adds: `engine::observe(cmd_str, ctx)` (wraps in `/bin/sh -c`) and `engine::enforce(cmd_str, entry, ctx)`.
- Test: integration on a shell one-liner doing fs+writes (no pnpm): observe → entry; feed entry to enforce → allowed ops pass, an added disallowed op blocked.
- Accept: observe→synthesize→enforce loop works on a raw command.

**C6.2 — mode decision**
- Adds: `decide(profile_dir, pkg, os, mode) -> Decision::{Enforce(Entry), Observe, Fail(reason)}` (strict → Fail when no matching entry; dev → Observe).
- Test: unit over {entry present for os, present for other os only, absent} × {dev, strict}.
- Accept: correct branch each case.

---

## M7 — CLI + pnpm shim (`creance` bin)

**C7.1 — `creance -c "<cmd>"` shim**
- Adds: argv dispatch: if invoked as `-c <cmd>`, read `CREANCE_MODE`, `CREANCE_DIR`, `npm_package_name`, `npm_package_version`, `PNPM_SCRIPT_SRC_DIR` → `decide` → observe (write/merge profile) | enforce | fail-closed (non-zero exit). 
- Test: integration calling `creance -c 'echo hi; echo x > "$PWD/out"'` with `CREANCE_MODE=observe` + a temp `CREANCE_DIR` + fake `npm_package_*`; assert a profile was written and `out` exists. Then `CREANCE_MODE=enforce` with that profile; assert success; then a tampering command (write outside) → non-zero.
- Accept: shim is a working gatekeeper; unknown+strict ⇒ non-zero before running.

**C7.2 — `creance observe [pnpm args…]` command wiring**
- Adds: runs `pnpm install --config.script-shell=<self-abs-path> --config.dangerously-allow-all-builds=true --config.child-concurrency=…`
  with `CREANCE_MODE=observe`, `CREANCE_DIR=<cwd>/.creance`, and pass-through
  pnpm args. Each lifecycle script reaches C7.1 in real installs, but this
  commit only wires the command.
- Test: integration places a fake `pnpm` executable first in `PATH`; it records
  argv/env and exits 0. Assert the script-shell points at the current `creance`,
  Creance env is set, and user pnpm args are preserved.
- Accept: observe command wiring is verified without network or fixture work.

**C7.3 — `creance install [pnpm args…]` + `--strict` command wiring**
- Adds: install/enforce mode wiring with `CREANCE_MODE=enforce`, optional
  `--strict`, shared pnpm-runner code from C7.2, and pass-through pnpm args.
- Test: fake-`pnpm` integration records argv/env for normal and `--strict`
  invocations; unit coverage for strict/dev mode parsing stays with `decide`.
- Accept: install command wiring is verified without coupling this commit to the
  first e2e fixture.

---

## M8 — e2e fixtures (committed profiles)

Source fixtures: `target/fixture-candidates/REPORT.md`. For each, **vendor only the
install inputs** (`package.json`, `pnpm-workspace.yaml`, `pnpm-lock.yaml`) into
`e2e/fixtures/<name>/` (committed), run observe to generate the profile, **commit
the generated `.creance/profiles/...` and the generated sandbox profile JSON**,
check that the sandbox JSON is the expected enforce surface for the fixture
(read/write allows plus proxy-only egress), then test enforce. e2e tests are
`#[ignore]` (network + pnpm) and run via `mise run e2e`.

Each e2e commit's test asserts: (1) `creance install --strict` succeeds and the
package's expected side effect happens; (2) a **deliberate violation** (a patched
copy of the fixture whose script writes outside `${PKG_DIR}` or contacts an
un-profiled host) is **blocked**; (3) re-running observe reproduces the same
allow-lists (determinism, ignoring the `creance` version field); (4) regenerating
the sandbox profile JSON from the committed Creance profile produces the same
JSON as the committed sandbox golden.

Fixture commits do not introduce main CLI features. If a fixture requires new
harness capability, split that harness capability into the same commit only when
the fixture is the first real use of it; otherwise land it separately with the
smallest fixture that proves it.

**C8.1 — fixture: aspect `@aspect-test/c@2.0.0` (tiny, deterministic)**
- Why first: minimal, offline-ish, postinstall writes `data.json` (REPORT §Aspect).
- Adds: `e2e/fixtures/aspect-c/` inputs; committed `.creance/profiles/@aspect-test/c/2.0.0.json`; committed generated sandbox profile JSON; `e2e/harness.rs` (vendor→install→assert helpers).
- Test: observe → profile + sandbox JSON committed and reviewed as expected; `creance install --strict` → `data.json` present; violation variant blocked; freshly-generated sandbox JSON equals the committed JSON.
- Accept: full observe→commit→enforce loop on one real package.

**C8.2 — fixture: `better-sqlite3` via `url-shortener` (node-gyp native)**
- Exercises node-gyp compile reads/writes + cache paths.
- Test: enforce builds the native addon; out-of-tree write blocked; freshly-generated sandbox JSON equals the committed JSON.
- Accept: a node-gyp build runs sandboxed.

**C8.3 — fixture: `node-pty` via `tasuku` (native, install+prepare+postinstall)**
- Exercises multiple lifecycle stages on one package.
- Test: each lifecycle stage enforces from the committed profile; a freshly-generated sandbox JSON equals the committed JSON.
- Accept: all stages enforced from one profile.

**C8.4 — workspace fixture harness expansion**
- Adds: harness support for workspace fixtures with multiple package roots,
  multiple generated profiles, multiple generated sandbox JSON goldens, and
  per-package side-effect assertions.
- Test: adapt the smallest already-landed fixture or a trimmed workspace sample
  to exercise multiple profile and sandbox JSON files without adding a new large
  dependency set.
- Accept: harness can assert more than one profile, sandbox JSON, and package
  root cleanly.

**C8.5 — fixture: realistic workspace (`angular-calendar` or `CloudSaver`)**
- Multiple build deps (`esbuild`, `@parcel/watcher`, `lmdb`/`bcrypt`/`sqlite3`).
- Test: each gets a committed profile and sandbox JSON; install enforces all; one injected violation blocked; each freshly-generated sandbox JSON equals its committed JSON.
- Accept: multi-package install under enforcement.

**C8.6 — fixture: multi-version same dep (`kudos`: `esbuild` ×N, `better-sqlite3`)**
- Validates per-`name@version` profile files don't collide.
- Accept: distinct profiles and sandbox JSON files per version; all freshly-generated sandbox JSON matches the committed JSON; both enforce.

**C8.7 — fixture: network/cache (`sharp` via `kindle-ai-export` or `hiof` demo)**
- `sharp` downloads a prebuilt → exercises domain capture into `entry.domains`.
- Test: observe records the CDN host(s); enforce allows exactly those; an extra host blocked; freshly-generated sandbox JSON equals the committed JSON. Document the proxy-honoring caveat if `sharp` bypasses the proxy (DESIGN §8) and pin the domain manually if so.
- Accept: domain allowlisting demonstrated end-to-end.

---

## M9 — Example project + README

These are the final commits, after the behavior and e2e fixture suite are stable.
The README must not contain invented transcripts: every command/output block is
captured from real runs against files under `./examples`.

**C9.1 — example project: famous build-script dependency**
- Adds: `examples/native-build-script/`, a small pnpm project whose dependency is
  a well-known package with an install/build script, preferably `sharp` because
  it is famous and exercises prebuilt native binary download/build behavior. Pin
  versions in `package.json`/`pnpm-lock.yaml`; include a tiny script that imports
  the dependency and proves it installed correctly.
- Adds: committed Creance profiles generated by `creance observe` for the
  example, plus generated output files under `examples/native-build-script/out/`
  that demonstrate observe/install/proof commands.
- Test: run `creance observe` and `creance install --strict` in the example
  project through the mise-pinned Node/pnpm toolchain; run the proof script;
  capture stdout/stderr/exit codes into the example output files.
- Accept: a fresh checkout can run the documented example and reproduce the
  committed outputs, modulo normalized timing/cache paths.

**C9.2 — README with real step-by-step guide and outputs**
- Adds: `docs/README.md` with step-by-step install/observe/review/enforce
  guide using only `examples/native-build-script/`.
- The README embeds or links to exact output snippets generated in
  `./examples/native-build-script/out/`; do not hand-write plausible output.
  Include commands for regenerating those files and note any normalized fields
  such as elapsed time, temp dirs, or cache-specific paths.
- Test: rerun the README commands from a clean copy of the example project and
  refresh the output files before committing the README. Commands must use the
  mise-pinned Node/pnpm versions.
- Accept: README commands are executable as written, outputs match committed
  files after documented normalization, and the guide shows both observe and
  strict enforcement.

---

## M10 — Private GitHub repo + CI

This is the final operational step. Do it only after all local docs, examples,
profiles, and `fspy` dependency decisions are complete.

**C10.1 — publish private repo and make CI green**
- Adds: `.github/workflows/ci.yml` that runs on pull requests and pushes. CI uses
  mise for pinned Node/pnpm/non-Rust tools, Rust from `rust-toolchain.toml`, and
  runs at least `cargo fmt --check`, `cargo clippy -- -D warnings`, and
  `cargo test` on `macos-latest`. Add an explicit job or workflow dispatch for
  ignored/network e2e tests if they are too slow or flaky for every push.
- Before push: ensure any local `../vite-task` `fspy` changes are pushed to a new
  branch in `voidzero-dev/vite-task`, and Creance depends on that remote
  branch/revision rather than a local path.
- Action: create a private GitHub repository for Creance, add it as the remote,
  push the branch, watch CI, inspect failures, and fix or document real
  environment gaps until required CI checks pass.
- Accept: private GitHub repo exists, branch is pushed, CI workflow is present,
  required checks are green, and any deferred e2e/manual checks are documented in
  `docs/README.md` or `docs/PLAN.md`.

---

## Implementation notes ledger

`IMPLEMENTATION_NOTES.md` is maintained throughout implementation. Add or update
an entry whenever reality diverges from `DESIGN.md`, the design omits an
important detail, an autonomous decision closes ambiguity, an upstream `fspy`
branch is used, or a temporary test is introduced. Each entry records:

- the area affected;
- the design gap/deviation/oversight;
- the decision made and why;
- the follow-up condition for retiring the note, upstream branch, or temporary
  test.

Do not pause implementation waiting for design clarification; make the smallest
defensible decision, document it there, and continue.

---

## Cross-cutting / later (only when a test needs it)

- **`git`-hosted dep `prepare`** handling (DESIGN §8) — add when a fixture needs it.
- **First-party vs third-party** script policy (root/workspace scripts) — add when an e2e fixture's own scripts appear.
- **Read-generalization tuning** (DESIGN §12) — revisit if e2e profiles are noisy/brittle.

Each of the above becomes its own commit *with the fixture/test that motivates it*
— never added speculatively.

## Determinism note for committed profiles

`observe` writes real observed behavior; the committed profile is the golden
artifact. To keep goldens stable: the `creance` version field is normalized out of
determinism comparisons; paths are templated + sorted; domains sorted. The e2e
"re-observe reproduces" check compares `entries[].{read,write,domains}` only.
The committed generated sandbox profile JSON is the second golden: tests
regenerate it from the committed Creance profile and compare it to the committed
JSON after the same stable ordering/normalization used by the generator.
