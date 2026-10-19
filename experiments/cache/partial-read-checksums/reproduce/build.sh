#!/usr/bin/env bash
set -euo pipefail
root=/home/ubuntu/feuer-partial-checksums-replay
cd "$root"
export PATH="$HOME/.cargo/bin:$PATH" TMPDIR=/mnt/local-ssd
for revision in baseline candidate; do
    export CARGO_TARGET_DIR="$root/targets/$revision"
    cd "$root/$revision"
    cargo test --locked -p feuer-replay > "$root/$revision-tests.log" 2>&1
    cargo clippy --locked -p feuer-replay --all-targets -- -D warnings > "$root/$revision-clippy.log" 2>&1
    cargo build --locked --release -p feuer-replay --bin replay > "$root/$revision-build.log" 2>&1
    cp "$CARGO_TARGET_DIR/release/replay" "$root/bin/$revision"
done
cd "$root"
sha256sum bin/baseline bin/candidate > binary-sha256.txt
printf '0\n' > build-exit.txt
