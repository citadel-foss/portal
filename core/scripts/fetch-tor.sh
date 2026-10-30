#!/bin/sh
# Fetches the Tor Project's official Tor expert bundle for one Rust target and keeps only what
# Portal embeds: the `tor` executable, the libraries it loads from its own directory, and its
# license. Run by core/build.rs when the bundle for the target being built is missing.
#
#   fetch-tor.sh <rust-target-triple> <output-dir>
#
# To move to a newer Tor: bump TOR_BROWSER_VERSION and the four hashes, taken from that
# release's sha256sums-signed-build.txt (verify its .asc against the Tor Browser signing key,
# EF6E286DDA85EA2A4BA7DE684E2C6E8793298290, first).

set -eu

TOR_BROWSER_VERSION=16.0

target=${1:?usage: fetch-tor.sh <rust-target-triple> <output-dir>}
out=${2:?usage: fetch-tor.sh <rust-target-triple> <output-dir>}

case "$target" in
  aarch64-apple-darwin)
    platform=macos-aarch64
    sha256=b8da251eccbe9e82f3681ee3af3c97fb4291c4a4e153e5b824bde48a1117cd6a
    files="tor libevent-2.1.7.dylib" ;;
  x86_64-apple-darwin)
    platform=macos-x86_64
    sha256=5b1860215c26f2d408392d1d79e798a2f81d65eb939510bb94c11f237bd0bfc3
    files="tor libevent-2.1.7.dylib" ;;
  x86_64-unknown-linux-gnu)
    platform=linux-x86_64
    sha256=cc950ebe9acb80188325c3a2cd8817def7dd7237d6fdcd861bb87b234a0cb2fd
    files="tor libevent-2.1.so.7 libssl.so.3 libcrypto.so.3" ;;
  aarch64-unknown-linux-gnu)
    platform=linux-aarch64
    sha256=bed98744e093a0dc3a22be0eecfdce7a33cb36866d3f15fcf011a35bf3513d0d
    files="tor libevent-2.1.so.7 libssl.so.3 libcrypto.so.3" ;;
  *)
    echo "fetch-tor.sh: no Tor expert bundle for target $target" >&2
    exit 1 ;;
esac

archive=tor-expert-bundle-$platform-$TOR_BROWSER_VERSION.tar.gz
url=https://dist.torproject.org/torbrowser/$TOR_BROWSER_VERSION/$archive

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

echo "fetch-tor.sh: downloading $url" >&2
curl --fail --silent --show-error --location --proto '=https' --tlsv1.2 \
  --output "$work/$archive" "$url"

if command -v sha256sum >/dev/null 2>&1; then
  actual=$(sha256sum "$work/$archive" | cut -d ' ' -f 1)
else
  actual=$(shasum -a 256 "$work/$archive" | cut -d ' ' -f 1)
fi
if [ "$actual" != "$sha256" ]; then
  echo "fetch-tor.sh: checksum mismatch for $archive" >&2
  echo "  expected $sha256" >&2
  echo "  got      $actual" >&2
  exit 1
fi

mkdir -p "$work/x"
tar -xzf "$work/$archive" -C "$work/x"

# Assembled next to the destination and renamed into place, so an interrupted run never leaves
# a half-written bundle that a later build would embed.
staging="$out.partial"
rm -rf "$staging"
mkdir -p "$staging"
for file in $files; do
  cp "$work/x/tor/$file" "$staging/$file"
done
cp "$work/x/docs/tor.txt" "$staging/LICENSE-tor.txt"

# The macOS `tor` ships with a signature its own post-link edits invalidated, and Apple Silicon
# kills a binary whose signature does not verify. An ad-hoc signature is all it needs to run.
case "$target" in
  *-apple-darwin)
    for file in $files; do
      codesign --force --sign - "$staging/$file"
    done ;;
esac

rm -rf "$out"
mv "$staging" "$out"
echo "fetch-tor.sh: Tor $TOR_BROWSER_VERSION bundle for $target is in $out" >&2
