# Adaptive tier-scoring search

Question: can memory request cost and demand-history half-life reduce source or disk traffic at fixed cache capacities?

**68 unique measurements: 36 range and 32 full-object configurations**, in waves of 14, 16, 22, 12 and 4. No replay was repeated or truncated. All 68 replay processes exited zero. The first fourteen reports were recovered from original captures, preserving wrapper exit status 1. The other 54 reports succeeded directly.

**Larger scoring steps produced a strong observed full-object disk-read reduction, but not a meaningful source or overall-performance gain.** This is unreplicated exploratory evidence, not statistical proof or a universal production recommendation.

See [RESULTS.md](RESULTS.md) for exact settings, every new case’s absolute/relative changes against baseline and metric-specific previous best, predeclared selection decisions, and all baseline/selected-case metrics. [results.csv](results.csv) retains every configuration and captured summary field. [LEADERS.md](LEADERS.md) records the separate objective rankings and Pareto frontier. There is no weighted aggregate score.

## Input volume and memory fill

The available archive spans 2,694.824824702 seconds of arrivals (44m 54.825s), with 17,369,893 request starts. Only its first 600 seconds were replayed. Those 1,606,645 requests sum to 2,800.015 GB with repetition, but contain just 123,885 distinct exact ranges totaling **129.662 GB**. Unioning overlapping ranges by original object gives **129.575 GB**. The full-object baseline's final disk payload counter independently equals the exact-range total, 129,661,511,335 bytes.

| Seconds into arrivals | Distinct exact-range bytes requested, GB | Full-object baseline memory charge, GB |
| ---: | ---: | ---: |
| 60 | 77.574 | 34.903 |
| 180 | 96.894 | 66.302 |
| 300 | 121.303 | 68.162 |
| 600 | 129.662 | 89.471 |

Memory charge includes overhead and buffers, not just payload, and is not RSS. Values are from the closest one-second sample. The 48-slot source model delayed data delivery and allowed duplicate queued fetches. By 600 seconds, full-object source downloads had completed 336.386 GB, but successful disk batches contained only 89.828 GB of payload, and 292,587 source requests still waited. Thus total demand volume substantially overstates both distinct data and data actually available to the cache during arrivals.

First sampled memory eviction occurred at 168.939 seconds in range mode and 444.987 seconds in full-object mode, with total charges of 63.787 GB and 79.546 GB respectively. Capacity is divided among 64 shards, so a shard can evict while others retain spare capacity. This is primarily a cold-fill/backlog experiment with limited disk reuse, not sustained disk-cache pressure. Offline analyses and raw values are in `raw/trace-duration.json`, `raw/trace-footprint.json` and `raw/memory-fill-timing.json`. No further replay was run to obtain them.

## Main result and tradeoffs

The lowest observed full-object read bytes are from **case-065**, with:

- Memory request cost **9,133,289**.
- Cache-wide demand half-life **4,096 requests**.
- Disk request cost **256,000**, both per-byte coefficients **1**.
- All capacities, source settings, source/binary/input identities, worker type and arrival duration unchanged.

The previous read-byte and total-disk best was case-049: memory request cost **281,600**, half-life **212,337**, same disk and per-byte coefficients.

| Metric | Full-object baseline | Previous read-byte best, case-049 | Case-065 |
| --- | ---: | ---: | ---: |
| Disk read bytes | 283,838,786 | 246,824,876 | **80,709,704** |
| Disk read calls | 54 | 48 | 34 |
| Disk write GB | 141.593804800 | 141.593862144 | 141.595914240 |
| Total disk GB | 141.877643586 | 141.840687020 | 141.676623944 |
| Source GETs | 518,835 | 518,835 | 518,831 |
| Source GB | 537.875868784 | 537.875224232 | 537.872828743 |
| Memory caller hits | 67.703569% | 67.703942% | 67.705062% |
| Disk caller hits | 0.003361% | 0.002988% | 0.002116% |
| Combined caller hits | 67.706930% | 67.706930% | 67.707179% |
| Memory byte hits | 80.780116% | 80.781461% | 80.787479% |
| Disk byte hits | 0.010137% | 0.008815% | 0.002882% |
| Combined byte hits | 80.790253% | 80.790276% | 80.790362% |
| Request-phase CPU seconds | 56 | 55 | 56 |
| OS maximum RSS GB | 4.053811200 | 4.022845440 | 3.794182144 |
| Scheduled-to-complete mean s | 150.544273 | 150.546061 | 150.542499 |
| Scheduled-to-complete p99 s | 719.604891 | 719.618456 | 719.612275 |
| Execution / drain s | 1318.425949 / 719.066034 | 1318.441294 / 719.081379 | 1318.432939 / 719.073024 |

Decimal GB = 1,000,000,000 bytes. Disk traffic includes the five-second observation tail.

