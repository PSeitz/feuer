# Key hashing

Feuer stores 128-bit key digests as object identities. It does not retain original
keys for collision checks. A digest collision can return another key's payload.
The current implementation uses SipHash-1-3-128 with a random secret 128-bit key per
cache. Cloned handles share the key. Disk caches persist it across reopens, and
memory-only caches generate a new key on each open. Protect the backing file because
it contains the secret key. Use `cache.hash_key(&key)` for matching scorer metadata.

Keys implement Rust `Hash`. Include all distinguishing fields and namespaces, and
keep this encoding stable when reusing disk entries. Rust does not guarantee its
stability across compiler versions or platforms. Change `payload_version` when key
encoding or payload semantics change. Disk format 16 invalidates older caches.

## SipHash-1-3 comparison

An experiment on 2026-11-12 compared Feuer commit `d3be0fc` with a change to
`siphasher::sip128::SipHasher13`, using `finish128()` and a fixed 128-bit key.
The experiment left production unchanged at the time. SipHash-1-3-128 has since been
adopted with a random per-cache key. The measurements below describe the fixed-key
experiment, rather than a benchmark of the adopted implementation.

The target was `m8g-32cpu-local-ssd-2`, AWS Graviton4 / Neoverse-V2, Linux aarch64.
Both binaries used Rust 1.99.0, release mode, and `-C target-cpu=native`.
Dependencies came from Feuer's lockfile, with siphasher 1.0.4 added.

### Method

- Replace only `ObjectKeyHash::from_hash` in an isolated experimental checkout.
- Hash complete Rust `Hash` inputs, including strings, terminators and scalar fields.
- Use synthetic keys matching the shapes in Pomsky's Feuer integration at `886175e55`.
  Slice keys are `(namespace, URI, start, end)`. Predicate keys are `(split_id, AST JSON)`.
  Request keys reproduce the derived-Hash field order and types for the exercised
  `SearchRequest` values, followed by the effective time range. `search_after` is always absent.
- Construct and serialize keys before timing. Cycle through 256 keys per case.
- Measure hashing, `Cache::get` hits and misses, and `get_or_fetch` hits independently.
  Cache operations use four memory shards, 64 MiB capacity, 256-byte payloads,
  no-op metrics, and no disk. All populated keys fit without eviction.
- Pin to CPU 4. Take five samples in one paired experiment, alternating algorithm
  order. Each measurement has 30 ms warmup and at least 150 ms timed execution.
  Report medians. Samples describe throughput timing, not per-request latency percentiles.

### Results

Times are nanoseconds per operation. Key bytes count the complete Hash input.

| Key | Hash bytes | Hash writes | XXH3 hash | Sip13 hash | XXH3 memory hit | Sip13 memory hit |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Split slice | 82 | 6 | 38.6 | 41.5 | 184.6 | 189.5 |
| Predicate, match all | 48 | 4 | 30.7 | 28.0 | 184.7 | 179.0 |
| Predicate, one term | 75 | 4 | 34.0 | 32.4 | 185.2 | 183.3 |
| Predicate, 16 terms | 948 | 4 | 87.9 | 254.0 | 241.7 | 405.5 |
| Predicate, 64 terms | 3,636 | 4 | 253.0 | 937.3 | 425.5 | 1,090.0 |
| Predicate, 256 terms | 14,388 | 4 | 863.9 | 3,667.5 | 1,026.4 | 3,821.7 |
| Predicate, 64 KiB value | 65,608 | 4 | 3,840.1 | 16,599.3 | 4,039.0 | 16,688.8 |
| Partial request, one term | 220 | 30 | 188.0 | 97.7 | 338.9 | 246.2 |
| Partial request, 64 terms | 3,781 | 30 | 396.7 | 989.0 | 580.7 | 1,131.9 |
| Partial request, 64 terms + aggregation | 3,880 | 32 | 415.1 | 1,020.8 | 579.0 | 1,162.5 |

SipHash is roughly even for short keys and faster for the tested small structured
request. Many small writes make the streaming hasher's per-write overhead relevant.
For larger AST strings, SipHash hashing is approximately 3–4.3 times slower.
Absolute memory-hit overhead ranges from about 0.16 microseconds for the 948-byte
predicate to 12.65 microseconds for the 65,608-byte predicate.

These measurements exclude serialization, query execution, disk I/O, concurrent
contention and production key-size distributions. They do not estimate whole-query
latency. The fixed key also excludes runtime seed-loading and persistence costs.

The SipHash experimental variant passed all 35 Feuer memory-only library tests on
Linux. Existing hash-specific XXH3 unit tests were not part of that test command.

## Integration considerations

At the inspected Pomsky Feuer integration, AST strings reach Feuer without an
upstream digest. Changing Feuer's identity hash therefore covers the complete AST.
The non-Feuer Pomsky cache already uses randomly keyed SipHash-1-3-128.

The predicate cache separately hashes the same key for scorer metadata on hits and
inserts. It pays the hashing cost twice. At 3,636 bytes, the measured added hashing
cost alone would be about 1.37 microseconds across those two calls. This is an
arithmetic estimate, not a timed Pomsky lookup. These scorer lookups must now use
`cache.hash_key(&key)` so their identities match the cache's identities.

The adopted implementation preserves the full 128-bit output. Widening a 64-bit
`finish()` result to u128 does not increase collision resistance. The fixed key was
suitable for this performance experiment. Production uses secret random key material
persisted with the disk cache to reuse entries across restarts. Algorithm changes,
key rotation or loss, and Hash encoding changes require invalidating old identities.
The experiment measures performance, not the cryptographic security margin of SipHash-1-3.

## Local artifacts and reproduction

Configurations, source, lockfile, patch, raw samples, machine details and summaries
are stored locally under ignored `experiments/siphash-13/`.
The remote copy is `/home/ubuntu/feuer-siphash-13.WfUlou` on the benchmark host.
`run.sh` builds both variants and runs the paired experiment. `summarize.py`
produces per-case medians, ranges, ratios and deltas from `raw.csv`.
No disk replay was run. Access traces contain already-hashed identities and would
omit the AST hashing cost being measured here.
