# Codex Sandbox Implementation Index

Reference notes on the sandbox implementation in [openai/codex](https://github.com/openai/codex),
researched 2026-07-02 at commit `129ea2aaf5fb426d8ba683ee53f290742f41dd31` (2026-07-01).
All paths below are relative to the codex repo root; line numbers are pinned to that
commit. The Rust workspace lives under `codex-rs/`.

Relevant crates: `codex-rs/protocol` (policy data model), `codex-rs/sandboxing`
(OS-agnostic manager + per-OS command transforms), `codex-rs/linux-sandbox`,
`codex-rs/bwrap`, `codex-rs/windows-sandbox-rs`, `codex-rs/execpolicy`, and the
glue in `codex-rs/core` (`spawn.rs`, `sandboxing/mod.rs`, `tools/orchestrator.rs`).

## Architecture overview

Policy is described platform-independently, then transformed per OS:

- **macOS**: rewrite the argv to `/usr/bin/sandbox-exec -p <profile> -Dkey=val -- <cmd>`.
- **Linux**: rewrite the argv to a helper (`codex-linux-sandbox`, dispatched via an
  argv[0] trick from the multitool binary) which sets up **bubblewrap** for the
  filesystem view, re-execs itself inside, applies **seccomp** + `no_new_privs`,
  then execs the real command.
- **Windows**: run the command under a **restricted token** (`WRITE_RESTRICTED`)
  whose write access is granted via ACLs on writable roots; an opt-in "elevated"
  backend adds dedicated local users, firewall + WFP network blocks, and a job object.

Sandbox denial is detected heuristically after the fact and feeds an
approval/escalation loop (retry unsandboxed with user approval).

## Policy model (shared layer)

Two coexisting models with bidirectional bridges:

### Legacy `SandboxPolicy` — `protocol/src/protocol.rs:984`

Modes: `danger-full-access`, `read-only { network_access }`,
`external-sandbox { network_access }`, `workspace-write { writable_roots,
network_access, exclude_tmpdir_env_var, exclude_slash_tmp }`.

- Reads are always full-disk: `has_full_disk_read_access()` hardcodes `true`
  (`protocol.rs:1136`). Only writes are restricted.
- `get_writable_roots_with_cwd` (`protocol.rs:1161`): `workspace-write` roots =
  configured roots + cwd + `/tmp` (unix, if a dir, unless excluded) + `$TMPDIR`
  (if set, unless excluded).

### Richer `FileSystemSandboxPolicy` — `protocol/src/permissions.rs:217`

`{ kind: Restricted|Unrestricted|ExternalSandbox, glob_scan_max_depth, entries }`
where each entry is `{ path, access }`:

- `access: FileSystemAccessMode = Read | Write | Deny` — conflict precedence
  **deny > write > read**; among overlapping path entries the **most specific
  (deepest) path wins**, tie-broken by that mode order (`resolved_entry_precedence`,
  `permissions.rs:1446`).
- `path: FileSystemPath = Path | GlobPattern (deny-only) | Special`, where
  `Special = Root | Minimal | ProjectRoots{subpath} | Tmpdir | SlashTmp | Unknown`
  (`Unknown` is deliberate forward-compat, `permissions.rs:150`).
- So "writable tree except X" is `[{project_roots: write}, {X: deny}]`, and a more
  specific `write` under X re-grants it.
- Resolution outputs: `get_writable_roots_with_cwd`, `get_readable_roots_with_cwd`,
  `get_unreadable_roots_with_cwd`, `get_unreadable_globs_with_cwd`
  (`permissions.rs:973-1151`).

The runtime container is `PermissionProfile = Managed{file_system, network} |
Disabled | External{network}` (`protocol/src/models.rs:404`). The legacy bridge
`to_legacy_sandbox_policy` can fail (writes outside workspace root), in which case
the richer policy must be enforced directly (`permissions.rs:1153`, `:944`).

### `WritableRoot` — `protocol/src/protocol.rs:1043`

```rust
pub struct WritableRoot {
    pub root: AbsolutePathBuf,
    pub read_only_subpaths: Vec<AbsolutePathBuf>,  // carve-outs under root
    pub protected_metadata_names: Vec<String>,     // ".git", ".agents", ".codex"
}
```

Every writable root automatically gets read-only carve-outs for `.git`, `.agents`,
`.codex` if present (`default_read_only_subpaths_for_writable_root`,
`permissions.rs:1593`). `.codex` at the cwd root is protected **even before it
exists** so the sandboxed process cannot create it. `.git` handling covers all
three repo shapes: `.git` dir, and `.git` *pointer file* (worktree/submodule) whose
`gitdir:` target is resolved and also protected (`permissions.rs:1857`).

### Path normalization

- Roots and carve-outs are canonicalized at policy-resolution time; carve-out logic
  deliberately **preserves the literal in-root path** (not just the resolved
  target) so per-OS backends can mask the symlink itself (`permissions.rs:1035-1088`).
- Missing `/tmp` / unset `$TMPDIR` are skipped; missing writable roots are skipped
  on Linux (`bwrap.rs:376`), only-existing paths get ACLs on Windows.
- Dedup by canonical form; readable roots nested under writable roots are dropped.

### Network policy

Just `Restricted | Enabled` at the shared layer (`permissions.rs:84`), plus managed
proxy plumbing. No per-host allowlist in the policy model.

## macOS (Seatbelt)

Entry: `create_seatbelt_command_args` (`sandboxing/src/seatbelt.rs:623`). Spawns
`/usr/bin/sandbox-exec` (path pinned against PATH injection, `seatbelt.rs:26`) —
no in-process `sandbox_init`, no `pre_exec`. Final profile = base + file-read
section + file-write section + glob denies + network section (+ optional
platform read-only defaults for `:minimal`), joined in that order
(`seatbelt.rs:741`).

- **Base profile** (`sandboxing/src/seatbelt_base_policy.sbpl`, `include_str!`ed):
  `(deny default)`, then a curated allowlist modeled on Chrome's sandbox:
  `process-exec`, `process-fork`, `signal`/`process-info*` limited to
  `(target same-sandbox)`, named `sysctl-read` list, one IOKit class, a few
  `mach-lookup` names, POSIX sem/shm for Python/PyTorch, `pseudo-tty` +
  `/dev/ptmx` + `/dev/ttys*`, `user-preference-read`, and `file-write-data` to
  `/dev/null` guarded by `(vnode-type CHARACTER-DEVICE)`. No file or network
  rules in the base — layered per policy.
- **Roots as parameters, not string interpolation**: each root becomes
  `-DWRITABLE_ROOT_n=<path>` / `READABLE_ROOT_n`, referenced as
  `(subpath (param "..."))` (`build_seatbelt_access_policy`, `seatbelt.rs:352`).
- **Carve-outs** inside a root: `(require-all (subpath root) (require-not
  (literal excl)) (require-not (subpath excl)) ...)`. Both `literal` and
  `subpath` are needed — `subpath` alone leaves a gap for first-time creation of
  the protected dir itself (`mkdir .codex`) (`seatbelt.rs:381`). Protected
  metadata names additionally become anchored-regex `require-not`s.
- **Reads are policy-driven**: full-disk read ⇒ `(allow file-read*)`; restricted
  read ⇒ per-root `file-read*` allows; unreadable roots become `require-not`
  carve-outs of a `/`-rooted allow. Deny-globs are translated to anchored regex
  `(deny file-read* ...)` plus `(deny file-write-unlink ...)` so denied paths
  can't be probed via unlink (`seatbelt.rs:441`).
- **Canonicalization**: every root/carve-out goes through
  `normalize_path_for_sandbox` (`seatbelt.rs:172`) — absolute required, then
  `canonicalize()` (e.g. `/tmp` → `/private/tmp`), falling back to the literal
  path if resolution fails.
- **Network**: disabled ⇒ no network rules at all (deny-default catches it).
  Enabled ⇒ `(allow network-outbound)` + `(allow network-inbound)` + a static
  `.sbpl` of TLS/DNS support services (`com.apple.SecurityServer`, `networkd`,
  `trustd`, DNS config, `AF_SYSTEM` socket). Proxy mode allows loopback/proxy
  ports only, fail-closed if no usable endpoint. Unix sockets: allow-all or
  per-path `(subpath (param ...))` allowlist.
- Full-disk write emits `(allow file-write* (regex #"^/"))` with a comment that
  it is "allegedly more permissive than `(allow file-write*)`" (`seatbelt.rs:643`).

## Linux (bubblewrap + seccomp; Landlock is legacy)

Helper crate `codex-rs/linux-sandbox`; README is the canonical doc
(`linux-sandbox/README.md`). Two-stage: outer stage builds the bwrap argv whose
inner command is the helper itself with `--apply-seccomp-then-exec`
(`linux_run_main.rs:213`, `:1401`); inner stage applies seccomp + `no_new_privs`
then `execvp`s the command. seccomp comes *after* bwrap because bwrap may rely
on setuid, which `no_new_privs` would break (`linux_run_main.rs:110`).

- **bwrap sourcing**: first system `bwrap` on PATH, else a **bundled, vendored
  build** (`codex-rs/bwrap` compiles bubblewrap C sources via the `cc` crate),
  SHA-256-verified and run via `/proc/self/fd/N` (`launcher.rs`, `bundled_bwrap.rs`).
- **Mount layout** (`bwrap.rs::create_filesystem_args:367`):
  - Full-read policy: `--ro-bind / /`; restricted-read: `--tmpfs /` + scoped
    `--ro-bind` per readable root (+ platform defaults `/bin /sbin /usr /etc /lib
    /lib64 /nix/store /run/current-system/sw` for `:minimal`).
  - `--dev /dev` (minimal device set: null, zero, full, random, urandom, tty).
  - Writable roots: `--bind root root`; carve-outs re-applied **after** as
    `--ro-bind sub sub` so protected subpaths win; denied dirs become
    `--tmpfs` masked `--remount-ro` with perms `000` (or `111` when a writable
    descendant needs traversal).
  - Missing protected-metadata paths are masked with `/dev/null` binds or empty
    tmpfs, plus an inotify `ProtectedCreateMonitor` that deletes protected paths
    the child creates and fails the run (`bwrap.rs:1074`, `linux_run_main.rs:549`).
  - Namespaces: always `--unshare-user --unshare-pid`, `--die-with-parent`,
    `--new-session`; `--unshare-net` unless network enabled. `--proc /proc` with
    a preflight `/bin/true` run and `--no-proc` retry for restrictive containers.
- **Fail-closed symlink rule**: a read-only or deny carve-out whose path crosses
  a **writable symlink** is refused outright (TOCTOU — the child could retarget
  the link after the startup-time bind) (`bwrap.rs:1026`, `:1144`).
- **seccomp** (`linux-sandbox/src/landlock.rs:169`, seccompiler 0.5): default
  allow; denies `ptrace`, `process_vm_readv/writev`, `io_uring_*`
  unconditionally. Network-restricted mode denies `connect/bind/listen/accept/
  sendto/...` but deliberately allows `recvfrom` (socketpair-based tooling) and
  `socket(AF_UNIX)` only. Proxy mode inverts: `socket(AF_INET/6)` only.
  `128+SIGSYS` exit is treated as a sandbox denial.
- **Landlock** (`linux-sandbox/src/landlock.rs:128`): legacy opt-in fallback
  only ("currently unused... kept for reference"). ABI **V5**, compat
  **best-effort**, read-only on `/` + rw on `/dev/null` + rw per writable root.
  Cannot express restricted reads or carve-outs — it bails on such policies;
  that limitation is exactly why filesystem enforcement moved to bubblewrap.
- Full write + full network + no deny-globs ⇒ runs unwrapped; full write but
  restricted network still wraps with `--bind / /` just for the netns.

## Windows (restricted token; elevated backend)

Crate `codex-rs/windows-sandbox-rs`. Opt-in (`[windows] sandbox = "elevated" |
"unelevated"`), off by default. Not AppContainer, not low-integrity: a
**restricted token** via `CreateRestrictedToken(DISABLE_MAX_PRIVILEGE | LUA_TOKEN
| WRITE_RESTRICTED)` (`token.rs:427`) — writes succeed only where the DACL names
one of the restricting SIDs; reads are unaffected.

- **"Capability" SIDs** are random ordinary group SIDs (`S-1-5-21-...`),
  persisted per-root in `CODEX_HOME/cap_sid` (`cap.rs`). Writable roots get
  inheritable allow-ACEs for their root's SID; carve-outs get deny-write ACEs
  (denies are ordered before allows) (`acl.rs`). Deny carve-outs that don't
  exist are **created as dirs before launch** so the child can't
  create-then-bypass them (`spawn_prep.rs:280`).
- Legacy backend cannot restrict reads (WRITE_RESTRICTED only affects writes) —
  it refuses to run policies needing read restriction (`lib.rs:530`). Network
  restriction is env-only/cooperative: proxy vars to `127.0.0.1:9`,
  `CARGO_NET_OFFLINE`, `PIP_NO_INDEX`, ssh/scp stub denybin, `SBX_NONET_ACTIVE=1`.
- **Elevated backend**: UAC-provisioned local users `CodexSandboxOffline/Online`
  + `CodexSandboxUsers` group (DPAPI-stored random passwords), read restriction
  via read/exec ACLs granted only on intended roots, deny-read ACEs on secrets
  (`$USERPROFILE` expansion excludes `.ssh/.aws/.gnupg/...`, `setup.rs:50`), real
  network enforcement via per-account Windows Firewall rules + persistent WFP
  BLOCK filters (ICMP, DNS 53/853, SMB) keyed on `ALE_USER_ID`, command run by a
  `codex-command-runner` helper over framed pipe IPC, job object
  `KILL_ON_JOB_CLOSE`, optional private desktop. A world-writable-directory audit
  applies deny ACEs so `Everyone`-writable dirs aren't an escape.

## Cross-cutting

- **Env sanitization** (`protocol/src/shell_environment.rs`,
  `config_types.rs:189`): child env built from scratch (`env_clear()` then an
  explicit map). Policy: inherit `All|Core|None` (`Core` = PATH/HOME/USER/...),
  default-exclude vars matching `*KEY*`/`*SECRET*`/`*TOKEN*`, custom
  exclude/set/include_only.
- **Markers**: `CODEX_SANDBOX_NETWORK_DISABLED=1` whenever network restricted;
  `CODEX_SANDBOX=seatbelt` on macOS only (`core/src/spawn.rs:12-25`).
- **Denial detection** (`sandboxing/src/denial.rs`): heuristic — nonzero exit +
  stderr/stdout keywords (`operation not permitted`, `permission denied`,
  `read-only file system`, `seccomp`, `sandbox`, `landlock`, ...), quick-reject
  exit codes 2/126/127, `128+SIGSYS` under Linux. Feeds the orchestrator's
  approve-then-retry-unsandboxed escalation (`core/src/tools/orchestrator.rs:1-8`);
  never auto-escalates under `AskForApproval::Never`.
- **execpolicy** (`codex-rs/execpolicy`): a separate command-classification layer
  (prefix rules in Starlark → `allow | prompt | forbidden` per argv), independent
  of and complementary to the filesystem sandbox.

## Gap analysis: hobble vs codex (as of 2026-07-02)

Hobble at time of writing: `SandboxProfile { allowed_reads, allowed_writes }`
(union-only allowlist, missing entries skipped at spawn — paths created later
stay denied, dir ⇒ subtree, spawn-time symlink resolution); macOS via in-process
`sandbox_init` in `pre_exec`; Linux via Landlock ABI V1 hard-requirement. Glob
support is an explicit non-goal.

Missing / divergent, roughly by importance:

1. **Network policy — absent entirely.** *Decided out of scope for hobble's
   sandbox layer (2026-07).* Hobble's macOS profile denies only
   `file-read*`/`file-write*` (no `deny default`), so network, mach, IOKit etc.
   are open; Landlock V1 doesn't touch network at all. Codex denies network by
   default on every platform (deny-default profile / `--unshare-net` + seccomp /
   firewall+WFP). Kept here as reference in case scope changes.
2. **Deny / read-only carve-outs inside writable roots.** Hobble's union-only
   model can't express them by design. Codex treats them as load-bearing
   (`.git`/`.codex` protection, deny > write > read precedence). Feasibility if
   ever needed: Seatbelt `require-not` (cheap); **Landlock cannot do it** — this
   is the main reason codex moved Linux enforcement to bubblewrap.
3. **Linux mechanism headroom.** Hobble is on Landlock ABI V1 with
   `HardRequirement`: no carve-outs, cross-directory rename/link always fails
   (`EXDEV`; `Refer` needs ABI ≥ V2), rules pin inodes, network out of scope.
   Codex's endpoint is namespaces + bind mounts + seccomp with a vendored bwrap
   fallback. Cheap intermediate step: bump ABI and reconsider compat level.
4. **Device files on Linux.** Codex explicitly grants `/dev/null` (Landlock
   path) or mounts a minimal `--dev` set (bwrap). Hobble's Landlock ruleset has
   no `/dev` rule yet. Covered by hobble's per-platform default allowlist
   (documented in the `hobble_sandbox` crate); implementation pending.
5. **macOS confinement breadth.** Codex: `(deny default)` + curated allowlist
   (sysctl/mach/pty/shm needs of real toolchains, `/dev/null` write,
   same-sandbox signal scoping). Hobble: default-allow except file read/write.
   Codex's base profile is a ready-made checklist of what npm/python/java
   toolchains actually need if hobble tightens to deny-default.
6. **Env sanitization.** *Decided out of scope for hobble's sandbox layer
   (2026-07).* Hobble spawns with the inherited environment; fs sandboxing
   doesn't stop secret exfiltration via env (`*_TOKEN`, `*_KEY`). Codex's
   `env_clear` + core-var allowlist + pattern excludes documented here for
   reference.
7. **Sandbox-denial detection.** Codex's `is_likely_sandbox_denied` heuristics +
   SIGSYS mapping would serve hobble's propose-profile-delta loop (knowing a
   script failed *because of the sandbox* vs on its own).
8. **Process lifecycle hardening.** die-with-parent / new-session (bwrap flags,
   job objects on Windows) and seccomp denial of `ptrace`/`process_vm_*`/
   `io_uring` — introspection/escape channels Landlock does not cover.
9. **Windows support** — none in hobble. Codex's split is instructive: an
   unprivileged `WRITE_RESTRICTED`-token backend can restrict writes only; real
   read/network restriction required the elevated-accounts design.
10. **Symlink TOCTOU rule.** Only relevant if carve-outs are added: codex
    fails closed when a protected subpath crosses a writable symlink.

Deliberate hobble divergences (not gaps): no globs; single profile model rather
than legacy + rich pair; network and env sanitization out of scope.
