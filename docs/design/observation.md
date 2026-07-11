# Observation

How observe mode records what a script does. Planned — the fspy crate
exists, the integration does not yet.

fspy is a `std::process::Command`-like observer: `DYLD_INSERT_LIBRARIES` on
macOS, `LD_PRELOAD` on glibc Linux, `seccomp_unotify` for static binaries.
It records every path access attempt pre-syscall, with canonical paths, over
the whole subtree, without sudo. Observe mode is fspy plus the proxy in
log-everything mode — no OS sandbox involved.

## Synthesis

Raw output is thousands of paths, so observed accesses are generalized
before writing the profile: writes to the narrowest sensible root
(`${PKG_DIR}/dist`, `${RUN_TMP}`), reads to coarse roots (`${PKG_DIR}`,
`${STORE}`), runtime paths dropped in favor of the built-in baseline.
Output is sorted and deduplicated so diffs are stable.

Because fspy logs attempts even when something else denies them, running it
alongside enforcement later gives exact "denied: write ~/.zshrc" error
messages. That is the plan for violation reporting; today the shim reports
exit status only.

## Caveats

- Observe only updates the entries for scripts that actually ran. An
  incremental install runs few scripts, so the profile file is merged, never
  regenerated. Scripts run in parallel, so the merge takes a file lock.
- pnpm's `side-effects-cache` must be off during observe (it is off by
  default): a cached build replays artifacts without running the script, so
  there is nothing to observe. Verified with default config: registry
  dependencies re-run their builds on a fresh `node_modules` even with a
  warm store; `file:` tarball dependencies may not.
- Re-observing one package means reinstalling
  (`rm -rf node_modules && HOBBLE_OBSERVE=1 pnpm install`): `pnpm rebuild`
  bypasses `script-shell` (see [interception.md](interception.md)), so it
  can't drive observe, and `pnpm install --force` does not re-run builds.
