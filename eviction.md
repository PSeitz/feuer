# Eviction

## Retention objective

Minimize expected future retrieval cost within each layer's capacity.
Decayed past requests estimate future demand. Payload size represents the cost of
keeping an object. Each layer uses total request demand independently of the contents
of other layers, so disk retains objects valuable as a fallback even while memory serves them.

Costs can represent I/O time, CPU time, or another application cost. Use one
consistent unit. Source cost is the work needed to obtain a missing result.
Both scoring models below estimate value per payload byte kept.

## Current scoring

Both tiers sample eligible objects and evict the object with the lowest retention score.
Every request for the object contributes to its decayed count. Both scorers normalize
by payload length, using a one-byte denominator for empty payloads. Unknown keys score zero.
All cost coefficients must be finite and nonnegative. Intermediate costs and scores
must remain finite for the workload.

### Fetch cost: the default

`FetchCostScorer` estimates the fallback fetch cost and treats cache reads as free:

```text
fetch_cost = cost_per_request + payload_len × cost_per_byte
score = decayed_request_count × fetch_cost / max(payload_len, 1)
```

`cost_per_request` is the fetch cost independent of payload size. `cost_per_byte` is
the fetch cost per payload byte. Defaults are `0` and `1`. For nonempty objects, payload
size cancels out and the score equals the decayed request count. This is a frequency-based
default. A positive fixed fetch cost favors smaller objects at equal demand.
To preserve the former fixed-equivalent-byte policy:

```rust
let scorer = std::sync::Arc::new(feuer::FetchCostScorer {
    cost_per_request: 256_000.0,
    cost_per_byte: 1.0,
});
let cache = feuer::Cache::builder()
    .memory_capacity(64 << 20)
    .disk_capacity(1 << 30)
    .with_memory_retention_scorer(scorer.clone())
    .with_disk_retention_scorer(scorer)
    .open()?;
```

The builder accepts separate memory and disk scorers. Clone an `Arc` into both
setters to share one scorer, as above. Omit either setter to use that tier's default
scorer. Both scorers receive the same request history. Set the candidate sample
size for both tiers with `.reclaim_sample_size(n)`.

### Cache read savings: compare two paths

`CacheReadSavingsScorer` scores the cost saved by reading from cache instead of fetching
from the fallback. It accepts costs for both paths:

```text
fallback_cost = fallback_cost_per_request + payload_len × fallback_cost_per_byte
cache_read_cost = cache_read_cost_per_request + payload_len × cache_read_cost_per_byte
score = decayed_request_count × (fallback_cost − cache_read_cost) / max(payload_len, 1)
```

For disk, use source fetch costs and disk read costs. For memory, choose an assumed disk
or source fallback and supply memory read costs separately. Both paths' costs are nonnegative.
Negative savings produce negative scores, prioritizing eviction when cache reads cost more
than fallback fetches. With zero cache read costs, this scorer gives the same scores as
`FetchCostScorer`.
Choosing the savings scorer with nonzero cache read costs changes retention decisions and
should be evaluated separately. Existing defaults and replay settings use `FetchCostScorer`.

### Approximation limits

Both scorers evaluate caller-supplied estimates using payload length and total request
history. Neither inspects the contents of other layers. Applications must include
alignment, checksum work, multiple I/O submissions, and contention in their estimates
or custom scorers. Estimated savings and past request counts approximate future value.

Disk alignment and memory allocation buckets make costs non-linear. For
example, a 10-byte payload may require a 4-KiB disk read and occupy a 32-KiB pooled
memory buffer. Callback results of type `Bytes` charge payload length.
`AlignedBuffer` results and disk promotions charge backing capacity. These charges
enforce memory capacity. Scorers receive payload length.

Applications can supply a custom scorer when linear size-based costs are
insufficient. Eviction accounts for size, demand, and avoided fixed costs.
Caller-supplied admission filters can independently reject memory insertion or disk
queueing. Both default to accepting all objects. Configure them through
`.with_memory_admission_filter(...)` and `.with_disk_admission_filter(...)` on the
builder. Each predicate receives the key hash, payload length, and shared request
history. Memory rejection still permits disk admission. Fetched bytes reach callers
regardless of admission. Memory filtering also applies to disk-hit promotions.
Existing entries remain readable, including recovered disk entries. Admission
filters run synchronously outside cache shard locks and may run concurrently.

## Access history

Each started request records its object once, before lookup, in `feuer-historian`.
Failed requests and requests canceled after starting count as demand. Raw cache
reads, insertions, and evictions do not record accesses or delete history.

History hashes keys into 64 independently locked maps, separate from cache
shards. One atomic request clock supplies request-age decay across all keys.
The default half-life is 262,144 requests. Each object's counter survives cache
eviction until history cleanup removes it. History defaults to a soft limit of
2,000,000 entries per cache, configurable through `FEUER_HISTORY_ENTRY_LIMIT` or
the builder's `.history_entry_limit(n)`.

One background thread checks the limit each second and removes the least recently
requested keys, targeting 90% of the limit. It locks one shard at a time and exits
after the history is dropped. Removing history resets demand without removing
cached payloads. History is not persisted across restarts.

## Custom retention scores

A `RetentionScorer` receives the key hash, object payload length, and request history. It can own per-object computation costs keyed by
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