- **Reads:** case-065 saves **203,129,082 bytes (71.564949%)** versus baseline and **166,115,172 bytes (67.300823%)** versus the previous read-byte best. Calls fall by 20 (37.037037%) versus baseline, or 13 (27.659574%) versus the separate previous read-call minimum of 47 in case-028.
- **Total disk:** only **201,019,642 bytes (0.141685%)** below baseline and **164,063,076 bytes (0.115667%)** below case-049, because approximately 141.6 GB of writes dominate. Writes increase by 2,109,440 bytes (0.001490%) versus baseline.
- **Source:** only **4 fewer GETs (0.000771%) / 3,040,041 fewer bytes (0.000565%)** than baseline. Against the previous source best, case-050, case-065 uses **16 more GETs / 32,989,568 more bytes**. The strong read effect mainly shifts hits from disk to memory, not from source to cache.
- **Latency/CPU:** no meaningful gain. Execution and p99 are respectively 6.990 ms and 7.385 ms longer than baseline, while mean latency is 1.774 ms lower. CPU is unchanged. RSS is 0.259629056 GB (6.404567%) below baseline, but zero-filled downloads and single-run sampling limit interpretation.
- **Queue/memory/completion:** case-065 peaks at 294,506 waiting source requests and 48 active. Source wait averages 466.056701 s. Settled memory charge is 99.158030895 GB, idle buffers 0.050331648 GB, and charge minus idle 99.107699247 GB. It has 123,885 queued/published writes, 394,263 covered admissions, 683 redundant admissions, and 2,117 memory-evicting insertions. Disk never evicts. All pending work and disk writes drain to zero. These memory charges are not payload occupancy.

The minimum full-object **total** disk traffic instead occurs at **case-063: memory request cost 9,133,289, half-life 16,384**. Its 141.675056230 GB total saves **202,587,356 bytes (0.142790%)** against baseline and **165,630,790 bytes (0.116772%)** against case-049. The 4,096 half-life refinement saves only 1,684,510 additional read bytes (2.044452%) versus case-063, while slightly increasing total traffic.

The minimum full-object source bytes occur at **case-064: memory request cost 36,533,156, half-life 16,384**, with 537.838137114 GB. This is only **37,731,670 bytes (0.007015%)** below baseline and **1,702,061 bytes (0.000316%)** below previous source best case-050. Source GETs remain lowest at **case-050: memory request cost 281,600, half-life 317,194**, with 518,815. Neither difference establishes a source-performance improvement.

**Range contrasts did not qualify.** The prior read/total-disk best remains **case-017: memory request cost 309,760, half-life 262,144**, with 3.837841374 GB reads / 145.401618398 GB total. The prior source best remains **case-031: memory request cost 186,624, half-life 262,144**, with 517,457 GETs / 537.780983224 GB. Request-heavy range case-054, at memory request cost 1,000,000,000 and half-life 262,144, reduces read calls by 56.505914% but raises read bytes by **139.287638%**. Fewer calls alone are not a win.

## Strong contrasts and conditional refinements

The initial 52-case local search used ±10% coordinate steps. A separate continuation tested larger steps. [settings.json](settings.json) preserves the original plan. [contrast-settings.json](contrast-settings.json) fixes the continuation plan before its measurements.

The twelve new contrasts tested each of these memory-cost/half-life pairs in both modes, always keeping disk request cost 256,000:

| Memory request cost | Half-life, requests |
| ---: | ---: |
| 0 | 262,144 |
| 9,133,289 | 262,144 |
| 1,000,000,000 | 262,144 |
| 256,000 | 16,384 |
| 256,000 | 1,048,576 |
| 9,133,289 | 16,384 |

Further testing required improvement over the previous eligible best exceeding the greater of a practical floor and five times the original no-op disk-coefficient spread, with regression guards on the other selection metrics. Practical floors were 20% of baseline disk-read bytes/calls and 0.1% of baseline source bytes/GETs. These are operational stopping thresholds, not confidence intervals. Conditional disk ratios alone could not trigger more runs.

Only full-object disk-read bytes qualified: case-063 saved 164,430,662 bytes versus the previous best, above the unchanged 56,767,757.2-byte requirement. Its 10-call improvement was below the 10.8-call threshold. Neither source objective qualified. Range kept its 1,055,135,385-byte incremental read threshold and had no qualifying contrast. Detailed `contrast_search.assess` decisions are preserved in `raw/contrast-decisions.json` and tabulated in RESULTS.

Exactly four full-object refinements followed, using `(memory request cost, half-life)` pairs **(36,533,156, 16,384), (9,133,289, 4,096), (2,283,322.25, 16,384), (9,133,289, 65,536)**. The predeclared one-refinement-wave limit then stopped the search at **68**, below the maximum 72. Thresholds were never lowered, no other wave was launched, and no measurement was repeated. This stopping rule does not establish convergence.

