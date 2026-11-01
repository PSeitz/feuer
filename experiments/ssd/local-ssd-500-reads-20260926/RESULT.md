# Actual latency to complete 500 random 4 KiB reads

- **Workload:** batches of 500 random 4 KiB reads, allowing 32 or 64 reads in flight.
- **Measurement:** time to complete the entire batch, from first submission to last completion.
- **Background writes:** sequential 1 MiB requests, with up to 32 writes in flight and no rate limit.
- **Tool:** native Linux AIO with direct I/O.
- **Repetitions:** two rounds of 100 warmup batches and 1,000 measured batches per case. Table ranges span the two rounds.

| Maximum outstanding reads | Background writes | Mean batch latency ms | Batch p99 ms |
| ---: | --- | ---: | ---: |
| 32 | None | 1.499–1.504 | 1.629–1.636 |
| 64 | None | 1.053–1.056 | 1.242–1.262 |
| 32 | Uncapped 1 MiB writes, QD32 | 3.743–4.073 | 4.928–4.939 |
| 64 | Uncapped 1 MiB writes, QD32 | 2.545–2.607 | 2.971–3.318 |
