---
title: Disk I/O
sidebar_position: 3
---

A disk cache must serve reads while writing newly cached objects to the same
SSD. Feuer batches small writes, reuses read buffers, and limits read and write
concurrency separately.

The measurements below used an EC2 m8gd.8xlarge with a local NVMe SSD and direct
I/O.

## Start with disk demand

Cache accesses become different disk traffic: memory hits avoid reads, small
objects share writes, and metadata adds its own I/O. Recorded disk sizes,
offsets, and arrival times describe that traffic more directly than a histogram
of logical requests.

Small reads can be limited by operations per second, while large reads consume
more bandwidth. Read/write interference therefore depends on the request mix
and timing. In a synthetic test with equal numbers of 4-KiB and 1-MiB reads,
adding 450 MB/s of sequential writes preserved roughly 3.9 GB/s of read
throughput. The small-read-only test responded differently.

[Mixed-size results](https://github.com/PSeitz/feuer/blob/main/experiments/ssd/local-ssd-mixed-read-sizes-20260926/RESULT.md)
· [Small-read results](https://github.com/PSeitz/feuer/blob/main/experiments/ssd/local-ssd-random-read-write-20260925/RESULT.md)

These saturation tests measure capacity under sustained load. Choosing
concurrency requires the application's actual IOPS, bandwidth demand, and
latency target, including periods when reads and writes overlap.

Feuer shares I/O queues across caches and allows up to 64 active reads and eight
active writes per queue set. Separate limits control outstanding work in each
direction. A shallow queue can still sustain high bandwidth, so request sizes
and arrival rates also matter.

## Batch small writes

Batching makes each write more efficient. In a sequential write-size test at
QD1, increasing requests from 4 KiB to 256 KiB raised throughput from 217 to
1,829 MB/s. Increasing the request size to 1 MiB left throughput unchanged.

[Full transfer-size results](https://github.com/PSeitz/feuer/blob/main/experiments/ssd/local-ssd-benchmark-20260919/RESULT.md)

Feuer therefore separates object size from disk-write size. Small objects are
packed into a shared buffer and their payloads are written together. A full
buffer becomes one 1-MiB payload write, even when it contains many small objects.
The buffer size matches Feuer's disk chunks and falls within the measured
throughput plateau.

Larger objects use consecutive chunks, keeping each object's payload in one
contiguous byte range.

```text
Small objects share a chunk       A large object spans chunks
┌─────────────────────────┐      ┌───────────────────────────────┐
│ object A │ B │ object C │      │           object D            │
└─────────────────────────┘      └───────────────────────────────┘
          1 MiB                        consecutive chunks
```

A mixed-read follow-up compared 1-MiB writes with 64-KiB write requests.
Splitting the writes into smaller requests showed no consistent read-throughput
benefit in the tested workloads.

[Full write-size comparison during reads](https://github.com/PSeitz/feuer/blob/main/experiments/ssd/local-ssd-write-chunks-20260926/RESULT.md)

## Reuse read buffers

Feuer reuses read buffers through the memory cache's buffer pool. This avoids
repeatedly allocating backing memory, which is particularly useful for small
reads at high request rates.

Request queues, completion notifications, and shared ownership also add CPU
work per read. Keeping that coordination inexpensive matters alongside SSD
bandwidth.

## Share concurrent work

Avoiding duplicate I/O also leaves more capacity for useful work. When several
callers request the same object, Feuer can share the disk read. If the object is
missing, callers can share the source fetch too.

```text
Caller A ─┐
Caller B ─┼─ same object ── one disk read or source fetch
Caller C ─┘                         │
                              shared result
```

For `get_or_fetch`, requests within the same cache share in-flight work when
the key and callback error type match. The fetch callbacks must be
interchangeable, since one caller's callback may supply the result for all of
them.

This avoids repeated work for concurrent requests. Its benefit depends on how
often requests overlap, and was not measured by the SSD experiments above.
