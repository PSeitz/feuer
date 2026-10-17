# Same-range compaction: captured metrics

One baseline/candidate pair. Decimal GB. Phase counters exclude opening, while OS/device totals include startup and reporting.
Sampled peaks can miss spikes. Device counters can include other processes. CPU phase counters have one-second resolution.

| Metric | Baseline | Candidate | Change |
| --- | ---: | ---: | ---: |
| DataFile reads GB | 126.381 | 90.9651 | -28.02% |
| DataFile read calls | 30,983 | 12,922 | -58.29% |
| Device reads GB | 126.513 | 91.0589 | -28.02% |
| DataFile writes GB, including observation tail | 105.037 | 101.983 | -2.91% |
| DataFile write calls, including observation tail | 31,783 | 32,407 | +1.96% |
| Device writes GB | 105.132 | 102.076 | -2.91% |
| Source GB | 140.331 | 138.001 | -1.66% |
| Source GETs | 129,592 | 127,076 | -1.94% |
| Memory hits | 1.44435e+06 | 1.46523e+06 | +1.45% |
| Disk/buffer hits | 32,704 | 14,337 | -56.16% |
| Memory returned GB | 2,592.44 | 2,622.66 | +1.17% |
| Disk/buffer returned GB | 67.2415 | 39.3566 | -41.47% |
| Combined request hit rate % | 91.934 | 92.0906 | +0.17% |
| Combined byte hit rate % | 94.9882 | 95.0714 | +0.09% |
| Memory request hit rate % | 89.8985 | 91.1982 | +1.45% |
| Memory byte hit rate % | 92.5867 | 93.6658 | +1.17% |
| CPU seconds during requests | 186 | 207 | +11.29% |
| Average cores during requests | 0.310325 | 0.345361 | +11.29% |
| Sampled peak RSS GB | 107.182 | 108.59 | +1.31% |
| OS maximum RSS GB | 108.344 | 108.69 | +0.32% |
| OS user seconds | 42.84 | 40.48 | -5.51% |
| OS system seconds | 162.95 | 186.6 | +14.51% |
| Process reads GB | 126.513 | 91.0589 | -28.02% |
| Process writes GB | 105.125 | 102.071 | -2.91% |
| Execution seconds | 599.372 | 599.372 | +0.00% |
| Backlog drain seconds | 0.0120995 | 0.0121722 | +0.60% |
| Peak cache charge GB | 100.027 | 100.033 | +0.01% |
| Peak pending payload GB | 9.66992 | 8.65027 | -10.54% |
| Peak unstarted arrivals | 337 | 361 | +7.12% |
| Peak active lookups | 1,100 | 822 | -25.27% |
| Memory eviction-triggering insertions | 13,485 | 5,256 | -61.02% |
| Disk eviction-triggering insertions | 0 | 0 | 0 |
| Disk no capacity | 0 | 0 | 0 |
| Queue-full write skips | 26,904 | 37,655 | +39.96% |
| Final resident charge GB | 99.1592 | 98.4993 | -0.67% |
| Final idle buffers GB | 0.838861 | 1.49524 | +78.25% |
| Tail read GB | 0 | 0 | 0 |
| Tail write GB | 0.0674079 | 0.0663511 | -1.57% |
| Tail CPU seconds | 0 | 0 | 0 |
| Device read await ms | 5.29868 | 5.65352 | +6.70% |
| Device write await ms | 0.187311 | 0.178598 | -4.65% |
| Device average queue | 8.67697 | 6.64964 | -23.36% |
| Device utilization % | 7.89951 | 6.69449 | -15.25% |

## Exact request latencies

Milliseconds. These are exact request distributions, not histogram estimates.

