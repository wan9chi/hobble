# Creance

Creance observes and enforces pnpm dependency lifecycle scripts on macOS. In
observe mode it records the filesystem paths and CONNECT proxy domains a package
script used. In strict install mode it replays that package script under a
deny-by-default macOS Seatbelt sandbox using the reviewed profile.

This README assumes `creance` is already installed and available on `PATH`.

## What Creance Protects

Creance targets third-party dependency scripts executed from pnpm's virtual
store, such as:

```text
node_modules/.pnpm/<package>@<version>/node_modules/<package>
```

First-party project scripts are skipped by this proof of concept. The goal is to
review and constrain dependency install scripts, not your own workspace commands.

## Requirements

- macOS
- pnpm project with a lockfile
- `creance` on `PATH`

Check the installed version:

```sh
creance --version
```

Output:

```text
creance 0.1.0
```

## Basic Workflow

Run these commands from the root of a pnpm project.

1. Observe a normal install.

   ```sh
   creance observe
   ```

   Example output:

   ```text
   .../node_modules/@aspect-test/c postinstall$ echo "{\"answer\":\"42*\"}" > data.json
   .../node_modules/@aspect-test/c postinstall: Done

   dependencies:
   + @aspect-test/c 2.0.0

   Done in ... using pnpm ...
   ```

   Creance writes a profile for each observed dependency script. For the example
   above, the profile path is:

   ```text
   .creance/profiles/@aspect-test/c/2.0.0.json
   ```

2. Review the generated profile.

   The profile content looks like this:

   ```json
   {
     "package": {
       "name": "@aspect-test/c",
       "version": "2.0.0"
     },
     "creance": "0.1.0",
     "lifecycle": "postinstall",
     "entries": [
       {
         "os": [
           "darwin"
         ],
         "read": [
           "${PKG_DIR}",
           "${PROJECT_ROOT}",
           "/private/var/db/timezone/tz/2026b.1.0/zoneinfo/Asia/Shanghai"
         ],
         "write": [
           "${PKG_DIR}/data.json"
         ],
         "domains": []
       }
     ]
   }
   ```

   Review the `read`, `write`, and `domains` lists before trusting the profile.
   Small, package-local writes are usually easier to justify than broad project,
   home, cache, or network access.

3. Run strict enforcement.

   ```sh
   creance install --strict
   ```

   Example output for the reviewed profile above:

   ```text
   .../node_modules/@aspect-test/c postinstall$ echo "{\"answer\":\"42*\"}" > data.json
   .../node_modules/@aspect-test/c postinstall: Done

   dependencies:
   + @aspect-test/c 2.0.0

   Done in ... using pnpm ...
   ```

   The package can still create its reviewed side effect:

   ```text
   node_modules/.pnpm/@aspect-test+c@2.0.0/node_modules/@aspect-test/c/data.json
   ```

   File content:

   ```json
   {"answer":"42*"}
   ```

4. Commit the reviewed policy.

   Commit the relevant files under:

   ```text
   .creance/profiles/
   ```

   Keep those profiles under review just like lockfile changes. A profile update
   means a dependency script asked for a new filesystem or network capability.

## When Enforcement Blocks a Script

If a dependency script tries to write outside its reviewed profile, strict mode
fails the script. A blocked project-root write looks like this:

```text
/bin/sh: .../project/pwned: Operation not permitted
```

For a missing profile in strict mode, Creance fails closed. Observe the project
first, review the new profile, then retry strict install.

## Generated Profile JSON

Profiles are the policy source Creance uses during strict installs. They live at:

```text
.creance/profiles/<package-name>/<version>.json
```

Important fields:

- `package`: the npm package name and version this profile applies to.
- `creance`: the Creance version that wrote the profile.
- `lifecycle`: the lifecycle event that produced the profile, such as
  `install` or `postinstall`.
- `entries`: OS-specific policy entries. The current macOS proof of concept
  writes `darwin` entries.
- `read`: filesystem paths the dependency script may read.
- `write`: filesystem paths the dependency script may write.
- `domains`: CONNECT proxy hostnames the dependency script may contact during
  strict enforcement.

When one package runs more than one lifecycle script for the same package
version, Creance merges the observed allowlists into the same OS entry. That
keeps one reviewed profile per `name@version`.

## Path Templates

Profile paths can be absolute macOS paths or template variables:

- `${PKG_DIR}`: the dependency package directory under pnpm's virtual store.
- `${PROJECT_ROOT}`: the directory where `creance observe` or
  `creance install` was run.
- `${STORE}`: the pnpm store path.
- `${HOME}`: the user's home directory.
- `${CACHE}`: the user's cache directory.
- `${RUN_TMP}`: Creance's per-package temporary runtime directory.

Entries ending in `/**` are subtree allowances. For example:

```text
${PKG_DIR}/build/**
```

This allows access below the dependency package's own `build` directory. It does
not allow arbitrary writes elsewhere in the project.

## Practical Review Rules

- Prefer package-local writes such as `${PKG_DIR}/build/**`.
- Treat `${PROJECT_ROOT}/**`, `${HOME}/**`, and `${CACHE}/**` as broad
  permissions that need a clear reason.
- Review every new network domain in `domains`.
- Re-run `creance observe` only when dependencies or lifecycle behavior changed.
- Run `creance install --strict` in automation after profiles are reviewed.
