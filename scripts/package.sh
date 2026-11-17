#!/bin/bash

# Builds Debian and RPM packages for the sshx binaries (see issue #35).
#
# Packages are built for a single target at a time, so that statically
# linked musl binaries can be packaged for Linux. Requires `cargo-deb` and
# `cargo-generate-rpm`, and gzipped manual pages are generated in-place.
#
# Usage: ./scripts/package.sh [target]
#
# The target defaults to x86_64-unknown-linux-musl. Supported targets are
# x86_64-unknown-linux-musl and aarch64-unknown-linux-musl. Output files are
# written under target/debian/ and target/${target}/generate-rpm/.

set -euo pipefail

# The target we are building packages for.
target="${1:-x86_64-unknown-linux-musl}"

case "$target" in
  x86_64-unknown-linux-musl | aarch64-unknown-linux-musl) ;;
  *)
    echo "error: unsupported target: $target" >&2
    exit 1
    ;;
esac

# Compress the manual pages in the format expected by the packages.
for crate in sshx sshx-server; do
  man="crates/$crate/packaging/man"
  gzip -9 -c "$man/$crate.1" > "$man/$crate.1.gz"
done

# Build release binaries for the target. The binaries are built into
# target/$target/release/, and the packaging tools resolve the magic
# "target/release" prefix against this same directory.
export CARGO_TARGET_DIR="$PWD/target"
cargo build --release --target "$target" -p sshx -p sshx-server

# Build .deb packages, copying manual pages from the metadata in Cargo.toml.
(cd crates/sshx && cargo deb --target "$target" --no-build)
(cd crates/sshx-server && cargo deb --target "$target" --no-build)

# Build .rpm packages, same as above but for the RPM package manager.
(cd crates/sshx && cargo generate-rpm --target "$target" --auto-req auto)
(cd crates/sshx-server && cargo generate-rpm --target "$target" --auto-req auto)
