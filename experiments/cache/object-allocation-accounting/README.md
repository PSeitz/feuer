# Whole-object slice allocation accounting

## Change

Checkout: `~/Development/feuer`, `main` at `eacaf12` plus working-tree edits.
The payload-length comparison reproduces the behavior of the published pin
`71224fcd317a0f3a4f9370fbe8dce75180144ca6`.

`get_or_fetch_object_with_allocation_charge` accepts a callback returning
`(Bytes, usize)`. It passes the supplied backing-capacity charge to `Download`
without copying. The ordinary whole-object callback still returns `Bytes` and
charges payload length.

The no-range Quickwit adapter passes `download.bytes_in_range(requested_range)`
and `download.allocation_charge()`. Exact-slice keys and coalescing are unchanged:

`../pomsky/release-feuer-9-no-range/quickwit/quickwit-storage/src/split_range_cache/storage.rs`

Entries sharing an allocation each charge its full capacity, so the total can
exceed the retained buffer bytes. Accounting does not release unused bytes inside
an allocation. Soft per-shard targets, including oversized-entry admission, are
unchanged. Caller-only results and pending disk writes remain outside the budget.

## Native Linux validation after review

Host: `m8g-32cpu-local-ssd-2`, aarch64, Rust 1.96.
Workspace: `/home/ubuntu/feuer-object-allocation-accounting.Sw1dGt`.

Settings:

- Disk and idle-buffer retention disabled.
- Slice comparison: 256 MiB target, enough for two 2 MiB charges in one shard.
- Eviction comparison: 2 MiB target, 65 separate 32 KiB allocations exposing 1 KiB each.
- Caller results and source downloads leave scope before measurement.
- Returned slice pointers checked against the source download.

Measured bytes:

| Read method and scenario | Entries | Charged | Retained buffer capacity |
| --- | ---: | ---: | ---: |
| Range, one 1 KiB slice | 1 | 32,768 | 32,768 |
| Object payload charge, one 1 KiB slice | 1 | 1,024 | 32,768 |
| Object allocation charge, one 1 KiB slice | 1 | 32,768 | 32,768 |
| Range, two slices of one download | 1 | 2,097,152 | 2,097,152 |
| Object payload charge, two slices of one download | 2 | 2,048 | 2,097,152 |
| Object allocation charge, two slices of one download | 2 | 4,194,304 | 2,097,152 |
| Object payload charge, eviction comparison | 65 | 66,560 | 2,129,920 |
| Object allocation charge, eviction comparison | 43 | 1,409,024 | 1,409,024 |

The allocation-charge case evicted 22 entries. Exact residency depends on
process-local shard assignment. The reproduction checks allocation accounting
and eviction, not a specific victim count.

Commands from the native workspace:

```sh
PATH="$HOME/.cargo/bin:$PATH" TMPDIR=/mnt/local-ssd \
  cargo +1.96 test --locked -p feuer -p feuer-memory -p feuer-historian -p feuer-types --lib
PATH="$HOME/.cargo/bin:$PATH" TMPDIR=/mnt/local-ssd \
  cargo +1.96 run --locked \
  --manifest-path experiments/cache/object-allocation-accounting/reproduction/Cargo.toml
```

All 107 native library tests and 91 local library tests passed after review.
The two new tests check per-entry charges, shared-buffer lifetime, zero-copy
memory hits, and capacity-driven eviction. Generic callback-error and inherited
assertion tests were removed because they exercise no new accounting behavior.
Tests decreased from 136 to 118 lines. The reproduction decreased from 207 to
172 lines by removing duplicate checks and an unsupported range-read branch
from the object-eviction comparison.

Quickwit's 33 split-cache tests passed with a local dependency override before
review. The adapter's executable code is unchanged by review. `make fmt` passed.
Clippy passed with warnings treated as errors for the Feuer library, its tests,
and the reproduction.
The published dependency pin and tags are unchanged. Shipping the adapter patch
requires the new Feuer revision or a local override.

## Final validation

The existing API and tests required no further source changes. Validated `main`
at `cce6ce5b0ce1d6341d384921d1eef49780918711` plus the working-tree changes in a
fresh source workspace on `m8g-32cpu-local-ssd-2`:
`/home/ubuntu/feuer-allocation-charge-finish.sMw6GO`, Rust 1.96.1.

- 108 Feuer, memory, historian, and types library tests passed.
- 130 storage library tests passed with `TMPDIR=/mnt/local-ssd`. The manual
  recovery benchmark remained ignored.
- 92 local library tests passed. Formatting and Linux Feuer library/test Clippy
  passed with warnings treated as errors.
- Both new regression tests failed when the snapshot discarded the supplied
  charge and used the original payload-length constructor. The shared-slice
  test observed 2,048 charged bytes instead of 4,194,304, and the eviction test
  retained all 65 entries. Both passed after restoring allocation charging.
- The reproduction matched the table above, including 22 evictions and
  1,409,024 charged and retained bytes in the allocation-charge case.

The regression mutation was confined to the disposable Linux snapshot and
restored afterward. Logs and source hashes are under `raw/finish-*`.

## Shipping together

Commit the allocation-aware API, its documentation, and regression tests in
Feuer. Then ship the existing Pomsky caller change in
`quickwit/quickwit-storage/src/split_range_cache/storage.rs` together with a
`quickwit/Cargo.lock` update to that immutable Feuer commit. Its current lock
still resolves Feuer to `cce6ce5b0ce1d6341d384921d1eef49780918711`, which lacks
the new API. The Git-sourced Feuer crates must resolve to the same fixed
revision. An API-only release does not repair callers still using payload-only
charging. No dependency pin, tag, deployment, or production setting was changed.

Raw logs and the resolved reproduction lock are local under `raw/`, ignored by Git.
