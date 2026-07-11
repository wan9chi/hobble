# Developer Experience

Hobble sandboxes dependency lifecycle scripts during `pnpm install`. First run
is observed and recorded into a profile. The profile is reviewed and committed.
Every later run is sandboxed to exactly what the profile allows.

Everything hooks into pnpm itself — `pnpm install` and `pnpm add` pass
through hobble. There is no wrapper CLI to remember.

Implementation topics live in [design/](design/):

- [design/interception.md](design/interception.md) — how hobble gets between
  pnpm and the scripts
- [design/sandbox.md](design/sandbox.md) — the OS-level filesystem sandbox
- [design/observation.md](design/observation.md) — recording what a script
  does
- [design/profiles.md](design/profiles.md) — the committed profile file
  format
- [design/network.md](design/network.md) — egress control (planned)
- [design/threat-model.md](design/threat-model.md) — what hobble defends
  against
- [design/default-allowlist.md](design/default-allowlist.md) — the default
  policy and review guidance
- [design/architecture.md](design/architecture.md) — the pieces and status

## 1. Setup (once per repo)

### 1a. Allow builds explicitly

Keep pnpm's build gate on. Do NOT set `dangerouslyAllowAllBuilds`. List the
packages that may run build scripts in `pnpm-workspace.yaml`:

```yaml
onlyBuiltDependencies:
  - esbuild
  - sharp
```

This is the first line of defense: a package not on this list runs no scripts
at all. Hobble is the second line: a package on the list runs its scripts only
inside the sandbox.

(pnpm 11 renamed the setting to `allowBuilds`, a name-to-boolean map.)

### 1b. Install hobble as a config dependency

```console
$ pnpm add --config hobble
```

### 1c. Add `.pnpmfile.cjs`

```js
const { wrapHooks } = require('hobble')

module.exports = {
  hooks: wrapHooks({
    // your other hooks, or empty
  }),
}
```

(It must be `.pnpmfile.cjs` — pnpm silently ignores `.pnpmfile.mjs`.)

`wrapHooks` composes your hooks with hobble's and makes every dependency
build script run through hobble. Your own project's and workspace packages'
scripts are trusted and run unsandboxed. How the interception works is in
[design/interception.md](design/interception.md).

Windows is not supported yet: there, scripts run unwrapped and hobble prints
a warning.

## 2. Observe: `HOBBLE_OBSERVE=1 pnpm install`

With `HOBBLE_OBSERVE=1`, scripts run unsandboxed and instrumented, and every
file access and contacted domain is recorded into
`.hobble/profile-<platform>.json` (`.hobble/profile-darwin-arm64.json`, etc).

- Observe prevents nothing. It is the already-unsandboxed first install,
  instrumented. Safety comes from the next step: nothing takes effect until
  the diff is reviewed and committed. Observe untrusted new dependencies in a
  disposable environment (CI, container) when possible.
- To re-observe (after a version bump), reinstall:

  ```console
  $ rm -rf node_modules && HOBBLE_OBSERVE=1 pnpm install
  ```

  Do not use `pnpm rebuild` — it bypasses hobble entirely (a pnpm
  limitation, see [design/interception.md](design/interception.md)).

- Run observe in CI for each platform and commit the profile files.

Then review the `.hobble/` git diff and commit. The review is the approval
gate — see [design/default-allowlist.md](design/default-allowlist.md) for
what to look for.

## 3. Profiles

One JSON file per platform, committed to the repo. Each package is keyed by
its exact version:

```json5
{
  "packages": {
    "some-package@1.0.0": {
      "allowedReads": ["${PKG_DIR}"],
      "allowedWrites": ["${PKG_DIR}/dist"],
      "allowedDomains": ["example.org"],
    },
  },
}
```

A new version has no entry and is rejected until observed again — a profile
is never inherited across versions. The usual diff after a version bump is
"old key removed, new key added, same paths", which takes seconds to review.
Anything else is exactly what deserves attention.

`allowedDomains` is recorded but not enforced yet; do not rely on it today.

The full format is in [design/profiles.md](design/profiles.md).

## 4. Enforce: plain `pnpm install`

Without `HOBBLE_OBSERVE`, each script's package is looked up as
`name@version` in the current platform's profile file:

- **Entry found** — the script runs inside the sandbox, allowed only the
  entry's paths plus the built-in runtime baseline (see
  [design/default-allowlist.md](design/default-allowlist.md)).
- **No entry** — fail closed. New version and missing platform are the same
  case, with the same fix:

  ```
  ✗ esbuild@0.21.6 postinstall: no profile for darwin-arm64
    run: rm -rf node_modules && HOBBLE_OBSERVE=1 pnpm install
    then review the .hobble/ diff and commit
  ```

- **Violation** — the sandbox denies the access, the script usually fails,
  and hobble reports the package, version, lifecycle event, and exit code,
  and suggests re-observing:

  ```
  ✗ left-pad@1.3.0 postinstall failed inside the sandbox (exit 1)
    if this is a permission denial, the package's behavior changed:
    reinstall with HOBBLE_OBSERVE=1 and review the diff
  ```
