#!/bin/sh
set -eu

project_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$project_root"
if test -n "$(git status --porcelain --untracked-files=normal)"; then
    echo "Commit the release source before packaging." >&2
    exit 2
fi
compiler_version=$(rustc --version | cut -d ' ' -f 2)
if test ! -f "licenses/rust-standard-library-$compiler_version/COPYRIGHT-library.html"; then
    echo "Retain the Rust $compiler_version standard-library notices before packaging." >&2
    exit 2
fi
cargo build --locked --release -j 2
output=${1:-.local/dist}
mkdir -p "$output"
output=$(CDPATH= cd -- "$output" && pwd)
binary="$project_root/target/release/repo-doctor"
version=$(sed -n 's/^version = "\([^"]*\)"$/\1/p' Cargo.toml)
test "$("$binary" --version)" = "repo-doctor $version"
host=$(rustc -vV | sed -n 's/^host: //p')
case "$host" in
    *-apple-darwin|*-unknown-linux-gnu) ;;
    *) echo "Only native macOS and GNU/Linux archives are prepared." >&2; exit 2 ;;
esac
name="repo-doctor-$version-$host"
stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT HUP INT TERM
mkdir "$stage/$name"
cp "$binary" "$stage/$name/"
git archive HEAD LICENSE README.md CHANGELOG.md THIRD_PARTY_NOTICES.md licenses schemas examples > "$stage/source.tar"
tar -xf "$stage/source.tar" -C "$stage/$name"
{
    echo "Repo Doctor $version"
    echo "Target: $host"
    rustc --version
    echo "Source commit: $(git rev-parse HEAD)"
} > "$stage/$name/BUILD-INFO.txt"
COPYFILE_DISABLE=1 tar -czf "$output/$name.tar.gz" -C "$stage" "$name"
cd "$output"
if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$name.tar.gz" > "$name.tar.gz.sha256"
else
    shasum -a 256 "$name.tar.gz" > "$name.tar.gz.sha256"
fi
echo "$output/$name.tar.gz"
