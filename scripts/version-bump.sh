#!/usr/bin/env bash
# Usage: scripts/version-bump.sh <vX.Y.Z-oi-ioN>
# Example: scripts/version-bump.sh v1.0.9-oi-io3
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."

raw="${1:?Usage: scripts/version-bump.sh <vX.Y.Z-oi-ioN> (e.g. v1.0.9-oi-io3)}"
# Strip the optional leading "v"; package.json/Cargo expect a bare semver.
version="${raw#v}"

if ! [[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+-oi-io[0-9]+$ ]]; then
  echo "Invalid version '${raw}': expected vX.Y.Z-oi-ioN (e.g. v1.0.9-oi-io3)" >&2
  exit 1
fi

npm run version:set -- "$version"

# Prepend a matching release entry to the flatpak metainfo (appstream
# expects releases in descending order); refresh the date if it exists.
metainfo="flatpak/com.edgeterm.desktop.metainfo.xml"
date="$(date +%F)"
if grep -q "<release version=\"$version\"" "$metainfo"; then
  sed -i "s|<release version=\"$version\" date=\"[0-9-]*\"/>|<release version=\"$version\" date=\"$date\"/>|" "$metainfo"
else
  sed -i "s|  <releases>|  <releases>\n    <release version=\"$version\" date=\"$date\"/>|" "$metainfo"
fi

git add package.json package-lock.json src-tauri/Cargo.toml src-tauri/Cargo.lock "$metainfo"
git commit -m "chore: bump version to ${version}"
git push
