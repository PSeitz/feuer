# Pomsky replay concurrency — 2026-10-12

## Scope and change

The [completed scorer pair](../cost-scorer-20261012/README.md) used 32 serial consumers
and 32 Tokio threads. A consumer awaited a cache lookup before accepting another,
so the harness admitted at most 32 lookups. Pomsky admits split warmups and fans out
concurrent reads within each split. These are different limits.

`feuer-replay/src/bin/replay.rs` now starts one task for each timestamp-paced arrival,
without waiting for previous lookups. Runtime threads still follow available CPUs,
which is 32 on the SSD host. No split permits or new cache-read limit are added.
Recorded cache starts already reflect application admission. Feuer's existing I/O
queues provide their normal backpressure: 64 active read submissions plus 64 waiting,
and 8 active writes plus 8 waiting. Each submission is at most 1 MiB, not necessarily
one whole cache lookup.

Only descriptors and unstarted tasks wait outside the lookup, without payload
allocation. Completed task observations are collected during arrivals and drained
after the sender closes. Every selected request must complete exactly once.

- `queue_wait` measures emission to lookup-task start, including runtime scheduling.
- `service` measures the lookup, including waiting inside Feuer.
- Scheduled completion remains sender lateness + queue wait + service.
- `replay_queued_requests` includes descriptors and tasks not yet started.
- `replay_active_requests` counts started lookups awaiting completion.
- `summary.csv` identifies the scheduler as `task_per_arrival`. `workers` now counts
  only runtime threads. Gauge samples are not an atomic accounting snapshot.

Scoring, capacities, trace selection, range/full-object choice, synthetic downloads,
and the observation tail are unchanged. **No capture benchmark was launched.** The
old pair's latency discrepancy remains unresolved. This fixes a confirmed scheduling
mismatch, not a demonstrated cause of the original seconds-level disk tails.

## Conditional 32-core chart profile

Inspected clean `../pomsky-helm-charts` at
`aa8171b851ee0f83a0a142b1c200af0a316ce897`. Helm v4.2.2 rendering confirms the following
with `searcher.podSize=8xlarge`, empty `searcher.resources`, and no config overrides:

| Setting | Rendered value |
| --- | ---: |
| CPU request | 31600m |
| CPU limit | Not set |
| Memory request and limit | 120900Mi |
| `max_num_concurrent_split_searches` | 370 |
| `fast_field_cache_capacity` | 47.9375G |
| `split_footer_cache_capacity` | 3.6875G |
| `partial_request_cache_capacity` | 1.84375G |
| `aggregation_memory_limit` | 500M |

Sources under `charts/quickwit/`: `sizing-map.yaml:153–180`,
`templates/configmap.yaml:1–12`, and `templates/searcher-statefulset.yaml:123–133`.
All twelve existing config/resource sizing fixture expectations matched Helm renders.

Important precedence:

- The chart defaults to `xlarge`, not `8xlarge`, with concurrency 40.
- Explicit nonempty `config.searcher` fields override the sizing defaults. Unspecified
  fields retain them. Tested zero/null concurrency values instead retain the default.
- **Any nonempty `searcher.resources` disables all searcher sizing-map config defaults.**
  With no explicit `config.searcher`, no searcher section renders. The supplied
  `datadog-values.yaml` takes this path.
- `templates/_helpers.tpl:302–308` derives `QW_NUM_CPUS` from the CPU request, not a
  literal 32-thread setting.
- The chart supplies no warmup-memory or Feuer disk-cache settings. Searcher storage
  defaults to `emptyDir: {}`, not a sized PVC. Indexer disk settings are unrelated.

Application defaults at inspected Pomsky commit `5cb0b087c` are **100 GB warmup
memory** and **300 MB maximum initial split reservation**, with a 10 MB floor and
smaller reservations based on split body size and document count.
`quickwit-config/src/node_config/mod.rs:797–817` and
`quickwit-search/src/search_permit_provider.rs:95–112,390–424` establish these values
and admission on both slots and available memory. At 300 MB each, at most 333 initial
reservations fit in 100 GB, and searches retaining memory can reduce availability.
This is not a universal split count or read limit. The warmup budget is not Feuer's
memory-cache capacity.

