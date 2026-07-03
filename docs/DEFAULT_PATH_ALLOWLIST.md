# Default Path Allowlist

This document proposes a default filesystem policy for dependency lifecycle
scripts. The goal is to make the common case work without per-package profiles,
while keeping review focused on genuinely risky package-specific deltas.

## Principles

- Default allowances should be symbolic, not absolute local paths.
- Default toolchain paths should be resolved at runtime and treated as
  read-only.
- Writes should default to Hobble-controlled scratch/cache locations, not the
  user's real home or toolchain directories.
- Package-specific profiles should contain only behavior outside this default:
  package-local artifact writes, package-specific caches, and domains.
- This document only lists allow candidates. Every path not listed here, or in a
  reviewed package-specific profile delta, is denied by default.

## Default Read Allowlist

| Path | Allow? | Sensitive data possibility | Notes |
| --- | --- | --- | --- |
| `${PKG_DIR}/**` | Yes | Low | The dependency lifecycle script can already read its own package files. This may include bundled package secrets if the publisher made a mistake, but not user-local secrets. |
| `${STORE}/**` | Yes, preferably dependency closure only | Medium | The pnpm store contains package source for many dependencies. It normally should not contain user credentials, but it can contain private package source or accidentally published secrets. Prefer limiting this to the current package's dependency closure rather than the entire store when feasible. |
| `${RUN_TMP}/**` | Yes | Low | Hobble-created temporary directory. May contain transient build data from the current script. Should be per-run and cleaned up. |
| `${RUN_HOME}/**` | Yes | Low to Medium | Synthetic home directory for the script. Safe if created empty by Hobble. It can become sensitive during the run if tools write tokens/config into it, so it should be per-run or per-project cache-scoped and not shared broadly. |
| `${CACHE}/**` | Yes, Hobble-controlled only | Medium | Build/package-manager caches can contain downloaded packages, metadata, generated code, and logs. They should not contain real user secrets, but logs can include URLs or environment-derived metadata. Prefer subpaths such as `${CACHE}/npm/**`, `${CACHE}/corepack/**`, `${CACHE}/pnpm/**`. |
| `${NODE_ROOT}/**` | Yes, read-only | Low | Resolved active Node.js installation. Needed for Node/npm startup. Usually code and runtime assets, not secrets. |
| `${PNPM_HOME}/**` | Yes, read-only if real; read/write only if Hobble-controlled | Low to Medium | Contains package-manager shims/binaries. A real user path should not be writable by dependency scripts. A Hobble-controlled synthetic path can be writable if needed. |
| `${COREPACK_HOME}/**` | Yes if Hobble-controlled | Medium | Corepack may download package-manager releases. Keep this under Hobble control. A user's real Corepack home should not be writable by dependency scripts. |
| `${MISE_DATA_DIR}/**` | Yes, read-only and only if it contains the resolved toolchain | Medium | Local developer toolchain state. It may contain installed runtimes, manifests, plugins, and metadata. It should not be committed into package profiles and should not be writable. Prefer allowing only the resolved runtime/tool subtrees, not the entire mise data directory. |
| `${VITE_PLUS_ROOT}/**` | Yes, read-only and only if it contains the resolved toolchain | Medium | Local developer/runtime manager state observed in research. It can include runtime caches and local metadata. Do not put it in package profiles; resolve it as base runtime policy. Writes should go to `${RUN_HOME}`/`${CACHE}`, not the real directory. |
| `/usr/**`, `/System/**`, `/Library/**` on macOS | Yes, read-only | Low | OS/runtime files needed by shells, dynamic loader, certificates, and system libraries. Generally not user secrets, though some machine metadata may be readable. Keep as built-in runtime roots. |
| `/bin/**`, `/sbin/**`, `/lib/**`, `/lib64/**`, `/usr/**`, `/etc/**` on Linux | Yes, read-only, with care for `/etc` | Low to Medium | Needed for runtime startup, dynamic loader, CA certs, timezone, NSS, shells, and system libraries. `/etc` may contain machine configuration; avoid broadening beyond what is required if the implementation can split runtime files more tightly. |
| `/dev/null`, `/dev/zero`, `/dev/random`, `/dev/urandom` | Yes | Low | Required runtime devices. No persistent sensitive data. |
| Timezone/locale/certificate roots | Yes, read-only | Low to Medium | Needed by Node, TLS, and libc. Certificates are public; timezone/locale data is low sensitivity. Avoid allowing unrelated system config by accident. |

## Default Write Allowlist

| Path | Allow? | Sensitive data possibility | Notes |
| --- | --- | --- | --- |
| `${RUN_TMP}/**` | Yes | Low | Primary default write target. Set `TMPDIR` to this path so random temp writes generalize safely. |
| `${RUN_HOME}/**` | Yes | Low to Medium | Synthetic home for tools that insist on writing under home. Should not be the real user home. |
| `${CACHE}/npm/**` | Yes | Medium | npm cache and logs. Can include downloaded package metadata and debug logs. Keep Hobble-controlled and review logs if needed. |
| `${CACHE}/corepack/**` | Yes | Medium | Corepack package-manager downloads. Keep Hobble-controlled. |
| `${CACHE}/pnpm/**` | Yes | Medium | pnpm cache/metadata. Prefer Hobble-controlled cache paths. |
| `${CACHE}/yarn/**` | Yes | Medium | Yarn cache/metadata. Prefer Hobble-controlled cache paths. |

## Common Package-Specific Write Deltas

These are not default allowances. They are common shapes that an observed package
may propose as a reviewed profile delta.

| Path | Sensitive data possibility | Notes |
| --- | --- | --- |
| `${PKG_DIR}/dist/**` | Low to Medium | Common generated output path. Review whether the package really generated multiple stable files under this root. |
| `${PKG_DIR}/bin/**` | Low to Medium | Common location for downloaded or generated platform binaries. Review carefully because executable output can affect later package use. |
| `${PKG_DIR}/vendor/**` | Low to Medium | Common location for vendored native binaries or assets. Review domains and hashes when available. |
| `${PKG_DIR}/<exact-file>` | Low to Medium | Preferred shape for one-off artifact writes such as `legacy.js`, `parser.js`, or `*.min.js`. Exact files are easier to review than broad package writes. |

## Review Guidance

When Hobble proposes a package-specific delta, reviewers should ask:

- Is the new read outside `${PKG_DIR}`, `${STORE}`, or runtime roots?
- Is the new write outside `${RUN_TMP}`, `${RUN_HOME}`, or Hobble-controlled
  cache?
- Does a package-local write target a narrow artifact path, or a broad package
  tree?
- Is the domain allowlist exact and expected for the package's install behavior?

Any profile that grants a broad write path, especially `${PKG_DIR}/**`, should
be treated as high-risk and require explicit justification.
