# Interception

How hobble gets between pnpm and every dependency lifecycle script:
`script-shell`, set from the pnpmfile's `updateConfig` hook.

The repo's `.pnpmfile.cjs` exports `wrapHooks(userHooks)` (see
[../DX.md](../DX.md) for the setup). Hobble's part is the `updateConfig`
hook: it sets pnpm's `scriptShell` to the absolute path of the `hobble-exec`
binary for the current platform. pnpm then runs **every** lifecycle script
as:

```
hobble-exec -c '<the full script string>'
```

The whole script — `&&`, pipes, quotes, everything — arrives as one argv
element, so there is no quoting problem and nothing can escape the wrapper.
`hobble-exec` runs it via `sh -c` under observation or enforcement.

Whatever pnpm decides to run — `install`, `add` — goes through hobble, and
pnpm's own build gate (`onlyBuiltDependencies`) stays on as the first line
of defense.

## Verified (pnpm 10.34.4 and 11.10.0, macOS arm64)

- `updateConfig` → `scriptShell` works on both; the shim receives dependency
  scripts and root-project scripts alike.
- The pnpmfile must be `.pnpmfile.cjs`. A `.pnpmfile.mjs` is silently
  ignored by both versions.
- **Rewriting scripts via `readPackage` does not work** — do not go back to
  this. pnpm runs lifecycle scripts from the on-disk `package.json` of the
  installed package; `readPackage` mutations to `scripts` (rewrite or even
  delete) do not change what executes.
- `hobble-exec` identifies the package from the env pnpm sets for every
  lifecycle script (`npm_package_name`, `npm_package_version`,
  `npm_lifecycle_event`, `PNPM_SCRIPT_SRC_DIR` — all confirmed present).
  The run mode comes from `HOBBLE_*` variables, which pnpm passes through
  (it strips inherited `npm_*`, not `HOBBLE_*` — confirmed).
- The absolute path matters: config dependencies live in
  `node_modules/.pnpm-config/`, which is not on the script `PATH`.
  `wrapHooks` knows its own location and picks the binary for the current
  platform.

## Known gaps (verified)

- **`pnpm rebuild` bypasses `script-shell` entirely** on both 10.34.4 and
  11.10.0, even with an explicit `--config.script-shell` flag: scripts run
  raw, unobserved and unsandboxed. Until this is fixed upstream or worked
  around, `pnpm rebuild` must be treated as an unsandboxed escape hatch,
  and re-observing a single package is done by reinstalling
  (`rm -rf node_modules && pnpm install`). `pnpm install --force` does not
  re-run builds and deleting a package's virtual-store dir does not either.
- **The build gate's name matching differs by version and dependency
  type.** pnpm 10: `onlyBuiltDependencies` matches registry dependencies by
  name, but does not match `file:` tarball dependencies. pnpm 11: the
  setting is `allowBuilds` (a name-to-boolean map), and an ignored build is
  a hard error instead of a warning.

## The shim binary

`hobble-exec` is a native binary. All platforms ship inside the hobble npm
package (config dependencies can't have their own dependencies, so the
per-platform optional-dependency trick is not available):

- `hobble-exec-darwin-arm64`
- `hobble-exec-darwin-x64`
- `hobble-exec-linux-arm64`
- `hobble-exec-linux-x64`
- `hobble-exec-win32-x64.exe`
- `hobble-exec-win32-arm64.exe`

Windows binaries exist but enforcement is not implemented there yet. On
win32, `wrapHooks` does not set `scriptShell` and prints a warning.

pnpm runs scripts in parallel, so observe mode takes a file lock when
merging results into the profile file (see
[observation.md](observation.md)).

## First-party scripts

The project's own and workspace packages' scripts are trusted. Since
`script-shell` applies to them too (confirmed: the root project's
`postinstall` goes through the shim), `hobble-exec` checks
`PNPM_SCRIPT_SRC_DIR` at run time: inside the project and not under
`node_modules` means first-party, and the script runs unsandboxed. The check
lives in the binary, not the pnpmfile, so it can't drift.
