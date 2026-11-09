# Module template

A copyable starter for a slim module. It builds as-is and ships two commands:

- `echo` - text in, text out (also wired as `/echo`).
- `scene` - an interactive counter you bump with a control or reset by tapping, wired as a launchable **app**. It shows the whole scene loop (initial frame, then `{action, state}` on each press or tap) in a few lines.

See the deployment's `docs/modules/building-modules.md` for the full contract.

## Layout

```
manifest.json     what slim installs: id, version, permissions, extension points, the pinned artifact
src/lib.rs        the ABI shim (alloc/run/memory). Rarely needs changing.
src/command.rs    your logic. This is the part you write.
Cargo.lock        pins the dependencies the wasm is built from. Commit it.
```

## Make it yours

1. Copy this directory to `modules/<your-id>/` and, in `manifest.json`, set `id`, `name`, `summary`, `description`, `author`, and `homepage`. The `id` is a lowercase-hyphen slug and is also the `/id` alias for an app launch.
2. Set the crate `name` in `Cargo.toml` (cosmetic - it only names the built .wasm).
   Then run `cargo test` once so `Cargo.lock` follows the rename, and commit it.
   CI builds with `--locked`, and the pinned wasm only rebuilds to the same bytes from a committed lock.
3. Write your commands in `src/command.rs`: add an arm to `apply` per command, and declare each as a `command` extension point in the manifest. Add `slash-command`, `code-block-runner`, or `app` extension points to surface them.
4. Grant your commands the right `permission` (declared in `permissions`); nobody holds a module's permission implicitly, so an admin grants it to a role.

## Build and publish

From the repo root:

```bash
scripts/package-module.sh modules/<your-id>
```

That builds the release wasm, copies it to `modules/<your-id>/<version>/module.wasm`, and pins its SHA-256 into the manifest. Then:

1. Add (or update) your module's row in `index.json`.
2. Commit the manifest, the versioned `.wasm`, and the index.
3. Serve the registry over HTTPS and point a deployment's Dock at it.

Bump `version` (in both `manifest.json` and `index.json`) for every change, even a manifest-only one: a deployment records a module's extension points at install time, so a version bump is what offers the upgrade.

## Test

Your logic is plain Rust, testable without any wasm:

```bash
cd modules/<your-id> && cargo test
```

The template's own tests are in `src/command.rs`.
