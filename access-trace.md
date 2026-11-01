# Access and disk-I/O traces

Register an async closure with `cache.with_trace(deliver_package)` before sharing the
cache. The callback receives uncompressed trace packages as `bytes::Bytes`.
Persist them unchanged. See [the file-writing example](feuer/examples/trace_to_disk.rs).
Read and replay FETR packages with [the replay tools](feuer-replay/README.md).

- Cloned handles share request IDs and a 262,144-event queue. Lookups and disk I/O
  wait when the queue is full. A background task batches events into packages and calls
  `deliver_package` serially when a package reaches 8 MiB (at most one event over),
  or when all trace owners are dropped. Packages have no delivery timer.
  Trace buffers are outside cache capacity. Slow delivery delays traced work.
- The delivery callback owns retries and error reporting. Returning an error stops
  delivery and causes later events to be ignored. Tracing preserves cache results.
- Dropping all trace owners triggers delivery of the remaining events and returns
  without waiting. Background disk work can retain the trace after cache handles drop.
  The application must wait for persistence before runtime shutdown.
  Queued events live in memory and can be lost on abrupt shutdown.
- Each lookup emits a request-start event and a lookup-outcome event. Cancellation
  can leave an unmatched start. Callers sharing a disk read or source fetch each
  emit their own outcome on completion. Source-fetch retries and coalescing happen
  within those lookups and have no separate events. Events contain key hashes and
  payload lengths rather than payload bytes. Key hashes can still identify objects,
  so treat traces as sensitive.
- Tracing always includes disk reads and writes, including metadata, from the cache's
  backing file. Enable it once, before cloning the cache. Opening and recovery happen
  before tracing is enabled. Each event records a `DataFile` operation's start, rather
  than individual kernel submissions. Failed or cancelled operations can have starts.
  Completion times and outcomes are outside this format.

## FETR v2

Each package starts with eight bytes `FETR\x02\x00\x00\x00`, followed by one or more
**41-byte events**. Integers are little-endian, without padding. Persist one package per file.
Access events retain this layout:

| Event offset | Type | Field |
| --- | --- | --- |
| 0 | `u8` | Kind: 0 start, 1 memory hit, 2 disk/buffered hit, 3 callback success, 4 callback error, 5 lookup-only miss |
| 1 | `u64` | Request ID, starts at 1, local to this trace |
| 9 | `u64` | Elapsed nanoseconds since trace creation |
| 17 | `u128` | Object key hash, seed-zero XXH3-128 |
| 33 | `u64` | Object payload length for successful kinds 1–3, otherwise zero |

Successful lookups of empty objects have length zero. A `get` returning
`None` emits kind 5. Access replay learns object lengths from successful outcomes and
rejects objects whose length is never observed. It ignores disk events.

Disk events use the same event size and clock:

| Event offset | Type | Field |
| --- | --- | --- |
| 0 | `u8` | Kind: 6 read start, 7 write start |
| 1 | `u64` | Unused, zero |
| 9 | `u64` | Elapsed nanoseconds since trace creation |
| 17 | `u64` | Byte offset in the cache file |
| 25 | `u64` | Unused, zero |
| 33 | `u64` | Requested read length or padded write length |

Read offsets and lengths exclude alignment padding. Writes describe the padded disk
byte range. Events contain no payload bytes, file IDs, or I/O IDs.

Timestamps are captured before enqueue, so queue order can differ from timestamp
order. Join access start/outcome by request ID. Python layout: `<B5Q`, with the hash split
into low/high halves. Check the package header, event size, and kinds. Missing events
or packages can leave the remaining packages parseable.
