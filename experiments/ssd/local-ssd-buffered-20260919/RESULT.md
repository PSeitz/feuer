# Buffered-I/O screening

14 fio/libaio cases, nominal QD32, buffered I/O. 2 s warmup + 10 s
measurement. Compared with the earlier 30-second direct-I/O sweep.

4-KiB random reads: 57.8 MB/s buffered versus 1,432.8 MB/s direct.
Buffered submission could block, so nominal QD32 did not imply equivalent
concurrency. Direct QD1 reached 59.0 MB/s.

64-KiB writes reported 3,666.6 MB/s but left 21.4 GB for a subsequent
11.71-second fdatasync outside measurement. Short buffered runs measured
page-cache acceptance and deferred writeback, not sustained SSD bandwidth.