Thus 370 is a conditional split warmup slot count, **not 370 reads**, and not yet the
originating benchmark's verified setting. Its actual release values, image, resource
and environment overrides, and enabled caches remain unknown. No replay capacity
was changed based on this profile.

## Remaining fidelity gaps

| Difference | Consequence / next decision |
| --- | --- |
| Exact ranges versus `--full-objects` | Pomsky passes original object identity and range. Full objects prevent covering/overlapping requests from sharing entries. Range replay is available already, but changing this in a comparison changes the workload. |
| Miss coalescing | Pomsky batches for 1 ms, merges gaps up to 1 MiB into ranges up to 8 MiB, and shares covered in-flight downloads. Replay independently downloads exact requested bytes. Concurrent replay does not implement that source behavior. |
| Source timing and buffer charges | Pomsky allocates one cache-managed buffer after finalizing a download range, holds it across the storage read, and returns the full download. Replay immediately allocates zero-filled `Bytes` independently. This changes transient memory and write traffic. |
| Recorded callback download shape | The original prefix reports 116.299 GB requested and 135.208 GB downloaded across successful callback outcomes. These are per-caller trace sums, not unique source/network bytes, because coalesced downloads can be shared. Recorded shapes describe original misses, not counterfactual replay misses. |
| Initial state and failures | Replay starts empty and makes every miss succeed. Original incomplete starts and failures do not establish successful downloads or cold cache contents. |
| Other shared caches | Pomsky can share Feuer queues across split-range, partial-response, and predicate caches. Only split-range traffic is captured. Enablement and omitted traffic in the benchmark remain unverified. |
| Original lookup spans | They can include start-trace backpressure and do not isolate SSD service or read admission. Do not inject recorded disk delays into replay. |

Before another SSD benchmark, agree whether to keep a scheduling-only synthetic
comparison or model Pomsky's range/coalesced source path. Recorded outcome durations
alone cannot define latency for new misses. No speculative source model was added.
Use a fresh pinned build and the [timed replay procedure](../../storage/timed-replay.md),
with the default 600-second prefix only when a new replay is requested.

## Validation and artifacts

The original source snapshot includes pre-existing uncommitted full-object and scorer
changes. Those source changes are excluded from the scheduling commit. The working-tree
and scheduling-only versions were validated separately.

On `m8g-32cpu-local-ssd-2`, in an isolated source and target workspace:

```sh
cargo test --locked -p feuer-replay
cargo clippy --locked -p feuer-replay --all-targets -- -D warnings
cargo build --locked -p feuer-replay --bin replay
```

All **12 tests passed**: six reader tests, four binary tests, and two writer/integration
tests. A focused single-thread test starts 65 blocked lookups in two batches before
releasing any, verifies that later arrivals start, checks active/queued accounting,
and verifies exactly-once completion after sender closure. Another test checks lookup
error propagation. Integration tests check scheduler identity and zero unfinished
requests with disk disabled/enabled and ranges/full objects. These are tiny generated
test inputs, not benchmark captures. Strict Clippy and the normal Linux binary build
passed, as did local formatting and `git diff --check`.

Before committing, the staged scheduling-only tree was exported and tested in a second
isolated Linux workspace. All **11 tests passed**, along with formatting and strict
Clippy. That tree has five reader tests, four binary tests, and two writer/integration
tests, without the uncommitted full-object feature. No capture benchmark was run.

[`settings.json`](settings.json) records tested source/binary hashes and conditional
chart findings. Local ignored `raw/` contains the source snapshot and validation
logs. Remote workspace: `/home/ubuntu/feuer-pomsky-replay-concurrency-20261012`.
The completed scorer pair's source snapshots, binaries, and results remain unchanged.
