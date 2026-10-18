# Baseline read I/O queue metrics

Base: `4e7a7d12478c645d4d534ca6ef0fc33d87d279ca`.

Add instrumentation to the existing read I/O channel without payload-read admission
or bypass. See `metrics.md` for the two metrics and their scope. The gauge counts
pending channel reservations, not the channel's admitted requests or active I/O.
The histogram includes waits ending in admission, cancellation, or queue closure.

## Validation

Host: `m8g-32cpu-local-ssd-2`.
Workspace: `/home/ubuntu/feuer-read-queue-metrics.rnnaRi`.
Environment: `PATH="$HOME/.cargo/bin:$PATH"`, `TMPDIR=/mnt/local-ssd`,
`CARGO_TARGET_DIR="$HOME/feuer-simplify-target"`.

- `cargo test --locked -p feuer-storage --lib`: 143 passed, one failed, one ignored.
- The failure is `disk_cache::adversarial_tests::overflowing_fields_preserve_chunk_ownership`.
  Its generated regression seed reproduces on an unchanged archive of the base
  commit in `baseline/`. The failure is unrelated to these metrics.
- The storage suite with only that test excluded: 143 passed, one ignored.
- `cargo test --locked -p feuer --lib`: 30 passed.
- `cargo clippy --locked -p feuer-storage --all-targets --features io-bench -- -D warnings`: passed.
- Formatting and whitespace checks passed.

The new test covers immediately ready reservations, repeated pending polls, shared
registry aggregation across files, admission, cancellation, and queue closure.
Local logs are in ignored `raw/`. No benchmark or replay was run during validation.

## Benchmark follow-up

Update Pomsky to the pushed Feuer revision, build and deploy its image, then repeat
the last Yahoo benchmark (`sweep_20261018T160235Z_cold` settings). Record the exact
image and dependency revision, queue gauge time series, wait histogram deltas,
read throughput/latency, and query metrics through stages and inter-stage gaps.
This is a baseline measurement, not a bypass-policy comparison.
