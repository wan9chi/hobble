# Threat Model

## Problem

Every dependency's `preinstall` / `install` / `postinstall` / `prepare`
script runs arbitrary code with full user privileges, before you have read a
line of that dependency. This is the main npm supply-chain attack surface:
stealing `~/.ssh` / `~/.aws` / `~/.npmrc`, editing shell rc files,
exfiltrating to attacker domains, planting persistence.

## Goal

Record what each package's scripts legitimately do — files read, files
written, domains contacted — commit that as a profile, then run the scripts
in a deny-by-default sandbox that allows only the recorded behavior. Anything
new — a malicious update, a time bomb, a typosquat — is blocked and surfaced.

## Assumptions

A lifecycle script may be actively malicious and know hobble is in use. So:

1. **Enforcement is kernel-level and deny-by-default**, over the whole
   process subtree.
2. **Observe prevents nothing; enforce is the only boundary.** Observe is the
   already-unsandboxed first install, instrumented, so the recorded profile
   is faithful. Safety comes from never auto-trusting it: a profile takes
   effect only after a human reviews the git diff and commits it.
3. **Profiles are keyed by exact `name@version`.** pnpm guarantees a version
   is fixed bytes, so changed code means a new version, which has no profile
   and cannot inherit one.

Hard constraints:

- Pure Rust native binaries. No Node runtime in the enforcement path.
- No `sudo`, ever — observe and enforce both run with ordinary user
  privileges, on macOS and Linux.
- v1 targets pnpm on macOS and Linux. Windows is deferred.

Non-goals:

- Not a replacement for auditing dependency source; it constrains behavior.
- Not payload inspection. The network gate is per-hostname; exfil over an
  allowed host remains possible (see residual risks).

## Security model

Reads, writes, and egress are all deny-by-default allowlists built from the
observed run:

- **Exfiltration** is cut by the read allowlist (can't reach `~/.ssh`, other
  projects) and, once enforced, the domain allowlist (nowhere to send).
- **Tampering and persistence** are cut by the write allowlist (can't touch
  rc files, `PATH` dirs, git hooks, other packages).

## Residual risks

| Vector | Answer |
|---|---|
| Exfil during observe | Accepted by design — observe is the already-unsandboxed first install. Nothing observed is auto-trusted; observe untrusted deps in a disposable env (CI, container). |
| Exfil over an allowed domain | The gate is per-hostname, no payload inspection. An allowed write-capable host (e.g. a registry) is a residual channel. |
| Too-broad profile gets committed | Human review of the `.hobble/` diff is the gate. See [default-allowlist.md](default-allowlist.md) for review guidance. |
| Malicious update inherits a profile | Impossible by construction: exact-version keys, new version means no entry, fail closed. |
| `pnpm rebuild` runs scripts raw | Verified pnpm limitation — `rebuild` bypasses `script-shell` (see [interception.md](interception.md)). Documented as an escape hatch users must avoid until fixed or worked around. |
| Symlink tricks | Entries are resolved at spawn; links inside allowed trees don't extend access (tested). |
| Daemon outliving the install | Own process group, killed on exit/timeout (planned in the shim). |
| Script behaves differently under enforce (time bomb) | Deny-by-default: the divergent action is blocked. |
| Toolchain bump breaks read rules | Cost, not attack: re-observe. Kept rare by coarse read roots and the built-in baseline. |
