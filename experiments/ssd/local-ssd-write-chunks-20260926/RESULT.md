# Write request sizes under mixed reads

fio/libaio, direct I/O. QD32 reader mixes 4-KiB/1-MiB requests 50/50
by count. Compare uncapped 1-MiB/QD1 writes with 64-KiB writes at QD1
or QD16, using batched or rolling submission. Two shuffled rounds,
3 s warmup + 15 s per case.

Across writer configurations, reads reached 3,241–3,314 MB/s, writes
1,157–1,192 MB/s, and 4-KiB read p99 was 6.62–6.85 ms.

Splitting 1-MiB writes into 64-KiB requests showed no consistent benefit
in this short screening. Read-only 4-KiB p99 was about 5.53–5.54 ms.
