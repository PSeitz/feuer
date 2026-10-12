# Cost-model findings

## Completion and independent acceptance

Five parallel experiment agents each provisioned one isolated m8gd.8xlarge under ../benchmarks/terraform/feuer-cost-20261011T2130/<lane>. Exactly seven planned sequential same-host pairs / fourteen600-second cases completed. No extra timed case, replacement, knob sweep, capacity change or policy combination. Pre-timing validation failures and a no-instance Terraform timeout were reconciled and retained, not hidden as successful measurement.

Parent read all five reports and inspected scoring/publication/conditional-demand contracts. Independently verified all five evidence archive hashes and3649 retrieved file hashes, reproduced all common reports from raw metrics/device/OS/summary files, enforced exact workload/settings/error/pending/admission gates, reviewed each one-instance create/destroy plan and queried AWS/state again. All fourteen cases completed1606645 requests and2800015397526 returned bytes with no callback/invalid-download/checksum/I/O/write errors and zero pending/queued writes. All admission/terminal balances pass.

All five owned instances terminated, all five root volumes deleted, all five isolated Terraform states empty. Ownership-tag query found exactly the five terminated instances and no additional owned resource. Tracked production source matched the effective baseline at measurement completion, and the paused commit/defaults were unchanged. The subsequent repository-local results move and `benchmarks/` → `experiments/` rename only changed organization and path references. No experiment source or raw evidence was committed.

## Structural fallback modelling was actually tested

Unlike the previous fixed-weight sweep, both memory-model lanes maintained a mirror of published disk copies and selected source fallback for entries without a usable published copy. Queue admission, waiting writes, queue-full skips and pending small-entry buffers were not treated as published disk. Publication/removal/recovery/checksum invalidation and I/O uncertainty were covered by deterministic Linux tests. Scoring reads the mirror without acquiring another cache tier's locks and releases its metadata lock before consulting historian demand. Controls retained the same observers/counters. Whole-object-only mirror is sufficient for this replay, not a production-general range interface.

### Linear fallback model

Against fixed256000 default, source callbacks fell674 and source payload fell0.236GB, while physical SSD reads rose1.363GB (+21.44%), writes fell0.592GB, request CPU rose48→56 seconds and completion p99 was0.271→0.580ms. Memory request/byte hit shares fell0.20484/0.03995 percentage points and useful final memory fell0.323GB.

The same-host assumed-disk→actual-fallback ablation holds linear coefficients and disk victim scorer fixed. Source callbacks fell1172, source bytes fell0.457GB, physical reads rose0.962GB (+14.25%) and writes fell0.512GB. This is evidence of an active fallback-state mechanism with a mixed traffic outcome, not proof that the historical cost ratio is calibrated or optimal.

### Aligned/submission/checksum fallback model

Uses actual whole-entry alignment and1-MiB I/O queue splitting from storage helpers. Historical fixed cost applies per queued read request, historical transfer term applies per aligned byte, exact production checksum contributes isolated measured computation seconds per payload byte. This additive work proxy is not parallel elapsed latency. One API read can submit multiple I/O requests, which differ again from kernel/device requests.

The sole checksum calibration ran before replay, sizes4096/1048576/12582912/64000000 bytes,32 iterations each,128 total computations. Allocation excluded, exact production xxHash64, black-box input/result, no disk benchmark or additional coefficient fitting. Coefficient6.923245120755452e-11 seconds/byte. First-touch/cache warmth/scheduling remain in the wall-time computation measurement. Disk/source historical median fits are still assumptions with different load regimes, not newly calibrated means or bills.

Against fixed256000 default, source callbacks fell716 and source payload fell0.277GB, physical SSD reads rose1.229GB (+19.12%), writes fell0.595GB and request CPU rose53→60 seconds. Completion p99 was0.507→0.682ms. Memory request/byte shares fell0.19948/0.03373pp and useful final memory fell0.330GB.

The assumed-disk→actual-fallback ablation reduced callbacks1064/source0.429GB while increasing reads0.796GB (+11.67%) and reducing writes0.208GB. CPU62→60 seconds, completion p99 1.218→1.099ms. Both disk/source scorer snapshot populations were exercised in every dynamic case. These repeated score populations are sampling work, not unique objects or requests.

**No matched dynamic-linear→dynamic-whole-entry pair was run.** Therefore the two independent lanes do not isolate the incremental effect of alignment, submissions or checksum terms. Similar outcomes across lanes are context, not cross-host matched evidence. Neither model is an unconditional memory-retention/SSD-traffic improvement under this workload. Source reduction is only0.18–0.21% versus default and single-pair significance is unestablished. Historical modeled seconds are not the selection criterion.

Fallback mirrors are observations, not promises of future availability. Pending buffers are deliberately classified conservatively despite being readable in some states. Recovered metadata is presumed usable until checksum-verified reading detects uncertainty. The linear prototype restores usability after a matching verified read and uses read identity to protect stale completions. The whole-entry prototype conservatively retains I/O uncertainty until republish/recovery, and address/length matching is not a general replacement-generation guarantee. No replay case had read errors, replacement pressure or disk eviction, so these error-path differences did not exercise the measured decision seam and are production review limitations.