## Fixed measurement contract and limitations

Each case uses **100,000,000,000 memory bytes**, **500,000,000,000 disk bytes**, a fresh SSD cache, 32 runtime threads on m8gd.8xlarge, jemalloc, and the same first 600 seconds of input. The last scheduled arrival is 599.359914970 s. Observation continues for five seconds after every request finishes. Each case completes **1,606,645 requests / 2,800,015,397,526 returned bytes**. Per-request indices are checked exactly once by the binary, and captured counters independently cross-check completion. All lookup/disk-I/O errors, failed admissions/writes, no-capacity writes and final pending gauges are zero. Actual tuning, capacities, source settings, manifests, replay/wrapper exits and every archive hash were independently revalidated.

Source callbacks simulate warmed S3 GETs with **48 concurrent slots** and service time `0.108599 seconds + requested_bytes / 84,101,038.08 bytes/second`. Each queues independently and allocates its own zero-filled payload. Replay arrivals and lookups remain unrestricted. Calibration used the 48-concurrent medians in [the S3 measurements](../../s3_benchmark.md), from another host and much larger objects. This is not real S3 and does not model variability, retries, failures, coalescing or shared bandwidth. The expected source backlog dominates latency and was always allowed to drain completely.

Full-object mode treats each exact recorded range as an object, not the original source file. Memory eviction occurs in every case. Exact-range source downloads are admitted asynchronously to disk, which never evicts at this capacity and trace length. Disk-cost effects are therefore unidentifiable, and disk coefficients were not explored further after the first wave.

Each configuration has one observation. Adaptive selection, few full-object disk reads, cross-host timing, and mixed availability zones in the first wave prevent statistical or universal tuning claims. The first wave used eleven workers in us-east-1f and three in us-east-1a. Later waves used us-east-1a. Actual memory-payload occupancy is unavailable because the binary does not emit that metric. Memory charge, idle buffers and RSS are separate quantities, and sampled peaks may miss short spikes. Exact replay latency quantiles and bucket-interval source/disk histograms must not be confused.

RESULTS includes memory/disk/combined and conditional hit rates, separate disk reads/writes/totals, source GETs/bytes, CPU/RSS, exact latency, source queue/service/drain, admission/eviction, memory charge and completion. Full one-second metrics, phase snapshots, device counters, iostat/pidstat, commands, histograms and inventories are retained locally under ignored `raw/`.

## Source and binary identity

The source base below includes a working-tree patch, not just the named commit. The measured source, binary, input, runner, allocator and capacities were unchanged across the search. Only post-processing and coordinator control changed.

- Source base: `eacaf12ad1fa2870ddcf55fba6ed7d05c32659fc`
- Source archive SHA256: `a38ab14b43daea9e6edfff6e71bc54da25fbd48a3c99e80c31753d1ffb34be98`
- Source patch SHA256: `bc2d1dc0cfa456e7e457aa9a30ac915c3c65390d11cbdaf5e01156706572a2c6`
- Replay binary SHA256: `dec9095c5d9924c42780ad707576c871ecc9ce95314c99690c205892afdf0292`
- Input SHA256: `a13aeae5c77da3709814fef42708d6cb55a867a553ba01dfc8c7e8338e7807f7`
- Corrected-reporter artifact SHA256: `4c72f4f1715eb97c6bb4aef89eebd7693ba5d136ed1f618b63e080dede95edc8`

## Reproduction and report recovery

On the recorded Linux worker type, stage the pinned binary at `$ROOT/bin/replay`, the pinned input at `$ROOT/input/trace.tar.zst`, and `report.py` plus `report-metrics.py` at `$ROOT/`. The runner requires `/mnt/local-ssd`, jemalloc, sysstat, util-linux and `/usr/bin/time`, with `$ROOT` outside the SSD mount. For example, the full-object baseline command is:

```sh
bash run-case.sh "$ROOT" case-007 full_objects 256000 256000 262144
```

Use the per-case parameters in `results.csv` for other configurations. This is a historical binary experiment, not a test of the current checkout. The frozen source archive and patch are local under `raw/`, and the input and binary must be supplied separately. The committed files alone do not reconstruct the executable or private trace.

Python 3.10’s missing `hashlib.file_digest` and a reporter expecting an unsupported memory-payload metric caused first-wave post-processing failures. Original archives and wrapper failures remain under `raw/cases/` and `raw/collection-recovery/`. Corrected first-wave reports/inventories are under `raw/reported/`. All fourteen replay exits are zero, original wrapper exits remain one, and `status=report_recovered` remains in the CSV. No replay was rerun. Operational history is retained under `raw/`, including `raw/curation-originals/`.
