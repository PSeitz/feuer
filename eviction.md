# Eviction

## Retention objective

Retain entries that avoid the most retrieval cost per byte of cache capacity.
Memory avoids the disk or source work that would otherwise serve a request.
Disk saves source retrieval cost minus the disk read cost. Moving a disk hit into
memory lowers disk's hit percentage but avoids a read, so tier hit percentages
alone do not measure policy quality.

Costs can represent I/O time, CPU time, or another application cost. Use one
consistent unit. Source cost is the work needed to obtain a missing result.

## Current scoring

Both tiers sample eligible entries and evict the entry with the lowest retention score.
`AvoidedRetrievalCostScorer` computes:

```text
decayed_object_count × max(cost_per_request + payload_len × cost_per_requested_byte, 0)
────────────────────────────────────────────────────────────────────────────────────
                              max(entry_size_bytes, 1)
```

Every request for the object contributes to its decayed count. The denominator
is the allocation charge in memory and payload length on disk. Disk alignment
and metadata overhead are excluded. A zero entry size uses a one-byte denominator.
Negative avoided costs are clamped to zero.
Coefficients and resulting scores must be finite.

`cost_per_request` is the cost avoided once per request, regardless of requested
byte count. `cost_per_requested_byte` is the cost avoided for each requested byte.
Defaults are `cost_per_request = 0` and `cost_per_requested_byte = 1`, preserving
byte-weighted demand. To preserve the former fixed-equivalent-byte policy:

```rust
let scorer = std::sync::Arc::new(feuer::AvoidedRetrievalCostScorer {
    cost_per_request: 256_000.0,
    cost_per_requested_byte: 1.0,
});
// Pass scorer to TieredMemoryDiskCache::open_with_retention_scorer.
```

`open_with_retention_scorer` shares one scorer between tiers.
`open_with_retention_scorers` accepts separate memory and disk scorers. Both receive
the same request history. `CacheConfig` keeps candidate sampling settings in
`SampledEvictionConfig`, separate from resource settings and scoring parameters.

### Tier-specific costs and limitations

Separate scorers can model memory's fallback cost and disk's source-minus-disk
cost. The linear model does not inspect residency or account for aligned reads,
checksum work, multiple I/O submissions, or contention.

Assuming every memory miss falls back to disk can undervalue memory: pending or
skipped writes, disk eviction, and read failures may leave only source fallback.
Both scorers use all caller demand, including memory hits, so
disk can also overvalue copies of memory-hot objects.

Disk alignment and memory allocation buckets make costs non-linear. For
example, a 10-byte payload may require a 4-KiB disk read and occupy a 32-KiB pooled
memory buffer. `get_or_fetch` charges payload length, while disk
promotions and `get_or_fetch_buffer` charge backing capacity. A custom
scorer receives this charge as `entry_size_bytes`.

Applications can supply a custom scorer when linear size-based costs are
insufficient. Eviction accounts for size, demand, and avoided fixed costs.
Objects are queued for disk after memory admission.

## Access history

Each started request records its object once, before lookup, in `feuer-historian`.
Failed requests and requests canceled after starting count as demand. Raw cache
reads, insertions, and evictions do not record accesses or delete history.

History hashes keys into 64 independently locked maps, separate from cache
shards. One atomic request clock supplies request-age decay across all keys.
The default half-life is 262,144 requests. Each object's counter survives cache
eviction for the history object's lifetime. Counter metadata has no capacity
limit and is not persisted across restarts.

## Custom retention scores

A `RetentionScorer` receives the key hash, object payload length, tier-specific
entry size, and request history. It can own per-object computation costs keyed by
`ObjectKeyHash::from(key.as_str())`. Higher finite scores favor retention. Any
byte normalization belongs to the scorer.

Scoring runs concurrently under shard locks. Avoid I/O, cache re-entry, and lock
ordering that could deadlock with cache operations. Applications own the persistence
and cleanup of their scoring metadata, including after eviction. Separate tier
scorers identify the tier without adding a tier argument to the scoring contract.

See [environment variables](README.md#environment-variables) for tuning settings.

## Lessons from historical comparisons

Residency-aware memory scoring traded source work for disk reads. One 600-second
pair reduced source payload by 0.277 GB while increasing SSD reads by 1.229 GB.
It established no total-cost improvement. Matched-coefficient scorer consolidation
preserved scoring arithmetic, while corrected-scheduler comparisons established
no performance improvement. These runs exercised no disk eviction.

Historical linear coefficients combined median SSD latency with median S3 transfer
measurements from different workloads. They were approximations rather than
measured mean retrieval costs or production defaults.