## Write admission: prediction matters more than the formula

The reuse-admission lane counted only memory-missing requests before disk lookup. It reused the caller's global request clock and exact decay, atomically returned prior conditional demand excluding the current request, and carried that prior through source admission. Both modes collected the same counters. Candidate admitted only when prior demand × nonnegative historical source-minus-aligned-read cost exceeded an uncalibrated aligned write-cost proxy. First conditional miss had no prior evidence and was rejected. No future trace/outcome oracle or size threshold.

Result: physical writes99.159→1.477GB (−98.51%), physical reads6.523→0.005GB, source129.978→136.186GB (+6.208GB,+4.78%) and callbacks+1893. Request CPU45→17 seconds, useful final memory96.886→99.170GB and completion p99 0.366→0.119ms. Combined byte hit share fell95.35794→95.13624%.

Candidate rejected124385 actual source offers, including first-miss payloads and848 insufficient-savings offers. Only2417 source writes were queued/published. There were123885 zero-prior conditional misses, one per distinct slice, and4771 positive-prior misses. Memory-hot reuse does not supply conditional disk-demand evidence. The assumed future horizon equating exponentially weighted past misses with future disk requests is unvalidated. The savings inequality is structurally different, but its predictor/write coefficients are not calibrated. This experiment is primarily a large write/source-avoidance tradeoff, not a validated optimal admission policy. The retrieval-only historical proxy increased, but it excludes write costs and is not a complete ranking.

## Realistic large-object tail

Demand inspection preceded matrix selection. Inclusive decimal32MB cutoff has535 distinct larger slices,5096 requests and185 reused slices. Inclusive64MB has32 larger slices requested once each.128MB has no exposure, so was excluded. These are request demand counts, not promotions/writes.

Both tail experiments additionally measured actual eligible source-write offers. Their limits only gated source enqueue after unchanged memory admission. No scorer/pool/history/queue changes.

| Inclusive cutoff | Actual skipped source offers / logical bytes | Physical writes GB | Physical reads GB | Source byte delta | Callback delta | CPU seconds | Completion p99 ms |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
|32000000 bytes|535 /30395609908|99.115→70.657|6.437→6.453|−18.792MB|−121|46→40|0.552→2.259|
|64000000 bytes|32 /2121236387|93.726→91.739|6.335→6.378|+42.790MB|−83|42→42|1.180→0.299|

Each row is its own same-host pair. Baseline totals differ, including queue-full outcomes, so do not compare cross-host baselines as controls. Logical skipped bytes are not physical savings or proof that every baseline offer published.32MB saved28.458GB physical writes (−28.71%), substantially larger than64MB's1.986GB (−2.12%), with little observed aggregate source-byte change. This is the strongest traffic lead for avoiding unusually large copies in this window, not an adoption recommendation.

Many >32MB slices had caller reuse but no second eligible source offer in these cases. Actual loss of useful disk reuse cannot be inferred from all-demand reuse alone. The64MB tail has no exact-slice second request, yet aggregate source bytes increased42.8MB from other case differences. Neither result proves exactly zero extra source work or durable source preservation.

The32MB scheduled-completion p99 increase coincides with sender-lateness p99 0.417→1.774ms, while service p99 was0.03086→0.03002ms.64MB's lower completion p99 similarly includes changing sender timing. Report these full distributions rather than attributing scheduling variation to the gate. No causal latency improvement/regression or significance is established.

## Limits and next decisions

- Zero disk eviction throughout. Conditional disk-victim selection remains untested, as does any pressure-inducing replay/capacity change.
- Application-specific recomputation/source metadata were unavailable. Per-object heterogeneous source costs and production network/coalescing remain untested, not rejected.
- No calibrated contemporary SSD read/write-contended model or production source-cost unit. Historical S3 TTFB is not a price/CPU cost and medians are not means. The isolated checksum term does not fix these issues.
- Instrumentation is matched within each pair but not free. Default controls are instrumented defaults. No per-lock timing or uninstrumented-overhead comparison was authorized.
- Pool geometry, history/sampling and policy combinations were unchanged. Do not combine32MB admission with a fallback model based on these isolated outcomes.
- Original pacing, independent zero-filled callbacks and fixed decimal capacities retained. Logical memory accounting is not RSS. Device/API scopes differ. Five-second tail is not flushing/durability.
- Seven ordered pairs, no extra repetition. Small hit/source differences are inconclusive. Do not rank on the historical modeled seconds.

No production change recommended on this evidence alone. If further work is requested, choose between validating the32MB lead and calibrating source-versus-disk/write prediction, rather than automatically launching another sweep. Any repetition, policy combination, new calibration or resource needs a new finite request. All experiment infrastructure is already cleaned up.

Full side-by-side metrics: [REPORT.md](REPORT.md). Exact distributions, registered metric families, histogram intervals, startup/later windows, detailed lane reports, and independent audit receipts are retained locally in `measurements.tar.zst` (not committed). Model descriptions, tested settings, and source patches are indexed in [README.md](README.md).
