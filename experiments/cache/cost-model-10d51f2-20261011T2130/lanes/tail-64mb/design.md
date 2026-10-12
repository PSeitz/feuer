# Source disk admission at 64 MB decimal

Description: optionally reject immediate disk writes of source payloads longer than 64000000 bytes, after unchanged memory insertion. The inclusive maximum admits length 64000000. The existing prototype's `max_source_payload_disk_admission_bytes` names this maximum. None is the default and the matched control.

No scorer, pool, historian, queue, disk layout or promotion changes. Existing published disk copies and compaction remain untouched. Whole-object-only workload prevents compaction from bypassing this gate, covered by retained tests.

Description: count source payloads actually offered to immediate disk admission, and their payload bytes, separately at or below and above 64000000. Name: `source_disk_offered`. Offer recording happens only after memory insertion accepts an exact-range source download and before the size gate. Existing queue outcomes record real enqueue success/rejection. Existing policy skip size bands are extended with an exact >64000000 band so the actual offered tail and skipped tail are visible in both modes. These are logical counts/bytes, not physical writes.

Counters are registered in both modes. No added metadata or locks. The existing cache lookup/insertion ordering is retained. Metric increments do not re-enter caches or history. No promised publication, future availability or durability. Source offers reconcile with queued + queue_full + queue_closed, and policy_size. Callback balance also includes already_covered and redundant. Queued writes reconcile with published + terminal already_covered + no_capacity + failed + canceled after the observation tail, with zero pending and queued entries required.

Validation includes retained admission, cancellation, promotion and whole-object tests, numeric CLI settings, and exact decimal boundary tests at 63999999, 64000000, 64000001 and 67108864. Runtime summary remains numeric (-1 means absent). First600s all-demand exposure above 64MB is 32 slices/32 requests with no exact-slice reuse. It does not establish actual source-write exposure.
