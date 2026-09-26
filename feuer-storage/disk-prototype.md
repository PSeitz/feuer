# Disk allocation and recovery prototype

Experimental work toward `DiskRangeCache`, not a selected production allocator. The public cache remains
memory-only. `tiered-plan.md` remains authoritative.

## First slice

`src/allocation.rs` is a test-only allocator candidate. It reserves aligned physical byte ranges, returns
freed space, and delays reuse while read guards exist. It does not implement range lookup, eviction policy,
population, checksums, or recovery. Tests model entry eviction by dropping its allocations. Keeping this
module test-only avoids introducing a production API before the storage format is tested.

One allocator owns one arena, with one metadata mutex and no I/O under that mutex. Independent arenas would
need independent allocators; this prototype is not a cache-wide concurrency design.

## Candidate physical layout

- The arena consists of 1-MiB units. The first 4 KiB of every unit is reserved for its embedded metadata
  index and is never returned by the payload allocator. Thus each unit has 255 allocatable 4-KiB blocks.
- Payload and any metadata that does not fit in the embedded index must both consume allocations. The first
  slice measures allocation overhead only, not the as-yet-unimplemented entry metadata footprint.
- Large entries use multiple units. Their final partial unit is allocated in 4-KiB blocks from subdivided
  units, shared with other entries. Physical pieces need not be adjacent. Object-relative order follows
  the returned list of regions; an entry owns all of its regions together.
- Small values can share one immutable, fully written 4-KiB block. Packing happens before its write, not by
  adding bytes to a published block. Entries refer to byte offsets in that shared block. Reclamation removes
  all entries in the block; no live-value relocation is implemented in the first slice. A single small value
  still costs a block when there is nothing with which to batch it.
- A read guard protects the complete physical region, including alignment padding and neighboring packed
  values. Dropping a cache entry does not make that region reusable until all guards and shared owners are
  gone. Returned `Bytes` must not retain these guards.
- Entirely free units are tracked as coalesced runs. Only units with some but not all payload blocks free
  need a 256-bit bitmap. Allocation can consume nonadjacent free blocks rather than requiring contiguous
  tails. A unit returns to the whole-unit free pool when its last payload block is released.

This deliberately pays one index page per unit: 0.390625% of capacity. At 40 TiB, reading just these pages
would require 160 GiB of recovery I/O, before reading overflow metadata. This is a significant cost to
measure against larger index spacing, not a claim of fast recovery. Free-space initialization is constant
size; live entry mappings and fragmented free-space metadata are not.

## Ownership and reuse ordering

1. Reserve all required storage before submitting writes. Failed allocation leaves the arena unchanged.
2. A task that owns the regions awaits submitted writes through completion. Canceling the requester does
   not abort that task. The allocator itself neither submits I/O nor detects cancellation.
3. On known write completion, revalidate publication under the future range index's lock. Publish only
   successful, current, still-admitted writes. Unsubmitted or known-completed abandoned reservations can
   be released. Unknown-completion regions are quarantined for the rest of this allocator's lifetime.
4. A lookup obtains its read guards while holding the same index lock that protects removal of the mapping,
   then releases the lock before I/O. Eviction first removes mappings; reuse waits for the last owner or
   reader. Multiple reads may hold guards concurrently.
5. Index-page updates must also have their own write ownership through completion. The allocator's
   permanently excluded index pages are not a license for overlapping metadata writes.

## Recovery direction — still to implement

Embedded index pages identify entry descriptors without scanning payload. Descriptors must contain complete
object keys, exact object ranges, ordered physical mappings, allocation identities, and per-block integrity
information. Variable-length keys and block checksum tables require charged overflow metadata, not an
unbounded fixed-size index record. All parsing must check file bounds and integer overflow before allocating
or reading from a recovered pointer.

Descriptors and index pages need versioned checksums that bind their physical address and allocation identity.
Payload validation must bind the expected object bytes to the recovered descriptor. A valid index checksum
alone never establishes payload integrity or durability. Subrange reads validate the touched blocks; a
checksum of the entire download is insufficient for bounded read overhead.

Runtime publication follows successful payload and metadata writes plus generation revalidation. Neither
completion order nor direct I/O establishes persistence order. Recovery must tolerate an old index pointing
at reused payload, new metadata with missing payload, torn descriptor chains, and inconsistent ownership.
Uncertain or conflicting mappings must be rejected before they can authorize allocation or reads. A persisted
publication record cannot be treated as proof of durable payload without an explicit ordering protocol.

Before implementing recovery, settle and test the precise record format, allocation-identity reuse rules,
index-page update protocol, and whether persistence barriers or conservative lazy validation are needed to
recover a useful safe subset. No existing allocator test establishes those properties. Unsupported-format
reset must likewise prevent old metadata from being accepted under the new format.

## Validation

Run on Linux with real io_uring and direct I/O; no buffered substitute:

```sh
TMPDIR=/mnt/local-ssd/<isolated-test-directory> cargo test --locked -p feuer-storage
```

The initial host is `m8g-32cpu-local-ssd`, ARM64 Linux, ext4 on `/mnt/local-ssd`. Pure allocation tests cover
small through 100-MiB reservations, fragmented tails, capacity exhaustion, failure rollback, shared-block
ownership, concurrent readers, quarantine, and sparse 40-TiB accounting. Real-file tests exercise packed
unaligned reads and reuse after read guards are released.

Remaining gates include the descriptor/recovery implementation, injected partial writes and crashes,
concurrent publication, trace-driven utilization/latency/metadata measurements, and comparison with
size-segregated slabs, append-packed cleaning, and a Foyer-style block baseline. Do not treat these first
allocation tests as allocator selection or end-to-end disk-cache acceptance.
