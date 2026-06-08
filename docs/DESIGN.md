# Creance — a pure-Rust sandbox for npm/pnpm lifecycle scripts

> *A creance is the long, light line a falconer uses to tether a hawk during
> training — letting it fly, but never out of control. This tool does the same
> for the untrusted code that runs during `pnpm install`.*

## 1. Problem & goals

When you run `pnpm install`, every dependency's `preinstall` / `install` /
`postinstall` / `prepare` script executes **arbitrary code on your machine**,
with your full user privileges, before you have run or even read a single line
of that dependency. This is the primary supply-chain attack surface of the npm
ecosystem (credential theft from `~/.ssh` / `~/.aws` / `~/.npmrc`, writing to
shell rc files, exfiltration to attacker domains, planting persistence).

**Goal.** Capture exactly what each dependency's lifecycle scripts *legitimately*
do (files read/written, domains contacted), freeze that as a committed,
version-pinned **profile**, and from then on run those scripts inside a
deny-by-default sandbox that allows only the recorded behaviour. Anything new — a
malicious update, a time-bomb, a typo-squat — is blocked and surfaced.

### Threat model — **supply-chain defense**

A dependency's lifecycle script may be **actively malicious and aware that
creance is in use**. This drives:

1. **Enforcement is deny-by-default and kernel-enforced**, over the whole
   process subtree.
2. **Observe prevents nothing; enforce is the only boundary.** The observe pass
   runs the script unsandboxed (it *is* the already-unsandboxed first install,
   instrumented) so the captured profile is faithful. Safety comes from never
   auto-trusting what was observed: the profile is a *proposal* that takes effect
   only when a human reviews the git diff and commits it, and enforce is
   deny-by-default.
3. **Profiles are keyed by package name + version.** A code change ships as a new
   version (pnpm guarantees a given version is fixed bytes), so it gets no profile
   and can't inherit a trusted one.

### Hard constraints

- **Pure Rust** — a single `creance` binary, no Node/TS runtime or external
  sandbox daemons.
- **No `sudo`, ever** — for *observation* or *enforcement*, on macOS and Linux.
  Everything runs with ordinary user privileges (see §3.3).
- v1 targets **pnpm on macOS and Linux**.

### Non-goals

- Not a replacement for auditing dependency *source*; it constrains behaviour.
- Not perfect network DLP. The proxy gates on hostname (CONNECT), not payload;
  exfil over an *allowed* host is a residual risk (§9).
- Windows is designed-around but deferred.

## 2. The pieces (all pure Rust)

### 2.1 `fspy` — filesystem **observer** (existing crate)

`std::process::Command`-like API; on child exit returns every path touched:
`path_accesses: [{ path, mode: READ | WRITE | READ_DIR }]`, canonical paths,
records the **attempt** pre-syscall (so it logs even paths an outer sandbox
blocks), follows the whole subtree. macOS: `DYLD_INSERT_LIBRARIES`; Linux glibc:
`LD_PRELOAD`; Linux static/musl: `seccomp_unotify`. **No `sudo`.** Observer only.

### 2.2 `creance-sandbox` — OS **enforcer** (new crate)

Deny-by-default filesystem + "egress only to the proxy", per OS, rootless. The
**network gate is static and package-agnostic** — the same "only the proxy is
reachable" rule for observe, enforce, and every package — so *all* network policy
(domains, allow/deny, logging) lives in one place, the proxy (§2.3),
and the OS-layer trusted surface stays tiny. Only the **FS** rules vary per
package.

