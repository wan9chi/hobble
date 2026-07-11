# Profiles

The committed profile file format.

One file per platform, `.hobble/profile-<platform>.json`, committed to the
repo. One file per platform means parallel CI observe jobs never conflict.

```json5
{
  "hobble": 1,                      // schema version
  "platform": "darwin-arm64",
  "global": {
    // machine toolchain paths every script may read
    "allowedReads": [
      "~/.local/share/mise",
      "~/.config/mise",
    ],
  },
  "packages": {
    // exact version, always "name@version"
    "some-package@1.0.0": {
      "allowedReads": [
        "${PKG_DIR}",
        "${PROJECT}/.some-package",
      ],
      "allowedWrites": [
        "${PKG_DIR}/dist",
      ],
      "allowedDomains": [
        "example.org",
      ],
    },
  },
}
```

- **Keys are exact versions.** A new version has no entry and fails closed
  until observed again. Combined with pnpm's fixed-bytes guarantee, a
  profile can never be inherited by changed code (see
  [threat-model.md](threat-model.md)).
- **Paths are literal, no globs**, matching the sandbox semantics (see
  [sandbox.md](sandbox.md)): a directory entry grants its subtree, a file
  entry grants that file. Template variables are resolved to absolute paths
  at run time: `${PKG_DIR}` (the package's directory), `${PROJECT}` (the
  workspace root), `${STORE}` (the pnpm store), `${RUN_TMP}`
  (hobble-created temp dir, set as `TMPDIR`), `${CACHE}` (hobble-controlled
  cache root). `~` means the user's home.
- **`allowedDomains` is recorded but not enforced yet** (see
  [network.md](network.md)).
- **Stale entries** (removed packages, old versions) are left in the file.
  They can never grant anything, since lookup is by exact installed
  version. A prune command comes later.
- The `hobble` field is a schema version so the layout can migrate cheaply.