| Metric | Baseline | Candidate | Change |
| --- | ---: | ---: | ---: |
| sender_lateness_mean | 0.0378087 | 0.048334 | +27.84% |
| sender_lateness_p50 | 0.030176 | 0.030364 | +0.62% |
| sender_lateness_p95 | 0.053256 | 0.053304 | +0.09% |
| sender_lateness_p99 | 0.055526 | 0.203815 | +267.06% |
| sender_lateness_max | 3.70805 | 5.99 | +61.54% |
| queue_wait_mean | 1.89407 | 2.45937 | +29.85% |
| queue_wait_p50 | 0.045386 | 0.04742 | +4.48% |
| queue_wait_p95 | 2.14326 | 3.02665 | +41.22% |
| queue_wait_p99 | 66.4223 | 74.5627 | +12.26% |
| queue_wait_max | 430.882 | 469.135 | +8.88% |
| service_mean | 2.92616 | 1.46764 | -49.84% |
| service_p50 | 0.001015 | 0.000996 | -1.87% |
| service_p95 | 0.029632 | 0.009411 | -68.24% |
| service_p99 | 12.7408 | 2.60048 | -79.59% |
| service_max | 1436.55 | 1214.3 | -15.47% |
| scheduled_to_complete_mean | 4.85804 | 3.97534 | -18.17% |
| scheduled_to_complete_p50 | 0.078966 | 0.07887 | -0.12% |
| scheduled_to_complete_p95 | 3.3517 | 3.65962 | +9.19% |
| scheduled_to_complete_p99 | 102.416 | 78.3858 | -23.46% |
| scheduled_to_complete_max | 1508.66 | 1297.1 | -14.02% |

## All scalar metrics and buffer buckets

Counters show request-phase deltas. Gauges show values at request completion. Full tail deltas, peaks and histogram buckets remain in each case's metric-inventory.csv.

