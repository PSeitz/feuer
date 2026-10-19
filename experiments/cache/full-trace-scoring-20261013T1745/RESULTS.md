# Full-trace scoring: OOM failure report

Question: can request-cost weighting or shorter demand-history half-lives reduce source and disk traffic on the full trace when source downloads use cache-pool buffers?

**All 16 cases were kernel OOM-killed (replay and wrapper exit 137).** No case reached the last trace arrival, full drain, or the five-second observation tail. **There is no complete baseline and no valid scoring comparison or winner.** This report preserves partial telemetry only, not full-trace performance results.

Exactly one wave tested eight configurations per mode, with one observation per configuration. [settings.json](settings.json) records all scorer values, capacities, source settings and binary/input identities. No failed measurement was rerun.

## Frozen build and workload

- Frozen source archive SHA-256: `a2075db34c972befa7ab4bfea1f63cbea3d7538b40d5132d783e421c594cad34`.
- Replay source SHA-256: `711487d5a72891f5c96ca3c315de9851822d1543ef0301de1743f5a326e27f3c`.
- New release binary SHA-256: `4be4d5d412cef54eb70d4caa448707d60ca88b41599eca12a0fc67c7c501126a`. The old binary was not used.
- Every live start verified `source_buffer_pool=1`, decimal 100-GB memory / 500-GB disk, 32 runtime threads, 48 source slots and the exact scorer settings. All workers used matched jemalloc. Fourteen workers were in us-east-1a, two missing slots moved to us-east-1b after EC2 startup failures. No measurement was rerun.
- Full trace selected: 17,369,893 requests, 31,592,172,620,377 requested returned bytes, 2,694.824824702-second arrival span. `--trace-seconds` was absent. These are selected workload totals, **not completed totals**.
- Each miss independently acquires and zero-fills the cache pool buffer and preserves actual backing capacity via `into_download`. There is no coalescing or shared merged allocation. Full-object mode means one object per exact recorded range, not whole original files.

## Failure and interpretation

Kernel OOM messages identify each replay PID and agree with `/usr/bin/time` signal 9 and wrapper/replay exit 137. The kernel-reported anonymous RSS was about 131 GB decimal near termination. The 100-GB cache charge is not a process RSS cap. Sampled RSS can miss the final spike. These captures establish memory exhaustion, but do not isolate the additional allocation responsible. The source model changed from the historical exact-length Vec runs, which are not paired controls.

Range cases died around 24–25 minutes into arrivals. Full-object cases died around 41 minutes, before the 44.9-minute last arrival. Neither baseline completed, so percent improvements/regressions would compare unequal incomplete work and are deliberately omitted. Source GETs below count started callbacks, including unfinished ones. Source bytes count buffers allocated after service. Caller hit rates include only completed lookups at the last sample and are completion-biased, not full-trace hit rates. All 16 captures recorded zero disk eviction-triggering insertions before they stopped, so there is no measured disk-eviction pressure to assess.

## Last exported sample before OOM

OS maximum RSS covers the whole process through termination, not only the last exported sample.

| Case | Elapsed s | Completed requests | Source waiting | OS max RSS GB | Pending payload bytes |
| --- | ---: | ---: | ---: | ---: | ---: |
| range-baseline | 1496.241 | 4,002,909 | 635,374 | 131.133 | 36,448,339 |
| range-byte-cost | 1462.257 | 3,900,540 | 616,561 | 131.318 | 9,131,552,446 |
| range-source-cost | 1496.234 | 4,003,626 | 635,357 | 131.303 | 36,567,104 |
| range-half-life-65536 | 1516.231 | 4,142,122 | 652,447 | 131.158 | 10,620,981 |
| range-half-life-16384 | 1516.237 | 4,142,502 | 652,283 | 131.222 | 10,548,377 |
| range-half-life-4096 | 1517.322 | 4,150,865 | 653,480 | 131.243 | 110,693,420 |
| range-source-cost-half-life-16384 | 1517.261 | 4,149,664 | 653,610 | 131.319 | 14,302,104 |
| range-source-cost-half-life-4096 | 1516.265 | 4,142,653 | 652,552 | 131.157 | 10,851,002 |
| full_objects-baseline | 2447.439 | 10,295,137 | 2,425,941 | 131.211 | 32,014,163 |
| full_objects-byte-cost | 2447.457 | 10,295,575 | 2,425,783 | 131.151 | 33,756,783 |
| full_objects-source-cost | 2447.416 | 10,295,097 | 2,425,635 | 131.123 | 33,742,563 |
| full_objects-half-life-65536 | 2447.441 | 10,295,508 | 2,425,588 | 131.117 | 33,012,055 |
| full_objects-half-life-16384 | 2447.436 | 10,295,645 | 2,425,389 | 131.139 | 32,171,239 |
| full_objects-half-life-4096 | 2447.456 | 10,296,032 | 2,425,324 | 131.149 | 32,230,034 |
| full_objects-source-cost-half-life-16384 | 2447.448 | 10,295,509 | 2,425,652 | 131.259 | 33,755,093 |
| full_objects-source-cost-half-life-4096 | 2447.441 | 10,295,404 | 2,425,692 | 131.264 | 33,755,093 |

