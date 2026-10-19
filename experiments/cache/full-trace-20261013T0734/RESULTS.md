# Full-trace baseline results

**Validation blocked for range disk costs.** Exactly one run per mode. Both replay processes exited zero and completed every request. Full-object validation passes. Range validation rejects **8,191,403 pending payload bytes after the five-second tail**, so its complete disk drain/costs cannot be claimed. Range wrapper exit is 1 from this validation failure, not from the replay. Full-object wrapper exit is 0. No measurement was rerun.

**17,369,893 requests and 31,592,172,620,377 returned bytes per mode.** Arrival span: **2,694.824824702 seconds**. No prefix limit, sampling, repetitions, parameter search, or source-code changes. Same pinned binary/input, baseline scoring, decimal 100-GB memory / 500-GB disk, and 48-slot simulated source as the earlier experiment.

Full-object mode means one object per exact recorded range, not whole original files. The loader joined 17,346,171 recorded outcomes and retained 23,722 unmatched starts. All starts, including recorded failures/incomplete operations, receive successful synthetic downloads when required. This is not a reconstruction of production cancellation or real S3 behavior.

## Range validation blocker

Range reports the same **8,191,403 pending payload bytes** at request finish and after the tail, with zero queued disk-write entries and no tail DataFile I/O. Its admitted-write count exceeds the captured terminal-outcome total by 198 entries. The capture cannot distinguish deferred work from a telemetry/accounting discrepancy. The binary has exited, so further waiting cannot produce a drained snapshot. No flush, modified binary, longer-tail rerun, or reporting override was introduced. **Range I/O below is captured work only, not validated complete disk cost.** Request counts/bytes and error counters pass all other maintained checks. Full-object pending payload falls from 21,268,178 bytes at request finish to zero after the tail.

## Caller hits and returned bytes

| Metric | Range | Full objects |
| --- | ---: | ---: |
| Memory requests | 12,588,947 | 12,706,925 |
| Disk/buffered requests | 351,837 | 190,305 |
| Source requests | 4,429,109 | 4,472,663 |
| memory caller hit % | 72.475674 | 73.154884 |
| disk caller hit % | 2.025557 | 1.095603 |
| Combined caller hit % | 74.501230 | 74.250486 |
| memory returned bytes | 25,505,260,947,488 | 25,698,264,292,764 |
| disk returned bytes | 602,541,578,433 | 403,408,364,496 |
| callback returned bytes | 5,484,370,094,456 | 5,490,499,963,117 |
| memory byte hit % | 80.732849 | 81.343770 |
| disk byte hit % | 1.907250 | 1.276925 |
| Combined byte hit % | 82.640098 | 82.620695 |
| Conditional disk caller hit % | 7.359150 | 4.081199 |
| Conditional disk byte hit % | 9.898970 | 6.844497 |

Caller rates use all requests, byte rates use all returned bytes. Conditional disk rates exclude memory hits. Disk-returned bytes can include buffered hits and need not equal DataFile read bytes.

## Disk and source work

