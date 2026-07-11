# Architecture

The pieces and where they stand.

```
npm package "hobble"
  wrapHooks            .pnpmfile.cjs glue: sets script-shell       (planned)
  hobble-exec-*        native shim: observe | enforce | pass-through (planned)

Rust workspace
  hobble               CLI / shim entry                            (stub)
  hobble_sandbox       facade picking the platform backend         (done)
  hobble_sandbox_macos Seatbelt backend                            (done)
  hobble_sandbox_linux Landlock backend                            (done)
  hobble_sandbox_profile  shared path-resolution rules             (done)
  fspy                 filesystem observer                         (external crate)
  proxy                CONNECT proxy for network policy            (planned)
```

One topic per document:

- [interception.md](interception.md) — how hobble gets between pnpm and the
  scripts
- [sandbox.md](sandbox.md) — the OS-level filesystem sandbox
- [observation.md](observation.md) — recording what a script does
- [profiles.md](profiles.md) — the committed profile file format
- [network.md](network.md) — egress control (planned)
- [threat-model.md](threat-model.md) — what hobble defends against
- [default-allowlist.md](default-allowlist.md) — the default policy and
  review guidance

## Status

Done:

- FS sandbox on macOS and Linux with shared semantics and an integration
  suite that runs the checks inside sandboxed children.
- Interception mechanism verified against pnpm 10.34.4 and 11.10.0
  (see [interception.md](interception.md)).

Next, roughly in order:

1. `hobble-exec` shim: first-party check, profile lookup, enforce via
   `hobble_sandbox`, fail-closed messaging.
2. npm package: `wrapHooks`, binary distribution.
3. Observe: fspy integration, profile synthesis and merge.
4. Network: proxy crate, OS egress gates, `allowedDomains` enforcement.
5. Hardening: process-group teardown, prune command, denied-path reporting.
