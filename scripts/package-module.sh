#!/usr/bin/env bash
# Build a module to wasm, place it at its versioned artifact path, and pin its
# SHA-256 into the manifest - the three steps slim's registry layout needs.
# Needs the toolchain pinned in rust-toolchain.toml.
#
# Usage: scripts/package-module.sh <module-dir>
#   e.g. scripts/package-module.sh modules/_template
#
# Reads `id` and `version` from the module's manifest.json, builds the release
# wasm, copies it to modules/<id>/<version>/module.wasm, and rewrites the
# manifest's artifact.path + artifact.sha256 to match. Then add/update the
# module's row in index.json and commit.
set -euo pipefail

dir="${1:?usage: package-module.sh <module-dir>}"
manifest="$dir/manifest.json"
[[ -f "$manifest" ]] || { echo "no manifest at $manifest" >&2; exit 1; }

id=$(python3 -c "import json,sys;print(json.load(open('$manifest'))['id'])")
version=$(python3 -c "import json,sys;print(json.load(open('$manifest'))['version'])")
# The same plain-segment rule check-catalogue.py holds the manifest to, so no path leaves modules/<id>/.
[[ "$id" =~ ^[A-Za-z0-9_@+-][A-Za-z0-9._@+-]*$ && "$id" != "." && "$id" != ".." ]] || { echo "id '$id' is not a plain path segment" >&2; exit 1; }
[[ "$version" =~ ^[0-9A-Za-z.+-]+$ && "$version" != "." && "$version" != ".." ]] || { echo "version '$version' is not a plain path segment" >&2; exit 1; }
echo "packaging $id v$version"

# Build with the registry's one recipe, straight to the versioned artifact path
# the manifest will point at. CI rebuilds it the same way and compares.
artifact_path="modules/$id/$version/module.wasm"
"$(dirname "$0")/build-wasm.sh" "$dir" "$artifact_path"
sha=$(sha256sum "$artifact_path" | cut -d' ' -f1)
echo "sha256: $sha"

# Pin path + sha into the manifest.
python3 - "$manifest" "$artifact_path" "$sha" <<'PY'
import json, sys
manifest, path, sha = sys.argv[1], sys.argv[2], sys.argv[3]
m = json.load(open(manifest))
m["artifact"]["path"] = path
m["artifact"]["sha256"] = sha
json.dump(m, open(manifest, "w"), indent=2)
open(manifest, "a").write("\n")
PY

echo "done. Now add/update $id in index.json and commit:"
echo "  $artifact_path"
echo "  $manifest"