| Metric | Range | Full objects |
| --- | ---: | ---: |
| DataFile read calls | 301,248 | 182,985 |
| DataFile read bytes | 1,883,344,949,211 | 403,316,080,765 |
| DataFile read errors | 0 | 0 |
| DataFile write calls | 797,188 | 840,051 |
| DataFile write bytes | 1,302,173,749,248 | 1,275,548,585,984 |
| DataFile write errors | 0 | 0 |
| Total DataFile transfer bytes | 3,185,518,698,459 | 1,678,864,666,749 |
| Source GETs | 4,429,109 | 4,472,663 |
| Source bytes | 5,484,370,094,456 | 5,490,499,963,117 |
| Memory eviction-triggering insertions | 184,852 | 152,201 |
| Disk eviction-triggering insertions | 25,222 | 23,200 |
| Write admission: queued | 834,096 | 851,572 |
| Write admission: queue_full | 3,087 | 204 |
| Write admission: queue_closed | 0 | 0 |
| Write admission: canceled | 0 | 0 |
| Write admission: already_covered | 3,560,487 | 3,596,174 |
| Write admission: redundant | 31,439 | 24,713 |
| Write terminal: published | 827,309 | 846,855 |
| Write terminal: already_covered | 2,058 | 10 |
| Write terminal: no_capacity | 4,531 | 4,707 |
| Write terminal: failed | 0 | 0 |
| Write terminal: canceled | 0 | 0 |
| Tail read calls | 0 | 0 |
| Tail read bytes | 0 | 0 |
| Tail write calls | 0 | 147 |
| Tail write bytes | 0 | 67,448,832 |
| Pending payload after tail | 8,191,403 | 0 |
| Successfully written batch payload bytes | 1,194,273,330,688 | 1,169,323,946,172 |
| Successfully written batch chunk bytes | 1,299,954,270,208 | 1,273,135,890,432 |
| Batch payload / chunk bytes % | 91.870411 | 91.845965 |
| Entries in written payload regions | 829,346 | 846,855 |
| Settled disk payload_bytes | 374,129,238,193 | 375,802,496,105 |
| Settled disk entries | 207,218 | 251,756 |
| Settled disk allocated_chunks | 396,639 | 405,742 |
| Settled disk free_chunks | 80,198 | 71,095 |

DataFile counts are API operations, not device requests. Counters subtract the pre-request snapshot and include the five-second tail. The tail is an observation, not a forced drain, flush, or durability guarantee.

### Disk eviction timing limits retention conclusions

Disk eviction totals mostly describe the source drain, not the arrival window. The first sampled range disk eviction is at **2,614.508 seconds (43m 34.5s)**, with only **15 eviction-triggering insertions** in the nearest sample before the last arrival at 2,694.825 seconds. Full-object disk eviction first appears at **3,563.641 seconds (59m 23.6s)**, after arrivals have ended, and its nearest last-arrival sample has zero disk evictions. Memory eviction-triggering insertions at those last-arrival samples are 84,115 range and 55,477 full-object.

The full trace therefore exercises substantial memory eviction and disk reads during arrivals, but still offers little evidence about disk-retention tuning. Do not treat the final 25,222 / 23,200 disk-eviction counts as sustained disk pressure during caller demand. Evidence: `raw/eviction-timing.json`, derived from the original timestamped metrics without another replay.

## Runtime, CPU, memory charge and backlog

| Metric | Range | Full objects |
| --- | ---: | ---: |
| execution seconds | 11,485.689002 | 11,578.388622 |
| backlog drain seconds | 8,790.864177 | 8,883.563797 |
| cpu seconds during requests | 2,535 | 928 |
| average cores during requests | 0.220709 | 0.080149 |
| cpu seconds in observation tail | 0 | 0 |
| source queue mean seconds | 3,912.253448 | 3,955.023505 |
| source service mean seconds | 0.124447 | 0.124238 |
| OS user_seconds | 672.080000 | 493.510000 |
| OS system_seconds | 1,880.290000 | 454.250000 |
| OS max_rss_kib | 92,489,204 | 65,774,420 |
| OS file_inputs | 3,679,607,008 | 788,434,512 |
| OS file_outputs | 2,544,056,128 | 2,492,058,944 |
| Sampled peak replay_queued_requests | 3,629 | 1,501 |
| Sampled peak replay_active_requests | 3,389,731 | 3,429,631 |
| Sampled peak process_resident_memory_bytes | 91,362,254,848 | 67,125,440,512 |
| Sampled peak feuer_memory_used_bytes | 100,028,935,185 | 99,999,997,170 |
| Sampled peak feuer_disk_write_pending_bytes | 10,090,473,028 | 8,428,619,404 |
| Sampled peak replay_source_waiting_requests | 3,389,683 | 3,429,583 |
| Sampled peak replay_source_active_requests | 48 | 48 |
| finished memory used_bytes | 99,926,833,094 | 99,937,140,217 |
| finished memory idle_bytes | 928,514,048 | 881,852,416 |
| finished memory resident_charge_bytes | 98,998,319,046 | 99,055,287,801 |
| settled memory used_bytes | 99,926,833,094 | 99,937,140,217 |
| settled memory idle_bytes | 928,514,048 | 881,852,416 |
| settled memory resident_charge_bytes | 98,998,319,046 | 99,055,287,801 |

