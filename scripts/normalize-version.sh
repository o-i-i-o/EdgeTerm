#!/usr/bin/env bash
# Replace CJK/fullwidth full stops that a CJK input method can emit in place
# of "." when a version is edited by hand. Tauri derives every bundle name
# from the version, so a stray "。" would end up in the release assets.
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."

normalize() {
  local value="$1"
  value="${value//。/.}"
  value="${value//．/.}"
  printf '%s' "$value"
}

package_version="$(node -p "require('./package.json').version")"
cargo_version="$(awk -F '"' '/^\[package\]/{in_package=1; next} in_package && /^version = /{print $2; exit}' src-tauri/Cargo.toml)"

normalized_package="$(normalize "$package_version")"
normalized_cargo="$(normalize "$cargo_version")"

if [[ "$package_version" == "$normalized_package" && "$cargo_version" == "$normalized_cargo" ]]; then
  exit 0
fi

if [[ "$normalized_package" != "$normalized_cargo" ]]; then
  echo "Version mismatch after normalization: package.json=${normalized_package}, src-tauri/Cargo.toml=${normalized_cargo}" >&2
  echo "Run: npm run version:set -- <major.minor.patch>" >&2
  exit 1
fi

echo "Normalizing version separators: ${package_version} -> ${normalized_package}"
node scripts/set-version.mjs "$normalized_package"
