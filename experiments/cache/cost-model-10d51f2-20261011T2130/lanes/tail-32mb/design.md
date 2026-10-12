# Tail source-write admission

A source download that is an exact requested whole slice, is not already covered by disk, and is accepted by memory is offered to immediate disk admission. Count these offered payloads before the size gate. Names derived from that description: `source_disk_offered`, `record_source_disk_offer`, `feuer_source_disk_offered_total`, `feuer_source_disk_offered_bytes_total`.

The prototype's opt-in inclusive maximum remains absent in control and is exactly 32000000 decimal bytes in candidate. It skips only payloads strictly greater than the maximum. Memory insertion, existing disk promotion, fixed256000 scoring in both tiers, queues, pools, history and disk format remain unchanged. Whole-object demand cannot trigger range compaction.

Offered and skipped counters use disjoint numeric bands <=32000000, 32000001..64000000, >64000000. These count actual source-write exposure, not request-demand sizes. Both modes retain identical instrumentation. The existing policy_size queue outcome reconciles rejected offers. Successful callbacks reconcile with offers plus pre-write already_covered/redundant outcomes. Offers minus skips reconcile with queued/queue_full/queue_closed/canceled attempts. Queued admissions reconcile with terminal writes and zero queued/pending after tail.

No metadata map, new lock, or cache/history lock ordering change. Counters run at the existing source admission point after memory insertion. Metrics do not predict future disk availability. Tests cover exact 31999999/32000000/32000001/33554432 boundaries, absent defaults, CLI/summary integer units, disk publication/promotion, compaction bypass and cancellation.

Inspected provenance/tail-exposure.json: demand >32000000 is 5096 requests over 535 unique slices, 185 reused, 30395609908 unique bytes. Demand >64000000 is 32 requests over 32 slices with no reuse. Those values are context only. Acceptance requires actual nonzero offered >32000000 in control and candidate, zero control policy skips, and every candidate offered byte/count above 32000000 suppressed while <=32000000 is not policy-skipped. No alternative threshold or repetition.
