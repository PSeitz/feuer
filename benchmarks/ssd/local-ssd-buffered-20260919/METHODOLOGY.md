# Buffered-I/O screening methodology

## Scope

A **short 14-case screening** of the original fio workload without `O_DIRECT`, not a full rerun of all 66 original cases. Measurements were shortened at the user's request: **2 seconds warmup + 10 seconds measured**, versus 5 + 30 seconds originally. The complete buffered pass, including inter-case flushes, took **3 minutes 44 seconds**. All 14 cases completed without fio errors and their measured durations were checked. No repetitions or confidence intervals.

Same host: `m8g-32cpu-local-ssd` (`m8gd.8xlarge`, 32 Graviton4 vCPUs, 128 GiB advertised RAM). Same local NVMe instance-store SSD `/dev/nvme1n1`, ext4 at `/mnt/local-ssd`; Linux `6.17.0-1007-aws`, fio 3.36. Not EBS.

## Files and execution

The tests reuse fully initialized **128 GiB read** and **64 GiB write** files from the preceding Rust experiment. New hardlinks were created before that experiment removed its original filenames; no data copying or new initialization was needed. Buffered measurements waited for the preceding suite's `DONE` marker. **No benchmark cases overlapped.**

The files were originally initialized completely by sequential 1 MiB direct writes followed by `sync_all`. Their contents differ from the original fio-prepared dataset, but sizes and regular-file/directories-on-the-local-SSD layout match. The read file was not modified during measurement; all measured writes went sequentially to the separate write file.

Only these dedicated links were removed afterward. Results/logs are stored on EBS, outside the SSD under test.

## Configuration

fio performs the actual I/O; Python only orchestrates and samples system counters.

- `ioengine=libaio`, **`direct=0`**, `thread=1`, `numjobs=1`, configured `iodepth=32`.
- `invalidate=1`, `fallocate=none`, `allow_file_create=0`.
- `refill_buffers=1`, `randrepeat=0`, `norandommap=1`.
- `time_based=1`, `ramp_time=2`, `runtime=10`.
- Per-job bandwidth logs averaged over approximately one second.
- Case ordering randomized with seed `20260919`.

| Workload | Cases |
|---|---|
| Random reads | 4 KiB, 16 KiB, 64 KiB, 1 MiB |
| Sequential writes | 4 KiB, 16 KiB, 64 KiB, 1 MiB |
| Concurrent sequential reads/writes | Read sizes 16 KiB and 64 KiB, each with no writer, 450 MB/s writes, and 910 MB/s writes |

The mixed writer uses 1 MiB blocks, nominal QD32, with `rate=0,450000000` or `rate=0,910000000`. Reader and writer jobs run simultaneously. Requested rates and achieved rates are reported separately. No random writes.

## Page cache and writeback

`invalidate=1` remains as in the original benchmark: fio invalidates the relevant file cache before each case. This does **not** intentionally benchmark a fully hot dataset. Cache may fill and produce hits during the warmup/measurement window.

A file-specific `fdatasync` runs before the suite and **after every case, outside timed fio measurements**, to drain remaining writes before the next case. This avoids one case's delayed writeback becoming the next case's background workload. No global cache dropping or VM parameter tuning.

Recorded defaults: `vm.dirty_ratio=20`, `vm.dirty_background_ratio=10`, and both byte-based overrides zero. These are percentages of eligible memory, not fixed percentages of the nominal instance RAM.

**Buffered libaio can block in `io_submit`.** Configured QD32 does not imply 32 concurrent physical read requests. Therefore, a cold random-read slowdown against asynchronous direct I/O may reflect reduced effective concurrency, not a slower SSD or an inherent buffered-I/O limit. The original QD1 direct-read values are included for context.

Buffered write bandwidth means acceptance into the page cache; it is **not fsync-durable throughput**. Ten-second measurements can include cache bursts and dirty-page throttling. They are a quick behavior check, not steady-state throughput guarantees.

## Accounting and validation

- Application MB/s = fio JSON `bw_bytes / 1,000,000`. Block sizes use binary KiB/MiB.
- fio job errors and measurement durations are checked.
- `/sys/block/nvme1n1/stat` sector counters (512 bytes/sector) are captured around each fio process and its subsequent flush.
- Device-rate averages cover the whole fio process, including startup, cache invalidation, and warmup. They are **not exactly aligned** with fio's ten-second measured interval.
- Post-case physical write bytes and `fdatasync` duration expose deferred work; do not add them to the timed bandwidth numerator or label application bandwidth durable.
- `iostat -dxm -y 1` and one-second samples of device counters, Dirty, Writeback, Cached, and MemAvailable are retained outside the compact results folder.
- Direct comparisons use the original fio results from earlier in the session. Timing, dataset contents, and disk/cache state are not perfectly matched; this is a screening comparison, not a controlled single-variable repeated experiment.

## Reproduction

Raw results and runner are retained outside this results folder:

- Local: `~/Development/benchmarks/results/local-ssd-buffered-20260919/`
- Instance: `/home/ubuntu/ssd-bench-results/local-ssd-buffered-20260919/`

On the same idle instance, create a fresh results directory on EBS. If fully initialized files are no longer available, recreate them with the retained Rust initializer; this adds about two minutes of preparation:

```sh
BASE=/home/ubuntu/ssd-bench-results/local-ssd-buffered-20260919
RUST=/home/ubuntu/ssd-bench-results/local-ssd-rust-uring-20260919
RUN=/home/ubuntu/ssd-bench-results/buffered-repro-$(date -u +%Y%m%dT%H%M%SZ)
mkdir "$RUN"
cp "$BASE/run_buffered.py" "$RUN/"
"$RUST/target/release/ssd-bench" prepare "/mnt/local-ssd/$(basename "$RUN")" 137438953472 68719476736
cd "$RUN"
python3 -u run_buffered.py > progress.log 2>&1
```

The runner checks host/device identity and file sizes, and waits for the retained preceding suite's `DONE` marker. Ensure no other benchmark is active. Do not use Python `-O`, which disables assertions. The initializer refuses an existing data directory; do not format the device or remove unrelated files.

On success, `DONE` is created and the dedicated data links/directory are removed. Reporting uses the retained `summarize.py`, Python with `matplotlib==3.10.9`, and the original baseline archive alongside the working directory as `local-ssd-20260919`. Copy only the report, methodology, CSV, and chart into the final results folder.
