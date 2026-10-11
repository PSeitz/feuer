# Eviction

## Retention objective

The objective is to reduce retrieval cost, not maximize memory and disk hit
percentages independently. Treat memory retrieval as approximately free relative
to I/O. Moving a disk hit into memory lowers disk's hit percentage but avoids a
paid read. Combined cache hit rates measure source avoidance, not total cost.

Model disk and source retrieval separately, with fixed costs per request and
transfer costs per byte. Costs may vary per object, especially for computed
results. Use a consistent cost unit rather than adding latency and monetary
charges directly.

SSD random access generally has a lower fixed latency than S3. When their
throughputs are similar, byte-transfer costs can be similar, while S3 GET charges
add a per-request cost. Calibrate the model for the actual source. The source may
be S3, another cache, or computation. Its cost is the work needed to obtain a
missing result, not necessarily its original precomputation cost.

Retention should favor expected retrieval cost saved per charged byte. A memory
entry avoids the disk or source work that would otherwise serve its requests.
A disk entry saves source retrieval cost minus the disk read cost, for requests
that reach disk. Evaluate these benefits alongside eviction CPU and latency.

## Idea to test: tier-specific retrieval costs

Status: hypothesis only. No policy changes or replay comparisons yet.

Model disk retrieval from the implementation's actual work, not just returned
payload bytes. Whole-entry reads verify the payload checksum. Disk byte counts
round up to 4-KiB alignment, with a 4-KiB minimum, and reads larger than 1 MiB
require multiple I/O requests. Memory allocation charges can differ from both
payload length and disk bytes.

For a first model:

```text
disk_cost(object) = fixed_read_cost + aligned_disk_bytes(object) × disk_byte_cost
memory_value = expected_memory_requests × expected_fallback_cost / memory_charge
disk_value = expected_requests_reaching_disk × (source_cost(object) - disk_cost(object))
             / disk_charge
```

The disk-cost model is an approximation to calibrate, including the effect of
multiple I/O requests and checksum work. Memory fallback cost depends on whether
the object would be served from disk or the source. Source cost can vary by
object and need not be proportional to bytes.

### Per-object, per-tier costs

Estimate avoided retrieval cost for each object in each tier, rather than
assuming cost is linear in payload length. Disk-read cost depends on aligned
bytes, the number of I/O requests, fixed latency, and checksum work. Memory
capacity cost depends on the actual allocation charge, including bucket rounding.
Retention value combines expected demand with avoided cost per charged byte.

The smallest pooled memory bucket is 32 KiB. For entries charged at their pooled
backing capacity:

| Payload | Disk bytes read | Memory allocation charge |
| --- | ---: | ---: |
| 10 bytes | 4 KiB | 32 KiB |
| 4 KiB | 4 KiB | 32 KiB |
| 32 KiB | 32 KiB | 32 KiB |
| 40 KiB | 40 KiB | 256 KiB |

Disk alignment and memory buckets make the model stepped, not simply
proportional to payload bytes. A compact 10-byte memory entry can have high
retention value because it avoids a full disk operation for little memory.
A 10-byte entry with a 32-KiB pool charge instead has roughly the same memory
charge and disk-read cost as a 4-KiB entry with that charge, at equal demand.

Downloads created through `AlignedBuffer::into_download` and disk promotions
charge backing capacity. `Download::new` defaults to payload-length charges
unless the application supplies a capacity. Use the entry's actual charge, not
an assumed bucket size.

Per-object modeling does not necessarily require a stored cost field. Predicted
disk costs can be derived from existing entry metadata and calibrated disk
parameters. Source cost may also follow a size-based model. Application-owned
per-object metadata is needed when source work cannot be inferred, such as
equally sized results with different recomputation costs. Memory and disk still
need distinct avoided-cost models because their fallback paths differ.

### Placement hypotheses

- **Objects around 4 KiB:** test keeping their disk copies while evicting them
  from memory when the modeled fallback cost is low. Their payload fits the
  minimum disk read. The 4-KiB minimum alone does not establish that this is the
  best placement, because demand, fixed latency, and memory charge still matter.
- **Very large objects:** test evicting them from disk when source retrieval
  approaches disk-read cost. With similar transfer throughput, the savings per
  retained disk byte can become small. Size alone is not an eviction rule.
- **Expensive computed results:** use per-object source costs so a cheap object
  and an expensive object of the same size need not receive equal retention
  credit.

For whole objects, disk writes are already queued after memory admission.
Evicting from memory while retaining a disk copy is a placement decision, not a
new write-on-eviction mechanism. Test the scoring hypothesis without changing
that pipeline or increasing capacities.

## Current scoring

The current policy is an approximation of the objective, not a tier-specific
cost model. Both tiers sample eligible entries and evict the lowest retention score.
The default `RetrievalCostScorer` divides modeled retrieval cost by charged bytes:
allocation capacity in memory, payload length on disk (at least one byte for empty entries).
For each request fully contained in a cached range, the retrieval cost adds:

```text
decayed access count × (fixed request cost + requested bytes)
```

Fixed request cost defaults to `0`. Set it per cache with
`CacheConfig::with_fixed_retrieval_equivalent_bytes(n)`. Both tiers use this setting
unless a custom scorer is supplied. The default scorer does not model aligned
disk reads or store per-object source costs.

Each started request records its requested range or whole-object demand once, before any lookup,
in `feuer-historian`, shared by both tiers. Historian supplies request counts and recent events.
Retrieval-cost scoring belongs to the shared retention policy in `feuer-memory`.
Whole-object scores use the cached object's payload length.
Whole-object entries are evicted rather than trimmed to partial ranges.
Failed requests, invalid downloads, and requests canceled after starting all count as demand.
History hashes object keys into 64 independently locked maps, separate from cache shards.
One atomic request clock across all history shards preserves global request-age decay.
Raw cache reads, insertions, and evictions do not record accesses or delete history.

History keeps every distinct `(object key, requested range)` counter for its lifetime,
even after both tiers evict the object. Repeated exact requests update the same counter. This metadata
has no capacity limit and is not persisted across restarts. Its memory grows with distinct keys and ranges.
Scores still decay, measured in requests across all keys.

Memory may trim an entry to previously requested ranges instead of evicting it.
Trimming uses recent-access events with per-object count and age limits, not the full counter set.
A trimming plan uses a history snapshot. Newer accesses do not invalidate it. Cached-range changes
are still checked before publishing bytes copied outside the memory shard lock.

## Custom retention scores

Pass an `Arc<dyn RetentionScorer>` to
`TieredMemoryDiskCache::open_with_retention_scorer(config, metrics, io_queues, scorer)`.
Both tiers share it. The scorer can own computation costs keyed by
`ObjectKeyHash::from(key.as_str())` and combine them with the supplied historian's demand.
These costs live in application-owned metadata, not in cache entries. Feuer has no
built-in per-object cost setter, and scorer arguments do not explicitly identify
which tier is making the decision.

Higher finite scores favor retention. Any byte normalization belongs to the scorer.
Application metadata is neither persisted nor removed by the cache, including on eviction.
Scoring runs under shard locks. Keep it short and do not re-enter the cache.

See the [README](README.md#environment-variables) for tuning settings.