- **macOS — Seatbelt.** Generate an SBPL `.sb` profile and run the child under
  `/usr/bin/sandbox-exec`. No entitlement, no code-signing, no SIP change, **no
  sudo** (validated, §10). One mechanism covers **both** FS and network egress:
  - FS: **deny-by-default for reads and writes** — `(deny default)` then
    `(allow file-read* (subpath …))` / `(allow file-write* (subpath …))` for the
    profile's allowed paths, plus the base block's runtime read roots (below).
  - Egress: `(deny default)` blocks all network; `(allow network-outbound
    (remote ip "localhost:<proxyPort>"))` permits only the proxy. (`localhost`
    only — numeric IPs are unsupported in SBPL; mind the IPv6 `::ffff:`
    dual-stack gap.)
  - A **base allowance block** is mandatory or Node/zsh won't boot:
    `process-exec/fork`, `process-info* (target same-sandbox)`, specific
    `mach-lookup` global-names, `sysctl-read` for `hw.*`/`kern.*`,
    `(allow ipc-posix-shm*)` (fspy's IPC requires it), IOKit,
    `/dev/null|zero|random|urandom`, and read on the runtime roots
    (`/usr`, `/System`, `/Library`, the node install dir) so reads can be
    deny-by-default without the profile re-listing every system path.
- **Linux — Landlock (FS) + seccomp user-notify (egress).** Both rootless,
  needing only `prctl(PR_SET_NO_NEW_PRIVS)`.
  - FS: a Landlock ruleset (allow-list, deny-by-default for reads and writes);
    grant the profile's read/write paths + the runtime read roots.
  - Egress = "deny all network except the proxy", which is more than `connect()`:
    - **classic seccomp** (no supervisor): block `AF_INET`/`AF_INET6`
      **UDP/raw** socket creation (else `sendto()` exfiltrates without ever
      calling `connect()`) and block `io_uring_setup/enter/register` (else
      io_uring issues connects/sends the filter never sees).
    - **seccomp user-notify** on `connect()` (built on the existing
      `fspy_seccomp_unotify`): a userspace supervisor reads the target `sockaddr`
      and **allows the syscall only if it targets the proxy socket**
      (`127.0.0.1:<proxyPort>` / `::1:<proxyPort>`); everything else gets `EPERM`
      and is logged as a bypass attempt.
    - IP-precise, rootless, works on hardened Ubuntu (no user namespaces ⇒ no
      AppArmor `userns` gate, no sudo). macOS Seatbelt's single `(deny default)`
      + one allow covers TCP/UDP/raw in one rule; Linux needs these layers to
      match it.

Applied to the child via `pre_exec` (Linux: `no_new_privs` → Landlock
`restrict_self` → install seccomp filter → exec; allocate everything in the
parent) or by exec'ing `sandbox-exec` (macOS — avoids `sandbox_init`
fork-safety issues).

### 2.3 `creance-proxy` — network **brain** (new crate)

A small **tokio HTTP CONNECT proxy** — the domain-level policy point on *both*
OSes (the OS layer above just forces traffic here).

- Handles the `CONNECT host:port` method only (no MITM, no CA): reads the hostname
  from the CONNECT line, applies policy, then opaque-tunnels bytes (TLS passes
  through untouched — the proxy never sees plaintext).
- **Cleartext HTTP is intentionally unsupported for now** — a plain `GET http://…`
  (no CONNECT) is rejected. Effectively everything goes over HTTPS today; cleartext
  support is deferred, not overlooked. (A non-CONNECT request that bypasses the
  proxy is still blocked by the OS egress gate.)
- **Observe mode:** **allow every host and log it** (prevents nothing — §1/§5),
  recording each hostname into the entry's `domains` list. Nothing is enforced
  until the profile is reviewed and committed.
- **Enforce mode:** strict allowlist from the profile; deny + log everything
  else. The proxy resolves DNS, so the child needs none.

### 2.4 pnpm `script-shell` — the **interception point**

Verified against pnpm 10.34.1: setting `script-shell` makes pnpm run **every
dependency lifecycle script** as `<script-shell> -c "<cmd>"`, in the dep's dir,
with `npm_package_name`, `npm_package_version`, `npm_lifecycle_event`,
`npm_lifecycle_script`, `PNPM_SCRIPT_SRC_DIR`, `INIT_CWD` in env.

- pnpm 10 blocks build scripts by default; we set `dangerouslyAllowAllBuilds`
  and make **our shim the fail-closed gatekeeper** instead.
- Scripts run in parallel (`child-concurrency`, default 5); each is its own shim
  process, so observe and enforce work unchanged at any concurrency.
