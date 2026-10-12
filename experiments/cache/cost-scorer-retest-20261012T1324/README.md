# Corrected-scheduler scorer retest

Two retained same-host pairs use the required constants: **100,000,000,000 memory bytes and 500,000,000,000 disk bytes**. Lanes 03–10 used incorrect capacities and were deleted at the user's request, including their raw artifacts. They are not evidence for this comparison.

Both revisions use identical corrected one-task-per-arrival scheduling. Both tiers use fixed equivalent bytes 256000 and per-byte cost 1, preserving the original scoring formula. This is not a test of differentiated tier coefficients.

Each case uses a fresh cache, a 600-second trace prefix, 32 runtime threads, 64 shards, immediate independent zero-filled miss downloads, and a five-second observation tail. No measurement repetitions ran. See [source identities and validation](settings.json) and the lane reports for the captured metrics.

## Results

Hit rates are caller-weighted. Scheduled p99 is the all-request scheduled-completion percentile, not disk-hit latency. B → C means baseline → candidate.

| Lane | Workload | Combined request hit %, B → C | Combined byte hit %, B → C | Scheduled p99 ms, B → C | CPU seconds, B → C | Disk eviction-triggering insertions, B → C |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| [01](lane-01/README.md) | Full objects | 92.1604 → 92.1654 | 95.3593 → 95.3593 | 2.755 → 3.238 | 43 → 44 | 0 → 0 |
| [02](lane-02/README.md) | Ranges | 92.2062 → 92.2141 | 95.2483 → 95.2754 | 4.019 → 4.585 | 64 → 66 | 0 → 0 |

No improvement is established. Both pairs measured slightly higher CPU and scheduled p99 with the candidate. One pair per workload does not establish significance, equivalence, or scorer causation. Concurrent admission schedules can differ despite identical scoring arithmetic. These runs did not exercise disk eviction.

## Validation and limits

All four retained cases exited 0, completed 1,606,645 requests, and returned 2,800,015,397,526 bytes each. Lookup, download, and I/O errors and failed/canceled writes were zero. Active and queued lookups and pending disk writes settled to zero.

Both source snapshots passed storage-library tests, selected cache/replay tests, strict Clippy, and release builds on the designated Linux SSD host. Source, binary, and input identities are retained with each lane's evidence.

Synthetic immediate misses do not reproduce source latency. Useful payload accounting is not RSS. Sampled peaks can miss spikes, and CPU phase counts have one-second resolution. The observation tail does not guarantee flushing or durability. No range-versus-full-object efficacy claim is made.

Each lane report includes hit rates, CPU, memory accounting, latency distributions, histogram intervals, disk admission and I/O, device observations, and startup/tail windows. Raw evidence remains ignored under `raw/agents/lane-01/` and `raw/agents/lane-02/`.

All experiment instances were terminated and independent Terraform states emptied before deletion. Shared hosts and other experiments were not affected.
