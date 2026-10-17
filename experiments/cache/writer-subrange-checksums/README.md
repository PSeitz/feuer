# Checksums prepared from write buffers

## Change

Before publishing a disk entry, calculate checksums for recently requested parts that fit entirely inside its payload. Use the existing trusted write buffer, not another disk read. Deduplicate exact ranges and skip empty ranges and the whole-entry range, whose checksum already exists.

The change is in `feuer-storage/src/disk_cache/writer.rs`. It uses the existing history limits and checksum representation. Hashing happens outside the disk-index and metadata locks. Only the newly published entry receives its checksums. Discarded writes discard their checksums too.

This feature adds no payload copies, disk writes, disk-format changes, configuration, or metrics. Existing I/O counters verify its effect in tests. Checksums remain memory-only and disappear with their entry or on restart.

## Focused results

Tests use nonuniform bytes, high object offsets, unaligned ranges, packed entries at nonzero buffer offsets, and subranges crossing payload chunk boundaries. The first lookup requests 3,006 bytes:

| Payload length | First read with writer-prepared checksum | First read after restart, without the checksum |
| ---: | ---: | ---: |
| 98,321 bytes | 3,006 bytes | 98,321 bytes |
| 2,097,169 bytes | 3,006 bytes | 2,097,169 bytes |

These are logical DataFile payload bytes. Direct I/O still rounds transfers to 4-KiB alignment. They are focused test results, not workload savings.

The tests also verify no additional read/write bytes during checksum preparation, exact returned bytes, shortest checksum selection, duplicate and expired history, wrong-key and non-contained ranges, empty ranges, corruption detection, discarded publication, and late completions against replacement entries. Recovery does not recreate checksums from history alone, even when the same history object is supplied again.

## Validation

Linux SSD host: `m8g-32cpu-local-ssd-2`, with `PATH="$HOME/.cargo/bin:$PATH" TMPDIR=/mnt/local-ssd`.

A separate payload-version/header change was being edited concurrently. Full feature validation used an isolated patch against committed revision `6893e9bca3ddff8ae34fae2dcb8f6142a2089f75`, excluding that unrelated work. A second snapshot including the concurrent edits passed all 15 storage metric tests.

- Focused storage metric tests: **15 passed**.
- Full storage suite: **137 passed, 2 pre-existing failures, 1 ignored**.
- Final storage suite excluding those two failures: **137 passed, 1 ignored**.
- Tier tests: **30 passed**.
- Clippy with warnings denied: passed for storage and tier libraries/tests.
- Formatting and whitespace checks: passed.
- Independent static review: no correctness or minimality findings. Its cross-chunk test coverage suggestion was added and the filtered suite passed again.

The same two failures were reproduced against the unchanged committed baseline:

- `disk_cache::adversarial_tests::overflowing_fields_preserve_chunk_ownership`
- `disk_cache::adversarial_tests::payload_bytes_never_recover_as_metadata`

No replay or benchmark was run. Runtime savings remain unmeasured.

## Naming and test cleanup

Names now distinguish write-buffer offsets, per-entry checksum groups, ranges removed from request history, and checksum origin. Comments describe current checksum selection and publication rules. Duplicate read-learning, capacity, write-count, and range assertions were removed from the new tests without removing their existing coverage.

Linux validation after cleanup: 15 focused tests and 30 tier tests passed. The full storage suite had 137 passes, the same two baseline failures, and one ignored test. Clippy and formatting passed. The cleanup removes 6 net Rust lines. The isolated feature is 168 added and 15 removed Rust lines: 26 net implementation lines and 127 net test lines.

## Artifacts

`settings.json` records source identity and validation scope. Ignored `raw/` contains the isolated patch, source snapshots, preparation script, Linux logs, and review artifacts. The preparation script extracts only this feature's writer and test changes, avoiding concurrent edits in shared files.