## range: partial source and disk work

**Not comparable complete costs.** Values stop at different failure times. Counter deltas exclude opening. No settled snapshots exist, and work remained outstanding in every case.

| Setting | Source bytes | Source GETs started | Read bytes | Read calls | Write bytes | Write calls | Total disk bytes |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| baseline | 594,814,503,840 | 1,223,287 | 469,190,317,303 | 74,796 | 165,703,237,632 | 53,182 | 634,893,554,935 |
| byte-cost | 582,999,786,835 | 1,189,961 | 753,340,964,655 | 299,874 | 152,657,305,600 | 50,762 | 905,998,270,255 |
| source-cost | 594,788,584,165 | 1,223,216 | 500,827,004,208 | 32,372 | 165,697,847,296 | 53,136 | 666,524,851,504 |
| half-life-65536 | 600,009,969,599 | 1,248,349 | 557,602,198,285 | 114,510 | 165,917,818,880 | 53,510 | 723,520,017,165 |
| half-life-16384 | 600,070,682,765 | 1,248,282 | 626,313,257,472 | 140,675 | 165,940,850,688 | 53,504 | 792,254,108,160 |
| half-life-4096 | 600,263,088,737 | 1,249,614 | 674,385,772,121 | 157,771 | 165,954,576,384 | 53,530 | 840,340,348,505 |
| source-cost-half-life-16384 | 600,069,578,590 | 1,249,573 | 597,786,253,530 | 112,107 | 165,923,135,488 | 53,528 | 763,709,389,018 |
| source-cost-half-life-4096 | 599,990,408,349 | 1,248,431 | 656,043,844,039 | 145,713 | 165,882,810,368 | 53,392 | 821,926,654,407 |

### range: all captured summary metrics

