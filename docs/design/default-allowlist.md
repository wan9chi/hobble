# Default Allowlist

The default filesystem policy for dependency lifecycle scripts. The goal: the
common case works without per-package entries, so profile diffs stay small
and review stays focused on what a package actually does differently.

Everything not listed here, or in a reviewed package entry, is denied.

## Principles

- Defaults are symbolic (`${STORE}`, `${NODE_ROOT}`), never absolute local
  paths, so profiles stay portable across machines.
- Toolchain paths are resolved at run time and read-only.
- Writes default to hobble-controlled scratch and cache locations, never the
  real home or toolchain directories.
- Package entries hold only what is outside the default: package-local
  artifact writes, package-specific caches, domains.

Entries are literal paths — a directory grants its subtree (see
[sandbox.md](sandbox.md)).

## Default reads

| Path | Allow | Sensitive? | Notes |
| --- | --- | --- | --- |
| `${PKG_DIR}` | Yes | Low | The script can already read its own package. |
| `${STORE}` | Yes | Medium | The pnpm store holds source for all dependencies, possibly private packages. Prefer narrowing to the package's dependency closure when feasible. |
| `${RUN_TMP}` | Yes | Low | Hobble-created temp dir, per run, cleaned up. |
| `${RUN_HOME}` | Yes | Low–Medium | Synthetic home, created empty by hobble. Keep per-run or per-project; tools may write tokens into it during a run. |
| `${CACHE}` | Yes, hobble-controlled only | Medium | Prefer subpaths: `${CACHE}/npm`, `${CACHE}/pnpm`, `${CACHE}/corepack`. |
| `${NODE_ROOT}` | Yes, read-only | Low | The resolved Node installation; needed for startup. |
| `${PNPM_HOME}` | Read-only if real | Low–Medium | Package-manager shims. A real user path must not be writable by dependency scripts. |
| `${COREPACK_HOME}` | Yes, if hobble-controlled | Medium | Corepack downloads package managers. Keep under hobble control. |
| mise / vite-plus data dirs | Read-only, resolved toolchain only | Medium | Base runtime policy resolved at run time (`global.allowedReads`), not per-package entries. Prefer the resolved runtime subtrees, not the whole data dir. |
| macOS: `/usr`, `/System`, `/Library`, `/bin`, `/sbin`, `/opt/homebrew` | Built-in, read-only | Low | Runtime baseline compiled into the sandbox. |
| Linux: `/bin`, `/sbin`, `/lib`, `/lib64`, `/usr`, `/etc` | Built-in, read-only | Low–Medium | `/etc` carries machine config; keep as narrow as the implementation allows. |
| `/dev/null`, `/dev/zero`, `/dev/random`, `/dev/urandom` | Built-in | Low | Required device nodes. |
| Timezone / locale / CA certificate roots | Yes, read-only | Low | Needed by Node, TLS, libc. |

## Default writes

| Path | Allow | Sensitive? | Notes |
| --- | --- | --- | --- |
| `${RUN_TMP}` | Yes | Low | Primary write target. Set as `TMPDIR` so random temp writes land in one place. |
| `${RUN_HOME}` | Yes | Low–Medium | For tools that insist on writing under home. Never the real home. |
| `${CACHE}/npm`, `${CACHE}/pnpm`, `${CACHE}/corepack`, `${CACHE}/yarn` | Yes | Medium | Hobble-controlled cache paths only. |

## Common package-specific write entries

Not defaults — shapes that an observed package often proposes, to be
reviewed:

| Path | Notes |
| --- | --- |
| `${PKG_DIR}/dist` | Common build output. Check the package really generates files here. |
| `${PKG_DIR}/bin` | Downloaded or generated platform binaries. Review carefully — executable output affects later use of the package. |
| `${PKG_DIR}/vendor` | Vendored native binaries or assets. Check the domains too. |
| `${PKG_DIR}/<exact-file>` | Best shape for one-off artifacts (`parser.js`, `*.min.js`). An exact file is easier to review than a tree. |

## Reviewing a profile diff

When observe proposes a new or changed entry, ask:

- Is a new read outside `${PKG_DIR}`, `${STORE}`, and the runtime roots?
- Is a new write outside `${RUN_TMP}`, `${RUN_HOME}`, and hobble-controlled
  cache?
- Does a package-local write target a narrow artifact path or the whole
  package tree? A broad write like all of `${PKG_DIR}` is high-risk and
  needs explicit justification.
- Is the domain list exact and expected for what the package installs?
- On a version bump: are the paths identical to the old entry? Identical is
  a quick approve; anything new is the thing to scrutinize.
