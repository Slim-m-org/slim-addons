#!/usr/bin/env bash
# Build a module's release wasm the one way this registry builds it, so the same
# source gives the same bytes on any machine and CI can compare them.
#
# Usage: scripts/build-wasm.sh <module-dir> <out.wasm>
#
# Needs the toolchain pinned in rust-toolchain.toml. Every absolute path rustc
# would bake into the wasm (cargo home, the toolchain, the module checkout) is
# remapped to a fixed one. The rust-src component, when installed, is mapped to
# the /rustc/<commit> paths a toolchain without it already uses; otherwise the
# bytes would depend on whether that component is present. The build is always
# `--locked` against the module's committed Cargo.lock. A module's own
# `.cargo/config.toml` rustflags are merged in, because RUSTFLAGS in the
# environment replaces them rather than adding to them.
set -euo pipefail

dir="${1:?usage: build-wasm.sh <module-dir> <out.wasm>}"
out="${2:?usage: build-wasm.sh <module-dir> <out.wasm>}"
dir=$(cd "$dir" && pwd)
out=$(realpath -m "$out")

cargo_home="${CARGO_HOME:-$HOME/.cargo}"
sysroot=$(rustc --print sysroot)
commit=$(rustc -vV | sed -n 's/^commit-hash: //p')

flags=(
  "--remap-path-prefix=$cargo_home=/cargo"
  "--remap-path-prefix=$sysroot=/rustc-sysroot"
  "--remap-path-prefix=$sysroot/lib/rustlib/src/rust=/rustc/$commit"
  "--remap-path-prefix=$dir=/module"
)
if [[ -f "$dir/.cargo/config.toml" ]]; then
  while IFS= read -r flag; do
    flags+=("$flag")
  done < <(python3 - "$dir/.cargo/config.toml" <<'PY'
import sys, tomllib
cfg = tomllib.load(open(sys.argv[1], "rb"))
for flag in cfg.get("target", {}).get("wasm32-unknown-unknown", {}).get("rustflags", []):
    print(flag)
PY
)
fi

target_dir=$(mktemp -d)
trap 'rm -rf "$target_dir"' EXIT

[[ -f "$dir/Cargo.lock" ]] || { echo "no Cargo.lock in $dir; run cargo generate-lockfile there and commit it, or the bytes depend on the day's crates" >&2; exit 1; }

encoded=$(printf '%s\x1f' "${flags[@]}")
(
  cd "$dir"
  CARGO_ENCODED_RUSTFLAGS="${encoded%$'\x1f'}" CARGO_TARGET_DIR="$target_dir" \
    cargo build --locked --release --target wasm32-unknown-unknown
)

wasm=$(find "$target_dir/wasm32-unknown-unknown/release" -maxdepth 1 -name '*.wasm' | head -1)
[[ -f "$wasm" ]] || { echo "no .wasm built for $dir" >&2; exit 1; }
mkdir -p "$(dirname "$out")"
cp "$wasm" "$out"
