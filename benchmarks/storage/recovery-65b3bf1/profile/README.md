# Recovery CPU profile — commit 65b3bf1

Profiled the saved `65b3bf1abfe0ee4c872aa4df14931e0f7a7732f3` source snapshot on
`m8g-32cpu-local-ssd`. Three fresh recovery processes per workload; fixture creation
runs separately. Sampling is enabled immediately before recovery and disabled before
verification or teardown, with perf FIFO acknowledgments at both boundaries.

## Finding

The dominant sampled CPU cost for small and medium entries is kernel memory-map lock
contention during allocation for index reconstruction—not metadata decoding.

| Workload | `osq_lock` self CPU samples, three runs | Samples per run |
| --- | --- | --- |
| 1 KiB / 2,097,152 entries | 87.07%, 86.92%, 87.13% | 10,305 / 10,510 / 10,715 |
| 64 KiB / 262,144 entries | 81.98%, 77.13%, 68.79% | 1,221 / 927 / 753 |
| 4 MiB / 16,384 entries | 19.42%, 12.50%, 18.35% | 139 / 104 / 109 |

Representative 1-KiB run (`small-2-children.txt`), inclusive sampled CPU percentages:

```text
DiskEntryIndex::insert                         97.93%
  BTreeMap VacantEntry::insert_entry            96.14%
    malloc                                    96.04%
      __mprotect                              93.84%
        __arm64_sys_mprotect                   93.27%
          do_mprotect_pkey                     93.26%
            down_write_killable                90.22%
              rwsem_down_write_slowpath        87.14%
                osq_lock                      86.94% (86.92% self)
```

`rwsem_spin_on_owner` adds another 2.69% self CPU in that run. These are overlapping
inclusive call-tree percentages, not additive categories. The stack shows malloc-triggered
`mprotect` contending for the process memory-map write lock while rebuilding B-tree indexes.
Recovery launches up to 64 blocking shard tasks concurrently on the 32-core machine.
This explains the large aggregate CPU time despite much shorter wall time, and offers
an explanation for faster repeated opens in a process with already-used allocator state.
The latter is an inference, not an isolated experiment.

The 4-MiB workload has too few samples for precise attribution; its profile is spread
across kernel memory/page handling, locking and recovery code. Do not interpret it as
having the same dominant bottleneck as the smaller-entry workloads.

Next experiment: limit simultaneous shard recovery and compare fresh-process wall/CPU
time. No production changes or concurrency experiments were performed for this profile.
CPU sampling does not measure off-CPU I/O wait, so percentages are not fractions of wall time.

## Method and artifacts

- Release build with `RUSTFLAGS='-C force-frame-pointers=yes'` for usable stack traces.
- `perf record -F 499 -e cpu-clock:uk --call-graph fp --delay=-1` and control/ack FIFOs.
- Perf runs privileged to resolve kernel symbols; benchmark runs as the normal user.
- Self and inclusive reports use `sudo -n perf report -f --stdio --no-inline`.
- All nine recovery verifications passed, zero read errors; reports show zero lost samples.
- Fixtures were removed after each workload.
- `benchmark.rs` and `run.sh`: injected harness and recording commands, updated to the current
  `DiskCache` and `disk_cache` names. Raw profiles and logs retain the measured commits' symbols.
- `*-self.txt`, `*-children.txt`: resolved reports; `*.log`: setup, build and recording logs.
- Raw `.data`, exact profile executable and isolated source remain on the host under
  `/mnt/local-ssd/feuer-recovery-retest-65b3bf1/profile/`.

Build the isolated snapshot with the injected module as described in the parent benchmark
report, adding the frame-pointer flag above, then copy the resulting executable to the
profile directory's `benchmark` path and run `bash run.sh` on the host.

Profiled wall times (median): 737.589 / 115.927 / 38.746 ms. Sampling and the frame-pointer
build perturb timing: use the parent report's unprofiled timings for performance estimates.

Perf's DWARF call-graph recording failed on this host with software-event sampling, so
frame-pointer stacks were used instead. Initial unprivileged reports could not resolve
kernel symbols; the saved reports were regenerated privileged. Some libc frames remain
unnamed; the named malloc/mprotect and kernel-lock chain is still visible.