| Metric | baseline | byte-cost | source-cost | half-life-65536 | half-life-16384 | half-life-4096 | source-cost-half-life-16384 | source-cost-half-life-4096 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| CPU seconds through last sample | 692 | 1,222 | 702 | 800 | 845 | 944 | 871 | 903 |
| Average cores through last sample | 0.462492 | 0.835694 | 0.469178 | 0.527624 | 0.557301 | 0.622149 | 0.574061 | 0.595542 |
| Completed memory_hit requests | 3,332,615 | 3,007,135 | 3,376,503 | 3,418,320 | 3,391,811 | 3,382,089 | 3,429,304 | 3,386,930 |
| Completed disk_hit requests | 82,429 | 320,053 | 39,312 | 127,948 | 154,740 | 172,690 | 124,452 | 159,892 |
| Completed callback requests | 587,865 | 573,352 | 587,811 | 595,854 | 595,951 | 596,086 | 595,915 | 595,831 |
| Completed callback_error requests | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| Completed invalid_download requests | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| Completed memory returned bytes | 6,854,545,218,394 | 6,647,301,658,415 | 6,791,819,743,906 | 7,016,825,631,894 | 6,992,248,252,223 | 6,999,002,541,241 | 6,998,284,468,351 | 6,981,507,933,903 |
| Completed disk returned bytes | 179,572,923,816 | 137,737,213,792 | 239,283,963,478 | 239,824,799,194 | 267,428,703,022 | 281,720,601,990 | 282,384,217,411 | 280,836,775,953 |
| Completed callback returned bytes | 594,814,503,840 | 582,999,786,835 | 594,788,584,165 | 600,009,969,599 | 600,070,682,765 | 600,263,088,737 | 600,069,578,590 | 599,990,408,349 |
| Completed memory request hit % | 83.254828 | 77.095351 | 84.336124 | 82.525816 | 81.878319 | 81.479137 | 82.640383 | 81.757511 |
| Completed disk request hit % | 2.059227 | 8.205351 | 0.981910 | 3.088948 | 3.735424 | 4.160338 | 2.999081 | 3.859652 |
| Completed memory byte hit % | 89.849335 | 90.218062 | 89.062624 | 89.310537 | 88.962758 | 88.808714 | 88.802397 | 88.796876 |
| Completed disk byte hit % | 2.353841 | 1.869388 | 3.137783 | 3.052503 | 3.402510 | 3.574687 | 3.583220 | 3.571926 |
| Last replay_queued_requests | 49 | 0 | 51 | 0 | 0 | 0 | 0 | 0 |
| Last replay_active_requests | 637,255 | 617,036 | 636,536 | 653,579 | 653,241 | 654,594 | 654,498 | 653,427 |
| Last replay_source_waiting_requests | 635,374 | 616,561 | 635,357 | 652,447 | 652,283 | 653,480 | 653,610 | 652,552 |
| Last replay_source_active_requests | 48 | 48 | 48 | 48 | 48 | 48 | 48 | 48 |
| Last feuer_memory_used_bytes | 99,882,479,690 | 99,352,706,262 | 99,893,195,477 | 98,669,658,340 | 98,801,615,957 | 99,938,726,324 | 99,648,416,435 | 99,236,682,900 |
| Last process_resident_memory_bytes | 126,557,093,888 | 129,299,877,888 | 126,921,490,432 | 130,236,583,936 | 126,633,775,104 | 130,400,677,888 | 129,441,120,256 | 127,070,576,640 |
| Last feuer_disk_write_pending_bytes | 36,448,339 | 9,131,552,446 | 36,567,104 | 10,620,981 | 10,548,377 | 110,693,420 | 14,302,104 | 10,851,002 |
| Last feuer_disk_write_queued_entries | 0 | 252 | 0 | 0 | 0 | 9 | 0 | 0 |
| Peak replay_queued_requests | 1,385 | 586 | 1,429 | 1,359 | 681 | 1,602 | 954 | 504 |
| Peak replay_active_requests | 637,255 | 617,036 | 636,536 | 653,579 | 653,241 | 654,594 | 654,498 | 653,427 |
| Peak replay_source_waiting_requests | 635,374 | 616,561 | 635,357 | 652,447 | 652,283 | 653,480 | 653,610 | 652,552 |
| Peak replay_source_active_requests | 48 | 48 | 48 | 48 | 48 | 48 | 48 | 48 |
| Peak feuer_memory_used_bytes | 100,033,525,006 | 99,999,995,942 | 100,032,773,373 | 100,050,173,743 | 100,016,515,052 | 100,053,124,678 | 100,025,128,263 | 100,054,419,792 |
| Peak process_resident_memory_bytes | 128,164,978,688 | 129,299,877,888 | 128,499,765,248 | 130,236,583,936 | 129,816,367,104 | 130,400,677,888 | 130,373,664,768 | 129,384,325,120 |
| Peak feuer_disk_write_pending_bytes | 10,664,813,297 | 9,131,552,446 | 9,919,036,335 | 10,670,872,706 | 10,374,637,064 | 11,063,104,374 | 11,352,866,107 | 10,555,557,633 |
| Last idle buffer bytes | 1,438,777,344 | 1,104,052,224 | 980,746,240 | 891,420,672 | 1,063,010,304 | 1,842,413,568 | 1,620,246,528 | 1,127,153,664 |
| Write admission queued | 138,371 | 136,043 | 138,324 | 139,313 | 139,337 | 139,379 | 139,410 | 139,326 |
| Write admission queue_full | 540 | 0 | 525 | 508 | 500 | 465 | 418 | 487 |
| Write admission queue_closed | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| Write admission canceled | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| Write admission already_covered | 446,958 | 436,499 | 446,856 | 454,015 | 454,094 | 454,383 | 454,281 | 453,973 |
| Write admission redundant | 1,996 | 810 | 2,106 | 2,018 | 2,020 | 1,859 | 1,806 | 2,045 |
| Write terminal published | 137,385 | 134,658 | 137,331 | 138,505 | 138,540 | 138,521 | 138,569 | 138,500 |
| Write terminal already_covered | 326 | 305 | 314 | 338 | 336 | 338 | 337 | 345 |
| Write terminal no_capacity | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| Write terminal failed | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| Write terminal canceled | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| Written batch payload | 153,187,621,656 | 140,363,384,485 | 153,192,323,568 | 153,336,299,765 | 153,361,055,599 | 153,371,231,513 | 153,343,698,205 | 153,319,376,172 |
| Written batch chunk | 165,608,947,712 | 152,570,953,728 | 165,603,704,832 | 165,822,857,216 | 165,845,925,888 | 165,859,557,376 | 165,828,100,096 | 165,788,254,208 |
| Written batch written_entries | 137,707 | 134,948 | 137,644 | 138,837 | 138,874 | 138,851 | 138,899 | 138,833 |
| Last disk payload_bytes | 153,103,307,712 | 140,290,882,823 | 153,108,009,624 | 153,251,958,342 | 153,276,714,176 | 153,286,892,763 | 153,259,353,681 | 153,235,151,414 |
| Last disk entries | 125,470 | 126,537 | 125,407 | 126,590 | 126,627 | 126,605 | 126,651 | 126,586 |
| Last disk allocated_chunks | 157,992 | 145,619 | 157,988 | 158,197 | 158,217 | 158,289 | 158,205 | 158,162 |
| Last disk free_chunks | 318,845 | 331,218 | 318,849 | 318,640 | 318,620 | 318,548 | 318,632 | 318,675 |
| memory eviction insertions observed before last arrival | 23,424 | 70,768 | 13,017 | 24,172 | 26,940 | 28,766 | 22,180 | 26,280 |
| disk eviction insertions observed before last arrival | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| memory first eviction sample seconds | 125.924512 | 126.926804 | 125.925435 | 126.923209 | 125.923363 | 126.923917 | 126.926343 | 126.929877 |
| disk first eviction sample seconds | unavailable | unavailable | unavailable | unavailable | unavailable | unavailable | unavailable | unavailable |
| read I/O errors | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| write I/O errors | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| Disk lookup io_error | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| Disk lookup checksum_failed | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| OS user_seconds | 146.180000 | 237.250000 | 136.780000 | 170.780000 | 190.630000 | 199.980000 | 178.620000 | 192.310000 |
| OS system_seconds | 566.090000 | 1,001.530000 | 584.710000 | 644.560000 | 670.360000 | 759.940000 | 709.490000 | 728.970000 |
| OS max_rss_kib | 128,060,052 | 128,240,676 | 128,225,716 | 128,083,772 | 128,146,056 | 128,167,368 | 128,241,684 | 128,082,952 |
| OS file_inputs | 917,664,928 | 1,492,132,328 | 981,411,944 | 1,104,116,560 | 1,236,710,008 | 1,333,126,352 | 1,180,644,792 | 1,293,571,592 |
| OS file_outputs | 323,736,000 | 298,663,360 | 323,725,360 | 324,172,872 | 324,373,552 | 324,535,800 | 324,433,176 | 324,153,688 |
| Device reads | 3,622,960 | 6,058,576 | 3,849,344 | 4,375,040 | 4,908,362 | 5,296,444 | 4,672,758 | 5,134,432 |
| Device read_bytes | 469,842,939,904 | 763,959,103,488 | 502,475,141,120 | 565,306,241,024 | 633,193,553,920 | 682,559,123,456 | 604,437,012,480 | 662,308,331,520 |
| Device writes | 1,288,264 | 1,188,422 | 1,288,130 | 1,290,135 | 1,290,989 | 1,291,655 | 1,291,245 | 1,405,994 |
| Device write_bytes | 165,768,429,568 | 152,929,792,000 | 165,760,737,280 | 165,992,304,640 | 166,094,315,520 | 166,178,009,088 | 166,124,310,528 | 181,185,933,312 |
| Device aqu-sz | 7.903077 | 11.502510 | 8.522277 | 9.107080 | 10.199451 | 10.855538 | 9.696947 | 10.461018 |
| Device %util | 8.438848 | 10.699993 | 8.708418 | 9.517655 | 10.222090 | 10.764866 | 9.880959 | 10.460398 |
| Device r_await_ms_request_weighted | 3.227847 | 2.776172 | 3.280464 | 3.130477 | 3.130161 | 3.094547 | 3.128046 | 3.064867 |
| Device w_await_ms_request_weighted | 0.192447 | 0.153143 | 0.193191 | 0.191530 | 0.194444 | 0.194368 | 0.192022 | 0.213606 |

