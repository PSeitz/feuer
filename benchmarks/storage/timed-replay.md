# Timestamp-paced two-tier SSD replay

Use this procedure for matched cache-revision comparisons, not the memory-only replay
or isolated direct-I/O benchmarks. The private harness is in `../replay-feuer`;
its `DISK_REPLAY.md` documents the driver. Keep private traces, object identifiers,
and raw artifacts in that workspace or on the authorized host. Publish only
identifier-free commands and results.

## Defaults and workload contract

- Host: `ssh m8g-32cpu-local-ssd-2`, 32 CPUs, `/mnt/local-ssd`, device `nvme1n1`.
  Check for unrelated CPU/device activity; do not stop other users' jobs.
- **Use the original-time 180-second trace prefix.** It contains 469,587 requests
  in the existing input. The completed four-case matrix took about 16 minutes.
  A full trace can take 38–50+ minutes per case because backlog must finish;
  do not launch full traces or additional repetitions without an explicit request.
- Real `TieredMemoryDiskCache::get_or_fetch`: memory cache, SSD reads/writes,
  write-queue admission, packing, eviction, and the revision's metadata writer.
- One paced sender; one competing consumer per CPU. Preserve original arrival
  timing and exactly-once completion. Queue only descriptors before consumers
  allocate payloads. Do not throttle arrivals, drop requests, or hide backlog.
- Each source miss independently allocates zero-filled bytes for the requested
  logical range: `Bytes::from(vec![0; length])`. No network delay, shared payloads,
  aligned-download replacement, or experimental download pool.
- Same system **jemalloc** for both binaries, using
  `env -u MALLOC_CONF LD_PRELOAD=/lib/aarch64-linux-gnu/libjemalloc.so.2`.
  Record package version/library hash. The completed comparison used jemalloc
  5.3.0 and Rust 1.98.0. No profiling preload, manual trim, or allocator tuning;
  do not mix glibc, preload jemalloc, and direct-Rust-allocator results.

| Scenario | Memory | Disk | Required observed behavior |
| --- | ---: | ---: | --- |
| Pressure | 16 GiB | 128 GiB | Disk eviction occurs |
| No pressure | 16 GiB | 768 GiB | Zero disk eviction and zero no-capacity outcomes |

Both sizes use 64 shards in the historical comparison. Validate each revision's
actual scenario metrics; capacities alone do not prove pressure. Queue-full disk
admission skips can occur in both scenarios and are not dropped fetch requests.
Run one case at a time with a fresh cache. Keep input, sources, builds, and output
on EBS, not the measured SSD. Require disk capacity plus 32 GiB free headroom.
Remove only the runner's own cache and monitor processes afterward.

## Pin, build, and run

Pin explicit baseline/candidate commits; never silently benchmark whatever HEAD
happens to contain. The completed metadata comparison was **`f875941` versus
`23d241c`**, not subsequent revisions.

Create isolated `git archive` snapshots, each with this sibling layout so the
harness's Cargo path dependencies resolve:

```text
RUN_ROOT/
  baseline/{feuer,replay-feuer}/
  candidate/{feuer,replay-feuer}/
  bin/{baseline,candidate}
  input/{requests.bin,objects.txt}
```

Use identical harness source and Cargo.lock for both revisions. On Linux, from
each replay snapshot, with `PATH="$HOME/.cargo/bin:$PATH"`:

```sh
cargo test --locked --features disk --bin disk_trace
cargo clippy --locked --features disk --bin disk_trace -- -D warnings
cargo build --locked --release --features disk --bin disk_trace
```

Copy each executable to its named `bin/` path. Build before measurement; do not
use `--all-targets` (historical memory drivers have incompatible APIs). Preserve
commit IDs, driver/lock/input/binary SHA-256 hashes, `rustc -Vv`, OS/allocator
versions, host details, command, and environment with results.

The private runner sets these identically:

```text
FEUER_ACCESS_COUNT_HALF_LIFE=4096
FEUER_FIXED_RETRIEVAL_EQUIVALENT_BYTES=256000
FEUER_MAX_ACCESS_EVENTS_PER_KEY=256
FEUER_MAX_ACCESS_AGE_ACCESSES=262144
FEUER_RECLAIM_SAMPLE_SIZE=64
FEUER_IDLE_BUFFER_POOL_PERCENT=7
```

New revisions no longer read `FEUER_FIXED_RETRIEVAL_EQUIVALENT_BYTES`; the harness must
set `CacheConfig::with_fixed_retrieval_equivalent_bytes(256000)` to match the older revisions.

