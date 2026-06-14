# Creance

Creance is a macOS proof of concept for observing and enforcing pnpm lifecycle
script behavior. It records filesystem accesses and CONNECT proxy domains during
observe mode, stores a reviewed profile under `.creance/profiles`, then runs the
same package script under a deny-by-default macOS Seatbelt sandbox in strict
install mode.

## Build and Test

```sh
mise run fmt
mise run lint
mise run test
```

The ignored networked fixture test uses `e2e/fixtures/aspect-c` and fetches
`@aspect-test/c@2.0.0` through pnpm:

```sh
mise run e2e
```

## pnpm Flow

Observe a project:

```sh
creance observe
```

Review and commit the generated profile diff under `.creance/profiles`, then run
strict enforcement:

```sh
creance install --strict
```

For the committed Aspect fixture profile:

```sh
cd e2e/fixtures/aspect-c
creance install --strict --force --store-dir .pnpm-store
```

The fixture profile was generated from a real observe run and allows the package
postinstall to write only its observed `data.json` side effect.