## full_objects: partial source and disk work

**Not comparable complete costs.** Values stop at different failure times. Counter deltas exclude opening. No settled snapshots exist, and work remained outstanding in every case.

| Setting | Source bytes | Source GETs started | Read bytes | Read calls | Write bytes | Write calls | Total disk bytes |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| baseline | 1,081,181,135,291 | 3,376,950 | 349,376,319,796 | 198,826 | 285,273,059,328 | 90,209 | 634,649,379,124 |
| byte-cost | 1,081,444,233,183 | 3,376,867 | 326,746,247,783 | 817,335 | 285,263,785,984 | 90,240 | 612,010,033,767 |
| source-cost | 1,081,296,995,286 | 3,376,570 | 477,774,385,186 | 105,047 | 285,263,638,528 | 90,204 | 763,038,023,714 |
| half-life-65536 | 1,081,648,240,170 | 3,376,763 | 490,002,404,446 | 331,090 | 285,254,078,464 | 90,165 | 775,256,482,910 |
| half-life-16384 | 1,081,709,261,006 | 3,376,583 | 529,755,582,644 | 408,399 | 285,247,791,104 | 90,160 | 815,003,373,748 |
| half-life-4096 | 1,081,687,169,616 | 3,376,536 | 545,457,341,609 | 447,671 | 285,250,199,552 | 90,238 | 830,707,541,161 |
| source-cost-half-life-16384 | 1,081,578,905,912 | 3,376,731 | 541,029,082,592 | 348,599 | 285,249,990,656 | 90,187 | 826,279,073,248 |
| source-cost-half-life-4096 | 1,081,415,819,653 | 3,376,723 | 550,064,311,889 | 428,356 | 285,256,421,376 | 90,227 | 835,320,733,265 |