| Metric | Baseline | Candidate |
| --- | ---: | ---: |
| `feuer_disk_batch_bytes_total{kind="chunk"}` | 1.0486e+11 | 1.01806e+11 |
| `feuer_disk_batch_bytes_total{kind="payload"}` | 9.64206e+10 | 9.2984e+10 |
| `feuer_disk_chunks{state="allocated"}` | 100064 | 97154 |
| `feuer_disk_chunks{state="free"}` | 376772 | 379682 |
| `feuer_disk_entries` | 95477 | 99680 |
| `feuer_disk_eviction_triggering_insertions_total` | 0 | 0 |
| `feuer_disk_io_bytes_total{operation="read"}` | 1.26381e+11 | 9.09651e+10 |
| `feuer_disk_io_bytes_total{operation="write"}` | 1.0497e+11 | 1.01917e+11 |
| `feuer_disk_io_total{operation="read",outcome="error"}` | 0 | 0 |
| `feuer_disk_io_total{operation="read",outcome="success"}` | 30983 | 12922 |
| `feuer_disk_io_total{operation="write",outcome="error"}` | 0 | 0 |
| `feuer_disk_io_total{operation="write",outcome="success"}` | 31646 | 32273 |
| `feuer_disk_lookup_total{outcome="absent"}` | 129592 | 127076 |
| `feuer_disk_lookup_total{outcome="bypassed"}` | 0 | 0 |
| `feuer_disk_lookup_total{outcome="checksum_failed"}` | 0 | 0 |
| `feuer_disk_lookup_total{outcome="hit"}` | 32704 | 14337 |
| `feuer_disk_lookup_total{outcome="io_error"}` | 0 | 0 |
| `feuer_disk_payload_bytes` | 9.63652e+10 | 9.29285e+10 |
| `feuer_disk_recovered_chunks` | 0 | 0 |
| `feuer_disk_write_entries_total{outcome="already_covered"}` | 260 | 2935 |
| `feuer_disk_write_entries_total{outcome="canceled"}` | 0 | 0 |
| `feuer_disk_write_entries_total{outcome="failed"}` | 0 | 0 |
| `feuer_disk_write_entries_total{outcome="no_capacity"}` | 0 | 0 |
| `feuer_disk_write_entries_total{outcome="published"}` | 99336 | 103704 |
| `feuer_disk_write_pending_bytes` | 3.60007e+07 | 3.63865e+07 |
| `feuer_disk_write_queue_total{outcome="already_covered"}` | 270 | 153 |
| `feuer_disk_write_queue_total{outcome="canceled"}` | 0 | 0 |
| `feuer_disk_write_queue_total{outcome="queue_closed"}` | 0 | 0 |
| `feuer_disk_write_queue_total{outcome="queue_full"}` | 26904 | 37655 |
| `feuer_disk_write_queue_total{outcome="queued"}` | 100117 | 107155 |
| `feuer_disk_write_queue_total{outcome="redundant"}` | 2318 | 2028 |
| `feuer_disk_write_queued_entries` | 0 | 0 |
| `feuer_disk_written_entries_total` | 99589 | 103945 |
| `feuer_io_buffer_pool_bytes{bucket="1 MiB",status="idle"}` | 6.81574e+07 | 4.72908e+08 |
| `feuer_io_buffer_pool_bytes{bucket="1 MiB",status="used"}` | 1.87066e+09 | 1.16287e+09 |
| `feuer_io_buffer_pool_bytes{bucket="12 MiB",status="idle"}` | 7.17226e+08 | 4.27819e+08 |
| `feuer_io_buffer_pool_bytes{bucket="12 MiB",status="used"}` | 1.91134e+10 | 1.43068e+10 |
| `feuer_io_buffer_pool_bytes{bucket="16 MiB",status="idle"}` | 0 | 0 |
| `feuer_io_buffer_pool_bytes{bucket="16 MiB",status="used"}` | 2.68435e+09 | 3.42255e+09 |
| `feuer_io_buffer_pool_bytes{bucket="2 MiB",status="idle"}` | 4.1943e+06 | 5.4526e+07 |
| `feuer_io_buffer_pool_bytes{bucket="2 MiB",status="used"}` | 8.46411e+09 | 4.28658e+09 |
| `feuer_io_buffer_pool_bytes{bucket="24 MiB",status="idle"}` | 0 | 0 |
| `feuer_io_buffer_pool_bytes{bucket="24 MiB",status="used"}` | 1.27339e+10 | 1.07458e+10 |
| `feuer_io_buffer_pool_bytes{bucket="256 KiB",status="idle"}` | 3.40787e+07 | 3.67002e+06 |
| `feuer_io_buffer_pool_bytes{bucket="256 KiB",status="used"}` | 6.04871e+09 | 3.18479e+09 |
| `feuer_io_buffer_pool_bytes{bucket="3 MiB",status="idle"}` | 0 | 0 |
| `feuer_io_buffer_pool_bytes{bucket="3 MiB",status="used"}` | 4.10832e+09 | 3.40682e+09 |
| `feuer_io_buffer_pool_bytes{bucket="32 KiB",status="idle"}` | 524288 | 2.02867e+08 |
| `feuer_io_buffer_pool_bytes{bucket="32 KiB",status="used"}` | 1.56978e+09 | 2.17121e+08 |
| `feuer_io_buffer_pool_bytes{bucket="32 MiB",status="idle"}` | 0 | 0 |
| `feuer_io_buffer_pool_bytes{bucket="32 MiB",status="used"}` | 1.10394e+10 | 1.21803e+10 |
| `feuer_io_buffer_pool_bytes{bucket="4 MiB",status="idle"}` | 0 | 0 |
| `feuer_io_buffer_pool_bytes{bucket="4 MiB",status="used"}` | 1.32959e+09 | 1.46381e+09 |
| `feuer_io_buffer_pool_bytes{bucket="512 KiB",status="idle"}` | 6.29146e+06 | 2.91504e+08 |
| `feuer_io_buffer_pool_bytes{bucket="512 KiB",status="used"}` | 2.78344e+09 | 9.83564e+08 |
| `feuer_io_buffer_pool_bytes{bucket="64 MiB",status="idle"}` | 0 | 0 |
| `feuer_io_buffer_pool_bytes{bucket="64 MiB",status="used"}` | 1.38244e+10 | 1.42942e+10 |
| `feuer_io_buffer_pool_bytes{bucket="8 MiB",status="idle"}` | 8.38861e+06 | 4.1943e+07 |
| `feuer_io_buffer_pool_bytes{bucket="8 MiB",status="used"}` | 1.34386e+10 | 8.5396e+09 |
| `feuer_io_buffer_pool_bytes{bucket=">64 MiB",status="idle"}` | 0 | 0 |
| `feuer_io_buffer_pool_bytes{bucket=">64 MiB",status="used"}` | 1.3993e+08 | 1.3993e+08 |
| `feuer_lookup_bytes_total{source="callback"}` | 1.40331e+11 | 1.38001e+11 |
| `feuer_lookup_bytes_total{source="disk"}` | 6.72415e+10 | 3.93566e+10 |
| `feuer_lookup_bytes_total{source="memory"}` | 2.59244e+12 | 2.62266e+12 |
| `feuer_lookup_total{outcome="callback"}` | 129592 | 127076 |
| `feuer_lookup_total{outcome="callback_error"}` | 0 | 0 |
| `feuer_lookup_total{outcome="disk_hit"}` | 32704 | 14337 |
| `feuer_lookup_total{outcome="invalid_download"}` | 0 | 0 |
| `feuer_lookup_total{outcome="memory_hit"}` | 1.44435e+06 | 1.46523e+06 |
| `feuer_memory_capacity_bytes` | 1e+11 | 1e+11 |
| `feuer_memory_compacted_payload_bytes_total` | 2.31185e+09 | 2.63606e+10 |
| `feuer_memory_entries` | 88339 | 104925 |
| `feuer_memory_eviction_triggering_insertions_total` | 13485 | 5256 |
| `feuer_memory_operations_total{operation="compact"}` | 122 | 93846 |
| `feuer_memory_operations_total{operation="insert"}` | 158609 | 138041 |
| `feuer_memory_operations_total{operation="redundant"}` | 2328 | 2029 |
| `feuer_memory_operations_total{operation="remove"}` | 0 | 0 |
| `feuer_memory_operations_total{operation="replace"}` | 1089 | 1190 |
| `feuer_memory_used_bytes` | 9.99981e+10 | 9.99946e+10 |
| `process_cpu_seconds_total` | 186 | 207 |
| `process_max_fds` | 1024 | 1024 |
| `process_open_fds` | 15 | 15 |
| `process_resident_memory_bytes` | 1.02284e+11 | 1.02974e+11 |
| `process_start_time_seconds` | 1.79227e+09 | 1.79227e+09 |
| `process_threads` | 41 | 42 |
| `process_virtual_memory_bytes` | 1.28504e+11 | 1.51304e+11 |
| `replay_active_requests` | 0 | 0 |
| `replay_completed_total` | 1.60664e+06 | 1.60664e+06 |
| `replay_elapsed_seconds` | 599.042 | 599.04 |
| `replay_phase` | 2 | 2 |
| `replay_queued_requests` | 0 | 0 |
| `replay_source_active_requests` | 0 | 0 |
| `replay_source_bytes_total` | 1.40331e+11 | 1.38001e+11 |
| `replay_source_gets_total` | 129592 | 127076 |
| `replay_source_waiting_requests` | 0 | 0 |

