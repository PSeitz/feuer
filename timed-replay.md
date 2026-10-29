# Timestamp-paced two-tier SSD replay

Use this procedure for matched cache-revision SSD comparisons. The maintained
[FETR reader and replayer](feuer-replay/README.md) reads single-session `.tar.zst`
archives directly, using the same executable with disk disabled or enabled.
Keep tested settings, raw results, and execution workspaces in ignored local
folders under `experiments/cache/`. Preserve useful conclusions in project
documentation. Captures and raw artifacts may contain object identifiers.
Keep narrative summaries free of those identifiers.

## Defaults and workload contract

- Host: `ssh m8g-32cpu-local-ssd-2`, 32 CPUs, `/mnt/local-ssd`, device `nvme1n1`.
  Check for unrelated CPU/device activity. Leave other users' jobs running.
- **Memory capacity is 100 GB (`100000000000` bytes). Disk capacity is 500 GB
  (`500000000000` bytes).** Pass these exact decimal capacities for every case.
  Change them only when explicitly requested. Use 64 shards.
- **Replay the first 10 minutes with `--trace-seconds 600`.** Run additional
  repetitions only when explicitly requested.
- Preserve original arrival timing and exactly-once completion. Use one paced
  sender and one lookup task per arrival, with one runtime thread per CPU.
  Queue descriptors before tasks allocate payloads. Preserve backlog rather
  than throttling arrivals or dropping requests.
- Lookups exercise the real memory and disk caches, write admission, packing,
  eviction, and metadata writer. Matching misses share cache fetch work.
- Each source callback acquires an `AlignedBuffer` through `allocate_buffer`,
  zero-fills it, and returns it without copying. The cache charges backing
  capacity. Downloads are immediate unless `--simulate-s3` is selected.
- Match allocators between binaries. If using system jemalloc, record its
  version, hash, `LD_PRELOAD`, and `MALLOC_CONF` settings.

Before launching, verify the actual command and settings contain both exact
capacities. Run one case at a time with fresh cache and output directories.
Keep inputs, source, builds, and output on EBS, outside the measured SSD.
Require disk capacity plus 32 GiB free headroom. Clean up only the runner's
own cache and monitor processes afterward.

## Build and run

Pin explicit baseline and candidate commits. Build isolated snapshots before
measurement, using the same replay driver and settings where compatible.
Record source changes when comparing a working tree.

On the SSD host, from each snapshot:

```sh
export PATH="$HOME/.cargo/bin:$PATH"
TMPDIR=/mnt/local-ssd cargo test --locked -p feuer-storage --lib
cargo test --locked -p feuer-replay
cargo clippy --locked -p feuer-replay --all-targets -- -D warnings
cargo build --locked --release -p feuer-replay --bin replay

target/release/replay \
  --input /private/path/searcher-0.tar.zst \
  --cache-dir /mnt/local-ssd/fresh-replay-cache \
  --output /private/path/fresh-results \
  --memory-bytes 100000000000 --disk-bytes 500000000000 \
  --trace-seconds 600
```

Preserve commit IDs, driver/lock/input/binary SHA-256 hashes, `rustc -Vv`,
OS/allocator versions, host details, commands, environment, and exit status.
Monitor with `pidstat`, `iostat`, `/usr/bin/time`, and device counters. Capture
host activity before and after each case. Failed or interrupted cases are
not performance results.

## Capture and present the metrics

Retain all replay output: `metrics.prom` (one-second samples), `before.prom`,
`requests-finished.prom`, `settled.prom`, and `summary.csv`. Keep OS/device
monitor output and the comparison report alongside them. The five-second
observation tail observes deferred work. It provides no forced payload drain,
metadata flush, shutdown flush, or durability guarantee.

Present baseline and candidate side by side for every requested case:

- Sender lateness, caller-queue wait, service, and scheduled-to-completion:
  mean, p50, p95, p99, maximum. Separate execution and backlog-drain time.
- Memory/disk/callback outcomes, public request-weighted and byte-weighted hit
  rates, and returned bytes. Internal memory operations have a different scope
  from caller hit rates.
- CPU seconds/average cores, sampled and OS maximum RSS, cache accounting,
  idle/used buffer bytes, pending payload, queued arrivals, and active lookups.
- Write admission outcomes, eviction-triggering insertions, and batch packing.
- DataFile read/write calls, bytes, durations, and errors. Include process and
  device I/O, device await, queue size, and utilization. Show tail activity separately.
- All metric families, with histogram count/mean and quantile bucket intervals.
  Label histogram intervals as intervals rather than exact percentiles.
- Cold startup versus later behavior from timestamped deltas.

State measurement scopes. Phase counter deltas exclude opening. OS totals include
input loading and final reporting. Device totals can include other processes.
CPU phase snapshots have one-CPU-second resolution. Sampled peaks can miss short
spikes. Check exactly-once completion, exit 0, errors, pending bytes, and tail
activity before claiming complete costs. One pair cannot establish a small
speedup or regression.

4 KiB is the alignment requirement. DataFile writes may be larger, and DataFile
calls are API operations rather than kernel submissions or device requests.
Verify the tested implementation before attributing write counts or bytes to
payload versus metadata work.
