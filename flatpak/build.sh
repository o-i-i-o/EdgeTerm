#!/usr/bin/env bash
set -euo pipefail

root_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
manifest="$root_dir/flatpak/com.edgeterm.desktop.yml"
build_dir="$root_dir/flatpak-build"

if command -v flatpak-builder >/dev/null 2>&1; then
  builder=(flatpak-builder)
elif flatpak info --user org.flatpak.Builder//stable >/dev/null 2>&1 || flatpak info --system org.flatpak.Builder//stable >/dev/null 2>&1; then
  builder=(flatpak run --command=flatpak-builder org.flatpak.Builder//stable)
else
  printf '%s\n' 'flatpak-builder is required. Install it with your distribution package manager or: flatpak install --user flathub org.flatpak.Builder' >&2
  exit 1
fi

app_id="$(sed -n 's/^app-id:[[:space:]]*//p' "$manifest")"
ref="app/${app_id}/x86_64/master"

cd "$root_dir"
# Build and stage. No --install here: flatpak-builder's installer writes the
# system repo directly and fails with "open(O_TMPFILE): Permission denied"
# on hosts where the invoking user cannot write /var/lib/flatpak.
"${builder[@]}" \
  --force-clean \
  --disable-rofiles-fuse \
  --system \
  --install-deps-from=flathub \
  "$build_dir" \
  "$manifest"

# Deploy the staged build via a temporary local repo.
export_repo="$(mktemp -d)"
trap 'rm -rf "$export_repo"' EXIT

flatpak build-export "$export_repo" "$build_dir"
flatpak build-update-repo "$export_repo"

if sudo -n true >/dev/null 2>&1; then
  # Passwordless sudo: deploy into the system installation directly.
  sudo flatpak uninstall --system --noninteractive "$app_id" >/dev/null 2>&1 || true
  sudo flatpak install --system --noninteractive "$export_repo" "$app_id"
elif flatpak install --system --noninteractive "$export_repo" "$app_id"; then
  :
else
  printf '%s\n' 'System installation unavailable; installing for the current user instead.' >&2
  flatpak uninstall --user --noninteractive "$app_id" >/dev/null 2>&1 || true
  flatpak install --user --noninteractive "$export_repo" "$app_id"
fi
