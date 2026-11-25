#!/usr/bin/env bash
# Build EdgeTerm as a Flatpak bundle.
#
#   flatpak/build.sh            # produce dist/flatpak/EdgeTerm_<version>_<arch>.flatpak
#   flatpak/build.sh --install  # also install it for the current user
#
# The flow follows the reference flatpak packaging: generate the offline cargo
# sources, verify that flatpak-builder actually loaded them, download the
# sources once, then build with downloads disabled. The build itself does not
# fetch crates; cargo is pinned to the vendored directory by
# CARGO_NET_OFFLINE and the generated cargo/config.toml.
set -euo pipefail

root_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
manifest="$root_dir/flatpak/com.edgeterm.desktop.yml"
out_dir="$root_dir/dist/flatpak"
state_dir="$out_dir/state"
build_dir="$out_dir/build"
repo_dir="$out_dir/repo"
sources="$out_dir/cargo-sources.json"

arch="${FLATPAK_ARCH:-$(uname -m)}"
case "$arch" in
  x86_64 | aarch64) ;;
  *)
    printf 'Unsupported flatpak arch %s (x86_64|aarch64)\n' "$arch" >&2
    exit 2
    ;;
esac

if command -v flatpak-builder >/dev/null 2>&1; then
  builder=(flatpak-builder)
elif flatpak info --user org.flatpak.Builder//stable >/dev/null 2>&1 || flatpak info --system org.flatpak.Builder//stable >/dev/null 2>&1; then
  builder=(flatpak run --command=flatpak-builder org.flatpak.Builder//stable)
else
  printf '%s\n' 'flatpak-builder is required. Install it with your distribution package manager or: flatpak install --user flathub org.flatpak.Builder' >&2
  exit 1
fi

version="$(sed -n 's/^[[:space:]]*"version": "\([^"]*\)".*/\1/p' "$root_dir/package.json" | head -n 1)"
[ -n "$version" ] || { printf 'Could not read the version from package.json\n' >&2; exit 1; }

mkdir -p "$out_dir"

printf '[%s] 1/5 generating offline cargo sources\n' "$(date +%H:%M:%S)"
"$root_dir/flatpak/gen-cargo-sources.sh" >"$sources"

# flatpak-builder does not fail on an unreadable sources include: it prints a
# Json-WARNING, drops every source (including the module's own dir source) and
# later fails with a misleading "manifest path ... does not exist". Turn that
# warning into an early, actionable failure.
crate_sources="$("${builder[@]}" --show-manifest "$manifest" | grep -Ec '"dest" *: *"cargo/vendor/' || true)"
# Every vendored crate produces two entries: the .crate archive and its
# .cargo-checksum.json inline file. A missing include makes both disappear.
expected_sources=$(( $(grep -c '^checksum = ' "$root_dir/src-tauri/Cargo.lock") * 2 ))
if [ "$crate_sources" -lt "$expected_sources" ]; then
  printf 'PREFLIGHT FAILED: flatpak-builder sees %s vendored crate sources in %s (expected at least %s); the cargo-sources.json include did not load.\n' "$crate_sources" "$manifest" "$expected_sources" >&2
  exit 4
fi
printf '[%s] preflight: flatpak-builder sees %s vendored crate sources\n' "$(date +%H:%M:%S)" "$crate_sources"

printf '[%s] 2/5 downloading sources (network; the build phase does not fetch crates)\n' "$(date +%H:%M:%S)"
# --force-clean here as well: a warm re-run otherwise stops with "App dir is
# not empty" before the download phase can start. The build phase re-creates
# the app dir anyway.
"${builder[@]}" \
  --arch="$arch" \
  --force-clean \
  --download-only \
  --state-dir="$state_dir" \
  "$build_dir" \
  "$manifest"

printf '[%s] 3/5 building (offline cargo, arch %s)\n' "$(date +%H:%M:%S)" "$arch"
"${builder[@]}" \
  --arch="$arch" \
  --disable-download \
  --disable-updates \
  --force-clean \
  --disable-rofiles-fuse \
  --state-dir="$state_dir" \
  --repo="$repo_dir" \
  "$build_dir" \
  "$manifest"

bundle="$out_dir/EdgeTerm_${version}_${arch}.flatpak"
printf '[%s] 4/5 exporting bundle\n' "$(date +%H:%M:%S)"
# --runtime-repo: the bundle names flathub, so `flatpak install` can fetch
# org.gnome.Platform//50 itself.
flatpak build-bundle \
  --arch="$arch" \
  --runtime-repo=https://dl.flathub.org/repo/flathub.flatpakrepo \
  "$repo_dir" \
  "$bundle" \
  com.edgeterm.desktop \
  master
ls -l "$bundle"

if [ "${1:-}" = "--install" ]; then
  printf '[%s] installing %s for the current user\n' "$(date +%H:%M:%S)" "$bundle"
  flatpak install --user --noninteractive "$bundle"
fi

printf '[%s] 5/5 done: %s\n' "$(date +%H:%M:%S)" "$bundle"
