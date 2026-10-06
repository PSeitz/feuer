# Access traces

Register an async closure with `cache.with_trace(callback)` before sharing the
cache. The callback receives complete, uncompressed packages as `bytes::Bytes`;
persist them unchanged. See [the file receiver](feuer/examples/trace_to_disk.rs).

- Clones share IDs and a 65,536-event queue. A full queue waits, never drops.
  One worker delivers serially when the batch reaches 8 MiB (at most one event over),
  or when all handles are dropped. There is no timed delivery.
  Trace buffers are outside cache capacity; slow delivery delays lookups.
- The receiver owns retries and error reporting. Returning an error stops delivery;
  later events are ignored. Tracing never changes cache results.
- Dropping all handles drains the tail but doesn't wait. The receiver owns waiting
  for persistence before runtime shutdown; abrupt shutdown can lose the tail.
  There is no durable spool.
- Each lookup emits a start and one completed outcome; cancellation can leave an
  unmatched start. Callbacks may hide backend retries/coalescing; these aren't
  independently traced. Object keys are hashed, not anonymized; payloads aren't traced.

## FETR v1

Each package starts with eight bytes `FETR\x01\x00\x00\x00`, then nonempty **65-byte
events**. Integers are little-endian, without padding. Persist one package per file.

| Event offset | Type | Field |
| --- | --- | --- |
| 0 | `u8` | Kind: 0 start, 1 memory hit, 2 disk/buffered hit, 3 callback success, 4 callback error, 5 noncovering download |
| 1 | `u64` | Request ID, starts at 1; local to this trace |
| 9 | `u64` | Elapsed nanoseconds since trace creation |
| 17 | `u128` | Object key hash, seed-zero XXH3-128 |
| 33, 41 | `u64`, `u64` | Requested start, end |
| 49, 57 | `u64`, `u64` | Downloaded start, end; meaningful only for kinds 3 and 5, otherwise zero |

Ranges are half-open. Timestamps are captured before enqueue; queue order can differ
from timestamp order. Join start/outcome by request ID. Python layout: `<B8Q`, with
the hash split into low/high halves. Check magic, record size, kinds, and ranges;
a parseable package doesn't prove trace completeness.