- pnpm strips inherited `npm_*` but **not** `CREANCE_*` — so the parent passes
  the run mode (observe/enforce/strict) + the `.creance` path to the shim via
  `CREANCE_*` env.
- `.pnpmfile.cjs` `readPackage` **cannot** alter script execution (verified);
  `script-shell` is the one supported chokepoint.

## 3. Architecture

```
┌──────────────────────────────────────────────────────────────────────┐
│  creance (single Rust binary)                                         │
│                                                                        │
│  entry mode          `creance observe` / `creance install` / …        │
│    └─ drives pnpm with --config.script-shell=$(self) etc.             │
│                                                                        │
│  shim mode           `creance -c "<cmd>"`   (pnpm calls this per script)│
│    └─ reads CREANCE_* + npm_* env → observe | enforce | refuse        │
│       (fail-closed) → calls the engine                                 │
│                                                                        │
│  Engine (library crates, command-agnostic):                            │
│    creance-engine    observe() / synthesize() / enforce()             │
│      ├─ fspy             (FS observation)                              │
│      ├─ creance-sandbox  (Seatbelt | Landlock+seccomp-unotify)        │
│      └─ creance-proxy    (CONNECT proxy: domain policy + logging)      │
└────────────────────────────────────────────────────────────────────── ┘
```

Two decoupled layers:

- **Engine** (`creance-engine` + the three crates above) sandboxes/observes **any
  command** — reusable beyond pnpm.
- **CLI/shim** is the pnpm-specific glue.

### 3.1 Engine API (Rust)

```rust
pub struct ObserveOptions { pub cwd: PathBuf,
    pub env: Vec<(OsString, OsString)>, pub run_tmp: PathBuf, pub timeout: Duration }

pub struct Observation { pub status: ExitStatus,   // nothing blocked during observe
    pub fs_reads: Vec<PathAccess>, pub fs_writes: Vec<PathAccess>,
    pub domains: Vec<HostPort> }           // every host contacted (all allowed + logged)

pub async fn observe(cmd: &str, o: &ObserveOptions) -> Result<Observation>;
pub fn synthesize(o: &Observation, defaults: &Defaults, pkg: &PkgId) -> Profile;
pub async fn enforce(cmd: &str, p: &Profile, ctx: &EnforceContext) -> Result<ExitStatus>;
```

### 3.2 How observe composes (one pass, nothing blocked)

```
creance-proxy listens on 127.0.0.1:<port>   (allow + log every host)

both OSes:  fspy::Command(sh -c "<cmd>")   with HTTPS_PROXY=127.0.0.1:<port>
              └─ fspy records FS (DYLD / LD_PRELOAD)         no OS sandbox here
              └─ (optional) seccomp-unotify / DYLD hook in LOG-and-CONTINUE mode
                 to also record direct connect() destinations — still blocks nothing
```

- No Seatbelt/Landlock during observe — it only needs **fspy + the proxy in
  log mode**. The OS-sandbox machinery is **enforce-only**.
- One pass yields: **fspy = every path touched** and **proxy = every domain
  contacted** → the generated profile (reviewed via git diff before commit).
- `enforce` is the opposite: full OS sandbox + strict proxy + fspy dropped; kill
  the process group on exit/timeout.

### 3.3 Why this is `sudo`-free everywhere

| | Observe | Enforce |
|---|---|---|
| **macOS** | fspy (DYLD), no sudo | `sandbox-exec` (no entitlement/SIP/sudo) + proxy |
| **Linux** | fspy (`LD_PRELOAD` / `seccomp_unotify`), no sudo | Landlock + seccomp-unotify (`no_new_privs` only) + proxy |

No user namespaces (so no Ubuntu 24.04 AppArmor `userns` gate), no setuid
helper, no `sudo` — by construction.

## 4. Profiles — allow-only, OS-aware

### 4.1 Repo layout (committed)

```
.creance/
  profiles/
    esbuild/0.21.5.json
    sharp/0.33.2.json
```