Requires `libjemalloc2`, `iostat`, `pidstat`, `/usr/bin/time`, and Python 3.
Run the private harness's `run-disk-comparison.sh RUN_ROOT 180`. It runs pressure
baseline/candidate, then no-pressure candidate/baseline. It refuses existing
case directories; always choose a fresh output root. An explicitly requested
repeat uses `REVERSE_ORDER=1` and a new root with the same binaries/input.
Failed or interrupted cases are not performance results.

### Reuse the historical pinned binaries

On the SSD host, the completed comparison's workspace is
`/home/ubuntu/feuer-timed-replay.pSxcY7`. To reproduce **those two old revisions**:

```sh
workspace="$HOME/feuer-timed-replay.pSxcY7"
root=$(mktemp -d "$HOME/feuer-replay-180s.XXXXXX")
ln -s "$workspace/bin" "$root/bin"
ln -s "$workspace/input" "$root/input"
bash "$workspace/short-180s/run-disk-comparison.sh" "$root" 180
# Only if another matched repetition was requested, with another fresh root:
# REVERSE_ORDER=1 bash "$workspace/short-180s/run-disk-comparison.sh" "$other_root" 180
```

Verify binary hashes against `short-180s/manifest.txt` first. Rebuild isolated
snapshots when comparing new commits; these preserved binaries do not test them.

## Capture and present the metrics

Retain `metrics.prom` (all registered metrics, one-second samples), `before.prom`,
`requests-finished.prom`, `settled.prom`, `summary.csv`, `report.json`, commands,
environment, exit status, `pidstat.txt`, `iostat.txt`, `time.txt`, device counters,
and host-activity snapshots. The five-second observation tail is **not** a forced
payload drain, metadata flush, shutdown flush, or durability guarantee.

Present baseline/candidate side by side for **both** scenarios, not just runtime:

- Sender lateness, caller-queue wait, service, and scheduled-to-completion:
  mean, p50, p95, p99, maximum; execution and backlog-drain time separately.
- Memory/disk/callback outcomes; public request-weighted and byte-weighted hit
  rates and returned bytes. Internal memory operations are not caller hit rates.
- CPU seconds/average cores; sampled and OS maximum RSS; cache accounting,
  per-bucket idle/used buffer bytes, pending payload, and caller backlog.
- Write admission: queued, queue-full, published, stale, canceled, failed,
  no-capacity; memory/disk eviction-triggering insertions and batch packing.
- DataFile read/write calls, bytes, durations and errors; process and device
  I/O; device await, queue size and utilization. Show tail activity separately.
- All metric families, with histogram count/mean and quantile bucket intervals.
  Label histogram intervals as such; do not present them as exact percentiles.
- Cold startup versus later behavior from timestamped deltas.

State scopes: phase counter deltas exclude opening; OS totals include input
loading/final reporting; device totals can include other processes. CPU phase
snapshots have one-CPU-second resolution. Sampled peaks can miss short spikes.
Check exactly-once completion, exit 0, errors, pending bytes, and tail activity
before claiming complete costs. One pair cannot establish a small speedup/regression.

### Write sizes and metadata attribution

4 KiB is the alignment requirement, **not the size of every DataFile write**.
DataFile calls are API operations, not kernel submissions or device requests.

In the pinned `f875941`/`23d241c` comparison, payload writes were one call per
1-MiB payload chunk. Baseline metadata wrote new 1-MiB metadata chunks, then
4-KiB dirty pages. The candidate removed whole-chunk initialization and wrote
only 4-KiB dirty metadata pages. Do not describe baseline initialization as
current behavior.

For those revisions, with no failed/canceled payload batches, metadata work can
be derived as total write bytes minus batch payload-chunk bytes, and total write
calls minus payload-chunk bytes / 1 MiB. Verify the implementation before using
this derivation on another revision. Metadata-only latency/CPU was not measured.
Fewer small metadata calls need not materially reduce total payload-dominated bytes.

## Existing results

Private local artifacts:
`../replay-feuer/disk-replay-20261003/ssd-2/short-180s/`.
`REPORT.md` summarizes the completed matrix; `METRICS.md` displays the full metric
comparison, regenerated by `python3 report-metrics.py`. That helper reads the
adjacent `results/`; it does not launch a replay. `STATUS.md` in the parent replay
artifact root records the canceled full matrix and separate allocator experiments.
Do not mix those experiments or incomplete runs with the completed short comparison.
