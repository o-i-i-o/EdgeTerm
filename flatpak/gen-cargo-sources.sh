#!/bin/sh
# Generate a flatpak-builder `sources` array that vendors every crates.io
# dependency in src-tauri/Cargo.lock, so the Rust build needs NO network.
#
# This mirrors the reference flatpak packaging flow: the build sandbox gets no
# network for cargo, the crates are downloaded once in the manifest's download
# phase, and `cargo --offline` only ever reads the vendored directory.
#
# The local `[patch.crates-io] russh = { path = "patches/russh" }` is not a
# crates.io source, so it has no `source` line in Cargo.lock and is not
# vendored here; it travels with the source tree and cargo uses it directly.
# Any real git/registry replacement is refused: it would need a clone this
# script does not do, and a half-populated vendor dir fails deep inside the
# sandbox with a misleading error.
#
# usage: flatpak/gen-cargo-sources.sh [BUILDDIR] > cargo-sources.json
#   BUILDDIR: the module's build directory inside the sandbox; CARGO_HOME is
#   BUILDDIR/cargo and the vendor dir BUILDDIR/cargo/vendor. Default
#   /run/build/edgeterm (flatpak-builder's dir for the "edgeterm" module).
set -eu

cd "$(dirname "$0")/.."

LOCK=src-tauri/Cargo.lock
BUILDDIR=${1:-/run/build/edgeterm}
# The project already builds against the USTC mirror (it is reachable from both
# the maintainer's host and GitHub runners); the sha256 in Cargo.lock is what
# makes the download trustworthy. Override with the upstream CDN if needed:
#   CRATES_BASE=https://static.crates.io/crates flatpak/gen-cargo-sources.sh
CRATES=${CRATES_BASE:-https://mirrors.ustc.edu.cn/crates.io/crates}

[ -f "$LOCK" ] || { echo "no $LOCK" >&2; exit 2; }

bad=$(grep '^source = ' "$LOCK" | grep -v '^source = "registry+https://github.com/rust-lang/crates.io-index"$' || true)
if [ -n "$bad" ]; then
  echo "REFUSED: non-crates.io source in $LOCK:" >&2
  echo "$bad" >&2
  echo "see the header of $0 — add git support deliberately, do not paper over it" >&2
  exit 3
fi

awk -v crates="$CRATES" -v builddir="$BUILDDIR" '
function flush() {
  if (name != "" && cksum != "") {
    printf "%s", sep; sep = ",\n"
    printf "  {\n"
    printf "    \"type\": \"archive\",\n"
    printf "    \"archive-type\": \"tar-gzip\",\n"
    printf "    \"url\": \"%s/%s/%s-%s.crate\",\n", crates, name, name, ver
    printf "    \"sha256\": \"%s\",\n", cksum
    printf "    \"dest\": \"cargo/vendor/%s-%s\"\n", name, ver
    printf "  },\n"
    # cargo refuses a vendored crate without its checksum file; "files": {} is
    # the documented way to say "do not re-verify the unpacked tree", the
    # package hash is what cargo checks.
    printf "  {\n"
    printf "    \"type\": \"inline\",\n"
    printf "    \"contents\": \"{\\\"package\\\": \\\"%s\\\", \\\"files\\\": {}}\",\n", cksum
    printf "    \"dest\": \"cargo/vendor/%s-%s\",\n", name, ver
    printf "    \"dest-filename\": \".cargo-checksum.json\"\n"
    printf "  }"
    n++
  }
  name = ""; ver = ""; cksum = ""
}
BEGIN { print "["; sep = "" }
/^\[\[package\]\]/ { flush(); next }
/^name = / { gsub(/^name = "|"$/, ""); name = $0; next }
/^version = / { gsub(/^version = "|"$/, ""); ver = $0; next }
/^checksum = / { gsub(/^checksum = "|"$/, ""); cksum = $0; next }
END {
  flush()
  printf "%s", sep
  printf "  {\n"
  printf "    \"type\": \"inline\",\n"
  printf "    \"contents\": \"[source.crates-io]\\nreplace-with = \\\"vendored-sources\\\"\\n\\n[source.vendored-sources]\\ndirectory = \\\"%s/cargo/vendor\\\"\\n\",\n", builddir
  printf "    \"dest\": \"cargo\",\n"
  printf "    \"dest-filename\": \"config.toml\"\n"
  printf "  }\n"
  print "]"
  printf "gen-cargo-sources: %d crates vendored from %s\n", n, crates > "/dev/stderr"
}
' "$LOCK"
