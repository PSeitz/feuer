# Cost scorer consolidation — 2026-10-12

## Change and result

One `RetrievalCostScorer { fixed_cost, cost_per_byte }` replaces the equivalent-byte
scorer and the separate avoided-time scorer. Costs use a consistent application-chosen
unit, including equivalent bytes, I/O time, or CPU time:

```text
score = sum(decayed_count × max(fixed_cost + requested_bytes × cost_per_byte, 0))
        / max(charged_bytes, 1)
```

Defaults remain zero fixed cost and one cost unit per byte.
`CacheConfig::with_fixed_retrieval_equivalent_bytes(n)` sets `fixed_cost = n` and
keeps `cost_per_byte = 1`, reproducing the original formula exactly. Separate memory
and disk scorers remain supported. Custom scorers can use application-owned per-object
costs. No residency tracking, access-history change, admission change, or historical
SSD/S3 coefficients were added to the default policy. The historical replay-only
`--tier-aware` path was removed.

**Score-equivalence and replay-correctness checks passed.** Retention outcomes were
nearly identical in the matched replay. This supports the behavior-preserving
consolidation, not a claim of performance equivalence: CPU rose from 45 to 47 seconds
and scheduled-completion p99 worsened from 0.267 to 2.094 ms. The timing difference
remains unresolved. See [latency and interpretation](#latency-and-interpretation).

## Matched configuration

Exactly one baseline → candidate pair ran on `m8g-32cpu-local-ssd-2`, with no repetitions.
Each replay used the first 600 seconds of the same capture, original pacing, a fresh
cache, **100,000,000,000 memory bytes and 500,000,000,000 disk bytes**, 32 workers,
64 shards, exact immutable full objects, jemalloc, and a five-second observation tail.
The 32 consumers each await one lookup before taking another, limiting concurrent cache
lookups to 32 independently of Tokio's 32 worker threads.
Every source miss independently allocated zero-filled `Bytes`, without network delay
or coalescing. Both tiers used fixed cost **256,000** and per-byte cost **1**.

The baseline was the initial working tree, including its existing uncommitted changes,
not bare HEAD. The candidate changed the scorer implementation while retaining those
changes. Source snapshots, binaries, input and lock hashes, environment, and reporting
proxy coefficients are recorded in [`settings.json`](settings.json). Runtime and policy
settings were matched. No knob search was performed.

## Correctness and validation

Both cases completed **1,606,645 requests exactly once**, returned
**2,800,015,397,526 bytes**, and exited 0. Callback errors, invalid downloads,
read/write errors, failed writes, no-capacity outcomes, and canceled writes were zero.
Pending payload and queued write entries were zero after the observation tail.
The capture's 841 recorded incomplete starts are input diagnostics, not unfinished
replay requests.

Linux validation passed:

```sh
PATH="$HOME/.cargo/bin:$PATH" TMPDIR=/mnt/local-ssd cargo test --locked -p feuer-storage --lib
cargo test --locked -p feuer -p feuer-memory -p feuer-historian -p feuer-types -p feuer-replay
cargo clippy --locked -p feuer -p feuer-memory -p feuer-storage -p feuer-replay -p feuer-memory-bench --all-targets -- -D warnings
```

There were **252 passing tests, one ignored storage test, and no failures**.
Tests compare the new scorer directly with the original arithmetic across range and
whole-object demand, zero and nonzero charges, and fixed costs through `u64::MAX`.
Other tests cover cost-unit scaling, per-request negative-cost clamping, memory
retention, disk victim selection, and shared/separate tier scorers.

Build verification caught a stale candidate artifact from a shared Cargo target
directory before candidate replay. The candidate was rebuilt in an isolated target
directory and its actual binary hash was verified distinct. Rebuilding happened after
the baseline process exited, with unchanged SSD counters. No additional replay occurred.
At experiment completion, all 61 tracked Rust source files matched the pinned candidate.
Documentation cleanup followed that validation and did not change tested Rust sources.
The later scheduling correction below is not part of those binaries or results.

## Cache outcomes and resources

GB and MB are decimal. Public hit percentages use caller requests and returned bytes.
DataFile counters include the observation tail. Physical SSD counters are whole-device
traffic and include alignment and possible unrelated I/O.

| Metric | Baseline | Candidate |
| --- | ---: | ---: |
| Memory hits | 1,477,911 | 1,477,948 |
| Disk hits | 2,443 | 2,438 |
| Source callbacks | 126,291 | 126,259 |
| Memory request hit % | 91.987402 | 91.989705 |
| Disk request hit % | 0.152056 | 0.151745 |
| Combined request hit % | 92.139458 | 92.141450 |
| Memory byte hit % | 95.129954 | 95.131365 |
| Disk byte hit % | 0.229447 | 0.229451 |
| Combined byte hit % | 95.359402 | 95.360816 |
| Memory returned GB | 2,663.653374 | 2,663.692872 |
| Disk returned GB | 6.424563 | 6.424670 |
| Source returned GB | 129.937461 | 129.897856 |
| Useful memory at finish, GB | 97.101101 | 96.902739 |
| Charged memory including idle buffers, GB | 99.247338 | 99.012996 |
| Idle buffers at finish, GB | 0.093356 | 0.087589 |
| Request-phase CPU, seconds | 45 | 47 |
| Average request-phase cores | 0.075079 | 0.078415 |
| Whole-process CPU, seconds | 54.98 | 57.51 |
| OS maximum RSS, GB | 11.945525 | 11.934417 |
| Sampled peak RSS, GB | 11.932463 | 11.921912 |
| Sampled peak pending payload, GB | 10.005218 | 10.003749 |
| Sampled peak caller backlog | 1,654 | 1,628 |
| Memory eviction-triggering insertions | 2,315 | 2,303 |
| Disk eviction-triggering insertions | 0 | 0 |
| DataFile read calls | 2,443 | 2,438 |
| DataFile read GB, before alignment | 6.424563 | 6.424670 |
| DataFile write calls | 30,048 | 30,242 |
| DataFile write GB | 98.408907 | 98.889073 |
| Physical SSD read GB | 6.432768 | 6.432895 |
| Physical SSD write GB | 98.438402 | 98.922676 |
| SSD request-weighted read await, ms | 4.316740 | 4.246841 |
| SSD request-weighted write await, ms | 0.127241 | 0.127633 |
| SSD mean queue size | 0.515726 | 0.512369 |
| SSD mean utilization % | 3.507162 | 3.541830 |
| Execution seconds | 599.371880 | 599.372163 |
| Backlog-drain seconds | 0.011965 | 0.012248 |
| Tail DataFile write MB | 66.363392 | 66.371584 |

The common historical SSD/S3 reporting proxy was **15,312.848119 → 15,308.902586
modeled seconds**, a −0.025766% difference. It is the sum of disk-hit fixed cost,
disk returned-byte cost, source-callback fixed cost, and source returned-byte cost.
These coefficients are reporting inputs only, not scorer settings, measured source
latency, or elapsed replay time. The separate request and byte counts remain above.

Useful payload accounting is not physical residency, especially for zero-filled
synthetic downloads. Phase CPU excludes opening and final reporting and has one-second
resolution. Whole-process CPU/RSS includes loading and reporting. Sampled peaks can
miss short spikes. Neither the tail nor zero pending payload establishes flushing or
durability. Zero disk eviction means this replay did not exercise disk victim selection.

### Write admission

| Outcome | Baseline | Candidate |
| --- | ---: | ---: |
| Queued | 102,623 | 103,707 |
| Queue-full skips | 21,891 | 20,817 |
| Already covered before queueing | 15 | 15 |
| Redundant | 1,762 | 1,720 |
| Published | 102,623 | 103,707 |
| Queue closed, canceled, failed, or no capacity | 0 | 0 |

## Latency and interpretation

Exact distributions from replay observations, in milliseconds:

| Latency | Baseline mean / p50 / p95 / p99 / max | Candidate mean / p50 / p95 / p99 / max |
| --- | --- | --- |
| Sender lateness | 0.034721 / 0.032941 / 0.055383 / 0.160802 / 0.607880 | 0.096400 / 0.037087 / 0.366066 / 1.630776 / 4.540345 |
| Caller queue wait | 0.683829 / 0.009724 / 0.016679 / 0.027890 / 492.295639 | 0.684407 / 0.009047 / 0.016011 / 0.029085 / 491.870512 |
| Cache service | 0.020693 / 0.001334 / 0.004030 / 0.033370 / 62.735100 | 0.020674 / 0.001341 / 0.003955 / 0.032615 / 57.794980 |
| Scheduled completion | 0.739244 / 0.045551 / 0.074806 / 0.266988 / 533.656289 | 0.801480 / 0.049403 / 0.450550 / 2.093766 / 534.192421 |

Mean scheduled-completion delay increased by **62.236 µs**, of which **61.678 µs
(99.10%)** was increased sender lateness, **0.577 µs** increased queue wait, and
**−0.019 µs** changed service time. The sender implementation was unchanged.
Percentiles are not additive, so these aggregates cannot decompose the p99 change.
The candidate's scheduled p99 was **7.84×** the baseline's, and request CPU was
**4.44% higher**. Stable cache-service timing does not prove CPU equivalence.

Host snapshots showed no competing benchmark, ample memory and no swap. Iostat
sampled approximately 99.7% host idle and zero steal, with similar device behavior.
Those samples do not explain the sender timing difference. It is measured and
unresolved, not established noise and not proven scorer-caused. One pair establishes
neither a small improvement nor performance equivalence. The consolidation preserves
scoring arithmetic and default weighting, without claiming those stronger results.

### Histogram observations

All histogram families during the request phase are shown below. Quantiles are bucket
intervals `(lower, upper]`, not exact percentiles. The p50/p95/p99 intervals were identical
in both cases. Means use sum/count deltas. Counts exclude opening and the tail.

| Histogram, unit | Baseline count / mean | Candidate count / mean | p50 interval | p95 interval | p99 interval |
| --- | --- | --- | --- | --- | --- |
| DataFile read, ms | 2,443 / 11.647678 | 2,438 / 11.649637 | (1.024, 2.048] | (32.768, 65.536] | (32.768, 65.536] |
| DataFile write, ms | 29,911 / 1.002270 | 30,103 / 0.999507 | (0.128, 0.256] | (2.048, 4.096] | (16.384, 32.768] |
| Disk lookup, ms | 2,443 / 11.837850 | 2,438 / 11.840364 | (1.024, 2.048] | (32.768, 65.536] | (32.768, 65.536] |
| DataFile requested read bytes | 2,443 / 2,629,784.417 | 2,438 / 2,635,221.325 | (262144, 524288] | (8388608, 16777216] | (8388608, 16777216] |
| Disk write queue wait, ms | 102,623 / 111.385319 | 103,707 / 109.495482 | (1.024, 2.048] | (524.288, 1048.576] | (1048.576, 2097.152] |
| Callback lookup, ms | 126,291 / 0.014430 | 126,259 / 0.014538 | (0.002, 0.004] | (0.032, 0.064] | (0.128, 0.256] |
| Disk-hit lookup, ms | 2,443 / 11.919331 | 2,438 / 11.926769 | (1.024, 2.048] | (32.768, 65.536] | (32.768, 65.536] |

### Startup versus later behavior

Windows use nearest one-second samples. Each cell is baseline → candidate, and these
are approximate windows, not independent repetitions. Caller backlog was zero at all
window endpoints. Reads were zero before 90 seconds, then 0.076129 → 0.076045 GB in
90–180 seconds and 6.348435 → 6.348625 GB afterward.

| Nominal seconds | Memory hits | Disk hits | Source callbacks | DataFile write GB |
| --- | ---: | ---: | ---: | ---: |
| 0–30 | 81,115 → 81,167 | 0 → 0 | 39,755 → 39,703 | 33.514177 → 33.968300 |
| 30–90 | 117,147 → 117,151 | 0 → 0 | 22,590 → 22,586 | 13.248963 → 13.293072 |
| 90–180 | 280,264 → 280,273 | 59 → 59 | 18,638 → 18,629 | 16.082706 → 16.116220 |
| 180–finish | 999,385 → 999,357 | 2,384 → 2,379 | 45,308 → 45,341 | 35.496698 → 35.445109 |

## Originating benchmark versus replay latency

A later read-only analysis joined original starts and outcomes from the same 600-second
trace prefix. It confirmed seconds-level tails in the originating benchmark:

| Disk-hit lookup span | Original trace | Baseline replay |
| --- | ---: | ---: |
| Count | 7,618 | 2,443 |
| Mean, ms | 152.682 | 11.919 |
| p95 | 461.971 ms | (32.768, 65.536] ms |
| p99 | 3.025088 seconds | (32.768, 65.536] ms |

For the 2,552 original disk-hit requests arriving in seconds **120–130**, p95 was
**2.959901 seconds**. Original values are exact start-to-outcome spans and can include
start-trace queue backpressure. They are not direct device I/O time. Replay values
are the public disk-hit lookup histogram, with quantiles reported as bucket intervals.
Neither population is the all-request scheduled-completion distribution above.

The replay preserves request arrivals, not original completion delays or cache contents.
It uses a fresh cache, turns ranges into independent whole objects, replaces downloads
with immediate exact-sized zero-filled allocations, and replaces recorded failures with
successes. Original successful callbacks took a mean 150.382 ms and p95 512.138 ms.
These differences change which requests reach disk and the load there. The replay did
not reproduce the originating benchmark's disk-tail behavior and does not establish
its cause or performance parity with that benchmark. No additional replay was run.

The identifier-free analysis is retained in `raw/recorded-latencies.json`, with its
reader in `raw/trace-latencies.py`, separately from the original artifact archive.

### Application queue path

Read-only inspection of `../pomsky/use_feuer/` at `5cb0b087c` confirmed that the
application does not use the replay's 32-consumer lookup limit. Search permits limit
split warmups, with a default of 100, not individual cache lookups. `quickwit-search/src/leaf.rs`
spawns split tasks, joins seven warmup categories concurrently, and uses `try_join_all`
for reads within those categories. `quickwit-storage/src/split_range_cache/storage.rs`
calls Feuer directly for each storage read.

`quickwit-search/src/service.rs` supplies one shared queue set to split-range,
partial-response, and predicate caches when their disk tiers are enabled. The split
trace is attached only to the split-range cache, so other caches' shared-queue traffic
is absent from this replay. Which other disk caches were enabled in the originating
benchmark has not been established. The application's Cargo.lock pins Feuer at
`76eb4ce827952986633d40fc50373fa52fa916df`, whose `uring.rs` and `file.rs` match the
current versions: 64 active read submissions plus 64 waiting, and 8 active writes
plus 8 waiting.

The replay moves excess arrivals into its outer caller queue, whose wait is included
in scheduled completion but excluded from disk-hit duration. It therefore changes
both concurrency feeding the real disk queue and the scope of disk-only waiting.
This is a confirmed harness mismatch, not a demonstrated cause of every original
stall. No replay or application code was changed during this inspection.

### Subsequent scheduling correction

The current replayer now starts one task per arrival rather than one serial consumer
per runtime thread. It retains CPU-sized Tokio threads, removes the 32-lookup cap,
and reports both unstarted and active requests. The matched pair above used the old
scheduler, as recorded in `settings.json`. No full-capture or benchmark replay has
been run with the correction, so these results cannot describe its performance.

[The follow-up investigation](../pomsky-replay-concurrency-20261012/README.md) records
Helm override precedence, the conditional 32-core profile, remaining fidelity gaps,
and focused Linux validation. It does not explain away either timing discrepancy.

## Local artifacts

Ignored `raw/artifacts.tar.gz` and extracted `raw/` retain source archives, the actual
binaries, hashes, commands, environment, test/build logs, all one-second metrics and
phase snapshots, reports, exact latency summaries, process/device monitoring, and
host snapshots. Each `raw/results/{baseline,candidate}/` includes `METRICS.md`,
`metric-inventory.csv`, and `histograms-and-windows.json`, covering every captured
metric family, including per-bucket used/idle buffer accounting and process I/O.
The reporting helpers are retained with the artifacts and do not launch replays.

The SSH workspace is `/home/ubuntu/feuer-cost-scorer-20261012`. The runner removed
only its own SSD cache directories and monitor processes. Only this summary and
settings are intended for Git, not private captures, raw data, binaries, or workspaces.
