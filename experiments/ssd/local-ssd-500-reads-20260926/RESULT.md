# Completing 500 random small reads

Native AIO, direct I/O, 500 random 4-KiB reads per batch. Two rounds,
100 warmup and 1,000 measured batches per case. Timing spans first
submission to last completion, excluding setup.

| Read QD | Mean batch ms, no writer | Mean batch ms, uncapped writer |
|---|---:|---:|
| 32 | 1.499–1.504 | 3.743–4.073 |
| 64 | 1.053–1.056 | 2.545–2.607 |

Writer: sequential 1-MiB requests, QD32. At read QD32, batch p99 rose
from 1.629–1.636 to 4.928–4.939 ms. Ranges cover two rounds, not
confidence intervals. This excludes cache lookup and Tokio overhead.