## Histograms

Quantiles are bucket intervals, not exact percentiles. Histogram units follow the metric name.

| Metric | Case | Count | Mean | p50 interval | p95 interval | p99 interval |
| --- | --- | ---: | ---: | --- | --- | --- |
| `feuer_disk_io_duration_seconds{operation="read",outcome="success"}` | baseline | 30983 | 0.03295146465284189 | (0.002048, 0.004096] | (0.131072, 0.262144] | (0.524288, 1.048576] |
| `feuer_disk_io_duration_seconds{operation="write",outcome="success"}` | baseline | 31646 | 0.0013879763169120894 | (0.000128, 0.000256] | (0.004096, 0.008192] | (0.016384, 0.032768] |
| `feuer_disk_lookup_duration_seconds` | baseline | 32704 | 0.13957080259735813 | (0.004096, 0.008192] | (0.524288, 1.048576] | (1.048576, 2.097152] |
| `feuer_disk_read_size_bytes` | baseline | 30983 | 4079034.504082884 | (262144.0, 524288.0] | (16777216.0, 33554432.0] | (33554432.0, 67108864.0] |
| `feuer_disk_write_queue_duration_seconds` | baseline | 100117 | 0.12152797485476746 | (0.002048, 0.004096] | (0.524288, 1.048576] | (1.048576, 2.097152] |
| `feuer_lookup_duration_seconds{outcome="callback"}` | baseline | 129592 | 0.0008729682974566483 | (8e-06, 1.6e-05] | (0.002048, 0.004096] | (0.016384, 0.032768] |
| `feuer_lookup_duration_seconds{outcome="disk_hit"}` | baseline | 32704 | 0.14010338251923318 | (0.004096, 0.008192] | (0.524288, 1.048576] | (1.048576, 2.097152] |
| `replay_source_queue_seconds` | baseline | 129592 | 5.257122353230568e-08 | (0, 0.001] | (0, 0.001] | (0, 0.001] |
| `replay_source_service_seconds` | baseline | 129592 | 0.0006797385047919679 | (0, 0.001] | (0.002, 0.004] | (0.008, 0.016] |
| `feuer_disk_io_duration_seconds{operation="read",outcome="success"}` | candidate | 12922 | 0.05297790931728836 | (0.002048, 0.004096] | (0.262144, 0.524288] | (0.524288, 1.048576] |
| `feuer_disk_io_duration_seconds{operation="write",outcome="success"}` | candidate | 32273 | 0.0012853560843119657 | (0.000128, 0.000256] | (0.004096, 0.008192] | (0.016384, 0.032768] |
| `feuer_disk_lookup_duration_seconds` | candidate | 14337 | 0.15409525634177365 | (0.004096, 0.008192] | (0.524288, 1.048576] | (1.048576, 2.097152] |
| `feuer_disk_read_size_bytes` | candidate | 12922 | 7039549.966800805 | (262144.0, 524288.0] | (33554432.0, 67108864.0] | (33554432.0, 67108864.0] |
| `feuer_disk_write_queue_duration_seconds` | candidate | 107155 | 0.13105370213802212 | (0.002048, 0.004096] | (0.524288, 1.048576] | (1.048576, 2.097152] |
| `feuer_lookup_duration_seconds{outcome="callback"}` | candidate | 127076 | 0.0010176102084107068 | (4e-06, 8e-06] | (0.002048, 0.004096] | (0.016384, 0.032768] |
| `feuer_lookup_duration_seconds{outcome="disk_hit"}` | candidate | 14337 | 0.15505902392871612 | (0.004096, 0.008192] | (0.524288, 1.048576] | (1.048576, 2.097152] |
| `replay_source_queue_seconds` | candidate | 127076 | 4.9553204381623004e-08 | (0, 0.001] | (0, 0.001] | (0, 0.001] |
| `replay_source_service_seconds` | candidate | 127076 | 0.0007543466746435353 | (0, 0.001] | (0.002, 0.004] | (0.016, 0.032] |

