# Private read-work design

## Descriptions before names

- A sharded mirror of whole entries actually in the published disk index, with their payload length, disk address and conservative read usability. Name: `PublishedDiskCopies`. It is not queue coverage and does not promise future availability.
- A memory retention scorer using caller demand times whole-entry fallback work divided by the actual supplied memory charge. Name: `WholeEntryScorer`.
- The aligned byte count and 1-MiB request count produced by the current payload-read implementation. Function: `payload_read_work`.
- Production payload XXHash64 with seed zero, excluding alignment padding. Function: `payload_checksum` shared by writer, verifier and finite calibration.

## Contract

Private whole-object workload only, immutable keys and zero offsets. Mirror lookups compare whole payload length. Queued, waiting and small buffered payloads are absent until index insertion following successful payload write. Publication and recovery use the same index insertion hook. Removal, replacement, checksum invalidation and teardown use index removal hooks. Transient I/O uncertainty marks the matching published address unusable in the mirror without removing the production index. It remains conservatively source until republished or recovered. Failed reads are not classified as checksum failures.

Index mutation holds disk-index and sometimes metadata locks before taking one mirror shard lock. Memory scoring takes a mirror snapshot and releases its lock before acquiring historian locks. No mirror lock is held across cache calls, history access, I/O or callbacks. No scorer re-enters either tier. Recovery runs before the cache opens. Teardown removes matching identities. A publication/removal after a snapshot can race the score, so the snapshot describes observed fallback, not future read success. Address checks avoid an old uncertain read marking a different published payload unusable.

Every measured mode uses the same observer hooks, mirror, scorer wrapper and binary. Mode 0 returns the original fixed256000 score, mode 1 assumes disk whole-entry work, mode 2 chooses observed published usable disk or historical source. Disk scorer is always fixed256000. Observer counters expose actual disk/source snapshots and chosen fallback, not inferred residency from admissions.

## Model and finite calibration

Read work is `ceil(max(payload,1)/4096)*4096` bytes and `ceil(aligned_bytes/1048576)` submissions. Exported helper derives those from the actual storage alignment/queue split constants rather than copying the constants into the scorer. ReadQueue queues disjoint 1-MiB pieces before waiting for replies, subject to 64 active and 64 waiting reads. Each piece is an I/O request/SQE, not necessarily a syscall, physical device request or a DataFile call. Pieces and callers can overlap. The sum of fixed-per-submission, aligned-transfer and checksum terms is additive modeled work, not parallel elapsed latency.

Historical disk assumptions remain 62.46274509803923 microseconds per submission and 8.187806372549019e-9 seconds per aligned byte. Source historical terms remain 0.108599 seconds and 1.1890459652219313e-8 seconds per payload byte. No disk benchmark or current fitted disk latency calibration is performed. Historical medians include load effects and omit checksum, so adding measured checksum CPU is a proxy, not a claim of independently calibrated end-to-end latency or no overlap/double counting.

Exactly one sequential checksum-only calibration before replay: payloads 4096, 1048576, 12582912 and 64000000 zero bytes, allocated before timing, exactly 32 computations each. Production `payload_checksum` calls XXHash64::oneshot(0,payload). Input and result cross `std::hint::black_box`. No warmup computations, disk I/O or opportunistic repeat. Raw monotonic nanosecond durations, protocol, binary/source and compiler hashes retained. Derive one positive seconds/byte as total measured seconds / total hashed payload bytes across all 128 computations. No additional coefficients or regression fit. This is isolated checksum computation wall time on one host, used as a CPU-work approximation, not load-dependent production checksum latency.
