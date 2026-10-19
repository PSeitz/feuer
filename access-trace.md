# Access traces

Register an async closure with `cache.with_trace(callback)` before sharing the
cache. The callback receives complete, uncompressed packages as `bytes::Bytes`;
persist them unchanged. See [the file receiver](feuer/examples/trace_to_disk.rs).
Read and replay FETR packages with [the replay tools](feuer-replay/README.md).

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
  unmatched start. Shared disk and callback results emit an outcome per completed
  caller, not per execution. Callbacks may also hide backend retries/coalescing,
  which aren't independently traced. Object keys are hashed, not anonymized;
  payloads aren't traced.

## FETR v2

Each package starts with eight bytes `FETR\x02\x00\x00\x00`, then nonempty **41-byte
events**. Integers are little-endian, without padding. Persist one package per file.
Every event refers to a complete object.

| Event offset | Type | Field |
| --- | --- | --- |
| 0 | `u8` | Kind: 0 start, 1 memory hit, 2 disk/buffered hit, 3 callback success, 4 callback error, 5 lookup-only miss |
| 1 | `u64` | Request ID, starts at 1, local to this trace |
| 9 | `u64` | Elapsed nanoseconds since trace creation |
| 17 | `u128` | Object key hash, seed-zero XXH3-128 |
| 33 | `u64` | Object payload length for successful kinds 1–3, otherwise zero |

Empty objects are valid, including successful lookups with length zero. A lookup-only
`get_object` returning `None` emits kind 5, not a callback error. Replay learns object
lengths from successful outcomes and rejects objects whose length is never observed.

Timestamps are captured before enqueue, so queue order can differ from timestamp
order. Join start/outcome by request ID. Python layout: `<B5Q`, with the hash split
into low/high halves. Check magic, record size, and kinds. A parseable package does
not prove trace completeness.

The replay reader also accepts historical FETR v1 captures. Their 65-byte records
use a different layout and kind numbering. They must be decoded by version, not
interpreted as v2 records. See [replay tools](feuer-replay/README.md) for legacy
input conversion.
