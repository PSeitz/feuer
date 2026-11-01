# Actual latency to complete 500 random 4 KiB reads

Native AIO, O_DIRECT. Each batch contains 500 random 4-KiB reads.
Initialized 8-GiB read and 4-GiB write files. Two rounds, each with
100 warmup and 1,000 measured batches per case. Timing spans first
submission to last completion, excluding setup. Table ranges cover the
two rounds, not confidence intervals. p99 describes whole batches.

| Maximum outstanding reads | Background writes | Mean batch latency ms | Batch p99 ms |
| ---: | --- | ---: | ---: |
| 32 | None | 1.499–1.504 | 1.629–1.636 |
| 64 | None | 1.053–1.056 | 1.242–1.262 |
| 32 | Uncapped 1 MiB writes, QD32 | 3.743–4.073 | 4.928–4.939 |
| 64 | Uncapped 1 MiB writes, QD32 | 2.545–2.607 | 2.971–3.318 |