The file path *is* the index: a script's profile is `profiles/<name>/<version>.json`,
looked up from `npm_package_name`/`npm_package_version`. No separate lockfile —
pnpm already guarantees a given `name@version` is fixed bytes, so a code change
ships as a new version → a new path → no profile → re-observe.

### 4.2 Built-in denies + per-package allows

Reads, writes, and egress are all **deny-by-default allow-lists**, composed from
two sources:

- **Built-in defaults** (compiled into the binary, same for every package): the
  runtime read roots so the toolchain boots (`/usr`, `/System`, the node dir, the
  pnpm store), write hard-denies (rc files, `$PATH`, git hooks), and strict egress.
- **`<pkg>.json` = the package's learned allows**: its read set, write set, and
  egress allowlist.

Profiles are **allow-only** — there are no deny fields, since everything is
deny-by-default and the denies are built-in.

Paths are **templated** for portability (`${PKG_DIR}`=`PNPM_SCRIPT_SRC_DIR`,
`${PROJECT_ROOT}`=`INIT_CWD`, `${STORE}`, `${HOME}`, `${CACHE}`, `${RUN_TMP}`);
the engine resolves + canonicalizes (`/tmp`→`/private/tmp`) at enforce time.

The allow-lists are **per OS/arch** (§4.3) — a package downloads a different
prebuilt binary on macOS vs Linux, so the same `name@version` carries a **flat
list of entries, each tagged with the `os`/arch it applies to** (an entry's `os`
is a list, so one entry can cover several identical platforms):

```json
{
  "package": { "name": "esbuild", "version": "0.21.5" },
  "creance": "0.1.0",
  "lifecycle": ["postinstall"],
  "entries": [
    {
      "os": ["darwin-arm64"],
      "read":    ["${PKG_DIR}/**", "${STORE}/**"],
      "write":   ["${PKG_DIR}/**", "${RUN_TMP}/**", "${CACHE}/esbuild/**"],
      "domains": ["registry.npmjs.org"]
    },
    {
      "os": ["linux-x64"],
      "read":    ["${PKG_DIR}/**", "${STORE}/**"],
      "write":   ["${PKG_DIR}/**", "${RUN_TMP}/**", "${CACHE}/esbuild/**"],
      "domains": ["registry.npmjs.org"]
    }
  ]
}
```

Engine merge at enforce (pick the entry whose `os` contains the current os-arch):
`allowRead = entry.read + runtime roots`, `allowWrite = entry.write`, `domains =
entry.domains`; `denyWrite` and strict egress come from the built-in defaults
(+ Seatbelt/Landlock built-ins).

### 4.3 OS-specific profiles

A package can download a **different prebuilt binary per platform**, so its
reads, writes, and even domains differ across OS/arch. Therefore:

- The profile's `entries` list holds **one item per platform** it was observed on
  (no union/flattening); enforce uses the entry whose `os` contains the current
  os-arch.
- A macOS-observed profile is **not** silently trusted on Linux CI: if no entry
  matches the current platform, `creance install --strict` → fail (re-observe
  required). This makes the "observe on macOS, enforce on Linux CI" gap explicit
  instead of unsound.

## 5. The observation pass (prevents nothing; the review gate is the safeguard)

`observe` runs the untrusted script **unsandboxed and to completion** — it is the
already-unsandboxed first install (overview step 1), instrumented. Nothing is
blocked, so the captured behaviour is faithful (a block would truncate the
script). It needs only **fspy + the proxy in log mode** — no Seatbelt/Landlock.

- **Network — allow + log everything.** The proxy lets every host through and
  records each one into the entry's `domains` list.
- **Filesystem — record reads and writes.** fspy captures both, which become the
  entry's `read` / `write` lists (generalized to roots, §6).
- **Nothing observed is auto-trusted.** The generated profile is a *proposal* that
  takes effect only once a human reviews the `.creance/` git diff and commits it —
  git review is the approval gate (no separate command).
- **Controlled `${RUN_TMP}`:** set as `TMPDIR` so "writes to a random temp path"
  generalize to one rule at synthesis time.