### full_objects: all captured summary metrics

| Metric | baseline | byte-cost | source-cost | half-life-65536 | half-life-16384 | half-life-4096 | source-cost-half-life-16384 | source-cost-half-life-4096 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| CPU seconds through last sample | 626 | 658 | 706 | 740 | 780 | 754 | 754 | 796 |
| Average cores through last sample | 0.255778 | 0.268850 | 0.288467 | 0.302357 | 0.318701 | 0.308075 | 0.308076 | 0.325238 |
| Completed memory_hit requests | 9,141,348 | 8,521,347 | 9,237,142 | 9,006,471 | 8,927,713 | 8,888,186 | 8,987,769 | 8,906,941 |
| Completed disk_hit requests | 202,828 | 823,192 | 107,068 | 337,910 | 416,783 | 456,682 | 356,709 | 437,480 |
| Completed callback requests | 950,961 | 951,036 | 950,887 | 951,127 | 951,149 | 951,164 | 951,031 | 950,983 |
| Completed callback_error requests | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| Completed invalid_download requests | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| Completed memory returned bytes | 18,422,211,043,231 | 18,445,512,866,194 | 18,290,716,440,151 | 18,262,980,169,422 | 18,213,162,608,511 | 18,194,027,044,737 | 18,200,567,598,616 | 18,187,971,235,979 |
| Completed disk returned bytes | 349,791,287,907 | 327,225,230,634 | 481,098,924,976 | 509,395,347,928 | 559,242,192,780 | 579,075,266,222 | 571,938,647,663 | 584,441,187,504 |
| Completed callback returned bytes | 1,081,181,135,291 | 1,081,444,233,183 | 1,081,296,995,286 | 1,081,648,240,170 | 1,081,709,261,006 | 1,081,687,169,616 | 1,081,578,905,912 | 1,081,415,819,653 |
| Completed memory request hit % | 88.792874 | 82.767082 | 89.723701 | 87.479617 | 86.713489 | 86.326325 | 87.297957 | 86.513759 |
| Completed disk request hit % | 1.970134 | 7.995590 | 1.039990 | 3.282111 | 4.048149 | 4.435515 | 3.464705 | 4.249275 |
| Completed memory byte hit % | 92.792227 | 92.904923 | 92.130222 | 91.986292 | 91.734955 | 91.635457 | 91.671651 | 91.609391 |
| Completed disk byte hit % | 1.761890 | 1.648143 | 2.423292 | 2.565703 | 2.816757 | 2.916552 | 2.880710 | 2.943720 |
| Last replay_queued_requests | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| Last replay_active_requests | 2,425,994 | 2,425,835 | 2,425,683 | 2,425,636 | 2,425,437 | 2,425,372 | 2,425,700 | 2,425,740 |
| Last replay_source_waiting_requests | 2,425,941 | 2,425,783 | 2,425,635 | 2,425,588 | 2,425,389 | 2,425,324 | 2,425,652 | 2,425,692 |
| Last replay_source_active_requests | 48 | 48 | 48 | 48 | 48 | 48 | 48 | 48 |
| Last feuer_memory_used_bytes | 99,941,789,767 | 99,997,681,008 | 99,962,191,872 | 99,967,304,991 | 99,943,729,951 | 99,999,454,765 | 99,998,892,024 | 99,994,225,439 |
| Last process_resident_memory_bytes | 115,630,280,704 | 115,527,581,696 | 116,066,852,864 | 115,522,793,472 | 115,451,838,464 | 115,234,144,256 | 115,561,050,112 | 115,283,275,776 |
| Last feuer_disk_write_pending_bytes | 32,014,163 | 33,756,783 | 33,742,563 | 33,012,055 | 32,171,239 | 32,230,034 | 33,755,093 | 33,755,093 |
| Last feuer_disk_write_queued_entries | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| Peak replay_queued_requests | 1,074 | 171 | 693 | 1,091 | 649 | 1,026 | 800 | 868 |
| Peak replay_active_requests | 2,425,994 | 2,425,835 | 2,425,683 | 2,425,636 | 2,425,437 | 2,425,372 | 2,425,700 | 2,425,740 |
| Peak replay_source_waiting_requests | 2,425,941 | 2,425,783 | 2,425,635 | 2,425,588 | 2,425,389 | 2,425,324 | 2,425,652 | 2,425,692 |
| Peak replay_source_active_requests | 48 | 48 | 48 | 48 | 48 | 48 | 48 | 48 |
| Peak feuer_memory_used_bytes | 100,044,952,636 | 100,054,523,057 | 100,041,927,087 | 100,053,357,785 | 100,051,196,833 | 100,047,606,286 | 100,020,594,029 | 100,056,209,717 |
| Peak process_resident_memory_bytes | 128,809,955,328 | 128,517,308,416 | 129,848,840,192 | 128,987,172,864 | 129,224,855,552 | 129,101,299,712 | 129,312,604,160 | 129,052,286,976 |
| Peak feuer_disk_write_pending_bytes | 6,991,152,852 | 7,218,981,524 | 7,112,273,212 | 6,939,791,393 | 6,608,404,449 | 8,002,595,558 | 6,829,454,867 | 6,829,296,437 |
| Last idle buffer bytes | 551,518,208 | 556,728,320 | 574,390,272 | 564,887,552 | 574,816,256 | 683,409,408 | 486,637,568 | 507,838,464 |
| Write admission queued | 228,314 | 228,323 | 228,310 | 228,325 | 228,334 | 228,334 | 228,325 | 228,325 |
| Write admission queue_full | 46 | 49 | 46 | 44 | 45 | 48 | 45 | 44 |
| Write admission queue_closed | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| Write admission canceled | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| Write admission already_covered | 721,013 | 720,653 | 720,762 | 720,997 | 720,994 | 720,647 | 720,836 | 720,809 |
| Write admission redundant | 1,588 | 2,011 | 1,769 | 1,761 | 1,776 | 2,135 | 1,825 | 1,805 |
| Write terminal published | 227,472 | 227,455 | 227,451 | 227,462 | 227,484 | 227,484 | 227,459 | 227,460 |
| Write terminal already_covered | 3 | 10 | 2 | 10 | 11 | 10 | 9 | 8 |
| Write terminal no_capacity | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| Write terminal failed | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| Write terminal canceled | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| Written batch payload | 262,184,165,667 | 262,183,067,160 | 262,182,440,131 | 262,183,434,389 | 262,184,224,054 | 262,184,279,598 | 262,182,352,627 | 262,182,662,417 |
| Written batch chunk | 285,125,640,192 | 285,116,203,008 | 285,116,203,008 | 285,106,765,824 | 285,100,474,368 | 285,102,571,520 | 285,102,571,520 | 285,108,862,976 |
| Written batch written_entries | 227,472 | 227,455 | 227,451 | 227,462 | 227,484 | 227,484 | 227,459 | 227,460 |
| Last disk payload_bytes | 262,184,165,667 | 262,183,067,160 | 262,182,440,131 | 262,183,434,389 | 262,184,224,054 | 262,184,279,598 | 262,182,352,627 | 262,182,662,417 |
| Last disk entries | 227,472 | 227,455 | 227,451 | 227,462 | 227,484 | 227,484 | 227,459 | 227,460 |
| Last disk allocated_chunks | 271,981 | 271,972 | 271,972 | 271,963 | 271,957 | 271,959 | 271,959 | 271,965 |
| Last disk free_chunks | 204,856 | 204,865 | 204,865 | 204,874 | 204,880 | 204,878 | 204,878 | 204,872 |
| memory eviction insertions observed before last arrival | 68,780 | 177,127 | 46,533 | 79,336 | 87,460 | 92,980 | 77,181 | 89,844 |
| disk eviction insertions observed before last arrival | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| memory first eviction sample seconds | 154.930901 | 154.938868 | 154.929012 | 154.933303 | 154.929533 | 154.936542 | 154.930054 | 154.928482 |
| disk first eviction sample seconds | unavailable | unavailable | unavailable | unavailable | unavailable | unavailable | unavailable | unavailable |
| read I/O errors | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| write I/O errors | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| Disk lookup io_error | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| Disk lookup checksum_failed | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| OS user_seconds | 231.640000 | 283.100000 | 229.240000 | 254.850000 | 258.380000 | 262.340000 | 254.590000 | 263.880000 |
| OS system_seconds | 431.410000 | 411.770000 | 510.760000 | 520.930000 | 553.110000 | 524.100000 | 534.860000 | 568.200000 |
| OS max_rss_kib | 128,135,796 | 128,076,960 | 128,049,400 | 128,044,048 | 128,065,436 | 128,075,208 | 128,182,704 | 128,187,036 |
| OS file_inputs | 686,620,896 | 645,066,592 | 937,138,776 | 961,506,584 | 1,039,930,640 | 1,071,220,984 | 1,061,546,344 | 1,079,875,240 |
| OS file_outputs | 557,332,400 | 557,314,744 | 557,313,824 | 557,295,456 | 557,285,264 | 557,289,992 | 557,287,496 | 557,300,088 |
| Device reads | 2,793,735 | 3,178,468 | 3,718,143 | 3,951,801 | 4,313,947 | 4,464,146 | 4,360,381 | 4,484,688 |
| Device read_bytes | 351,547,543,552 | 330,272,526,336 | 479,813,648,384 | 492,289,536,000 | 532,442,890,240 | 548,463,362,048 | 543,509,868,544 | 552,895,578,112 |
| Device writes | 2,214,214 | 2,321,025 | 2,214,180 | 2,214,056 | 2,214,023 | 2,214,111 | 2,214,075 | 2,214,151 |
| Device write_bytes | 285,389,541,376 | 299,390,111,744 | 285,381,091,328 | 285,371,613,184 | 285,366,173,696 | 285,367,545,856 | 285,364,981,760 | 285,372,227,584 |
| Device aqu-sz | 3.190864 | 2.981447 | 5.240341 | 5.042753 | 5.388779 | 5.658081 | 5.816381 | 5.701561 |
| Device %util | 5.437161 | 7.265091 | 5.954957 | 6.480272 | 6.824081 | 7.815777 | 6.820438 | 6.969785 |
| Device r_await_ms_request_weighted | 2.702230 | 2.173208 | 3.382438 | 3.060934 | 3.000229 | 3.028562 | 3.208939 | 3.059066 |
| Device w_await_ms_request_weighted | 0.145243 | 0.193720 | 0.154589 | 0.152915 | 0.154177 | 0.192574 | 0.156528 | 0.155143 |

