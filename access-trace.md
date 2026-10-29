# Access traces

Register an async closure with `cache.with_trace(deliver_package)` before sharing the
cache. The callback receives uncompressed access-trace packages as `bytes::Bytes`.
Persist them unchanged. See [the file-writing example](feuer/examples/trace_to_disk.rs).
Read and replay FETR packages with [the replay tools](feuer-replay/README.md).

- Cloned handles share request IDs and a 65,536-event queue. Lookups wait when the
  queue is full. A background task batches events into packages and calls
  `deliver_package` serially when a package reaches 8 MiB (at most one event over),
  or when all handles are dropped. Packages have no delivery timer.
  Access-trace buffers are outside cache capacity. Slow delivery delays lookups.
- The delivery callback owns retries and error reporting. Returning an error stops
  delivery and causes later events to be ignored. Tracing preserves cache results.
- Dropping all handles triggers delivery of the remaining events and returns
  without waiting. The application must wait for persistence before runtime shutdown.
  Queued events live in memory and can be lost on abrupt shutdown.
- Each lookup emits a request-start event and a lookup-outcome event. Cancellation
  can leave an unmatched start. Callers sharing a disk read or source fetch each
  emit their own outcome on completion. Source-fetch retries and coalescing happen
  within those lookups and have no separate events. Object keys are hashed. Hashing
  does not guarantee anonymity. Events contain payload lengths rather than payload bytes.

## FETR v1

Each package starts with eight bytes `FETR\x01\x00\x00\x00`, followed by one or more
**41-byte events**. Integers are little-endian, without padding. Persist one package per file.
Every event refers to an object.

| Event offset | Type | Field |
| --- | --- | --- |
| 0 | `u8` | Kind: 0 start, 1 memory hit, 2 disk/buffered hit, 3 callback success, 4 callback error, 5 lookup-only miss |
| 1 | `u64` | Request ID, starts at 1, local to this trace |
| 9 | `u64` | Elapsed nanoseconds since trace creation |
| 17 | `u128` | Object key hash, seed-zero XXH3-128 |
| 33 | `u64` | Object payload length for successful kinds 1–3, otherwise zero |

Successful lookups of empty objects have length zero. A `get_object` returning
`None` emits kind 5. Replay learns object
lengths from successful outcomes and rejects objects whose length is never observed.

Timestamps are captured before enqueue, so queue order can differ from timestamp
order. Join start/outcome by request ID. Python layout: `<B5Q`, with the hash split
into low/high halves. Check the package header, event size, and kinds. Missing events
or packages can leave the remaining packages parseable.
