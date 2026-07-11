# Sandbox

The OS-level filesystem sandbox. Implemented in the `hobble_sandbox` crates.

`hobble_sandbox::spawn_with_sandbox(command, profile)` runs a command with a
`SandboxProfile { allowed_reads, allowed_writes }`. Semantics, identical on
both platforms and covered by the integration tests:

- Pure allowlists over a default-deny baseline. No deny rules, so one entry
  can never narrow another.
- Entries are literal absolute paths, no globs. A directory grants its
  subtree; a file grants that file. Write implies read. Read implies exec.
- Entries resolve once at spawn. Missing entries (including dangling
  symlinks) are skipped and stay denied even if created later. Symlinks are
  fully resolved and both spellings granted. A symlink inside an allowed
  tree does not extend access to its target.
- Path metadata (`stat`, `readlink`) is not confined; content, listing, and
  writing are.
- A built-in read-only baseline makes shells and runtimes boot without
  per-profile boilerplate: on macOS, Apple's `system.sb` plus `/bin`,
  `/sbin`, `/usr`, `/opt/homebrew`; on Linux, `/bin`, `/sbin`, `/lib`,
  `/lib64`, `/usr`, `/etc`, the standard device nodes, the global `/proc`
  info files, and the process's own `/proc/self` (other processes' `/proc`
  stays denied).

## Backends

- **macOS — Seatbelt.** Generates an SBPL profile and applies it via
  `sandbox_init_with_parameters` in `pre_exec`, before exec. No
  entitlement, no SIP change, no sudo.
- **Linux — Landlock (ABI v1) + `no_new_privs`.** Ruleset built in the
  parent, applied in `pre_exec`. Fails if the ruleset is not fully enforced.
  No user namespaces, so no Ubuntu AppArmor gate.

Network is not confined by the sandbox yet — see [network.md](network.md)
for the plan.