- **Canonicalization:** synthesized rules are `realpath`-resolved (Seatbelt/
  Landlock resolve symlinks at *enforce* time — an un-canonicalized `/tmp` rule
  silently fails to match).

## 6. Synthesis — raw accesses → tight, non-brittle, allow-only profile

- **Writes → tight allowlist**, generalized to the narrowest template subpath
  (`${PKG_DIR}/**`, `${RUN_TMP}/**`, `${CACHE}/<tool>/**`).
- **Reads → allowlist, generalized to coarse roots.** A Node process reads
  thousands of paths, so collapse them to template roots (`${PKG_DIR}/**`,
  `${STORE}/**`, runtime roots are built-in) rather than listing every file —
  otherwise the list is unusable and breaks on the next toolchain bump.
- **Domains → pinned exactly** as observed, into the entry's `domains` list.
- Allow-only output; every deny is implied or from the built-in defaults.
- Determinism: paths and domains sorted and de-duplicated.

## 7. Security model — deny-by-default everywhere

Reads, writes, and egress are all deny-by-default allow-lists built from the
observed run, cutting both supply-chain chains:

- **Exfiltration** ← the **read allowlist** (can't reach `~/.ssh`, other projects,
  unrelated secrets) **and** the **egress allowlist** (a secret has nowhere to go).
- **Tampering/persistence** ← the **write allowlist** (can't touch rc files,
  `$PATH`, git hooks, other packages; Seatbelt/Landlock also hard-block many).

**Caveat:** an *allowed* write-capable domain (e.g. `registry.npmjs.org`, needed
by many legit packages) is itself an exfil channel, since the proxy gates on host,
not payload — documented as a residual risk in §8.

## 8. Bypasses & residual risks

| Vector | Mitigation |
|---|---|
| **Exfil during observe** | **Accepted by design** — observe is unsandboxed (= the already-unsandboxed first install, §1/§5). Mitigated by: nothing observed is auto-trusted (the profile only takes effect once the `.creance/` diff is reviewed and committed), and untrusted deps should be observed in a disposable env (container/VM/CI). |
| **Exfil over an allowed write-capable domain** | The egress allowlist is the only exfil control (no payload inspection / MITM), so an allowed write-capable host is a residual exfil channel. |
| **git-hosted deps run `prepare` in a fetch phase** bypassing the build gate | Detect git/URL deps from the lockfile; `ignore-scripts` on fetch + a controlled sandboxed rebuild; strict mode hard-fails an unprofiled git dep. |
| **Client ignores `HTTPS_PROXY` and connects directly** | macOS Seatbelt / Linux seccomp-unotify **deny** the direct connect (contained). Such a package fails and is surfaced; it needs an explicit allowance. |
| **Client pre-resolves DNS then connects to IP** | Direct connect denied (only proxy reachable). Document; optionally allow a localhost DNS forwarder. |
| **Long-lived daemon outliving install** | Own process group; kill the group on exit/timeout. |
| **Time-bomb / divergent behaviour at enforce** | Enforce is deny-by-default — divergent actions are blocked. |
| **Read-allowlist brittleness** (cost, not attack) | A toolchain bump that moves runtime paths breaks reads → re-observe. Mitigated by generalizing reads to coarse roots (§6) and keeping runtime roots in the built-in base. |
| **Profile poisoning (too-broad commit)** | Human review of the `.creance/` git diff at commit time (git is the integrity record). |
| **Malicious update inherits old profile** | New version → new path → no profile → re-observe/fail; pnpm guarantees a given version is fixed bytes. |
| **Symlink/canonicalization evasion** | Canonicalize all rules; Seatbelt emits anti-bypass `file-write-unlink/create` denies; Landlock operates on resolved paths. |
| **seccomp-unotify TOCTOU** on `connect()` | Re-validate via `SECCOMP_IOCTL_NOTIF_ID_VALID`; read `sockaddr` with `process_vm_readv`; on any doubt, deny. |
| **DYLD/preload stripped** | dyld prunes `DYLD_*` only for restricted/hardened/platform binaries; `__fspy-run` is our own unsigned helper, and the profile grants `file-read*` to the dylib, so the preload survives. |
| **First-party vs third-party confusion** | The shim distinguishes the **root project's / workspace packages'** own scripts (trusted, may be unsandboxed or run under a looser project policy) from **dependency** scripts (always sandboxed), keyed off `INIT_CWD`/`PNPM_SCRIPT_SRC_DIR`. |

