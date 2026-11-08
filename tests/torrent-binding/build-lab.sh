#!/bin/sh
set -eu
mkdir -p /work/project/torrent /work/cargo
cp -a /input/torrent/src /input/torrent/Cargo.toml /input/torrent/Cargo.lock /input/torrent/README.md /work/project/torrent/
test -e /work/project/vendor || ln -s /input/vendor /work/project/vendor
cat > /work/source-config.toml <<'EOF'
[source.crates-io]
replace-with="approved-local"
[source.approved-local]
directory="/input/registry"
EOF
cd /work/project/torrent
CARGO_HOME=/work/cargo cargo test --offline --locked --features lab --config /work/source-config.toml --target-dir /work/target --no-run
CARGO_HOME=/work/cargo cargo build --offline --locked --features lab --config /work/source-config.toml --target-dir /work/target