## Metrics unavailable because replay did not finish

Exact sender-lateness, caller-queue, service and scheduled-to-completion mean/p50/p95/p99/max summaries were not emitted. Execution completion time, backlog drain time, tail I/O and settled pending bytes are unavailable. Do not substitute last-sample time for full runtime or histogram intervals for exact request percentiles. The partial request/source/I/O/write-queue histograms, with counts, means and quantile bucket intervals, are retained per case.

## Evidence and scopes

- `raw/cases/CASE/METRICS.md`, `metric-inventory.csv`, and `histograms-and-windows.json` inventory every sampled metric series, including per-bucket buffers, packing, source queues, all histograms, and cold/later-arrival windows. Each case inventories all 344 sampled series across 46 families. `raw/metric-coverage.json` verifies complete last frames and inventory coverage. No finished/settled snapshots were fabricated.
- `feuer_memory_used_bytes` is charge including idle buffers, not payload occupancy or RSS. DataFile operations are not device requests. Whole-device totals can include other processes. OS totals include input loading and the kill interval. Request-phase CPU has one-second resolution.
- Eviction timing records observed insertions before the last arrival and first positive samples. No case reached that last arrival, so these are incomplete arrival-phase counts and provide no drain-phase evidence.
- `raw/cases/CASE.tar.gz` retains the original collected telemetry with verified hashes. Each directory retains `metrics.prom`, `before.prom`, partial `summary.csv`, commands, environment, manifest, exits, process/device I/O, host snapshots, wrapper logs and kernel OOM evidence. Failed full-report recovery is recorded separately. `incomplete-report.json` is explicitly partial.
- `raw/build/` records the frozen build and tests. `raw/start.json`, `raw/status.json`, `raw/completion.json` and `raw/infra/` retain exact live settings, all failures, provisioning IDs, ownership/deletion checks and independent cleanup verification.
## Reproduction limitations