`used_bytes` is cached allocation charge plus idle buffer capacity. `resident_charge_bytes` is charge minus idle, **not payload occupancy**. Zero-filled simulated payloads can consume much less RSS than their charge. Sampled peaks can miss short spikes. Request-phase CPU has one-second resolution. OS totals include trace loading and final reporting.

## Exact request latency distributions

All values below are seconds, from individual request observations, not histogram interpolation.

| Mode | Latency | Mean | p50 | p95 | p99 | Maximum |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| range | sender_lateness | 0.000046236 | 0.000032239 | 0.000053496 | 0.000291322 | 0.014215274 |
| range | queue_wait | 0.001000356 | 0.000020916 | 0.002087050 | 0.029056065 | 0.600848779 |
| range | service | 997.616836316 | 0.000001140 | 6713.035550635 | 8380.676924040 | 8790.895948516 |
| range | scheduled_to_complete | 997.617882909 | 0.000071175 | 6713.035615589 | 8380.676986821 | 8790.895974241 |
| full_objects | sender_lateness | 0.000032314 | 0.000031750 | 0.000053314 | 0.000054414 | 0.004475090 |
| full_objects | queue_wait | 0.000165217 | 0.000018042 | 0.000519256 | 0.002038664 | 0.405341182 |
| full_objects | service | 1018.431952219 | 0.000001383 | 6803.176480820 | 8473.261221756 | 8883.595554848 |
| full_objects | scheduled_to_complete | 1018.432149750 | 0.000066761 | 6803.176545124 | 8473.261260741 | 8883.595595565 |

## Whole-device I/O

| Metric | Range | Full objects |
| --- | ---: | ---: |
| reads | 14,527,083 | 3,182,767 |
| read_bytes | 1,883,958,788,096 | 403,678,470,144 |
| writes | 10,482,288 | 10,327,794 |
| write_bytes | 1,304,231,927,808 | 1,278,066,204,672 |
| aqu-sz | 5.961439 | 1.003055 |
| %util | 6.324498 | 4.008978 |
| r_await_ms_request_weighted | 4.594448 | 3.146058 |
| w_await_ms_request_weighted | 0.175737 | 0.157208 |

Device counters can include other processes and differ from DataFile calls. `pidstat.txt` retains process/thread I/O rates, `time.txt` retains whole-process filesystem block counts, and `iostat.txt` retains per-second device samples.

## Cold, later arrivals and drain

Nearest one-second samples define approximate windows. Cold is 0–180 s, later arrivals are 180–2,694.824824702 s, and drain begins at the last recorded arrival. Final totals above are exact. Callback outcomes in windows count completed lookups, not source starts.