## 9. Validation status

Verified on macOS 26.5 (arm64):

1. **Seatbelt via `sandbox-exec`** enforces FS + egress in one mechanism, with
   **no sudo / SIP / entitlement**: FS write to an allowed subpath succeeds, write
   elsewhere is blocked; direct `https://example.com` is blocked while
   `http://127.0.0.1:<port>` (the proxy) succeeds.
2. **fspy** records read/write/readdir with canonical paths, logging *attempts*
   even when an outer sandbox blocks them.
3. **fspy runs inside the OS sandbox** — `DYLD_INSERT_LIBRARIES` survives Seatbelt
   provided the profile grants `(allow ipc-posix-shm*)` for fspy's IPC.
4. **pnpm `script-shell`** invokes `<shim> -c "<cmd>"` per dependency script with
   the documented env (pnpm 10.34.1).

Known constraints surfaced by the above: rules must be canonicalized; the SBPL
base-allowance block is required or Node/zsh won't start; SBPL `localhost:PORT`
works but numeric IPs don't.

Not yet validated: the Linux seccomp-unotify `connect()` supervisor (on
`fspy_seccomp_unotify`) and the Landlock FS ruleset.

## 10. User experience

```console
$ creance observe            # learn profiles (unsandboxed run, instrumented)
  ▸ esbuild@0.21.5   postinstall  ✔  1 domain, 2 writes
  ▸ sharp@0.33.2     install      ✔  2 domains, 14 writes
generated/updated profiles → .creance/
next:  review the .creance/ git diff, then commit.

$ creance install            # enforce; drop-in for pnpm install
  ▸ esbuild@0.21.5   postinstall  ✔ sandboxed
  ✗ left-pad@1.3.0   postinstall  BLOCKED: connect api.evil.tld; write ${HOME}/.zshrc
install failed: left-pad@1.3.0 violated its profile.
```

- **New/changed dep** (no profile for this version/OS): dev → observe + write the
  profile (review the diff, commit); `--strict` (CI) → **fail**, like a frozen
  lockfile.
- Two commands: `creance observe` (learn → write profiles, reviewed via git diff)
  and `creance install` (enforce; drop-in for `pnpm install`).

## 11. Implementation plan

- **M1 — `creance-proxy`:** tokio CONNECT proxy; observe (allow + log) and
  enforce (strict) modes; per-instance binding; logging.
- **M2 — `creance-sandbox` (macOS):** SBPL generator (base block + FS + egress);
  `sandbox-exec` runner; profile templating/canonicalization.
- **M3 — `creance-sandbox` (Linux):** Landlock FS ruleset; **seccomp-unotify
  `connect()` supervisor** on `fspy_seccomp_unotify`; `pre_exec` ordering.
- **M4 — `creance-engine`:** `observe`/`synthesize`/`enforce`; profile schema;
  OS-keyed profile lookup.
- **M5 — `creance` CLI/shim:** entry + `-c` shim dispatch; per-package profile
  lookup from `npm_package_*`; observe/enforce/refuse decision (fail-closed);
  `--strict`.
- **M6 — hardening:** git-`prepare` handling; process-group teardown; first-party
  vs third-party policy.

## 12. Open questions

1. **Read generalization:** how coarse to roll up observed reads (per-root vs
   per-package-tree) to balance tightness against breaking on toolchain bumps.
2. **DNS on Linux:** hard-deny (force CONNECT-with-hostname, may break
   pre-resolving clients) vs a localhost allowlist-aware forwarder.
3. **Per-OS observation burden:** every platform needs its own observed entry in
   the `os` map; how teams cover platforms a developer can't run locally (CI
   observe jobs?).
4. **Scope beyond pnpm:** `script-shell` is npm-wide; how much of the CLI layer
   generalizes to npm/yarn.