## Cold versus later windows

Nearest one-second snapshots, not exact interval boundaries.

| Case | Nominal seconds | Read GB | Write GB | Memory hits | Disk hits | Source calls |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| baseline | [0, 30] | 0.538346 | 35.569459 | 80378 | 59 | 40433 |
| baseline | [30, 90] | 1.881896 | 14.119887 | 116392 | 607 | 22738 |
| baseline | [90, 180] | 19.441840 | 16.891044 | 274131 | 4886 | 20012 |
| baseline | [180, 300] | 43.943948 | 27.116810 | 247136 | 9777 | 13155 |
| baseline | [300, 599.35991497] | 60.517830 | 11.259924 | 725893 | 17346 | 33248 |
| baseline | [599.35991497, 599.372014515] | 0.056866 | 0.012583 | 419 | 29 | 6 |
| candidate | [0, 30] | 0.170259 | 35.117462 | 80677 | 9 | 40184 |
| candidate | [30, 90] | 0.756105 | 11.807674 | 117182 | 132 | 22423 |
| candidate | [90, 180] | 10.845042 | 17.153196 | 278157 | 1920 | 18952 |
| candidate | [180, 300] | 35.515208 | 25.964888 | 252146 | 5196 | 12726 |
| candidate | [300, 599.35991497] | 43.632380 | 11.861082 | 736646 | 7056 | 32785 |
| candidate | [599.35991497, 599.37208716] | 0.046072 | 0.012583 | 424 | 24 | 6 |