| Mode | Phase | Nominal seconds | Memory hits | Disk/buffered hits | Callbacks | Read GB | Write GB | Unstarted / active at end |
| --- | --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| range | cold | 0–30 | 27,371 | 0 | 10,616 | 0.000000 | 13.047599 | 0 / 82,883 |
| range | cold | 30–90 | 69,655 | 0 | 21,636 | 0.000000 | 37.714854 | 0 / 131,329 |
| range | cold | 90–180 | 178,634 | 4 | 32,841 | 0.073764 | 20.738568 | 0 / 218,811 |
| range | later_arrivals | 180–600 | 812,379 | 1,099 | 161,101 | 4.184555 | 26.438402 | 0 / 291,309 |
| range | later_arrivals | 600–900 | 427,356 | 2,344 | 118,456 | 29.065824 | 32.515846 | 0 / 298,673 |
| range | later_arrivals | 900–1800 | 3,396,869 | 68,584 | 365,523 | 501.894604 | 46.548423 | 0 / 989,941 |
| range | later_arrivals | 1800–2694.82 | 7,673,456 | 279,806 | 328,784 | 1348.126202 | 184.507265 | 0 / 3,389,240 |
| range | drain | 2694.82–11485.7 | 3,227 | 0 | 3,390,152 | 0.000000 | 940.662792 | 0 / 0 |
| full_objects | cold | 0–30 | 27,380 | 0 | 10,615 | 0.000000 | 13.049360 | 0 / 82,875 |
| full_objects | cold | 30–90 | 69,667 | 0 | 21,643 | 0.000000 | 37.722243 | 0 / 131,302 |
| full_objects | cold | 90–180 | 178,290 | 0 | 32,846 | 0.000000 | 20.734194 | 0 / 219,127 |
| full_objects | later_arrivals | 180–600 | 812,454 | 53 | 161,111 | 0.276398 | 26.465358 | 0 / 292,586 |
| full_objects | later_arrivals | 600–900 | 428,022 | 273 | 118,472 | 2.821522 | 32.494506 | 0 / 301,339 |
| full_objects | later_arrivals | 900–1800 | 3,426,294 | 25,202 | 366,986 | 65.770892 | 46.624068 | 0 / 1,005,101 |
| full_objects | later_arrivals | 1800–2694.82 | 7,761,434 | 164,777 | 330,962 | 334.447268 | 192.667660 | 0 / 3,428,935 |
| full_objects | drain | 2694.82–11578.4 | 3,384 | 0 | 3,430,028 | 0.000000 | 905.723748 | 0 / 0 |

## Earlier first-ten-minute baselines

These are **different workload durations**, not paired improvements or regressions. The earlier captures selected 1,606,645 requests and returned 2,800,015,397,526 bytes. The full trace has 10.81 times as many requests and 11.28 times as many bytes. Do not attribute longer-run differences solely to cache warmup or the scorer.

| Mode / duration | Requests | Caller hit % | Byte hit % | Read GB | Write GB | Source GB | Source GETs | Execution / drain s | Disk eviction insertions |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| range / first 10 minutes | 1,606,645 | 67.791205 | 80.792555 | 4.044885 | 141.564887 | 537.811422 | 517,481 | 1315.359125 / 715.999210 | 0 |
| range / full trace | 17,369,893 | 74.501230 | 82.640098 | 1883.344949 | 1302.173749 | 5484.370094 | 4,429,109 | 11485.689002 / 8790.864177 | 25,222 |
| full_objects / first 10 minutes | 1,606,645 | 67.706930 | 80.790253 | 0.283839 | 141.593805 | 537.875869 | 518,835 | 1318.425949 / 719.066034 | 0 |
| full_objects / full trace | 17,369,893 | 74.250486 | 82.620695 | 403.316081 | 1275.548586 | 5490.499963 | 4,472,663 | 11578.388622 / 8883.563797 | 23,200 |

## All metric families and evidence

Each case has `raw/cases/MODE/METRICS.md`, `metric-inventory.csv`, and `histograms-and-windows.json`. These report every captured metric family, including all buffer-size buckets, disk chunks/payload, memory operations, batch payload/packing, stale/discard outcomes, lookup/source/write-queue/I/O histogram counts and means, and p50/p95/p99 **bucket intervals**, not exact histogram percentiles. `metrics.prom` retains every original sample. `raw/metric-coverage.json` independently confirms all **344 sampled series across 46 families per mode** appear in the inventories.

Validation checks exactly-once completion, all returned bytes, pinned hashes, command and effective settings, zero lookup/I/O/write errors, and zero queued/active/source/pending-payload work after drain and tail. **The pending-payload check fails for range only and is preserved, not waived.** Both modes have zero remaining caller/source work and no captured errors. No measurements were repeated to recover reporting. The original validation rejection is in `raw/coordinator-completion.json`. Operational history remains under `raw/`, including `raw/curation-originals/`. See [README.md](README.md#historical-source-and-reproduction) for the source identity and reproduction requirements.

One run per mode cannot establish small performance differences. The two modes are different lookup semantics, not revisions.
