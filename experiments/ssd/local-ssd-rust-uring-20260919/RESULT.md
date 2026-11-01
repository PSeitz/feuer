# Rust AIO and io_uring replication

Independent Rust implementation, direct I/O, same 66-case matrix per
backend. 5 s warmup + 30 s per case, selected longer repeats and adjacent
fio controls. Backend phases ran sequentially.

At 1 MiB/QD32, both Rust backends reached approximately 3,879 MB/s random
reads and 1,829 MB/s sequential writes. At 4 KiB/QD32, io_uring changed
throughput by +0.3% for reads and -1.0% for writes versus Rust AIO.

The large-transfer plateau reproduced. No general engine speedup appeared.
Buffer generation, instrumentation, and phase timing differed across tests.
