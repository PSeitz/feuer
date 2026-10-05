# Buffer stealing

## Goal

Allow idle read buffers to use the memory capacity not occupied by cached entries,
without leaving that capacity trapped in buckets that no longer match requests.
Use the existing `FEUER_IDLE_BUFFER_POOL_PERCENT=100` setting to evaluate this;
keep the default at 7% until measurements justify changing it.

## Decision

Steal idle capacity, not allocation ownership: on a bucket miss, free idle buffers
from other buckets before allocating the requested bucket size. Start without
allocator resizing, splitting allocations, or serving requests with oversized buffers.

Idle read-buffer contents do not need preservation. A resize that moves the
allocation could copy bytes that the next read will overwrite. Free-and-allocate
is the simpler baseline; the allocator may reuse the released memory, but this
is not guaranteed.

## Existing contract

Implementation: `feuer-memory/src/buffer.rs`.

- Each allocation has a stored `Layout`: allocation size and 4096-byte alignment.
- `length` is the exposed byte count; `layout.size()` is the backing capacity.
- Allocation and deallocation use the stored layout. Changing the layout alone
  does not resize an allocation and would make deallocation incorrect.
- Idle buffers are exclusively owned by the pool. Checked-out buffers return
  only after their last owner releases them.
- Cached allocation charges and idle buffer capacity share the cache budget.
  Active reads and caller-only results are outside that budget.
- Cached entries take precedence. Admission frees idle buffers as needed;
  returning buffers never evicts cached entries.
- Requests above 64 MiB and write scratch buffers remain unpooled.

## Allocation policy

For a request that fits a pool bucket:

1. Determine the smallest fitting bucket using the existing size table.
2. Under the existing pool mutex, try to pop an idle buffer from that bucket.
   On a hit, preserve the current reuse path; do not free other buffers.
3. On a miss, remove idle buffers from other buckets, largest first, until their
   combined capacity is at least the requested bucket capacity, or no idle
   buffers remain. Use the same largest-first policy as cache admission.
4. Decrease idle byte accounting and the corresponding metrics for every removed
   buffer, and drop it before allocating the new buffer. Keep the existing
   locking approach; do not add another lock or reclamation queue.
5. Release the pool mutex and allocate the requested bucket capacity through
   `AlignedBuffer::allocate_zeroed`.
6. Set the exposed length and attach the pool and used-buffer metrics as today.
   On release, retain the buffer in its own size bucket if the existing budget
   permits it.

Free whole allocations. For example, a 1-MiB miss may release one 4-MiB idle
buffer; a 4-MiB miss may release several smaller idle buffers. Reclamation can
therefore exceed the requested capacity. Do not introduce per-bucket reservations
or another selection policy in this first implementation.

If allocation fails, propagate the existing error. The reclaimed buffers remain
freed; there is no rollback.

For unpooled requests, leave the current allocation path unchanged.

## Memory behavior

At a 100% idle ceiling, returns may retain up to the capacity remaining after
cached entry charges. This is not a process RSS limit. Concurrent active reads,
caller-owned results, allocator metadata, and allocator-retained pages can add
memory beyond the configured cache capacity.

Freeing unsuitable idle buffers before allocating reduces simultaneous live
allocation capacity compared with leaving those buffers idle. It does not promise
an immediate RSS reduction, an in-place replacement, or zero fragmentation.
Jemalloc can retain freed pages for reuse and purge them later. Retained virtual
address space must not be confused with resident memory or a leak.

## Changes

- `feuer-memory/src/buffer.rs`: add cross-bucket reclamation to the pooled miss
  path. Leave allocation layouts, return handling, and cache admission unchanged.
- `feuer-memory/src/buffer/tests.rs`: cover the new policy and adjust tests that
  currently assume a bucket miss leaves all other idle buckets untouched. Where
  tests need multiple populated buckets, allocate those buffers before returning
  them to the pool.
- `README.md` and `metrics.md`: describe miss-time reclamation and the distinction
  between the idle ceiling and process memory. No new configuration or metrics.

## Tests

- Exact-bucket hits reuse the allocation and leave other idle buckets untouched.
- A smaller request reclaims a larger idle buffer and receives the requested
  bucket capacity, not the donor capacity.
- A larger request reclaims multiple smaller idle buffers when needed.
- Reclamation stops after enough bytes are released, or after idle buffers run out.
- Empty pools, zero retention, and unpooled requests keep their existing behavior.
- Idle and used bucket gauges and shared byte accounting follow reclamation and
  return correctly.
- Cached entries and outstanding `Bytes` owners are not reclaimed.
- Returned buffers remain 4096-byte aligned and expose only the requested length.
- Existing cache-pressure, concurrent-return, and pool-lifetime tests still pass.
- Extend the existing environment-isolated 100% test to exercise reclamation and
  cached-entry priority without changing the process-wide configuration machinery.

Run the memory library tests. Run storage library tests on
`ssh m8g-32cpu-local-ssd-2` using the command in `AGENTS.md`.

## Evaluation

Before changing the default, compare the current policy against 100% retention
with cross-bucket reclamation. Follow `benchmarks/storage/timed-replay.md`: same
jemalloc version, matched 180-second trace prefixes, and all prescribed captured
metrics. Include RSS and idle/used bytes by bucket; runtime alone cannot establish
whether memory behavior improved. Do not launch full traces or extra repetitions
without an explicit request.

Actual allocator resizing is a separate follow-up only if measurements justify
it. It would need to preserve 4096-byte alignment, update the stored layout only
after a successful resize, and account for the resulting capacity. It is not
part of this baseline.
