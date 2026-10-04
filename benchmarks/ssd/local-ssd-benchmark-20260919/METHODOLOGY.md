# Testing methodology

## Environment

Measured on **2026-09-19**, on `m8g-32cpu-local-ssd` (EC2 **m8gd.8xlarge**):

- 32 Graviton4 vCPUs, 128 GiB advertised RAM.
- One nominal 1.9 TB Amazon EC2 NVMe instance-store SSD, `/dev/nvme1n1`.
- ext4 at `/mnt/local-ssd`, mounted `rw,relatime`; initially empty and idle.
- Linux `6.17.0-1007-aws`, aarch64; fio **3.36**.
- Scheduler `none`; `max_sectors_kb=128`; `nr_requests=127`.

**fio, not Python, performs the measured I/O.** Python only launches cases and collects results. The backend was Linux `libaio` with `direct=1` (`O_DIRECT`), bypassing the host page cache.

## Workload

Dedicated files: **128 GiB for reads**, **64 GiB for writes**, on the same SSD. Both were fully initialized with sequential 1 MiB writes and an end-of-initialization fsync. No reads from sparse or unwritten disk regions. Results/logs were written to the root EBS volume.

| Test | Matrix |
|---|---|
| Random reads | 4, 16, 64, 256 KiB; 1, 4 MiB × QD1 and QD32 |
| Sequential writes | Same block sizes × QD1 and QD32 |
| Sequential reads during sequential writes | Same read sizes at QD32; separate writer using 1 MiB blocks, up to QD32 |
| Concurrent write settings | No writer; 180, 450, 910, 1,370, 1,640 MB/s caps; uncapped |

QD is maximum outstanding requests per stream. Each stream is one fio job. Mixed reader/writer jobs run simultaneously. Streams wrap at file boundaries; **no random writes were performed**.

The five capped rates were selected as 10%, 25%, 50%, 75%, and 90% of a standalone write calibration, rounded down to multiples of 10 MB/s. Calibration measured **1,829.069236 MB/s**. A rate cap is not guaranteed achieved throughput; the report includes actual write rates.

Other fio settings: `thread=1`, `numjobs=1`, `invalidate=1`, `fallocate=none`, `allow_file_create=0`, `refill_buffers=1`, `randrepeat=0`, `norandommap=1`, `time_based=1`. Random reads permit revisits; random offsets are not repeated deterministically.

## Timing and validation

- **66 main cases:** 5-second warmup followed by 30 seconds of measurement; executed serially in an order shuffled with seed `20260919`.
- **Four repeats:** 5-second warmup + 60-second measurement, after recreating and initializing the files. Cases: 16 KiB reads at write caps 0, 450, 910 MB/s; 64 KiB reads at 910 MB/s.
- The 16 KiB/450 MB/s repeat was 11.2% faster than the original; the other repeats were within 1.2%. Repeats are reported separately, not averaged into the main tables.
- Approximately one-second per-job bandwidth logs and `iostat -dxm -y 5` were collected.
- All 75 fio outputs completed without errors: 66 main cases, four repeats, one calibration, four initialization passes. Runtime and initialization-size checks passed.
- The report and CSV were regenerated from raw JSON and matched exactly.
- Dedicated test files were deleted after completion. Existing data was not modified.

## Units and limitations

- **MB/s = fio `bw_bytes / 1,000,000`**, not fio's KiB/s `bw` field. Block sizes use binary KiB/MiB.
- CSV p99 latency is completion latency: `clat_ns.percentile["99.000000"] / 1,000,000` milliseconds.
- Measured writes do **not** fsync each operation. This is throughput, not transactional durability latency.
- Results describe these stream counts and queue depths, not proven device-wide maxima. Small-block reads can approach one fio CPU core's limit.
- Application blocks over 128 KiB may be split by the kernel.
- Single-run table cells are not confidence intervals. The SSD was low-occupancy, not full/aged or fully preconditioned for long-term steady-state testing.
- Direct I/O bypasses host page cache, not SSD/controller caches. No storage-content verification workload was run.

## Reproduction

The raw results and original runners are archived **outside this folder**:

- Locally: `~/Development/benchmarks/results/local-ssd-20260919/`
- On the instance: `/home/ubuntu/ssd-bench-results/local-ssd-20260919/`

Use that host, fio 3.36, Python 3, and `iostat` from `sysstat`. Schedule an idle period: this takes approximately 48 minutes and writes several TB, using about 192 GiB of temporary disk space. Do not overlap it with another benchmark.

On the instance, create a fresh result directory on EBS and copy **only the runners**, not old completion markers:

```sh
BASE=/home/ubuntu/ssd-bench-results/local-ssd-20260919
RUN=/home/ubuntu/ssd-bench-results/local-ssd-repro-$(date -u +%Y%m%dT%H%M%SZ)
mkdir "$RUN"
cp "$BASE/run_bench.py" "$BASE/validate.py" "$BASE/summarize.py" "$RUN/"
cd "$RUN"

# Pin the original numeric caps instead of recalculating them from a new calibration.
python3 - <<'PY'
from pathlib import Path
p = Path('run_bench.py')
s = p.read_text()
line, = [v for v in s.splitlines() if v.strip().startswith('rates = [0] + sorted')]
p.write_text(s.replace(line, "            rates = [0, 180, 450, 910, 1370, 1640, 'unlimited']"))
PY

python3 -u run_bench.py > progress.log 2>&1 && \
python3 -u validate.py > validation-progress.log 2>&1
```

Run inside `tmux` or another persistent session. The main runner checks device identity, mount point, and >250 GiB free space. Do not use Python `-O`, which disables its assertions. If device enumeration changes, inspect the actual device before adapting the checks. Do not format the SSD or run the archived `.fio` files directly; their paths refer to the original run.

Success requires `VALIDATION_DONE` and removal of the dedicated data directory. If a run fails, inspect logs and active processes before cleaning up only that run's files.

To regenerate results, run `summarize.py` in the new output directory using Python with `matplotlib==3.10.9` installed. Compare both read throughput and **achieved** concurrent write throughput. The generated CSV contains the 66 main cases; longer repeats appear separately in the report.