The frozen source archive is identified by hash above and by its local path in `settings.json`. It is a historical working-tree snapshot, not the current checkout. The private trace and historical binary must be supplied separately. The source simulation uses 48 slots and service time `0.108599 + requested_bytes / 84101038.08` seconds. It is not real S3 and does not model latency variability, retries or shared bandwidth.

Stage the pinned binary at `$ROOT/bin/replay`, the pinned trace at `$ROOT/input/trace.tar.zst`, and both Python reporters at `$ROOT/`. The recorded `m8gd.8xlarge` Linux workers used jemalloc, sysstat, util-linux, `/usr/bin/time` and `/mnt/local-ssd`, with `$ROOT` outside the SSD mount. The baseline invocation was equivalent to:

```sh
bash run-case.sh "$ROOT" range-baseline range 256000 256000 262144
```

Use the other mode and scorer values from `settings.json` for the remaining cases. These commands document a failed historical experiment, not a recommendation to repeat it unchanged. A 100-GB cache charge did not prevent process memory exhaustion, so this setup cannot support full-trace scoring decisions without first addressing that failure.

`report.py` and `report-metrics.py` expect completed phase snapshots and cannot produce this partial report from the OOM captures. Partial tables were derived by `raw/finalize.py` from each last complete exported frame minus `before.prom`. That historical script also performs infrastructure checks and writes status files, so it is not an offline-only reporting command. Original operational notes and unedited reports remain under `raw/`, including `raw/curation-originals/`.
