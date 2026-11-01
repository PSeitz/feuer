# Writer concurrency sweep

fio/libaio, direct I/O. Random reader QD32 with uncapped sequential
1-MiB writes. 5 s warmup + 30 s per case, selected longer repeats.

Write-only QD1 and QD32 both reached about 1,829 MB/s. Write p99 rose
from 0.461 to 24.248 ms.

With 16-KiB random reads, write QD1 and QD32 yielded 1,558 and
1,561 MB/s read throughput, versus 3,821 MB/s without writes.
Reducing writer concurrency alone did little to restore read throughput:
the writer still saturated its available bandwidth. Rate caps are a
separate control from concurrency limits.
