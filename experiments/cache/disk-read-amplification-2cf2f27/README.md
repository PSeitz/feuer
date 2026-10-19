# Covering disk entries amplify exact requests

## Scope

Investigation of Quickwit tags `cp-rc-0.1.35-feuer-17` and
`cp-rc-0.1.35-feuer-17-no-range`. Both pin Feuer
`2cf2f27f4515e1069dfb2ec8e613bfb68155f2c1`.

This is a deterministic disk-cache policy reproduction, not a timed workload
benchmark. It demonstrates amplification but does not establish how much of the
production difference this mechanism accounts for. It describes the pinned
range-aware implementation, before range support was removed in `d965556`.

## Reproduction

Test source: `read_amplification_test.rs`. Run against an isolated archive of the
pinned commit, adding `#[cfg(test)] mod read_amplification_test;` to
`feuer-storage/src/disk_cache.rs` and placing the test in its module directory.
No production source changes are needed.

1. Insert two exact 16-KiB slices inside the same object.
2. Insert an exact 4-MiB requested range covering both slices.
3. Read both small slices sequentially from disk.
4. Compare range keys with separate whole-object keys for each exact slice.

No expanded downloads occur. Both modes have the same requested ranges.
Range mode deletes the small entries on insertion of the broad entry and refuses
subsequent insertion of those small ranges. Exact-key mode retains all three.

Linux host: `m8g-32cpu-local-ssd-2`.
Workspace: `/home/ubuntu/feuer-range-read-repro-2cf2f27`.
Temporary disk files: `/mnt/local-ssd`, automatically removed by the test.

```sh
PATH="$HOME/.cargo/bin:$PATH" TMPDIR=/mnt/local-ssd \
CARGO_TARGET_DIR=$HOME/feuer-simplify-target \
cargo test --locked -p feuer-storage --lib \
  exact_requests_still_amplify_covering_disk_reads -- --nocapture
```

The retained run produced the following measurements. Its output is in ignored
`raw/test.log`.

| Mode | Returned bytes | Data-file read bytes |
| --- | ---: | ---: |
| Range keys | 32,768 | 8,388,608 |
| Exact whole-object keys | 32,768 | 32,768 |

Read bytes are deltas of `feuer_disk_io_bytes_total{operation="read"}` after
setup, not process/device totals. Requests and payload addresses in this test
are aligned. The observed read amplification is 256x.

## Source evidence at the pinned revision

- `feuer/src/cache.rs:370`: expanded callback downloads are not directly queued.
- `feuer-storage/src/disk_cache.rs:456-468`: inserting a covering disk entry
  removes contained entries.
- `feuer-storage/src/disk_cache/writer.rs:64`: covered writes are skipped.
- `feuer-storage/src/disk_cache.rs:548-552`: checksum verification reads the whole
  stored payload, regardless of requested slice length.
- `feuer-storage/src/disk_cache.rs:333-338`: slicing/shrinking happens after that read.
- `feuer/src/cache.rs:334-342`: only the requested slice is admitted to RAM.

Existing test `serves_100_mib_entry_subranges_only_after_checking_the_whole_entry`
also explicitly enforces whole-entry checking for a tiny request.

Potential fixes must preserve integrity checking. Keeping independently readable
exact slices requires allowing contained entries and changing lookup selection.
Alternatively, partial reads require independently verifiable payload byte ranges
and an associated checksum-format change. Simply shortening the read is not valid.
