# Mixed small and large reads

fio/libaio, direct I/O. QD32 reader selects 4-KiB and 1-MiB requests
50/50 by count, approximately 99.61% large-read bytes. Separate 1-MiB
writer QD32. Two shuffled rounds, 3 s warmup + 15 s per case.

Read-only throughput was 3,879 MB/s, with 4-KiB p99 of 5.54–5.55 ms.
Uncapped writes achieved 1,195–1,201 MB/s, while reads fell to about
3,244 MB/s and 4-KiB p99 rose to 6.82–6.84 ms.

Small reads can have substantial latency alongside large reads even
without writes. These short screens are not sustained-load estimates.
