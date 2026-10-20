# Full-object removal validation

Removed range-aware cache APIs, indexes, compaction, request history, and partial
payload checksums. Both tiers now cache complete objects by key. Disk format v15
uses 40-byte metadata records and clears older formats when opened. FETR v2
records complete-object lookups. The replay reader still accepts historical v1
captures, converting each exact recorded range into an independent object.

No capture benchmarks were run. Historical benchmark results were not changed.

## Validation

On `m8g-32cpu-local-ssd-2`, with temporary cache files on `/mnt/local-ssd`:

```sh
PATH="$HOME/.cargo/bin:$PATH" TMPDIR=/mnt/local-ssd \
  cargo test --locked --workspace -- --skip overflowing_fields_preserve_chunk_ownership
PATH="$HOME/.cargo/bin:$PATH" \
  cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
```

- 249 tests passed, including 135 storage tests and the writer/replay integration tests.
- Two benchmark tests remained ignored.
- Clippy passed with warnings denied.
- macOS tests passed for the workspace excluding the Linux-only storage crate.
- Formatting and whitespace checks passed.

The unfiltered storage suite exposed `overflowing_fields_preserve_chunk_ownership`:
it generates checksummed metadata containing overflowing lengths or addresses,
and recovery panics during arithmetic. Running that same test against the
unchanged baseline `44b79fce4005ced1e66cbc08607500ba1f11ebc8` also failed. This pre-existing issue was left unchanged and
excluded from the final combined validation, not counted as passing.

## Follow-up: remove overflow-input test

At the user's request, removed `overflowing_fields_preserve_chunk_ownership` and
its overflow-only input generators. Production recovery code is unchanged.
The unfiltered storage suite then passed on the same Linux SSD host:
**135 passed, 1 benchmark ignored, 0 filtered out**.

Commands, test logs, and the isolated Linux checkout location are stored locally
under ignored `raw/`.
