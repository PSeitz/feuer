# Workspace symbol inventory

Generated with `rust-analyzer 0.3.2929-standalone (7ea2b259ca 2026-06-07)` using `rust-analyzer symbols` on each workspace source file.

## Scope

- Cargo workspace: **6 packages**, **22 Rust source files**.
- Includes source declarations, fields, variants, implementation blocks, and local bindings reported by rust-analyzer.
- Includes tests, examples, the memory benchmark, and inactive `cfg` branches (including Linux-only storage code).
- Excludes the separate `foyer/` workspace, external dependencies, and generated build output.
- Syntax inventory, not an LSP `workspace/symbol` search or a type-check. Macro expansions and import/re-export aliases are not expanded.
- Function parameters appear in signatures. This is not a complete list of every identifier or pattern binding.
- Names are qualified by their source parent where rust-analyzer provides one. Locations distinguish duplicate names.
- This is a syntax snapshot with subsequent focused updates, not an automatically refreshed symbol index.

## Naming review

Each type below was described from its fields, construction, and use before choosing its name. The description is the naming test: every substantive word in the chosen name must refer to something in that description. A retained name is a decision, not an exemption for public APIs or conventional terminology. Associated items, fields, variants, constants, parameters, and local bindings were checked in source order in all 22 files. The complete symbol list follows this review. Every listed symbol has a review decision. A renamed parent's unchanged members are marked **Keep**, with the new parent visible in their qualified names. Symbols introduced or replaced by the subsequent storage update are marked separately below.

### Focused follow-up: payload-compaction thresholds

This follow-up reviews the two requested constants and their directly related symbols. It does not
repeat the earlier whole-workspace review. Descriptions precede the naming decisions:

| What it represents or does | Existing name | Decision / chosen name |
| --- | --- | --- |
| Minimum successful same-shard accesses since admission before payload compaction is allowed | `RANGE_TRIM_GRACE_ACCESSES` | `MIN_SHARD_ACCESSES_BEFORE_PAYLOAD_COMPACTION` |
| Minimum percentage of source payload bytes that payload compaction must save | `MIN_RECLAIM_DIVISOR` | `MIN_PAYLOAD_COMPACTION_SAVINGS_PERCENT` |
| A plan to trim a cached range: source range, retained ranges, and retained byte count | `RangeTrimPlan` | Keep |
| Plans which requested byte ranges to retain when trimming a cached range | `plan_range_trim` | Keep |
| Number of payload bytes reclaimed by the range-trimming plan | `RangeTrimPlan::reclaimed_bytes` | Keep |

The thresholds remain **64 accesses** and **25% savings**. Savings are now expressed directly as
percent rather than a denominator of 4. The calculation uses `u128` intermediates to preserve
upward rounding without overflowing for `u64` payload sizes. The first threshold restricts
payload compaction, not eviction. All production and test references use the new names.

### Types, one by one

The first column describes the represented data or responsibility. The other columns record the previous and chosen names. **Keep** means the existing name expresses that description in its containing module/type. These are naming changes only. No compatibility aliases or new runtime types are introduced.

| What it represents | Previous name | Decision / chosen name |
| --- | --- | --- |
| Configuration and memory-cache state shared by cloned cache handles | `CacheState` | Keep |
| A handle to the Feuer cache | `Cache` | Keep |
| An error from the cache's get-or-fetch operation | `GetOrFetchError` | Keep |
| The cache configuration: directory and disk/memory capacities | `Config` | `CacheConfig` |
| An error validating the cache configuration | `ConfigError` | `CacheConfigError` |
| Counters and gauges for the memory tier | `MemoryMetrics` | Keep |
| One requested byte-range access and its observation clock | `AccessEvent` | `RangeAccess` |
| A bounded history of requested byte-range accesses for one object | `AccessHistory` | `RangeAccessHistory` |
| A plan to trim a cached range: source range, ranges to retain, and retained byte count | `CompactionPlan` | `RangeTrimPlan` |
| One cached byte range, its payload, and admission metadata | `CachedRange` | Keep |
| An object's cached ranges, access history, and change generation | `ObjectCachedRanges` | Keep |
| Cached ranges superseded by a larger incoming range, with their total payload size | `SupersededRanges` | Keep |
| Cache usage removed during admission, counted in payload bytes and entries | `RemovedUsage` | `RemovedCacheUsage` |
| A cached range's identity: object key, start offset, and entry ID | `CachedRangeIdentity` | Keep |
| A rotating ring of cached-range candidates for reclaiming memory by trimming or eviction | `PressureCandidates` | `ReclaimCandidateRing` |
| A cached range considered for reclaiming memory, with its retrieval cost and retained size | `PressureCandidate` | `ReclaimCandidate` |
| The source payload, plan, and identity captured to trim a cached range outside its shard lock | `CompactionSource` | `RangeTrimSource` |
| Replacement payloads produced by trimming a cached range, awaiting identity/generation checks | `CompactionReplacement` | `RangeTrimReplacement` |
| Progress after a bounded admission attempt: complete, retry, or trim before retrying | `AdmissionStep` | `AdmissionProgress` |
| One memory-cache shard's indexes, history, candidate ring, and byte accounting | `MemoryShard` | `MemoryCacheShard` |
| The sharded in-memory cache of downloaded byte ranges | `MemoryCache` | Keep |
| Command-line arguments selecting workload replay runs and output | `Args` | `ReplayArgs` |
| The policy selecting exact or expanded byte ranges to download | `DownloadPolicy` | `DownloadRangePolicy` |
| Configuration of download expansion: coalescing distance and whole-object threshold | `DownloadConfig` | `DownloadExpansionConfig` |
| One request from the captured trace: object, byte range, size, and timestamp | `Access` | `TraceRequest` |
| A workload prepared for replay, including requests and expanded download ranges | `Workload` | `ReplayWorkload` |
| A cache queried and populated during workload replay | `ReplayCache` | Keep |
| Feuer's memory cache used for workload replay | `FeuerReplayCache` | Keep |
| An object-and-range key used by the Foyer cache | `NativeFoyerKey` | `FoyerRangeKey` |
| A download cached in Foyer: its range and payload bytes | `NativeFoyerValue` | `FoyerCachedDownload` |
| The choice of range in a Foyer key: exact request or expanded download | `NativeFoyerKeyMode` | `FoyerKeyRange` |
| The Foyer eviction policy selected for replay: S3FIFO or cost-aware | `NativeFoyerPolicy` | `FoyerEvictionPolicy` |
| Foyer's cache used for workload replay | `FoyerReplayCache` | Keep |
| Request, hit, and source-download traffic counted during a replay pass | `Traffic` | `ReplayTraffic` |
| A replay report containing run settings, traffic, memory use, and elapsed time | `Report` | `ReplayReport` |
| A planned download to coalesce with other requests, retaining its trace order | `PendingDownload` | `DownloadToCoalesce` |
| A coalesced download range and the trace requests it serves | `CoalescedDownload` | Keep |
| A test cache that becomes populated during warmup | `WarmupTestCache` | Keep |
| A test cache that checks whether one stored range covers the requested range | `RangeTestCache` | `CoveringRangeTestCache` |
| Completed I/O counts, byte counts, and latency samples for a measurement interval | `IoMeasurements` | Keep |
| The I/O operation being attempted, including opening and preparing the file | `IoOperation` | Keep |
| The kind of data-file error used for bounded diagnostic labels | `ErrorKind` | `DataFileErrorKind` |
| A data-file error with the failed operation and relevant context | `Error` (storage) | `DataFileError` |
| A data-file operation result using the data-file error type | `Result` (storage) | `DataFileResult` |
| Queue, path, and capacity state shared by data-file handles | `DataFileState` | Keep |
| A handle to the exclusively owned fixed-capacity payload data file | `DataFile` | Keep |
| Metric counters and histograms for one I/O operation class | `OperationMetrics` | `IoOperationMetrics` |
| Read and write I/O metric handles | `IoMetrics` | Keep |
| A receiver for an I/O request's result, used by queue tests | `IoResultReceiver` | Keep |
| A handle that submits requests to an I/O queue and joins its thread on drop | `IoQueueHandle` | Keep |
| Read/write I/O admission budgets for request slots and staging pages | `IoAdmissionBudgets` | Keep |
| An owned aligned allocation used as an I/O buffer | `AlignedBuffer` | `AlignedIoBuffer` |
| One admitted I/O request owning buffers and admission permits until completion | `IoRequest` | Keep |
| The thread-owned I/O queue, including pending/in-flight requests and kernel ring | `IoQueue` | Keep |
| One completed download, represented by its start offset and payload | `Download` | Keep |
| An error validating a completed download | `DownloadError` | Keep |
| The full immutable-object identity used as a cache key | `ObjectKey` | Keep |
| An exact, nonempty, half-open byte range within an object | `ByteRange` | Keep |
| The error associated with converting a Rust range into a validated byte range | `TryFrom::Error` | Keep: the trait requires this associated name |
| The rejected endpoints of an invalid byte range | `InvalidRange` | `InvalidByteRange` |

### Other symbols

- Range trimming is named consistently across its module, planner, admission progress variant, publication method, grace constant, tests, and internal metric handles. It copies retained subranges rather than packing unrelated cache entries. Published metric names and operation labels retain their existing spelling.
- Range-trim plans distinguish `source_range` and `retained_ranges`. Copied replacements distinguish `retained_payloads`. Snapshot generations are `object_generation`, because any access or structural change for that object invalidates publication, not just changes to the source entry.
- Access-clock observations use `observed_at_access` and `admitted_at_access`, not timestamp-like `*_at` names. No wall-clock time is involved.
- `covered_retrieval_cost` sums modeled source cost for requests wholly covered by a cached range. Candidate `retrieval_cost` and `compare_retrieval_cost_per_byte` describe the actual selection quantity. Byte-normalized comparison parameters distinguish cost from payload size.
- Removed/superseded usage uses `payload_bytes` and `entry_count`, not fields that could be mistaken for payload buffers or entry collections. Admission arithmetic uses `used_bytes_without_superseded` and `max_existing_bytes`: the existing bytes still charged after removing superseded ranges, and the maximum allowed before adding the incoming payload.
- Memory insertion's shared implementation is `admit_download`, not `insert_inner`. Admission advancement is `advance_admission`. The result is progress, not an already-executed action to be repeated blindly. Covering-access recording is `record_covering_access`.
- I/O admission semaphores are `request_slots` and `staging_pages`. The latter counts alignment-sized pages, not buffers or bytes. `_staging_pages_permit` holds that charge. It is not a disk-region read guard.
- I/O ownership names identify `directory_lock` and `wake_fd`. `reading_phase` distinguishes an RMW request's current direction from its caller's write operation. `queue_stopped_error` and `incomplete_io_error` identify their error conditions. Queue tests call their queue `queue`, not `driver`.
- The data-file measurement wrapper is `execute_measured`. Its bounded-chunk implementation is `execute_chunks`. Alignment validation is `check_direct_io_alignment`. Chunk input and accumulated output are `chunk_payload` and `read_bytes`.
- Benchmark trace entries are consistently `request` / `requests`, not recorded successful accesses. Requests use `requested_range`, planned downloads use `downloaded_range`, and pending/coalesced requests preserve `trace_index`. The Foyer adapter names its `key_range` and `eviction_policy`. No simulated download is renamed to imply it contains payload bytes before population.
- Byte parsers/printers distinguish `parse_byte_count_usize` from the u128 parser and `format_binary_bytes` from decimal formatting. The I/O smoke benchmark names its read-space byte limit, measured case, read size, and CPU percentage explicitly.
- Retained API operations (`new`, `get`, `insert`, `remove`, `open`, `read_at`, `write_at`, `capacity`, `used_bytes`, `into_parts`) name their action/data in their containing type. Range `start`/`end` remain endpoints, `len` remains length, and range predicates describe actual containment/overlap. Error variants identify their actual rejection/failure conditions. Their containing renamed types supply the subject.
- Retained constants state a concrete limit, unit, path, revision, environment key, or output header. Retained test functions state their scenario/assertion. Test fixture helpers (`range`, `cache`, `download`, `request`) name the value they construct. Single-scope fixtures such as `hot`, `cold`, `incoming`, `first`, and `second` describe their role in that test.
- Retained locals such as `range`, `bytes`, `entry`, `key`, `offset`, `index`, `count`, `source`, `result`, and `error` have an immediate, single referent in their block. Endpoint differences remain `start`/`end` where immediately used to slice the corresponding payload. Type variables, standard trait methods, `main`, and imported library APIs are not given project-specific replacement names.
- Re-export-only modules and imports were also checked. `LockFileExt` distinguishes file-lock methods from other file extension traits. `FoyerCache` distinguishes the imported Foyer implementation from Feuer's memory cache. Neither is a locally defined runtime type.

No reserved disk-region or read-guard types exist yet. Future implementations must use `DiskRegion` and `DiskRegionReadGuard` as required by `AGENTS.md`. The separate `foyer/` workspace is unchanged.

### Compatibility

Public source-level renames: `Config` → `CacheConfig`, `ConfigError` → `CacheConfigError`, `InvalidRange` → `InvalidByteRange`, and storage `Error` / `ErrorKind` / `Result` → `DataFileError` / `DataFileErrorKind` / `DataFileResult`. All workspace uses, re-exports, and current documentation references are updated. No old-name aliases are retained. The naming changes preserve CLI flags, environment variables, CSV columns, metric names, tracing labels, and algorithm behavior.

### Subsequent storage update

This inventory reflects the later change to caller-owned I/O conflict prevention, not just the naming-only snapshot:

- Removed `IoRequest::conflicts` and the queue's overlap scan. `IoQueue::schedule` now fills free slots in arrival order. Callers must prevent conflicting access to aligned byte ranges.
- Replaced the overlap-blocking test with `canceled_submitted_rmw_retains_resources_and_still_writes`, checking that abandoning a result does not release an active write's resources or stop its write phase.
- Changed the mixed-I/O test to `concurrent_mixed_io_with_caller_serialized_same_page_rmw`, with caller-side synchronization for shared pages.
- Symbols added or replaced by these test changes are marked **Storage update** rather than being presented as naming-only changes. Removed symbols no longer appear in the current inventory. Counts and source locations have been refreshed.

## Counts

| Kind | Count |
| --- | ---: |
| Const | 28 |
| Enum | 10 |
| Field | 178 |
| Function | 164 |
| Impl | 51 |
| Local | 544 |
| Method | 134 |
| Module | 25 |
| Struct | 44 |
| Trait | 1 |
| TypeAlias | 5 |
| Variant | 38 |
| **Total** | **1222** |

## Packages

| Package | Files | Items / fields / variants | Implementation blocks | Local bindings |
| --- | ---: | ---: | ---: | ---: |
| `feuer` | 3 | 42 | 3 | 51 |
| `feuer-memory` | 7 | 211 | 12 | 180 |
| `feuer-memory-bench` | 1 | 149 | 12 | 109 |
| `feuer-storage` | 7 | 181 | 18 | 196 |
| `feuer-tokio` | 1 | 0 | 0 | 0 |
| `feuer-types` | 3 | 44 | 6 | 8 |

## feuer

### `feuer/src/cache.rs`

| Line | Kind | Name / source parent | Signature or type | Review decision |
| ---: | --- | --- | --- | --- |
| [11](feuer/src/cache.rs#L11) | Struct | `CacheState` | — | Keep |
| [12](feuer/src/cache.rs#L12) | Field | `CacheState::config` | `CacheConfig` | Keep |
| [13](feuer/src/cache.rs#L13) | Field | `CacheState::memory` | `MemoryCache` | Keep |
| [23](feuer/src/cache.rs#L23) | Struct | `Cache` | — | Keep |
| [24](feuer/src/cache.rs#L24) | Field | `Cache::state` | `Arc<CacheState>` | Keep |
| [27](feuer/src/cache.rs#L27) | Impl | `impl fmt::Debug for Cache` | — | Keep |
| [28](feuer/src/cache.rs#L28) | Method | `impl fmt::Debug for Cache::fmt` | `fn(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result` | Keep |
| [36](feuer/src/cache.rs#L36) | Impl | `impl Cache` | — | Keep |
| [38](feuer/src/cache.rs#L38) | Function | `impl Cache::new` | `fn(config: CacheConfig) -> Self` | Keep |
| [46](feuer/src/cache.rs#L46) | Method | `impl Cache::config` | `fn(&self) -> &CacheConfig` | Keep |
| [62](feuer/src/cache.rs#L62) | Method | `impl Cache::get_or_fetch` | `fn<F, Fut, E>( &self, object_key: ObjectKey, requested_range: ByteRange, callback: F, ) -> Result<Bytes, GetOrFetchError<E>>` | Keep |
| [96](feuer/src/cache.rs#L96) | Enum | `GetOrFetchError` | — | Keep |
| [99](feuer/src/cache.rs#L99) | Variant | `GetOrFetchError::Callback` | — | Keep |
| [102](feuer/src/cache.rs#L102) | Variant | `GetOrFetchError::DownloadDoesNotCover` | — | Keep |
| [104](feuer/src/cache.rs#L104) | Field | `GetOrFetchError::DownloadDoesNotCover::requested_range` | `ByteRange` | Keep |
| [106](feuer/src/cache.rs#L106) | Field | `GetOrFetchError::DownloadDoesNotCover::downloaded_range` | `ByteRange` | Keep |
| [110](feuer/src/cache.rs#L110) | Function | `requested_slice` | `fn(bytes: &Bytes, downloaded_range: ByteRange, requested_range: ByteRange) -> Bytes` | Keep |
| [120](feuer/src/cache.rs#L120) | Module | `tests` | — | Keep |
| [133](feuer/src/cache.rs#L133) | Function | `tests::range` | `fn(start: u64, end: u64) -> ByteRange` | Keep |
| [137](feuer/src/cache.rs#L137) | Function | `tests::cache` | `fn(memory_capacity: u64) -> Cache` | Keep |
| [142](feuer/src/cache.rs#L142) | Function | `tests::cache_handle_is_send_sync_static` | `fn()` | Keep |
| [143](feuer/src/cache.rs#L143) | Function | `tests::cache_handle_is_send_sync_static::assert_send_sync_static` | `fn<T: Send + Sync + 'static>()` | Keep |
| [148](feuer/src/cache.rs#L148) | Function | `tests::callback_result_and_covering_memory_hit_return_the_exact_request` | `fn()` | Keep |
| [179](feuer/src/cache.rs#L179) | Function | `tests::every_concurrent_miss_invokes_its_own_callback` | `fn()` | Keep |
| [211](feuer/src/cache.rs#L211) | Function | `tests::callback_errors_are_returned_without_retry_or_population` | `fn()` | Keep |
| [232](feuer/src/cache.rs#L232) | Function | `tests::rejects_noncovering_but_retains_oversized_callback_results` | `fn()` | Keep |
| [272](feuer/src/cache.rs#L272) | Function | `tests::a_racing_contained_download_is_discarded_but_returns_its_own_bytes` | `fn()` | Keep |

<details>
<summary>Local bindings (48)</summary>

| Line | Kind | Name / source parent | Signature or type | Review decision |
| ---: | --- | --- | --- | --- |
| [39](feuer/src/cache.rs#L39) | Local | `impl Cache::new::memory` | — | Keep |
| [76](feuer/src/cache.rs#L76) | Local | `impl Cache::get_or_fetch::download` | — | Keep |
| [77](feuer/src/cache.rs#L77) | Local | `impl Cache::get_or_fetch::downloaded_range` | — | Keep |
| [85](feuer/src/cache.rs#L85) | Local | `impl Cache::get_or_fetch::requested_bytes` | — | Keep |
| [112](feuer/src/cache.rs#L112) | Local | `requested_slice::start` | — | Keep |
| [114](feuer/src/cache.rs#L114) | Local | `requested_slice::end` | — | Keep |
| [149](feuer/src/cache.rs#L149) | Local | `tests::callback_result_and_covering_memory_hit_return_the_exact_request::cache` | — | Keep |
| [150](feuer/src/cache.rs#L150) | Local | `tests::callback_result_and_covering_memory_hit_return_the_exact_request::key` | — | Keep |
| [151](feuer/src/cache.rs#L151) | Local | `tests::callback_result_and_covering_memory_hit_return_the_exact_request::payload` | — | Keep |
| [152](feuer/src/cache.rs#L152) | Local | `tests::callback_result_and_covering_memory_hit_return_the_exact_request::callback_count` | — | Keep |
| [154](feuer/src/cache.rs#L154) | Local | `tests::callback_result_and_covering_memory_hit_return_the_exact_request::count` | — | Keep |
| [155](feuer/src/cache.rs#L155) | Local | `tests::callback_result_and_covering_memory_hit_return_the_exact_request::callback_payload` | — | Keep |
| [156](feuer/src/cache.rs#L156) | Local | `tests::callback_result_and_covering_memory_hit_return_the_exact_request::result` | — | Keep |
| [166](feuer/src/cache.rs#L166) | Local | `tests::callback_result_and_covering_memory_hit_return_the_exact_request::count` | — | Keep |
| [167](feuer/src/cache.rs#L167) | Local | `tests::callback_result_and_covering_memory_hit_return_the_exact_request::result` | — | Keep |
| [180](feuer/src/cache.rs#L180) | Local | `tests::every_concurrent_miss_invokes_its_own_callback::cache` | — | Keep |
| [181](feuer/src/cache.rs#L181) | Local | `tests::every_concurrent_miss_invokes_its_own_callback::key` | — | Keep |
| [182](feuer/src/cache.rs#L182) | Local | `tests::every_concurrent_miss_invokes_its_own_callback::barrier` | — | Keep |
| [183](feuer/src/cache.rs#L183) | Local | `tests::every_concurrent_miss_invokes_its_own_callback::callback_count` | — | Keep |
| [184](feuer/src/cache.rs#L184) | Local | `tests::every_concurrent_miss_invokes_its_own_callback::mut tasks` | — | Keep |
| [187](feuer/src/cache.rs#L187) | Local | `tests::every_concurrent_miss_invokes_its_own_callback::cache` | — | Keep |
| [188](feuer/src/cache.rs#L188) | Local | `tests::every_concurrent_miss_invokes_its_own_callback::key` | — | Keep |
| [189](feuer/src/cache.rs#L189) | Local | `tests::every_concurrent_miss_invokes_its_own_callback::barrier` | — | Keep |
| [190](feuer/src/cache.rs#L190) | Local | `tests::every_concurrent_miss_invokes_its_own_callback::callback_count` | — | Keep |
| [212](feuer/src/cache.rs#L212) | Local | `tests::callback_errors_are_returned_without_retry_or_population::cache` | — | Keep |
| [213](feuer/src/cache.rs#L213) | Local | `tests::callback_errors_are_returned_without_retry_or_population::key` | — | Keep |
| [214](feuer/src/cache.rs#L214) | Local | `tests::callback_errors_are_returned_without_retry_or_population::callback_count` | — | Keep |
| [217](feuer/src/cache.rs#L217) | Local | `tests::callback_errors_are_returned_without_retry_or_population::invocation_count` | — | Keep |
| [218](feuer/src/cache.rs#L218) | Local | `tests::callback_errors_are_returned_without_retry_or_population::error` | — | Keep |
| [233](feuer/src/cache.rs#L233) | Local | `tests::rejects_noncovering_but_retains_oversized_callback_results::cache` | — | Keep |
| [234](feuer/src/cache.rs#L234) | Local | `tests::rejects_noncovering_but_retains_oversized_callback_results::key` | — | Keep |
| [236](feuer/src/cache.rs#L236) | Local | `tests::rejects_noncovering_but_retains_oversized_callback_results::error` | — | Keep |
| [250](feuer/src/cache.rs#L250) | Local | `tests::rejects_noncovering_but_retains_oversized_callback_results::result` | — | Keep |
| [258](feuer/src/cache.rs#L258) | Local | `tests::rejects_noncovering_but_retains_oversized_callback_results::unexpected_callback_count` | — | Keep |
| [259](feuer/src/cache.rs#L259) | Local | `tests::rejects_noncovering_but_retains_oversized_callback_results::count` | — | Keep |
| [260](feuer/src/cache.rs#L260) | Local | `tests::rejects_noncovering_but_retains_oversized_callback_results::result` | — | Keep |
| [273](feuer/src/cache.rs#L273) | Local | `tests::a_racing_contained_download_is_discarded_but_returns_its_own_bytes::cache` | — | Keep |
| [274](feuer/src/cache.rs#L274) | Local | `tests::a_racing_contained_download_is_discarded_but_returns_its_own_bytes::key` | — | Keep |
| [275](feuer/src/cache.rs#L275) | Local | `tests::a_racing_contained_download_is_discarded_but_returns_its_own_bytes::callback_entered` | — | Keep |
| [276](feuer/src/cache.rs#L276) | Local | `tests::a_racing_contained_download_is_discarded_but_returns_its_own_bytes::release_callback` | — | Keep |
| [278](feuer/src/cache.rs#L278) | Local | `tests::a_racing_contained_download_is_discarded_but_returns_its_own_bytes::pending` | — | Keep |
| [279](feuer/src/cache.rs#L279) | Local | `tests::a_racing_contained_download_is_discarded_but_returns_its_own_bytes::pending::cache` | — | Keep |
| [280](feuer/src/cache.rs#L280) | Local | `tests::a_racing_contained_download_is_discarded_but_returns_its_own_bytes::pending::key` | — | Keep |
| [281](feuer/src/cache.rs#L281) | Local | `tests::a_racing_contained_download_is_discarded_but_returns_its_own_bytes::pending::callback_entered` | — | Keep |
| [282](feuer/src/cache.rs#L282) | Local | `tests::a_racing_contained_download_is_discarded_but_returns_its_own_bytes::pending::release_callback` | — | Keep |
| [305](feuer/src/cache.rs#L305) | Local | `tests::a_racing_contained_download_is_discarded_but_returns_its_own_bytes::unexpected_callback_count` | — | Keep |
| [306](feuer/src/cache.rs#L306) | Local | `tests::a_racing_contained_download_is_discarded_but_returns_its_own_bytes::count` | — | Keep |
| [307](feuer/src/cache.rs#L307) | Local | `tests::a_racing_contained_download_is_discarded_but_returns_its_own_bytes::cached` | — | Keep |

</details>

### `feuer/src/config.rs`

| Line | Kind | Name / source parent | Signature or type | Review decision |
| ---: | --- | --- | --- | --- |
| [12](feuer/src/config.rs#L12) | Struct | `CacheConfig` | — | Rename from `Config` |
| [13](feuer/src/config.rs#L13) | Field | `CacheConfig::directory` | `PathBuf` | Keep |
| [14](feuer/src/config.rs#L14) | Field | `CacheConfig::disk_capacity` | `u64` | Keep |
| [15](feuer/src/config.rs#L15) | Field | `CacheConfig::memory_capacity` | `u64` | Keep |
| [18](feuer/src/config.rs#L18) | Impl | `impl CacheConfig` | — | Updated type references |
| [20](feuer/src/config.rs#L20) | Function | `impl CacheConfig::new` | `fn( directory: impl Into<PathBuf>, disk_capacity: u64, memory_capacity: u64, ) -> Result<Self, CacheConfigError>` | Keep |
| [40](feuer/src/config.rs#L40) | Method | `impl CacheConfig::directory` | `fn(&self) -> &Path` | Keep |
| [45](feuer/src/config.rs#L45) | Method | `impl CacheConfig::disk_capacity` | `fn(&self) -> u64` | Keep |
| [50](feuer/src/config.rs#L50) | Method | `impl CacheConfig::memory_capacity` | `fn(&self) -> u64` | Keep |
| [57](feuer/src/config.rs#L57) | Enum | `CacheConfigError` | — | Rename from `ConfigError` |
| [60](feuer/src/config.rs#L60) | Variant | `CacheConfigError::InvalidDiskCapacity` | — | Keep |
| [63](feuer/src/config.rs#L63) | Variant | `CacheConfigError::InvalidMemoryCapacity` | — | Keep |
| [67](feuer/src/config.rs#L67) | Module | `tests` | — | Keep |
| [71](feuer/src/config.rs#L71) | Function | `tests::requires_both_capacity_roles_to_be_explicit` | `fn()` | Keep |
| [80](feuer/src/config.rs#L80) | Function | `tests::represents_tib_scale_capacity` | `fn()` | Keep |
| [89](feuer/src/config.rs#L89) | Function | `tests::rejects_zero_capacities` | `fn()` | Keep |

<details>
<summary>Local bindings (3)</summary>

| Line | Kind | Name / source parent | Signature or type | Review decision |
| ---: | --- | --- | --- | --- |
| [72](feuer/src/config.rs#L72) | Local | `tests::requires_both_capacity_roles_to_be_explicit::config` | — | Keep |
| [81](feuer/src/config.rs#L81) | Local | `tests::represents_tib_scale_capacity::capacity` | — | Keep |
| [82](feuer/src/config.rs#L82) | Local | `tests::represents_tib_scale_capacity::config` | — | Keep |

</details>

### `feuer/src/lib.rs`

| Line | Kind | Name / source parent | Signature or type | Review decision |
| ---: | --- | --- | --- | --- |
| [8](feuer/src/lib.rs#L8) | Module | `cache` | — | Keep |
| [9](feuer/src/lib.rs#L9) | Module | `config` | — | Keep |

## feuer-memory

### `feuer-memory/src/lib.rs`

| Line | Kind | Name / source parent | Signature or type | Review decision |
| ---: | --- | --- | --- | --- |
| [8](feuer-memory/src/lib.rs#L8) | Module | `metrics` | — | Keep |
| [9](feuer-memory/src/lib.rs#L9) | Module | `store` | — | Keep |

### `feuer-memory/src/metrics.rs`

| Line | Kind | Name / source parent | Signature or type | Review decision |
| ---: | --- | --- | --- | --- |
| [9](feuer-memory/src/metrics.rs#L9) | Struct | `MemoryMetrics` | — | Keep |
| [10](feuer-memory/src/metrics.rs#L10) | Field | `MemoryMetrics::insert` | `BoxedCounter` | Keep |
| [11](feuer-memory/src/metrics.rs#L11) | Field | `MemoryMetrics::replace` | `BoxedCounter` | Keep |
| [12](feuer-memory/src/metrics.rs#L12) | Field | `MemoryMetrics::redundant` | `BoxedCounter` | Keep |
| [13](feuer-memory/src/metrics.rs#L13) | Field | `MemoryMetrics::access` | `BoxedCounter` | Keep |
| [14](feuer-memory/src/metrics.rs#L14) | Field | `MemoryMetrics::hit` | `BoxedCounter` | Keep |
| [15](feuer-memory/src/metrics.rs#L15) | Field | `MemoryMetrics::miss` | `BoxedCounter` | Keep |
| [16](feuer-memory/src/metrics.rs#L16) | Field | `MemoryMetrics::remove` | `BoxedCounter` | Keep |
| [17](feuer-memory/src/metrics.rs#L17) | Field | `MemoryMetrics::evict` | `BoxedCounter` | Keep |
| [18](feuer-memory/src/metrics.rs#L18) | Field | `MemoryMetrics::trim` | `BoxedCounter` | Rename from `compact` |
| [19](feuer-memory/src/metrics.rs#L19) | Field | `MemoryMetrics::trimmed_payload_bytes` | `BoxedCounter` | Rename from `compacted_payload_bytes` |
| [20](feuer-memory/src/metrics.rs#L20) | Field | `MemoryMetrics::payload_bytes` | `BoxedGauge` | Keep |
| [21](feuer-memory/src/metrics.rs#L21) | Field | `MemoryMetrics::entries` | `BoxedGauge` | Keep |
| [24](feuer-memory/src/metrics.rs#L24) | Impl | `impl fmt::Debug for MemoryMetrics` | — | Keep |
| [25](feuer-memory/src/metrics.rs#L25) | Method | `impl fmt::Debug for MemoryMetrics::fmt` | `fn(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result` | Keep |
| [30](feuer-memory/src/metrics.rs#L30) | Impl | `impl MemoryMetrics` | — | Keep |
| [32](feuer-memory/src/metrics.rs#L32) | Function | `impl MemoryMetrics::new` | `fn(registry: &BoxedRegistry) -> Arc<Self>` | Keep |
| [71](feuer-memory/src/metrics.rs#L71) | Method | `impl MemoryMetrics::record_insert` | `fn(&self, replaced: bool)` | Keep |
| [79](feuer-memory/src/metrics.rs#L79) | Method | `impl MemoryMetrics::record_redundant` | `fn(&self)` | Keep |
| [83](feuer-memory/src/metrics.rs#L83) | Method | `impl MemoryMetrics::record_access` | `fn(&self)` | Keep |
| [87](feuer-memory/src/metrics.rs#L87) | Method | `impl MemoryMetrics::record_lookup` | `fn(&self, hit: bool)` | Keep |
| [95](feuer-memory/src/metrics.rs#L95) | Method | `impl MemoryMetrics::record_remove` | `fn(&self)` | Keep |
| [99](feuer-memory/src/metrics.rs#L99) | Method | `impl MemoryMetrics::record_evictions` | `fn(&self, count: u64)` | Keep |
| [103](feuer-memory/src/metrics.rs#L103) | Method | `impl MemoryMetrics::record_range_trim` | `fn(&self, reclaimed_bytes: u64)` | Rename from `record_compaction` |
| [108](feuer-memory/src/metrics.rs#L108) | Method | `impl MemoryMetrics::increase_usage` | `fn(&self, bytes: u64, entries: u64)` | Keep |
| [113](feuer-memory/src/metrics.rs#L113) | Method | `impl MemoryMetrics::decrease_usage` | `fn(&self, bytes: u64, entries: u64)` | Keep |
| [118](feuer-memory/src/metrics.rs#L118) | Function | `impl MemoryMetrics::noop` | `fn() -> Arc<Self>` | Keep |
| [125](feuer-memory/src/metrics.rs#L125) | Module | `tests` | — | Keep |
| [129](feuer-memory/src/metrics.rs#L129) | Function | `tests::registers_and_updates_through_the_normal_registry_boundary` | `fn()` | Keep |

<details>
<summary>Local bindings (7)</summary>

| Line | Kind | Name / source parent | Signature or type | Review decision |
| ---: | --- | --- | --- | --- |
| [33](feuer-memory/src/metrics.rs#L33) | Local | `impl MemoryMetrics::new::operations` | — | Keep |
| [38](feuer-memory/src/metrics.rs#L38) | Local | `impl MemoryMetrics::new::trimmed_payload_bytes` | — | Rename from `compacted_payload_bytes` |
| [43](feuer-memory/src/metrics.rs#L43) | Local | `impl MemoryMetrics::new::payload_bytes` | — | Keep |
| [48](feuer-memory/src/metrics.rs#L48) | Local | `impl MemoryMetrics::new::entries` | — | Keep |
| [53](feuer-memory/src/metrics.rs#L53) | Local | `impl MemoryMetrics::new::operation` | — | Keep |
| [119](feuer-memory/src/metrics.rs#L119) | Local | `impl MemoryMetrics::noop::registry` | `BoxedRegistry` | Keep |
| [130](feuer-memory/src/metrics.rs#L130) | Local | `tests::registers_and_updates_through_the_normal_registry_boundary::metrics` | — | Keep |

</details>

### `feuer-memory/src/store/access_history.rs`

| Line | Kind | Name / source parent | Signature or type | Review decision |
| ---: | --- | --- | --- | --- |
| [6](feuer-memory/src/store/access_history.rs#L6) | Const | `FIXED_RETRIEVAL_EQUIVALENT_BYTES` | `u64` | Keep |
| [8](feuer-memory/src/store/access_history.rs#L8) | Const | `MAX_ACCESS_EVENTS_PER_KEY` | `usize` | Keep |
| [10](feuer-memory/src/store/access_history.rs#L10) | Const | `MAX_ACCESS_AGE_ACCESSES` | `u64` | Keep |
| [14](feuer-memory/src/store/access_history.rs#L14) | Struct | `RangeAccess` | — | Rename from `AccessEvent` |
| [15](feuer-memory/src/store/access_history.rs#L15) | Field | `RangeAccess::range` | `ByteRange` | Keep |
| [16](feuer-memory/src/store/access_history.rs#L16) | Field | `RangeAccess::observed_at_access` | `u64` | Rename from `observed_at` |
| [24](feuer-memory/src/store/access_history.rs#L24) | Struct | `RangeAccessHistory` | — | Rename from `AccessHistory` |
| [25](feuer-memory/src/store/access_history.rs#L25) | Field | `RangeAccessHistory::events` | `VecDeque<RangeAccess>` | Keep |
| [28](feuer-memory/src/store/access_history.rs#L28) | Impl | `impl RangeAccessHistory` | — | Updated type references |
| [29](feuer-memory/src/store/access_history.rs#L29) | Method | `impl RangeAccessHistory::record` | `fn(&mut self, range: ByteRange, access_clock: u64)` | Keep |
| [41](feuer-memory/src/store/access_history.rs#L41) | Method | `impl RangeAccessHistory::active_ranges` | `fn(&self, access_clock: u64) -> impl Iterator<Item = ByteRange> + '_` | Keep |
| [49](feuer-memory/src/store/access_history.rs#L49) | Method | `impl RangeAccessHistory::covered_retrieval_cost` | `fn(&self, cached_range: ByteRange, access_clock: u64) -> u64` | Rename from `retention_value` |
| [57](feuer-memory/src/store/access_history.rs#L57) | Method | `impl RangeAccessHistory::expire` | `fn(&mut self, access_clock: u64)` | Keep |
| [68](feuer-memory/src/store/access_history.rs#L68) | Method | `impl RangeAccessHistory::ranges` | `fn(&self) -> Vec<ByteRange>` | Keep |
| [73](feuer-memory/src/store/access_history.rs#L73) | Method | `impl RangeAccessHistory::len` | `fn(&self) -> usize` | Keep |
| [78](feuer-memory/src/store/access_history.rs#L78) | Function | `is_active` | `fn(event: RangeAccess, access_clock: u64) -> bool` | Keep |
| [83](feuer-memory/src/store/access_history.rs#L83) | Module | `tests` | — | Keep |
| [86](feuer-memory/src/store/access_history.rs#L86) | Function | `tests::range` | `fn(start: u64, end: u64) -> ByteRange` | Keep |
| [91](feuer-memory/src/store/access_history.rs#L91) | Function | `tests::bounds_events_without_coalescing_repeated_ranges` | `fn()` | Keep |
| [109](feuer-memory/src/store/access_history.rs#L109) | Function | `tests::retains_full_retrieval_value_until_expiration` | `fn()` | Keep |
| [132](feuer-memory/src/store/access_history.rs#L132) | Function | `tests::credits_only_cached_ranges_covering_the_exact_request` | `fn()` | Keep |

<details>
<summary>Local bindings (8)</summary>

| Line | Kind | Name / source parent | Signature or type | Review decision |
| ---: | --- | --- | --- | --- |
| [92](feuer-memory/src/store/access_history.rs#L92) | Local | `tests::bounds_events_without_coalescing_repeated_ranges::repeated` | — | Keep |
| [93](feuer-memory/src/store/access_history.rs#L93) | Local | `tests::bounds_events_without_coalescing_repeated_ranges::mut history` | — | Keep |
| [95](feuer-memory/src/store/access_history.rs#L95) | Local | `tests::bounds_events_without_coalescing_repeated_ranges::requested` | — | Keep |
| [110](feuer-memory/src/store/access_history.rs#L110) | Local | `tests::retains_full_retrieval_value_until_expiration::requested` | — | Keep |
| [111](feuer-memory/src/store/access_history.rs#L111) | Local | `tests::retains_full_retrieval_value_until_expiration::cached_range` | — | Keep |
| [112](feuer-memory/src/store/access_history.rs#L112) | Local | `tests::retains_full_retrieval_value_until_expiration::mut history` | — | Keep |
| [116](feuer-memory/src/store/access_history.rs#L116) | Local | `tests::retains_full_retrieval_value_until_expiration::expected` | — | Keep |
| [133](feuer-memory/src/store/access_history.rs#L133) | Local | `tests::credits_only_cached_ranges_covering_the_exact_request::mut history` | — | Keep |

</details>

### `feuer-memory/src/store/range_trim.rs`

| Line | Kind | Name / source parent | Signature or type | Review decision |
| ---: | --- | --- | --- | --- |
| [4](feuer-memory/src/store/range_trim.rs#L4) | Const | `MIN_PAYLOAD_COMPACTION_SAVINGS_PERCENT` | `u64` | Replace `MIN_RECLAIM_DIVISOR` with percent. See focused follow-up |
| [8](feuer-memory/src/store/range_trim.rs#L8) | Struct | `RangeTrimPlan` | — | Rename from `CompactionPlan` |
| [9](feuer-memory/src/store/range_trim.rs#L9) | Field | `RangeTrimPlan::source_range` | `ByteRange` | Rename from `source` |
| [10](feuer-memory/src/store/range_trim.rs#L10) | Field | `RangeTrimPlan::retained_ranges` | `Vec<ByteRange>` | Rename from `retained` |
| [11](feuer-memory/src/store/range_trim.rs#L11) | Field | `RangeTrimPlan::retained_bytes` | `u64` | Keep |
| [14](feuer-memory/src/store/range_trim.rs#L14) | Impl | `impl RangeTrimPlan` | — | Updated type references |
| [15](feuer-memory/src/store/range_trim.rs#L15) | Method | `impl RangeTrimPlan::source_range` | `fn(&self) -> ByteRange` | Rename from `source` |
| [19](feuer-memory/src/store/range_trim.rs#L19) | Method | `impl RangeTrimPlan::retained_ranges` | `fn(&self) -> &[ByteRange]` | Rename from `retained` |
| [23](feuer-memory/src/store/range_trim.rs#L23) | Method | `impl RangeTrimPlan::reclaimed_bytes` | `fn(&self) -> u64` | Keep |
| [34](feuer-memory/src/store/range_trim.rs#L34) | Function | `plan_range_trim` | `fn( source_range: ByteRange, requested_ranges: impl IntoIterator<Item = ByteRange>, ) -> Option<RangeTrimPlan>` | Rename from `plan_compaction` |
| [73](feuer-memory/src/store/range_trim.rs#L73) | Module | `tests` | — | Keep |
| [76](feuer-memory/src/store/range_trim.rs#L76) | Function | `tests::range` | `fn(start: u64, end: u64) -> ByteRange` | Keep |
| [81](feuer-memory/src/store/range_trim.rs#L81) | Function | `tests::projects_and_groups_only_exact_requests_covered_by_the_source` | `fn()` | Keep |
| [101](feuer-memory/src/store/range_trim.rs#L101) | Function | `tests::merges_adjacent_requests_so_each_original_request_stays_coverable` | `fn()` | Keep |
| [108](feuer-memory/src/store/range_trim.rs#L108) | Function | `tests::skips_empty_or_low_savings_plans` | `fn()` | Keep |

<details>
<summary>Local bindings (6)</summary>

| Line | Kind | Name / source parent | Signature or type | Review decision |
| ---: | --- | --- | --- | --- |
| [38](feuer-memory/src/store/range_trim.rs#L38) | Local | `plan_range_trim::mut retained_ranges` | `Vec<_>` | Rename from `mut retained` |
| [44](feuer-memory/src/store/range_trim.rs#L44) | Local | `plan_range_trim::mut grouped` | `Vec<ByteRange>` | Keep |
| [59](feuer-memory/src/store/range_trim.rs#L59) | Local | `plan_range_trim::retained_bytes` | — | Keep |
| [60](feuer-memory/src/store/range_trim.rs#L60) | Local | `plan_range_trim::reclaimed_bytes` | — | Keep |
| [82](feuer-memory/src/store/range_trim.rs#L82) | Local | `tests::projects_and_groups_only_exact_requests_covered_by_the_source::plan` | — | Keep |
| [102](feuer-memory/src/store/range_trim.rs#L102) | Local | `tests::merges_adjacent_requests_so_each_original_request_stays_coverable::plan` | — | Keep |

</details>

### `feuer-memory/src/store/shard.rs`

| Line | Kind | Name / source parent | Signature or type | Review decision |
| ---: | --- | --- | --- | --- |
| [17](feuer-memory/src/store/shard.rs#L17) | Const | `MIN_SHARD_ACCESSES_BEFORE_PAYLOAD_COMPACTION` | `u64` | Rename from `RANGE_TRIM_GRACE_ACCESSES`. See focused follow-up |
| [16](feuer-memory/src/store/shard.rs#L16) | Const | `RECLAIM_SAMPLE_SIZE` | `usize` | Rename from `PRESSURE_SAMPLE_SIZE` |
| [19](feuer-memory/src/store/shard.rs#L19) | Struct | `CachedRange` | — | Keep |
| [21](feuer-memory/src/store/shard.rs#L21) | Field | `CachedRange::id` | `u64` | Keep |
| [23](feuer-memory/src/store/shard.rs#L23) | Field | `CachedRange::range` | `ByteRange` | Keep |
| [25](feuer-memory/src/store/shard.rs#L25) | Field | `CachedRange::bytes` | `Bytes` | Keep |
| [27](feuer-memory/src/store/shard.rs#L27) | Field | `CachedRange::candidate_slot` | `usize` | Keep |
| [29](feuer-memory/src/store/shard.rs#L29) | Field | `CachedRange::admitted_at_access` | `u64` | Rename from `admitted_at` |
| [32](feuer-memory/src/store/shard.rs#L32) | Impl | `impl CachedRange` | — | Keep |
| [33](feuer-memory/src/store/shard.rs#L33) | Method | `impl CachedRange::requested_bytes` | `fn(&self, requested_range: ByteRange) -> Bytes` | Keep |
| [49](feuer-memory/src/store/shard.rs#L49) | Struct | `ObjectCachedRanges` | — | Keep |
| [51](feuer-memory/src/store/shard.rs#L51) | Field | `ObjectCachedRanges::by_start` | `BTreeMap<u64, CachedRange>` | Keep |
| [53](feuer-memory/src/store/shard.rs#L53) | Field | `ObjectCachedRanges::accesses` | `RangeAccessHistory` | Keep |
| [55](feuer-memory/src/store/shard.rs#L55) | Field | `ObjectCachedRanges::object_generation` | `u64` | Rename from `generation` |
| [58](feuer-memory/src/store/shard.rs#L58) | Impl | `impl ObjectCachedRanges` | — | Keep |
| [59](feuer-memory/src/store/shard.rs#L59) | Method | `impl ObjectCachedRanges::covering` | `fn(&self, range: ByteRange) -> Option<&CachedRange>` | Keep |
| [64](feuer-memory/src/store/shard.rs#L64) | Method | `impl ObjectCachedRanges::record_covering_access` | `fn<R>( &mut self, requested: ByteRange, access_clock: u64, project: impl FnOnce(&CachedRange) -> R, ) -> Option<R>` | Rename from `observe_covering` |
| [82](feuer-memory/src/store/shard.rs#L82) | Method | `impl ObjectCachedRanges::superseded_by` | `fn(&self, range: ByteRange) -> SupersededRanges` | Keep |
| [96](feuer-memory/src/store/shard.rs#L96) | Struct | `SupersededRanges` | — | Keep |
| [97](feuer-memory/src/store/shard.rs#L97) | Field | `SupersededRanges::ranges` | `Vec<ByteRange>` | Keep |
| [98](feuer-memory/src/store/shard.rs#L98) | Field | `SupersededRanges::payload_bytes` | `u64` | Rename from `bytes` |
| [103](feuer-memory/src/store/shard.rs#L103) | Struct | `RemovedCacheUsage` | — | Rename from `RemovedUsage` |
| [104](feuer-memory/src/store/shard.rs#L104) | Field | `RemovedCacheUsage::payload_bytes` | `u64` | Rename from `bytes` |
| [105](feuer-memory/src/store/shard.rs#L105) | Field | `RemovedCacheUsage::entry_count` | `u64` | Rename from `entries` |
| [110](feuer-memory/src/store/shard.rs#L110) | Struct | `CachedRangeIdentity` | — | Keep |
| [111](feuer-memory/src/store/shard.rs#L111) | Field | `CachedRangeIdentity::object_key` | `ObjectKey` | Keep |
| [112](feuer-memory/src/store/shard.rs#L112) | Field | `CachedRangeIdentity::start` | `u64` | Keep |
| [113](feuer-memory/src/store/shard.rs#L113) | Field | `CachedRangeIdentity::id` | `u64` | Keep |
| [118](feuer-memory/src/store/shard.rs#L118) | Struct | `ReclaimCandidateRing` | — | Rename from `PressureCandidates` |
| [119](feuer-memory/src/store/shard.rs#L119) | Field | `ReclaimCandidateRing::entries` | `Vec<CachedRangeIdentity>` | Keep |
| [120](feuer-memory/src/store/shard.rs#L120) | Field | `ReclaimCandidateRing::cursor` | `usize` | Keep |
| [123](feuer-memory/src/store/shard.rs#L123) | Impl | `impl ReclaimCandidateRing` | — | Updated type references |
| [124](feuer-memory/src/store/shard.rs#L124) | Method | `impl ReclaimCandidateRing::register` | `fn(&mut self, candidate: CachedRangeIdentity) -> usize` | Keep |
| [131](feuer-memory/src/store/shard.rs#L131) | Method | `impl ReclaimCandidateRing::remove` | `fn(&mut self, slot: usize, expected_id: u64) -> Option<CachedRangeIdentity>` | Keep |
| [144](feuer-memory/src/store/shard.rs#L144) | Method | `impl ReclaimCandidateRing::sample` | `fn(&mut self) -> (usize, usize)` | Keep |
| [156](feuer-memory/src/store/shard.rs#L156) | Struct | `ReclaimCandidate` | — | Rename from `PressureCandidate` |
| [157](feuer-memory/src/store/shard.rs#L157) | Field | `ReclaimCandidate::object_key` | `ObjectKey` | Keep |
| [158](feuer-memory/src/store/shard.rs#L158) | Field | `ReclaimCandidate::range` | `ByteRange` | Keep |
| [159](feuer-memory/src/store/shard.rs#L159) | Field | `ReclaimCandidate::id` | `u64` | Keep |
| [160](feuer-memory/src/store/shard.rs#L160) | Field | `ReclaimCandidate::retained_bytes` | `u64` | Keep |
| [161](feuer-memory/src/store/shard.rs#L161) | Field | `ReclaimCandidate::retrieval_cost` | `u64` | Rename from `retrieval_value` |
| [165](feuer-memory/src/store/shard.rs#L165) | Struct | `RangeTrimSource` | — | Rename from `CompactionSource` |
| [166](feuer-memory/src/store/shard.rs#L166) | Field | `RangeTrimSource::object_key` | `ObjectKey` | Keep |
| [167](feuer-memory/src/store/shard.rs#L167) | Field | `RangeTrimSource::start` | `u64` | Keep |
| [168](feuer-memory/src/store/shard.rs#L168) | Field | `RangeTrimSource::id` | `u64` | Keep |
| [169](feuer-memory/src/store/shard.rs#L169) | Field | `RangeTrimSource::object_generation` | `u64` | Rename from `generation` |
| [170](feuer-memory/src/store/shard.rs#L170) | Field | `RangeTrimSource::plan` | `RangeTrimPlan` | Keep |
| [171](feuer-memory/src/store/shard.rs#L171) | Field | `RangeTrimSource::source_bytes` | `Bytes` | Keep |
| [174](feuer-memory/src/store/shard.rs#L174) | Impl | `impl RangeTrimSource` | — | Updated type references |
| [176](feuer-memory/src/store/shard.rs#L176) | Method | `impl RangeTrimSource::copy_payload` | `fn(self) -> RangeTrimReplacement` | Keep |
| [201](feuer-memory/src/store/shard.rs#L201) | Struct | `RangeTrimReplacement` | — | Rename from `CompactionReplacement` |
| [202](feuer-memory/src/store/shard.rs#L202) | Field | `RangeTrimReplacement::object_key` | `ObjectKey` | Keep |
| [203](feuer-memory/src/store/shard.rs#L203) | Field | `RangeTrimReplacement::start` | `u64` | Keep |
| [204](feuer-memory/src/store/shard.rs#L204) | Field | `RangeTrimReplacement::id` | `u64` | Keep |
| [205](feuer-memory/src/store/shard.rs#L205) | Field | `RangeTrimReplacement::object_generation` | `u64` | Rename from `generation` |
| [206](feuer-memory/src/store/shard.rs#L206) | Field | `RangeTrimReplacement::plan` | `RangeTrimPlan` | Keep |
| [207](feuer-memory/src/store/shard.rs#L207) | Field | `RangeTrimReplacement::retained_payloads` | `Vec<(ByteRange, Bytes)>` | Rename from `retained` |
| [211](feuer-memory/src/store/shard.rs#L211) | Enum | `AdmissionProgress` | — | Rename from `AdmissionStep` |
| [212](feuer-memory/src/store/shard.rs#L212) | Variant | `AdmissionProgress::Complete` | — | Keep |
| [213](feuer-memory/src/store/shard.rs#L213) | Variant | `AdmissionProgress::Retry` | — | Keep |
| [214](feuer-memory/src/store/shard.rs#L214) | Variant | `AdmissionProgress::Trim` | — | Rename from `Compact` |
| [218](feuer-memory/src/store/shard.rs#L218) | Struct | `MemoryCacheShard` | — | Rename from `MemoryShard` |
| [219](feuer-memory/src/store/shard.rs#L219) | Field | `MemoryCacheShard::capacity` | `u64` | Keep |
| [220](feuer-memory/src/store/shard.rs#L220) | Field | `MemoryCacheShard::used_bytes` | `u64` | Keep |
| [221](feuer-memory/src/store/shard.rs#L221) | Field | `MemoryCacheShard::ranges` | `FxHashMap<ObjectKey, ObjectCachedRanges>` | Keep |
| [222](feuer-memory/src/store/shard.rs#L222) | Field | `MemoryCacheShard::access_clock` | `u64` | Keep |
| [223](feuer-memory/src/store/shard.rs#L223) | Field | `MemoryCacheShard::next_entry_id` | `u64` | Keep |
| [224](feuer-memory/src/store/shard.rs#L224) | Field | `MemoryCacheShard::candidates` | `ReclaimCandidateRing` | Keep |
| [225](feuer-memory/src/store/shard.rs#L225) | Field | `MemoryCacheShard::metrics` | `Arc<MemoryMetrics>` | Keep |
| [228](feuer-memory/src/store/shard.rs#L228) | Impl | `impl MemoryCacheShard` | — | Updated type references |
| [229](feuer-memory/src/store/shard.rs#L229) | Function | `impl MemoryCacheShard::new` | `fn(capacity: u64, metrics: Arc<MemoryMetrics>) -> Self` | Keep |
| [241](feuer-memory/src/store/shard.rs#L241) | Method | `impl MemoryCacheShard::used_bytes` | `fn(&self) -> u64` | Keep |
| [245](feuer-memory/src/store/shard.rs#L245) | Method | `impl MemoryCacheShard::get` | `fn(&mut self, object_key: &ObjectKey, requested_range: ByteRange) -> Option<Bytes>` | Keep |
| [263](feuer-memory/src/store/shard.rs#L263) | Method | `impl MemoryCacheShard::record_access` | `fn(&mut self, object_key: &ObjectKey, requested_range: ByteRange)` | Keep |
| [267](feuer-memory/src/store/shard.rs#L267) | Method | `impl MemoryCacheShard::record_successful_access` | `fn(&mut self, object_key: &ObjectKey, requested_range: ByteRange)` | Keep |
| [276](feuer-memory/src/store/shard.rs#L276) | Method | `impl MemoryCacheShard::advance_admission` | `fn( &mut self, object_key: &ObjectKey, range: ByteRange, bytes: &Bytes, requested_range: Option<ByteRange>, allow_range_trim: bool, ) -> AdmissionProgress` | Rename from `admission_step` |
| [338](feuer-memory/src/store/shard.rs#L338) | Method | `impl MemoryCacheShard::insert_admission` | `fn(&mut self, object_key: ObjectKey, range: ByteRange, bytes: Bytes)` | Keep |
| [360](feuer-memory/src/store/shard.rs#L360) | Method | `impl MemoryCacheShard::insert_trimmed` | `fn(&mut self, object_key: &ObjectKey, range: ByteRange, bytes: Bytes)` | Rename from `insert_compacted` |
| [380](feuer-memory/src/store/shard.rs#L380) | Method | `impl MemoryCacheShard::allocate_entry_id` | `fn(&mut self) -> u64` | Keep |
| [388](feuer-memory/src/store/shard.rs#L388) | Method | `impl MemoryCacheShard::remove_superseded` | `fn(&mut self, object_key: &ObjectKey, ranges: &[ByteRange]) -> RemovedCacheUsage` | Keep |
| [400](feuer-memory/src/store/shard.rs#L400) | Method | `impl MemoryCacheShard::remove` | `fn(&mut self, object_key: &ObjectKey, range: ByteRange) -> bool` | Keep |
| [409](feuer-memory/src/store/shard.rs#L409) | Method | `impl MemoryCacheShard::detach_entry` | `fn( &mut self, object_key: &ObjectKey, range: ByteRange, expected_id: Option<u64>, preserve_access_history: bool, ) -> Option<u64>` | Keep |
| [440](feuer-memory/src/store/shard.rs#L440) | Method | `impl MemoryCacheShard::unregister_candidate` | `fn(&mut self, slot: usize, expected_id: u64)` | Keep |
| [455](feuer-memory/src/store/shard.rs#L455) | Method | `impl MemoryCacheShard::select_reclaim_candidate` | `fn( &mut self, admitting_key: &ObjectKey, admitting_range: ByteRange, ) -> Option<ReclaimCandidate>` | Rename from `pressure_candidate` |
| [497](feuer-memory/src/store/shard.rs#L497) | Method | `impl MemoryCacheShard::range_trim_source` | `fn(&self, candidate: &ReclaimCandidate) -> Option<RangeTrimSource>` | Rename from `compaction_source` |
| [522](feuer-memory/src/store/shard.rs#L522) | Method | `impl MemoryCacheShard::publish_range_trim` | `fn(&mut self, replacement: RangeTrimReplacement) -> bool` | Rename from `publish_compaction` |
| [570](feuer-memory/src/store/shard.rs#L570) | Method | `impl MemoryCacheShard::entry_count` | `fn(&self) -> usize` | Keep |
| [575](feuer-memory/src/store/shard.rs#L575) | Method | `impl MemoryCacheShard::accessed_ranges` | `fn(&self, object_key: &ObjectKey) -> Vec<ByteRange>` | Keep |
| [582](feuer-memory/src/store/shard.rs#L582) | Method | `impl MemoryCacheShard::access_history_len` | `fn(&self, object_key: &ObjectKey) -> usize` | Keep |
| [587](feuer-memory/src/store/shard.rs#L587) | Method | `impl MemoryCacheShard::candidate_count` | `fn(&self) -> usize` | Keep |
| [593](feuer-memory/src/store/shard.rs#L593) | Function | `compare_retrieval_cost_per_byte` | `fn(left: &ReclaimCandidate, right: &ReclaimCandidate) -> Ordering` | Rename from `compare_retention` |
| [605](feuer-memory/src/store/shard.rs#L605) | Function | `compare_cost_per_byte` | `fn(left_cost: u64, left_bytes: u64, right_cost: u64, right_bytes: u64) -> Ordering` | Rename from `compare_value_density` |
| [609](feuer-memory/src/store/shard.rs#L609) | Impl | `impl Drop for MemoryCacheShard` | — | Updated type references |
| [610](feuer-memory/src/store/shard.rs#L610) | Method | `impl Drop for MemoryCacheShard::drop` | `fn(&mut self)` | Keep |

<details>
<summary>Local bindings (62)</summary>

| Line | Kind | Name / source parent | Signature or type | Review decision |
| ---: | --- | --- | --- | --- |
| [35](feuer-memory/src/store/shard.rs#L35) | Local | `impl CachedRange::requested_bytes::start` | — | Keep |
| [37](feuer-memory/src/store/shard.rs#L37) | Local | `impl CachedRange::requested_bytes::end` | — | Keep |
| [60](feuer-memory/src/store/shard.rs#L60) | Local | `impl ObjectCachedRanges::covering::(_, entry)` | — | Keep |
| [70](feuer-memory/src/store/shard.rs#L70) | Local | `impl ObjectCachedRanges::record_covering_access::projected` | — | Keep |
| [71](feuer-memory/src/store/shard.rs#L71) | Local | `impl ObjectCachedRanges::record_covering_access::projected::(_, entry)` | — | Keep |
| [83](feuer-memory/src/store/shard.rs#L83) | Local | `impl ObjectCachedRanges::superseded_by::mut superseded` | — | Keep |
| [125](feuer-memory/src/store/shard.rs#L125) | Local | `impl ReclaimCandidateRing::register::slot` | — | Keep |
| [133](feuer-memory/src/store/shard.rs#L133) | Local | `impl ReclaimCandidateRing::remove::last` | — | Keep |
| [135](feuer-memory/src/store/shard.rs#L135) | Local | `impl ReclaimCandidateRing::remove::moved` | — | Keep |
| [145](feuer-memory/src/store/shard.rs#L145) | Local | `impl ReclaimCandidateRing::sample::count` | — | Keep |
| [149](feuer-memory/src/store/shard.rs#L149) | Local | `impl ReclaimCandidateRing::sample::start` | — | Keep |
| [177](feuer-memory/src/store/shard.rs#L177) | Local | `impl RangeTrimSource::copy_payload::retained_payloads` | — | Rename from `retained` |
| [182](feuer-memory/src/store/shard.rs#L182) | Local | `impl RangeTrimSource::copy_payload::retained_payloads::start` | — | Keep |
| [184](feuer-memory/src/store/shard.rs#L184) | Local | `impl RangeTrimSource::copy_payload::retained_payloads::end` | — | Keep |
| [246](feuer-memory/src/store/shard.rs#L246) | Local | `impl MemoryCacheShard::get::access_clock` | — | Keep |
| [247](feuer-memory/src/store/shard.rs#L247) | Local | `impl MemoryCacheShard::get::accessed` | — | Keep |
| [252](feuer-memory/src/store/shard.rs#L252) | Local | `impl MemoryCacheShard::get::Some(bytes)` | — | Keep |
| [284](feuer-memory/src/store/shard.rs#L284) | Local | `impl MemoryCacheShard::advance_admission::superseded` | — | Keep |
| [295](feuer-memory/src/store/shard.rs#L295) | Local | `impl MemoryCacheShard::advance_admission::added_bytes` | — | Keep |
| [296](feuer-memory/src/store/shard.rs#L296) | Local | `impl MemoryCacheShard::advance_admission::used_bytes_without_superseded` | — | Rename from `effective_used` |
| [297](feuer-memory/src/store/shard.rs#L297) | Local | `impl MemoryCacheShard::advance_admission::max_existing_bytes` | — | Rename from `target` |
| [300](feuer-memory/src/store/shard.rs#L300) | Local | `impl MemoryCacheShard::advance_admission::removal` | — | Keep |
| [315](feuer-memory/src/store/shard.rs#L315) | Local | `impl MemoryCacheShard::advance_admission::Some(candidate)` | — | Keep |
| [325](feuer-memory/src/store/shard.rs#L325) | Local | `impl MemoryCacheShard::advance_admission::removed` | — | Keep |
| [340](feuer-memory/src/store/shard.rs#L340) | Local | `impl MemoryCacheShard::insert_admission::id` | — | Keep |
| [341](feuer-memory/src/store/shard.rs#L341) | Local | `impl MemoryCacheShard::insert_admission::candidate_slot` | — | Keep |
| [346](feuer-memory/src/store/shard.rs#L346) | Local | `impl MemoryCacheShard::insert_admission::entry` | — | Keep |
| [354](feuer-memory/src/store/shard.rs#L354) | Local | `impl MemoryCacheShard::insert_admission::entries` | — | Keep |
| [356](feuer-memory/src/store/shard.rs#L356) | Local | `impl MemoryCacheShard::insert_admission::replaced` | — | Keep |
| [361](feuer-memory/src/store/shard.rs#L361) | Local | `impl MemoryCacheShard::insert_trimmed::id` | — | Keep |
| [362](feuer-memory/src/store/shard.rs#L362) | Local | `impl MemoryCacheShard::insert_trimmed::candidate_slot` | — | Keep |
| [367](feuer-memory/src/store/shard.rs#L367) | Local | `impl MemoryCacheShard::insert_trimmed::entry` | — | Keep |
| [374](feuer-memory/src/store/shard.rs#L374) | Local | `impl MemoryCacheShard::insert_trimmed::entries` | — | Keep |
| [376](feuer-memory/src/store/shard.rs#L376) | Local | `impl MemoryCacheShard::insert_trimmed::replaced` | — | Keep |
| [389](feuer-memory/src/store/shard.rs#L389) | Local | `impl MemoryCacheShard::remove_superseded::mut removal` | — | Keep |
| [391](feuer-memory/src/store/shard.rs#L391) | Local | `impl MemoryCacheShard::remove_superseded::bytes` | — | Keep |
| [401](feuer-memory/src/store/shard.rs#L401) | Local | `impl MemoryCacheShard::remove::Some(bytes)` | — | Keep |
| [416](feuer-memory/src/store/shard.rs#L416) | Local | `impl MemoryCacheShard::detach_entry::(entry, object_is_empty)` | — | Keep |
| [417](feuer-memory/src/store/shard.rs#L417) | Local | `impl MemoryCacheShard::detach_entry::(entry, object_is_empty)::entries` | — | Keep |
| [418](feuer-memory/src/store/shard.rs#L418) | Local | `impl MemoryCacheShard::detach_entry::(entry, object_is_empty)::current` | — | Keep |
| [422](feuer-memory/src/store/shard.rs#L422) | Local | `impl MemoryCacheShard::detach_entry::(entry, object_is_empty)::entry` | — | Keep |
| [427](feuer-memory/src/store/shard.rs#L427) | Local | `impl MemoryCacheShard::detach_entry::(entry, object_is_empty)::object_is_empty` | — | Keep |
| [435](feuer-memory/src/store/shard.rs#L435) | Local | `impl MemoryCacheShard::detach_entry::bytes` | — | Keep |
| [441](feuer-memory/src/store/shard.rs#L441) | Local | `impl MemoryCacheShard::unregister_candidate::moved` | — | Keep |
| [442](feuer-memory/src/store/shard.rs#L442) | Local | `impl MemoryCacheShard::unregister_candidate::Some(moved)` | — | Keep |
| [445](feuer-memory/src/store/shard.rs#L445) | Local | `impl MemoryCacheShard::unregister_candidate::entry` | — | Keep |
| [460](feuer-memory/src/store/shard.rs#L460) | Local | `impl MemoryCacheShard::select_reclaim_candidate::(sample_start, sample_count)` | — | Keep |
| [461](feuer-memory/src/store/shard.rs#L461) | Local | `impl MemoryCacheShard::select_reclaim_candidate::candidate_count` | — | Keep |
| [462](feuer-memory/src/store/shard.rs#L462) | Local | `impl MemoryCacheShard::select_reclaim_candidate::mut selected` | `Option<ReclaimCandidate>` | Keep |
| [465](feuer-memory/src/store/shard.rs#L465) | Local | `impl MemoryCacheShard::select_reclaim_candidate::candidate` | — | Keep |
| [466](feuer-memory/src/store/shard.rs#L466) | Local | `impl MemoryCacheShard::select_reclaim_candidate::entries` | — | Keep |
| [470](feuer-memory/src/store/shard.rs#L470) | Local | `impl MemoryCacheShard::select_reclaim_candidate::entry` | — | Keep |
| [479](feuer-memory/src/store/shard.rs#L479) | Local | `impl MemoryCacheShard::select_reclaim_candidate::sampled` | — | Keep |
| [498](feuer-memory/src/store/shard.rs#L498) | Local | `impl MemoryCacheShard::range_trim_source::entries` | — | Keep |
| [502](feuer-memory/src/store/shard.rs#L502) | Local | `impl MemoryCacheShard::range_trim_source::entry` | — | Keep |
| [510](feuer-memory/src/store/shard.rs#L510) | Local | `impl MemoryCacheShard::range_trim_source::plan` | — | Keep |
| [523](feuer-memory/src/store/shard.rs#L523) | Local | `impl MemoryCacheShard::publish_range_trim::valid` | — | Keep |
| [534](feuer-memory/src/store/shard.rs#L534) | Local | `impl MemoryCacheShard::publish_range_trim::removed_bytes` | — | Keep |
| [542](feuer-memory/src/store/shard.rs#L542) | Local | `impl MemoryCacheShard::publish_range_trim::mut retained_bytes` | — | Keep |
| [543](feuer-memory/src/store/shard.rs#L543) | Local | `impl MemoryCacheShard::publish_range_trim::mut retained_entries` | — | Keep |
| [560](feuer-memory/src/store/shard.rs#L560) | Local | `impl MemoryCacheShard::publish_range_trim::reclaimed` | — | Keep |
| [611](feuer-memory/src/store/shard.rs#L611) | Local | `impl Drop for MemoryCacheShard::drop::entries` | — | Keep |

</details>

### `feuer-memory/src/store/tests.rs`

| Line | Kind | Name / source parent | Signature or type | Review decision |
| ---: | --- | --- | --- | --- |
| [14](feuer-memory/src/store/tests.rs#L14) | Function | `range` | `fn(start: u64, end: u64) -> ByteRange` | Keep |
| [18](feuer-memory/src/store/tests.rs#L18) | Function | `download` | `fn(expected_range: ByteRange, bytes: Bytes) -> Download` | Keep |
| [24](feuer-memory/src/store/tests.rs#L24) | Function | `populate` | `fn(cache: &MemoryCache, object_key: ObjectKey, download: Download)` | Keep |
| [28](feuer-memory/src/store/tests.rs#L28) | Function | `cache` | `fn(capacity: u64) -> MemoryCache` | Keep |
| [32](feuer-memory/src/store/tests.rs#L32) | Function | `accessed_ranges` | `fn(cache: &MemoryCache, key: &ObjectKey) -> Vec<ByteRange>` | Keep |
| [36](feuer-memory/src/store/tests.rs#L36) | Function | `access_history_len` | `fn(cache: &MemoryCache, key: &ObjectKey) -> usize` | Keep |
| [40](feuer-memory/src/store/tests.rs#L40) | Function | `candidate_count` | `fn(cache: &MemoryCache) -> usize` | Keep |
| [45](feuer-memory/src/store/tests.rs#L45) | Function | `covering_lookup_returns_only_requested_bytes_and_shares_the_allocation` | `fn()` | Keep |
| [64](feuer-memory/src/store/tests.rs#L64) | Function | `different_identity_or_noncovering_ranges_miss` | `fn()` | Keep |
| [76](feuer-memory/src/store/tests.rs#L76) | Function | `adjacent_entries_are_not_assembled_into_a_hit` | `fn()` | Keep |
| [86](feuer-memory/src/store/tests.rs#L86) | Function | `population_and_accesses_are_independent` | `fn()` | Keep |
| [111](feuer-memory/src/store/tests.rs#L111) | Function | `shared_download_population_is_deduplicated_but_each_waiter_records_an_access` | `fn()` | Keep |
| [129](feuer-memory/src/store/tests.rs#L129) | Function | `partially_overlapping_downloads_coexist_and_do_not_form_a_hit` | `fn()` | Keep |
| [150](feuer-memory/src/store/tests.rs#L150) | Function | `a_larger_download_replaces_contained_entries_but_not_partial_overlaps` | `fn()` | Keep |
| [177](feuer-memory/src/store/tests.rs#L177) | Function | `a_contained_download_is_discarded_without_replacing_cached_bytes` | `fn()` | Keep |
| [197](feuer-memory/src/store/tests.rs#L197) | Function | `capacity_is_charged_by_retained_download_payload_bytes` | `fn()` | Keep |
| [213](feuer-memory/src/store/tests.rs#L213) | Function | `repeated_redundant_insertions_do_not_change_usage_or_replace_data` | `fn()` | Keep |
| [227](feuer-memory/src/store/tests.rs#L227) | Function | `oversized_insertion_empties_its_shard_and_remains_cached` | `fn()` | Keep |
| [241](feuer-memory/src/store/tests.rs#L241) | Function | `accessed_ranges_survive_downloaded_range_replacement` | `fn()` | Keep |
| [261](feuer-memory/src/store/tests.rs#L261) | Function | `access_history_survives_same_object_eviction_during_replacement` | `fn()` | Keep |
| [288](feuer-memory/src/store/tests.rs#L288) | Function | `access_history_is_bounded_and_preserves_repeated_exact_requests` | `fn()` | Keep |
| [305](feuer-memory/src/store/tests.rs#L305) | Function | `repeated_requested_intervals_are_retained_over_single_accesses` | `fn()` | Keep |
| [328](feuer-memory/src/store/tests.rs#L328) | Function | `requested_bytes_contribute_to_retrieval_value` | `fn()` | Keep |
| [356](feuer-memory/src/store/tests.rs#L356) | Function | `retention_credit_is_projected_only_onto_the_requested_interval` | `fn()` | Keep |
| [371](feuer-memory/src/store/tests.rs#L371) | Function | `stale_frequency_eventually_expires` | `fn()` | Keep |
| [398](feuer-memory/src/store/tests.rs#L398) | Function | `range_trim_respects_grace_then_releases_unrequested_payload` | `fn()` | Rename from `compaction_respects_grace_then_releases_unrequested_payload` |
| [439](feuer-memory/src/store/tests.rs#L439) | Function | `range_trim_preserves_disjoint_requested_coverage_without_filling_gaps` | `fn()` | Rename from `compaction_preserves_disjoint_requested_coverage_without_filling_gaps` |
| [466](feuer-memory/src/store/tests.rs#L466) | Function | `range_trim_waits_for_pressure_and_adds_no_access` | `fn()` | Rename from `compaction_waits_for_pressure_and_adds_no_access` |
| [494](feuer-memory/src/store/tests.rs#L494) | Function | `candidate_state_tracks_entries_during_oversized_churn` | `fn()` | Keep |
| [507](feuer-memory/src/store/tests.rs#L507) | Function | `copied_range_trim_is_revalidated_before_publication_and_can_fall_back` | `fn()` | Rename from `copied_compaction_is_revalidated_before_publication_and_can_fall_back` |
| [545](feuer-memory/src/store/tests.rs#L545) | Function | `removing_the_last_cached_range_releases_its_access_history` | `fn()` | Keep |
| [557](feuer-memory/src/store/tests.rs#L557) | Function | `zero_target_still_retains_the_latest_entry` | `fn()` | Keep |
| [570](feuer-memory/src/store/tests.rs#L570) | Function | `configured_target_is_divided_without_losing_remainder_bytes` | `fn()` | Keep |
| [582](feuer-memory/src/store/tests.rs#L582) | Function | `shard_targets_can_collectively_exceed_the_configured_capacity` | `fn()` | Keep |
| [605](feuer-memory/src/store/tests.rs#L605) | Function | `concurrent_shards_respect_their_targets_for_regular_entries` | `fn()` | Keep |

<details>
<summary>Local bindings (85)</summary>

| Line | Kind | Name / source parent | Signature or type | Review decision |
| ---: | --- | --- | --- | --- |
| [19](feuer-memory/src/store/tests.rs#L19) | Local | `download::download` | — | Keep |
| [46](feuer-memory/src/store/tests.rs#L46) | Local | `covering_lookup_returns_only_requested_bytes_and_shares_the_allocation::cache` | — | Keep |
| [47](feuer-memory/src/store/tests.rs#L47) | Local | `covering_lookup_returns_only_requested_bytes_and_shares_the_allocation::key` | — | Keep |
| [48](feuer-memory/src/store/tests.rs#L48) | Local | `covering_lookup_returns_only_requested_bytes_and_shares_the_allocation::value` | — | Keep |
| [51](feuer-memory/src/store/tests.rs#L51) | Local | `covering_lookup_returns_only_requested_bytes_and_shares_the_allocation::result` | — | Keep |
| [65](feuer-memory/src/store/tests.rs#L65) | Local | `different_identity_or_noncovering_ranges_miss::cache` | — | Keep |
| [66](feuer-memory/src/store/tests.rs#L66) | Local | `different_identity_or_noncovering_ranges_miss::key` | — | Keep |
| [77](feuer-memory/src/store/tests.rs#L77) | Local | `adjacent_entries_are_not_assembled_into_a_hit::cache` | — | Keep |
| [78](feuer-memory/src/store/tests.rs#L78) | Local | `adjacent_entries_are_not_assembled_into_a_hit::key` | — | Keep |
| [87](feuer-memory/src/store/tests.rs#L87) | Local | `population_and_accesses_are_independent::cache` | — | Keep |
| [88](feuer-memory/src/store/tests.rs#L88) | Local | `population_and_accesses_are_independent::key` | — | Keep |
| [112](feuer-memory/src/store/tests.rs#L112) | Local | `shared_download_population_is_deduplicated_but_each_waiter_records_an_access::cache` | — | Keep |
| [113](feuer-memory/src/store/tests.rs#L113) | Local | `shared_download_population_is_deduplicated_but_each_waiter_records_an_access::key` | — | Keep |
| [114](feuer-memory/src/store/tests.rs#L114) | Local | `shared_download_population_is_deduplicated_but_each_waiter_records_an_access::shared` | — | Keep |
| [130](feuer-memory/src/store/tests.rs#L130) | Local | `partially_overlapping_downloads_coexist_and_do_not_form_a_hit::cache` | — | Keep |
| [131](feuer-memory/src/store/tests.rs#L131) | Local | `partially_overlapping_downloads_coexist_and_do_not_form_a_hit::key` | — | Keep |
| [151](feuer-memory/src/store/tests.rs#L151) | Local | `a_larger_download_replaces_contained_entries_but_not_partial_overlaps::cache` | — | Keep |
| [152](feuer-memory/src/store/tests.rs#L152) | Local | `a_larger_download_replaces_contained_entries_but_not_partial_overlaps::key` | — | Keep |
| [178](feuer-memory/src/store/tests.rs#L178) | Local | `a_contained_download_is_discarded_without_replacing_cached_bytes::cache` | — | Keep |
| [179](feuer-memory/src/store/tests.rs#L179) | Local | `a_contained_download_is_discarded_without_replacing_cached_bytes::key` | — | Keep |
| [198](feuer-memory/src/store/tests.rs#L198) | Local | `capacity_is_charged_by_retained_download_payload_bytes::cache` | — | Keep |
| [199](feuer-memory/src/store/tests.rs#L199) | Local | `capacity_is_charged_by_retained_download_payload_bytes::key` | — | Keep |
| [214](feuer-memory/src/store/tests.rs#L214) | Local | `repeated_redundant_insertions_do_not_change_usage_or_replace_data::cache` | — | Keep |
| [215](feuer-memory/src/store/tests.rs#L215) | Local | `repeated_redundant_insertions_do_not_change_usage_or_replace_data::key` | — | Keep |
| [228](feuer-memory/src/store/tests.rs#L228) | Local | `oversized_insertion_empties_its_shard_and_remains_cached::cache` | — | Keep |
| [229](feuer-memory/src/store/tests.rs#L229) | Local | `oversized_insertion_empties_its_shard_and_remains_cached::key` | — | Keep |
| [242](feuer-memory/src/store/tests.rs#L242) | Local | `accessed_ranges_survive_downloaded_range_replacement::cache` | — | Keep |
| [243](feuer-memory/src/store/tests.rs#L243) | Local | `accessed_ranges_survive_downloaded_range_replacement::key` | — | Keep |
| [262](feuer-memory/src/store/tests.rs#L262) | Local | `access_history_survives_same_object_eviction_during_replacement::cache` | — | Keep |
| [263](feuer-memory/src/store/tests.rs#L263) | Local | `access_history_survives_same_object_eviction_during_replacement::key` | — | Keep |
| [289](feuer-memory/src/store/tests.rs#L289) | Local | `access_history_is_bounded_and_preserves_repeated_exact_requests::cache` | — | Keep |
| [290](feuer-memory/src/store/tests.rs#L290) | Local | `access_history_is_bounded_and_preserves_repeated_exact_requests::key` | — | Keep |
| [306](feuer-memory/src/store/tests.rs#L306) | Local | `repeated_requested_intervals_are_retained_over_single_accesses::cache` | — | Keep |
| [307](feuer-memory/src/store/tests.rs#L307) | Local | `repeated_requested_intervals_are_retained_over_single_accesses::hot` | — | Keep |
| [308](feuer-memory/src/store/tests.rs#L308) | Local | `repeated_requested_intervals_are_retained_over_single_accesses::cold` | — | Keep |
| [309](feuer-memory/src/store/tests.rs#L309) | Local | `repeated_requested_intervals_are_retained_over_single_accesses::incoming` | — | Keep |
| [329](feuer-memory/src/store/tests.rs#L329) | Local | `requested_bytes_contribute_to_retrieval_value::cache` | — | Keep |
| [330](feuer-memory/src/store/tests.rs#L330) | Local | `requested_bytes_contribute_to_retrieval_value::small` | — | Keep |
| [331](feuer-memory/src/store/tests.rs#L331) | Local | `requested_bytes_contribute_to_retrieval_value::large` | — | Keep |
| [357](feuer-memory/src/store/tests.rs#L357) | Local | `retention_credit_is_projected_only_onto_the_requested_interval::cache` | — | Keep |
| [358](feuer-memory/src/store/tests.rs#L358) | Local | `retention_credit_is_projected_only_onto_the_requested_interval::key` | — | Keep |
| [359](feuer-memory/src/store/tests.rs#L359) | Local | `retention_credit_is_projected_only_onto_the_requested_interval::incoming` | — | Keep |
| [372](feuer-memory/src/store/tests.rs#L372) | Local | `stale_frequency_eventually_expires::cache` | — | Keep |
| [373](feuer-memory/src/store/tests.rs#L373) | Local | `stale_frequency_eventually_expires::stale` | — | Keep |
| [374](feuer-memory/src/store/tests.rs#L374) | Local | `stale_frequency_eventually_expires::fresh` | — | Keep |
| [375](feuer-memory/src/store/tests.rs#L375) | Local | `stale_frequency_eventually_expires::clock` | — | Keep |
| [399](feuer-memory/src/store/tests.rs#L399) | Local | `range_trim_respects_grace_then_releases_unrequested_payload::early_pressure` | — | Keep |
| [400](feuer-memory/src/store/tests.rs#L400) | Local | `range_trim_respects_grace_then_releases_unrequested_payload::early_key` | — | Keep |
| [413](feuer-memory/src/store/tests.rs#L413) | Local | `range_trim_respects_grace_then_releases_unrequested_payload::cache` | — | Keep |
| [414](feuer-memory/src/store/tests.rs#L414) | Local | `range_trim_respects_grace_then_releases_unrequested_payload::key` | — | Keep |
| [415](feuer-memory/src/store/tests.rs#L415) | Local | `range_trim_respects_grace_then_releases_unrequested_payload::incoming` | — | Keep |
| [416](feuer-memory/src/store/tests.rs#L416) | Local | `range_trim_respects_grace_then_releases_unrequested_payload::original` | — | Keep |
| [419](feuer-memory/src/store/tests.rs#L419) | Local | `range_trim_respects_grace_then_releases_unrequested_payload::returned` | — | Keep |
| [430](feuer-memory/src/store/tests.rs#L430) | Local | `range_trim_respects_grace_then_releases_unrequested_payload::retained` | — | Keep |
| [440](feuer-memory/src/store/tests.rs#L440) | Local | `range_trim_preserves_disjoint_requested_coverage_without_filling_gaps::cache` | — | Keep |
| [441](feuer-memory/src/store/tests.rs#L441) | Local | `range_trim_preserves_disjoint_requested_coverage_without_filling_gaps::key` | — | Keep |
| [467](feuer-memory/src/store/tests.rs#L467) | Local | `range_trim_waits_for_pressure_and_adds_no_access::cache` | — | Keep |
| [468](feuer-memory/src/store/tests.rs#L468) | Local | `range_trim_waits_for_pressure_and_adds_no_access::key` | — | Keep |
| [469](feuer-memory/src/store/tests.rs#L469) | Local | `range_trim_waits_for_pressure_and_adds_no_access::original` | — | Keep |
| [472](feuer-memory/src/store/tests.rs#L472) | Local | `range_trim_waits_for_pressure_and_adds_no_access::returned` | — | Keep |
| [488](feuer-memory/src/store/tests.rs#L488) | Local | `range_trim_waits_for_pressure_and_adds_no_access::retained` | — | Keep |
| [495](feuer-memory/src/store/tests.rs#L495) | Local | `candidate_state_tracks_entries_during_oversized_churn::cache` | — | Keep |
| [497](feuer-memory/src/store/tests.rs#L497) | Local | `candidate_state_tracks_entries_during_oversized_churn::key` | — | Keep |
| [508](feuer-memory/src/store/tests.rs#L508) | Local | `copied_range_trim_is_revalidated_before_publication_and_can_fall_back::cache` | — | Keep |
| [509](feuer-memory/src/store/tests.rs#L509) | Local | `copied_range_trim_is_revalidated_before_publication_and_can_fall_back::key` | — | Keep |
| [519](feuer-memory/src/store/tests.rs#L519) | Local | `copied_range_trim_is_revalidated_before_publication_and_can_fall_back::incoming` | — | Keep |
| [520](feuer-memory/src/store/tests.rs#L520) | Local | `copied_range_trim_is_revalidated_before_publication_and_can_fall_back::incoming_bytes` | — | Keep |
| [521](feuer-memory/src/store/tests.rs#L521) | Local | `copied_range_trim_is_revalidated_before_publication_and_can_fall_back::replacement` | — | Keep |
| [522](feuer-memory/src/store/tests.rs#L522) | Local | `copied_range_trim_is_revalidated_before_publication_and_can_fall_back::replacement::mut shard` | — | Keep |
| [523](feuer-memory/src/store/tests.rs#L523) | Local | `copied_range_trim_is_revalidated_before_publication_and_can_fall_back::replacement::AdmissionProgress::Trim(source)` | — | Rename from `AdmissionStep::Compact(source)` |
| [537](feuer-memory/src/store/tests.rs#L537) | Local | `copied_range_trim_is_revalidated_before_publication_and_can_fall_back::step` | — | Keep |
| [546](feuer-memory/src/store/tests.rs#L546) | Local | `removing_the_last_cached_range_releases_its_access_history::cache` | — | Keep |
| [547](feuer-memory/src/store/tests.rs#L547) | Local | `removing_the_last_cached_range_releases_its_access_history::key` | — | Keep |
| [558](feuer-memory/src/store/tests.rs#L558) | Local | `zero_target_still_retains_the_latest_entry::cache` | — | Keep |
| [559](feuer-memory/src/store/tests.rs#L559) | Local | `zero_target_still_retains_the_latest_entry::key` | — | Keep |
| [583](feuer-memory/src/store/tests.rs#L583) | Local | `shard_targets_can_collectively_exceed_the_configured_capacity::cache` | — | Keep |
| [584](feuer-memory/src/store/tests.rs#L584) | Local | `shard_targets_can_collectively_exceed_the_configured_capacity::mut keys` | — | Keep |
| [586](feuer-memory/src/store/tests.rs#L586) | Local | `shard_targets_can_collectively_exceed_the_configured_capacity::key` | — | Keep |
| [587](feuer-memory/src/store/tests.rs#L587) | Local | `shard_targets_can_collectively_exceed_the_configured_capacity::shard` | — | Keep |
| [593](feuer-memory/src/store/tests.rs#L593) | Local | `shard_targets_can_collectively_exceed_the_configured_capacity::[Some(first), Some(second)]` | — | Keep |
| [606](feuer-memory/src/store/tests.rs#L606) | Local | `concurrent_shards_respect_their_targets_for_regular_entries::cache` | — | Keep |
| [607](feuer-memory/src/store/tests.rs#L607) | Local | `concurrent_shards_respect_their_targets_for_regular_entries::mut threads` | — | Keep |
| [609](feuer-memory/src/store/tests.rs#L609) | Local | `concurrent_shards_respect_their_targets_for_regular_entries::cache` | — | Keep |
| [611](feuer-memory/src/store/tests.rs#L611) | Local | `concurrent_shards_respect_their_targets_for_regular_entries::key` | — | Keep |
| [613](feuer-memory/src/store/tests.rs#L613) | Local | `concurrent_shards_respect_their_targets_for_regular_entries::start` | — | Keep |

</details>

### `feuer-memory/src/store.rs`

| Line | Kind | Name / source parent | Signature or type | Review decision |
| ---: | --- | --- | --- | --- |
| [1](feuer-memory/src/store.rs#L1) | Module | `access_history` | — | Keep |
| [2](feuer-memory/src/store.rs#L2) | Module | `range_trim` | — | Rename from `compaction` |
| [3](feuer-memory/src/store.rs#L3) | Module | `shard` | — | Keep |
| [5](feuer-memory/src/store.rs#L5) | Module | `tests` | — | Keep |
| [22](feuer-memory/src/store.rs#L22) | Const | `MAX_SHARDS` | `usize` | Keep |
| [41](feuer-memory/src/store.rs#L41) | Struct | `MemoryCache` | — | Keep |
| [43](feuer-memory/src/store.rs#L43) | Field | `MemoryCache::capacity` | `u64` | Keep |
| [45](feuer-memory/src/store.rs#L45) | Field | `MemoryCache::shards` | `Box<[Mutex<MemoryCacheShard>]>` | Keep |
| [48](feuer-memory/src/store.rs#L48) | Impl | `impl fmt::Debug for MemoryCache` | — | Keep |
| [49](feuer-memory/src/store.rs#L49) | Method | `impl fmt::Debug for MemoryCache::fmt` | `fn(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result` | Keep |
| [58](feuer-memory/src/store.rs#L58) | Impl | `impl MemoryCache` | — | Keep |
| [60](feuer-memory/src/store.rs#L60) | Function | `impl MemoryCache::new` | `fn(capacity: u64) -> Self` | Keep |
| [65](feuer-memory/src/store.rs#L65) | Function | `impl MemoryCache::with_metrics` | `fn(capacity: u64, metrics: Arc<MemoryMetrics>) -> Self` | Keep |
| [72](feuer-memory/src/store.rs#L72) | Function | `impl MemoryCache::with_shards_for_benchmark` | `fn(capacity: u64, shard_count: usize) -> Self` | Keep |
| [76](feuer-memory/src/store.rs#L76) | Function | `impl MemoryCache::with_shard_count` | `fn(capacity: u64, metrics: Arc<MemoryMetrics>, shard_count: usize) -> Self` | Keep |
| [90](feuer-memory/src/store.rs#L90) | Method | `impl MemoryCache::capacity` | `fn(&self) -> u64` | Keep |
| [95](feuer-memory/src/store.rs#L95) | Method | `impl MemoryCache::used_bytes` | `fn(&self) -> u64` | Keep |
| [100](feuer-memory/src/store.rs#L100) | Method | `impl MemoryCache::get` | `fn(&self, object_key: &ObjectKey, requested_range: ByteRange) -> Option<Bytes>` | Keep |
| [110](feuer-memory/src/store.rs#L110) | Method | `impl MemoryCache::insert` | `fn(&self, object_key: ObjectKey, download: Download)` | Keep |
| [120](feuer-memory/src/store.rs#L120) | Method | `impl MemoryCache::insert_and_record` | `fn(&self, object_key: ObjectKey, download: Download, requested_range: ByteRange)` | Keep |
| [126](feuer-memory/src/store.rs#L126) | Method | `impl MemoryCache::admit_download` | `fn( &self, object_key: ObjectKey, downloaded_range: ByteRange, bytes: Bytes, requested_range: Option<ByteRange>, )` | Rename from `insert_inner` |
| [166](feuer-memory/src/store.rs#L166) | Method | `impl MemoryCache::record_access` | `fn(&self, object_key: &ObjectKey, requested_range: ByteRange)` | Keep |
| [174](feuer-memory/src/store.rs#L174) | Method | `impl MemoryCache::remove` | `fn(&self, object_key: &ObjectKey, range: ByteRange) -> bool` | Keep |
| [182](feuer-memory/src/store.rs#L182) | Method | `impl MemoryCache::shard_index` | `fn(&self, object_key: &ObjectKey) -> usize` | Keep |
| [189](feuer-memory/src/store.rs#L189) | Method | `impl MemoryCache::entry_count` | `fn(&self) -> u64` | Keep |
| [194](feuer-memory/src/store.rs#L194) | Function | `shard_capacity_for` | `fn(total: u64, shards: usize, index: usize) -> u64` | Keep |
| [199](feuer-memory/src/store.rs#L199) | Function | `default_shard_count` | `fn() -> usize` | Keep |

<details>
<summary>Local bindings (12)</summary>

| Line | Kind | Name / source parent | Signature or type | Review decision |
| ---: | --- | --- | --- | --- |
| [78](feuer-memory/src/store.rs#L78) | Local | `impl MemoryCache::with_shard_count::shards` | — | Keep |
| [101](feuer-memory/src/store.rs#L101) | Local | `impl MemoryCache::get::shard_index` | — | Keep |
| [111](feuer-memory/src/store.rs#L111) | Local | `impl MemoryCache::insert::(downloaded_range, bytes)` | — | Keep |
| [121](feuer-memory/src/store.rs#L121) | Local | `impl MemoryCache::insert_and_record::(downloaded_range, bytes)` | — | Keep |
| [133](feuer-memory/src/store.rs#L133) | Local | `impl MemoryCache::admit_download::shard_index` | — | Keep |
| [134](feuer-memory/src/store.rs#L134) | Local | `impl MemoryCache::admit_download::mut allow_range_trim` | — | Rename from `mut allow_compaction` |
| [136](feuer-memory/src/store.rs#L136) | Local | `impl MemoryCache::admit_download::step` | — | Keep |
| [150](feuer-memory/src/store.rs#L150) | Local | `impl MemoryCache::admit_download::replacement` | — | Keep |
| [167](feuer-memory/src/store.rs#L167) | Local | `impl MemoryCache::record_access::shard_index` | — | Keep |
| [175](feuer-memory/src/store.rs#L175) | Local | `impl MemoryCache::remove::shard_index` | — | Keep |
| [183](feuer-memory/src/store.rs#L183) | Local | `impl MemoryCache::shard_index::mut hasher` | — | Keep |
| [195](feuer-memory/src/store.rs#L195) | Local | `shard_capacity_for::shards` | — | Keep |

</details>

## feuer-memory-bench

### `benchmarks/memory/src/main.rs`

| Line | Kind | Name / source parent | Signature or type | Review decision |
| ---: | --- | --- | --- | --- |
| [17](benchmarks/memory/src/main.rs#L17) | Const | `PINNED_FOYER_REVISION` | `&str` | Keep |
| [18](benchmarks/memory/src/main.rs#L18) | Const | `SOURCE_FIXED_EQUIVALENT_BYTES` | `u64` | Keep |
| [20](benchmarks/memory/src/main.rs#L20) | Const | `FOYER_COST_SAMPLE_SIZE` | `usize` | Keep |
| [21](benchmarks/memory/src/main.rs#L21) | Const | `TRACE_FILE` | `&str` | Keep |
| [22](benchmarks/memory/src/main.rs#L22) | Const | `COALESCING_DISTANCE_ENV` | `&str` | Keep |
| [23](benchmarks/memory/src/main.rs#L23) | Const | `WHOLE_SPLIT_THRESHOLD_ENV` | `&str` | Keep |
| [24](benchmarks/memory/src/main.rs#L24) | Const | `DEFAULT_COALESCING_DISTANCE_BYTES` | `u64` | Keep |
| [25](benchmarks/memory/src/main.rs#L25) | Const | `DEFAULT_WHOLE_SPLIT_THRESHOLD_BYTES` | `u64` | Keep |
| [26](benchmarks/memory/src/main.rs#L26) | Const | `COALESCING_WINDOW_MILLIS` | `u64` | Keep |
| [27](benchmarks/memory/src/main.rs#L27) | Const | `CSV_HEADER` | `&str` | Keep |
| [31](benchmarks/memory/src/main.rs#L31) | Struct | `ReplayArgs` | — | Rename from `Args` |
| [39](benchmarks/memory/src/main.rs#L39) | Field | `ReplayArgs::capacities` | `Vec<usize>` | Keep |
| [43](benchmarks/memory/src/main.rs#L43) | Field | `ReplayArgs::shards` | `Vec<usize>` | Keep |
| [52](benchmarks/memory/src/main.rs#L52) | Field | `ReplayArgs::downloaders` | `Vec<DownloadRangePolicy>` | Keep |
| [56](benchmarks/memory/src/main.rs#L56) | Field | `ReplayArgs::warmup_iterations` | `usize` | Keep |
| [60](benchmarks/memory/src/main.rs#L60) | Field | `ReplayArgs::operations` | `Option<usize>` | Keep |
| [64](benchmarks/memory/src/main.rs#L64) | Field | `ReplayArgs::csv` | `bool` | Keep |
| [68](benchmarks/memory/src/main.rs#L68) | Enum | `DownloadRangePolicy` | — | Rename from `DownloadPolicy` |
| [69](benchmarks/memory/src/main.rs#L69) | Variant | `DownloadRangePolicy::Expanded` | — | Keep |
| [70](benchmarks/memory/src/main.rs#L70) | Variant | `DownloadRangePolicy::Exact` | — | Keep |
| [73](benchmarks/memory/src/main.rs#L73) | Impl | `impl DownloadRangePolicy` | — | Updated type references |
| [74](benchmarks/memory/src/main.rs#L74) | Method | `impl DownloadRangePolicy::name` | `fn(self) -> &'static str` | Keep |
| [83](benchmarks/memory/src/main.rs#L83) | Struct | `DownloadExpansionConfig` | — | Rename from `DownloadConfig` |
| [84](benchmarks/memory/src/main.rs#L84) | Field | `DownloadExpansionConfig::coalescing_distance_bytes` | `u64` | Keep |
| [85](benchmarks/memory/src/main.rs#L85) | Field | `DownloadExpansionConfig::whole_split_threshold_bytes` | `u64` | Keep |
| [88](benchmarks/memory/src/main.rs#L88) | Impl | `impl DownloadExpansionConfig` | — | Updated type references |
| [89](benchmarks/memory/src/main.rs#L89) | Function | `impl DownloadExpansionConfig::from_env` | `fn() -> Result<Self, String>` | Keep |
| [101](benchmarks/memory/src/main.rs#L101) | Struct | `TraceRequest` | — | Rename from `Access` |
| [102](benchmarks/memory/src/main.rs#L102) | Field | `TraceRequest::object_key` | `ObjectKey` | Keep |
| [103](benchmarks/memory/src/main.rs#L103) | Field | `TraceRequest::object_size` | `u64` | Keep |
| [104](benchmarks/memory/src/main.rs#L104) | Field | `TraceRequest::requested_range` | `ByteRange` | Rename from `requested` |
| [105](benchmarks/memory/src/main.rs#L105) | Field | `TraceRequest::timestamp_millis` | `u64` | Keep |
| [108](benchmarks/memory/src/main.rs#L108) | Impl | `impl TraceRequest` | — | Updated type references |
| [109](benchmarks/memory/src/main.rs#L109) | Method | `impl TraceRequest::validate` | `fn(&self, index: usize) -> Result<(), String>` | Keep |
| [121](benchmarks/memory/src/main.rs#L121) | Method | `impl TraceRequest::downloaded_range` | `fn(&self, range_policy: DownloadRangePolicy, config: DownloadExpansionConfig) -> ByteRange` | Keep |
| [134](benchmarks/memory/src/main.rs#L134) | Struct | `ReplayWorkload` | — | Rename from `Workload` |
| [135](benchmarks/memory/src/main.rs#L135) | Field | `ReplayWorkload::name` | `&'static str` | Keep |
| [136](benchmarks/memory/src/main.rs#L136) | Field | `ReplayWorkload::requests` | `Vec<TraceRequest>` | Rename from `accesses` |
| [137](benchmarks/memory/src/main.rs#L137) | Field | `ReplayWorkload::expanded_downloads` | `Vec<ByteRange>` | Keep |
| [138](benchmarks/memory/src/main.rs#L138) | Field | `ReplayWorkload::download_config` | `DownloadExpansionConfig` | Keep |
| [142](benchmarks/memory/src/main.rs#L142) | Trait | `ReplayCache` | — | Keep |
| [143](benchmarks/memory/src/main.rs#L143) | Method | `ReplayCache::name` | `fn(&self) -> &'static str` | Keep |
| [146](benchmarks/memory/src/main.rs#L146) | Method | `ReplayCache::get` | `fn(&mut self, request: &TraceRequest, downloaded_range: ByteRange) -> bool` | Keep |
| [148](benchmarks/memory/src/main.rs#L148) | Method | `ReplayCache::populate` | `fn(&mut self, request: &TraceRequest, downloaded_range: ByteRange, payload: Bytes) -> Result<(), String>` | Keep |
| [150](benchmarks/memory/src/main.rs#L150) | Method | `ReplayCache::used_payload_bytes` | `fn(&self) -> u64` | Keep |
| [154](benchmarks/memory/src/main.rs#L154) | Struct | `FeuerReplayCache` | — | Keep |
| [155](benchmarks/memory/src/main.rs#L155) | Field | `FeuerReplayCache::cache` | `MemoryCache` | Keep |
| [158](benchmarks/memory/src/main.rs#L158) | Impl | `impl FeuerReplayCache` | — | Keep |
| [159](benchmarks/memory/src/main.rs#L159) | Function | `impl FeuerReplayCache::new` | `fn(capacity: usize, shards: usize) -> Self` | Keep |
| [166](benchmarks/memory/src/main.rs#L166) | Impl | `impl ReplayCache for FeuerReplayCache` | — | Keep |
| [167](benchmarks/memory/src/main.rs#L167) | Method | `impl ReplayCache for FeuerReplayCache::name` | `fn(&self) -> &'static str` | Keep |
| [171](benchmarks/memory/src/main.rs#L171) | Method | `impl ReplayCache for FeuerReplayCache::get` | `fn(&mut self, request: &TraceRequest, _downloaded_range: ByteRange) -> bool` | Keep |
| [179](benchmarks/memory/src/main.rs#L179) | Method | `impl ReplayCache for FeuerReplayCache::populate` | `fn(&mut self, request: &TraceRequest, downloaded_range: ByteRange, payload: Bytes) -> Result<(), String>` | Keep |
| [187](benchmarks/memory/src/main.rs#L187) | Method | `impl ReplayCache for FeuerReplayCache::used_payload_bytes` | `fn(&self) -> u64` | Keep |
| [192](benchmarks/memory/src/main.rs#L192) | TypeAlias | `FoyerRangeKey` | `(ObjectKey, ByteRange)` | Rename from `NativeFoyerKey` |
| [195](benchmarks/memory/src/main.rs#L195) | Struct | `FoyerCachedDownload` | — | Rename from `NativeFoyerValue` |
| [196](benchmarks/memory/src/main.rs#L196) | Field | `FoyerCachedDownload::downloaded_range` | `ByteRange` | Rename from `downloaded` |
| [197](benchmarks/memory/src/main.rs#L197) | Field | `FoyerCachedDownload::payload` | `Bytes` | Keep |
| [201](benchmarks/memory/src/main.rs#L201) | Enum | `FoyerKeyRange` | — | Rename from `NativeFoyerKeyMode` |
| [203](benchmarks/memory/src/main.rs#L203) | Variant | `FoyerKeyRange::ExactRequest` | — | Keep |
| [205](benchmarks/memory/src/main.rs#L205) | Variant | `FoyerKeyRange::ExpandedDownload` | — | Keep |
| [208](benchmarks/memory/src/main.rs#L208) | Impl | `impl FoyerKeyRange` | — | Updated type references |
| [209](benchmarks/memory/src/main.rs#L209) | Method | `impl FoyerKeyRange::range` | `fn(self, request: &TraceRequest, downloaded_range: ByteRange) -> ByteRange` | Keep |
| [218](benchmarks/memory/src/main.rs#L218) | Enum | `FoyerEvictionPolicy` | — | Rename from `NativeFoyerPolicy` |
| [219](benchmarks/memory/src/main.rs#L219) | Variant | `FoyerEvictionPolicy::S3Fifo` | — | Keep |
| [220](benchmarks/memory/src/main.rs#L220) | Variant | `FoyerEvictionPolicy::CostAware` | — | Keep |
| [223](benchmarks/memory/src/main.rs#L223) | Impl | `impl FoyerEvictionPolicy` | — | Updated type references |
| [224](benchmarks/memory/src/main.rs#L224) | Method | `impl FoyerEvictionPolicy::engine_name` | `fn(self, key_range: FoyerKeyRange) -> &'static str` | Keep |
| [235](benchmarks/memory/src/main.rs#L235) | Struct | `FoyerReplayCache` | — | Keep |
| [236](benchmarks/memory/src/main.rs#L236) | Field | `FoyerReplayCache::cache` | `FoyerCache<FoyerRangeKey, FoyerCachedDownload>` | Keep |
| [237](benchmarks/memory/src/main.rs#L237) | Field | `FoyerReplayCache::key_range` | `FoyerKeyRange` | Rename from `key_mode` |
| [238](benchmarks/memory/src/main.rs#L238) | Field | `FoyerReplayCache::eviction_policy` | `FoyerEvictionPolicy` | Rename from `policy` |
| [241](benchmarks/memory/src/main.rs#L241) | Impl | `impl FoyerReplayCache` | — | Keep |
| [242](benchmarks/memory/src/main.rs#L242) | Function | `impl FoyerReplayCache::new` | `fn(capacity: usize, shards: usize, key_range: FoyerKeyRange, eviction_policy: FoyerEvictionPolicy) -> Self` | Keep |
| [250](benchmarks/memory/src/main.rs#L250) | Method | `impl FoyerReplayCache::key` | `fn(&self, request: &TraceRequest, downloaded_range: ByteRange) -> FoyerRangeKey` | Keep |
| [258](benchmarks/memory/src/main.rs#L258) | Impl | `impl ReplayCache for FoyerReplayCache` | — | Keep |
| [259](benchmarks/memory/src/main.rs#L259) | Method | `impl ReplayCache for FoyerReplayCache::name` | `fn(&self) -> &'static str` | Keep |
| [263](benchmarks/memory/src/main.rs#L263) | Method | `impl ReplayCache for FoyerReplayCache::get` | `fn(&mut self, request: &TraceRequest, downloaded_range: ByteRange) -> bool` | Keep |
| [275](benchmarks/memory/src/main.rs#L275) | Method | `impl ReplayCache for FoyerReplayCache::populate` | `fn(&mut self, request: &TraceRequest, downloaded_range: ByteRange, payload: Bytes) -> Result<(), String>` | Keep |
| [287](benchmarks/memory/src/main.rs#L287) | Method | `impl ReplayCache for FoyerReplayCache::used_payload_bytes` | `fn(&self) -> u64` | Keep |
| [292](benchmarks/memory/src/main.rs#L292) | Function | `foyer_cache` | `fn( capacity: usize, shards: usize, eviction_policy: FoyerEvictionPolicy, ) -> FoyerCache<FoyerRangeKey, FoyerCachedDownload>` | Keep |
| [312](benchmarks/memory/src/main.rs#L312) | Struct | `ReplayTraffic` | — | Rename from `Traffic` |
| [313](benchmarks/memory/src/main.rs#L313) | Field | `ReplayTraffic::requests` | `u64` | Keep |
| [314](benchmarks/memory/src/main.rs#L314) | Field | `ReplayTraffic::requested_bytes` | `u64` | Keep |
| [315](benchmarks/memory/src/main.rs#L315) | Field | `ReplayTraffic::hits` | `u64` | Keep |
| [316](benchmarks/memory/src/main.rs#L316) | Field | `ReplayTraffic::hit_bytes` | `u64` | Keep |
| [317](benchmarks/memory/src/main.rs#L317) | Field | `ReplayTraffic::source_requests` | `u64` | Keep |
| [318](benchmarks/memory/src/main.rs#L318) | Field | `ReplayTraffic::source_bytes` | `u64` | Keep |
| [321](benchmarks/memory/src/main.rs#L321) | Struct | `ReplayReport` | — | Rename from `Report` |
| [322](benchmarks/memory/src/main.rs#L322) | Field | `ReplayReport::workload` | `&'static str` | Keep |
| [323](benchmarks/memory/src/main.rs#L323) | Field | `ReplayReport::downloader` | `&'static str` | Keep |
| [324](benchmarks/memory/src/main.rs#L324) | Field | `ReplayReport::shards` | `usize` | Keep |
| [325](benchmarks/memory/src/main.rs#L325) | Field | `ReplayReport::capacity` | `usize` | Keep |
| [326](benchmarks/memory/src/main.rs#L326) | Field | `ReplayReport::engine` | `&'static str` | Keep |
| [327](benchmarks/memory/src/main.rs#L327) | Field | `ReplayReport::traffic` | `ReplayTraffic` | Keep |
| [328](benchmarks/memory/src/main.rs#L328) | Field | `ReplayReport::used_payload_bytes` | `u64` | Keep |
| [329](benchmarks/memory/src/main.rs#L329) | Field | `ReplayReport::elapsed` | `Duration` | Keep |
| [332](benchmarks/memory/src/main.rs#L332) | Impl | `impl ReplayReport` | — | Updated type references |
| [333](benchmarks/memory/src/main.rs#L333) | Method | `impl ReplayReport::cache_hit_rate` | `fn(&self) -> f64` | Keep |
| [337](benchmarks/memory/src/main.rs#L337) | Method | `impl ReplayReport::byte_hit_rate` | `fn(&self) -> f64` | Keep |
| [341](benchmarks/memory/src/main.rs#L341) | Method | `impl ReplayReport::source_cost_hit_rate` | `fn(&self) -> f64` | Keep |
| [352](benchmarks/memory/src/main.rs#L352) | Method | `impl ReplayReport::operations_per_second` | `fn(&self) -> f64` | Keep |
| [357](benchmarks/memory/src/main.rs#L357) | Function | `main` | `fn() -> Result<(), String>` | Keep |
| [431](benchmarks/memory/src/main.rs#L431) | Function | `trace_workload` | `fn(args: &ReplayArgs, download_config: DownloadExpansionConfig) -> Result<ReplayWorkload, String>` | Keep |
| [456](benchmarks/memory/src/main.rs#L456) | Function | `replay_cache` | `fn( mut cache: Box<dyn ReplayCache>, workload: &ReplayWorkload, downloader: DownloadRangePolicy, shards: usize, capacity: usize, warmup_iterations: usize, source_payload: &Bytes, ) -> Result<ReplayReport, String>` | Keep |
| [488](benchmarks/memory/src/main.rs#L488) | Struct | `DownloadToCoalesce` | — | Rename from `PendingDownload` |
| [489](benchmarks/memory/src/main.rs#L489) | Field | `DownloadToCoalesce::trace_index` | `usize` | Rename from `order` |
| [490](benchmarks/memory/src/main.rs#L490) | Field | `DownloadToCoalesce::request` | `&'a TraceRequest` | Rename from `access` |
| [491](benchmarks/memory/src/main.rs#L491) | Field | `DownloadToCoalesce::downloaded_range` | `ByteRange` | Rename from `downloaded` |
| [494](benchmarks/memory/src/main.rs#L494) | Struct | `CoalescedDownload` | — | Keep |
| [495](benchmarks/memory/src/main.rs#L495) | Field | `CoalescedDownload::downloaded_range` | `ByteRange` | Rename from `downloaded` |
| [496](benchmarks/memory/src/main.rs#L496) | Field | `CoalescedDownload::requests` | `Vec<(usize, &'a TraceRequest)>` | Rename from `accesses` |
| [499](benchmarks/memory/src/main.rs#L499) | Function | `expanded_download_ranges` | `fn(workload: &[TraceRequest], config: DownloadExpansionConfig) -> Vec<ByteRange>` | Keep |
| [550](benchmarks/memory/src/main.rs#L550) | Function | `max_download_len` | `fn(workload: &ReplayWorkload, downloader: DownloadRangePolicy) -> u64` | Keep |
| [571](benchmarks/memory/src/main.rs#L571) | Function | `execute_pass` | `fn<C: ReplayCache + ?Sized>( cache: &mut C, workload: &ReplayWorkload, downloader: DownloadRangePolicy, source_payload: &Bytes, traffic: &mut ReplayTraffic, ) -> Result<(), String>` | Keep |
| [599](benchmarks/memory/src/main.rs#L599) | Function | `coalesced_downloads` | `fn( mut pending: Vec<DownloadToCoalesce<'_>>, coalescing_distance_bytes: u64, ) -> Vec<CoalescedDownload<'_>>` | Keep |
| [641](benchmarks/memory/src/main.rs#L641) | Function | `record_request` | `fn(traffic: &mut ReplayTraffic, request: &TraceRequest, hit: bool)` | Keep |
| [650](benchmarks/memory/src/main.rs#L650) | Function | `source_payload_slice` | `fn(source_payload: &Bytes, downloaded_range: ByteRange) -> Result<Bytes, String>` | Keep |
| [659](benchmarks/memory/src/main.rs#L659) | Function | `requested_payload` | `fn(bytes: &Bytes, downloaded_range: ByteRange, requested_range: ByteRange) -> Bytes` | Keep |
| [668](benchmarks/memory/src/main.rs#L668) | Function | `print_human_header` | `fn(args: &ReplayArgs, workload: &ReplayWorkload)` | Keep |
| [687](benchmarks/memory/src/main.rs#L687) | Function | `print_human_report` | `fn(report: &ReplayReport)` | Keep |
| [702](benchmarks/memory/src/main.rs#L702) | Function | `human_engine_name` | `fn(engine: &str) -> &str` | Keep |
| [713](benchmarks/memory/src/main.rs#L713) | Function | `format_decimal_bytes` | `fn(bytes: u64) -> String` | Keep |
| [725](benchmarks/memory/src/main.rs#L725) | Function | `format_binary_bytes` | `fn(bytes: u64) -> String` | Rename from `format_bytes` |
| [737](benchmarks/memory/src/main.rs#L737) | Function | `format_rate` | `fn(operations_per_second: f64) -> String` | Keep |
| [747](benchmarks/memory/src/main.rs#L747) | Function | `print_csv_report` | `fn(report: &ReplayReport)` | Keep |
| [771](benchmarks/memory/src/main.rs#L771) | Function | `load_trace` | `fn() -> Result<Vec<TraceRequest>, String>` | Keep |
| [784](benchmarks/memory/src/main.rs#L784) | Function | `parse_trace_line` | `fn(line: &str) -> Result<TraceRequest, String>` | Keep |
| [800](benchmarks/memory/src/main.rs#L800) | Function | `parse_timestamp_millis` | `fn(value: &str) -> Result<u64, String>` | Keep |
| [861](benchmarks/memory/src/main.rs#L861) | Function | `find_json_string` | `fn(line: &str, field: &str) -> Result<String, String>` | Keep |
| [872](benchmarks/memory/src/main.rs#L872) | Function | `find_json_u64` | `fn(line: &str, field: &str) -> Result<u64, String>` | Keep |
| [885](benchmarks/memory/src/main.rs#L885) | Function | `field_value` | `fn<'a>(line: &'a str, field: &str) -> Result<&'a str, String>` | Keep |
| [898](benchmarks/memory/src/main.rs#L898) | Function | `ratio` | `fn(numerator: u64, denominator: u64) -> f64` | Keep |
| [906](benchmarks/memory/src/main.rs#L906) | Function | `byte_count_from_env` | `fn(name: &str, default: u64) -> Result<u64, String>` | Keep |
| [916](benchmarks/memory/src/main.rs#L916) | Function | `parse_byte_count_usize` | `fn(value: &str) -> Result<usize, String>` | Rename from `parse_bytes` |
| [921](benchmarks/memory/src/main.rs#L921) | Function | `parse_byte_count` | `fn(value: &str) -> Result<u128, String>` | Keep |
| [946](benchmarks/memory/src/main.rs#L946) | Module | `tests` | — | Keep |
| [949](benchmarks/memory/src/main.rs#L949) | Struct | `tests::WarmupTestCache` | — | Keep |
| [950](benchmarks/memory/src/main.rs#L950) | Field | `tests::WarmupTestCache::populated` | `bool` | Keep |
| [953](benchmarks/memory/src/main.rs#L953) | Impl | `tests::impl ReplayCache for WarmupTestCache` | — | Keep |
| [954](benchmarks/memory/src/main.rs#L954) | Method | `tests::impl ReplayCache for WarmupTestCache::name` | `fn(&self) -> &'static str` | Keep |
| [958](benchmarks/memory/src/main.rs#L958) | Method | `tests::impl ReplayCache for WarmupTestCache::get` | `fn(&mut self, _request: &TraceRequest, _downloaded_range: ByteRange) -> bool` | Keep |
| [962](benchmarks/memory/src/main.rs#L962) | Method | `tests::impl ReplayCache for WarmupTestCache::populate` | `fn( &mut self, _request: &TraceRequest, _downloaded_range: ByteRange, _payload: Bytes, ) -> Result<(), String>` | Keep |
| [972](benchmarks/memory/src/main.rs#L972) | Method | `tests::impl ReplayCache for WarmupTestCache::used_payload_bytes` | `fn(&self) -> u64` | Keep |
| [978](benchmarks/memory/src/main.rs#L978) | Struct | `tests::CoveringRangeTestCache` | — | Rename from `RangeTestCache` |
| [979](benchmarks/memory/src/main.rs#L979) | Field | `tests::CoveringRangeTestCache::entries` | `Vec<(ObjectKey, ByteRange)>` | Keep |
| [982](benchmarks/memory/src/main.rs#L982) | Impl | `tests::impl ReplayCache for CoveringRangeTestCache` | — | Updated type references |
| [983](benchmarks/memory/src/main.rs#L983) | Method | `tests::impl ReplayCache for CoveringRangeTestCache::name` | `fn(&self) -> &'static str` | Keep |
| [987](benchmarks/memory/src/main.rs#L987) | Method | `tests::impl ReplayCache for CoveringRangeTestCache::get` | `fn(&mut self, request: &TraceRequest, _downloaded_range: ByteRange) -> bool` | Keep |
| [993](benchmarks/memory/src/main.rs#L993) | Method | `tests::impl ReplayCache for CoveringRangeTestCache::populate` | `fn( &mut self, request: &TraceRequest, downloaded_range: ByteRange, _payload: Bytes, ) -> Result<(), String>` | Keep |
| [1004](benchmarks/memory/src/main.rs#L1004) | Method | `tests::impl ReplayCache for CoveringRangeTestCache::used_payload_bytes` | `fn(&self) -> u64` | Keep |
| [1010](benchmarks/memory/src/main.rs#L1010) | Function | `tests::warmup_preserves_cache_state_but_not_reported_traffic` | `fn()` | Keep |
| [1043](benchmarks/memory/src/main.rs#L1043) | Function | `tests::foyer_expanded_key_reuses_identical_expansions_for_distinct_requests` | `fn()` | Keep |
| [1068](benchmarks/memory/src/main.rs#L1068) | Function | `tests::expanded_downloader_assigns_one_coalesced_range_to_all_batch_members` | `fn()` | Keep |
| [1112](benchmarks/memory/src/main.rs#L1112) | Function | `tests::coalescing_distance_parameter_is_a_strict_upper_bound` | `fn()` | Keep |
| [1146](benchmarks/memory/src/main.rs#L1146) | Function | `tests::source_cost_baseline_uses_requested_bytes_not_downloaded_bytes` | `fn()` | Keep |
| [1168](benchmarks/memory/src/main.rs#L1168) | Function | `tests::parses_the_captured_trace_shape` | `fn()` | Keep |
| [1183](benchmarks/memory/src/main.rs#L1183) | Function | `tests::expanded_downloader_honors_the_whole_split_threshold` | `fn()` | Keep |
| [1216](benchmarks/memory/src/main.rs#L1216) | Function | `tests::timestamp_parser_handles_day_boundaries` | `fn()` | Keep |
| [1228](benchmarks/memory/src/main.rs#L1228) | Function | `tests::environment_byte_counts_accept_documented_units` | `fn()` | Keep |
| [1234](benchmarks/memory/src/main.rs#L1234) | Function | `tests::human_output_is_default_and_csv_is_opt_in` | `fn()` | Keep |

<details>
<summary>Local bindings (109)</summary>

| Line | Kind | Name / source parent | Signature or type | Review decision |
| ---: | --- | --- | --- | --- |
| [172](benchmarks/memory/src/main.rs#L172) | Local | `impl ReplayCache for FeuerReplayCache::get::Some(bytes)` | — | Keep |
| [180](benchmarks/memory/src/main.rs#L180) | Local | `impl ReplayCache for FeuerReplayCache::populate::download` | — | Keep |
| [264](benchmarks/memory/src/main.rs#L264) | Local | `impl ReplayCache for FoyerReplayCache::get::key` | — | Keep |
| [265](benchmarks/memory/src/main.rs#L265) | Local | `impl ReplayCache for FoyerReplayCache::get::Some(entry)` | — | Keep |
| [268](benchmarks/memory/src/main.rs#L268) | Local | `impl ReplayCache for FoyerReplayCache::get::value` | — | Keep |
| [270](benchmarks/memory/src/main.rs#L270) | Local | `impl ReplayCache for FoyerReplayCache::get::result` | — | Keep |
| [276](benchmarks/memory/src/main.rs#L276) | Local | `impl ReplayCache for FoyerReplayCache::populate::key` | — | Keep |
| [297](benchmarks/memory/src/main.rs#L297) | Local | `foyer_cache::builder` | — | Keep |
| [342](benchmarks/memory/src/main.rs#L342) | Local | `impl ReplayReport::source_cost_hit_rate::fixed_cost` | — | Keep |
| [343](benchmarks/memory/src/main.rs#L343) | Local | `impl ReplayReport::source_cost_hit_rate::baseline` | — | Keep |
| [344](benchmarks/memory/src/main.rs#L344) | Local | `impl ReplayReport::source_cost_hit_rate::actual` | — | Keep |
| [358](benchmarks/memory/src/main.rs#L358) | Local | `main::args` | — | Keep |
| [368](benchmarks/memory/src/main.rs#L368) | Local | `main::download_config` | — | Keep |
| [369](benchmarks/memory/src/main.rs#L369) | Local | `main::workload` | — | Keep |
| [386](benchmarks/memory/src/main.rs#L386) | Local | `main::max_download` | — | Keep |
| [387](benchmarks/memory/src/main.rs#L387) | Local | `main::max_download` | — | Keep |
| [388](benchmarks/memory/src/main.rs#L388) | Local | `main::source_payload` | — | Keep |
| [392](benchmarks/memory/src/main.rs#L392) | Local | `main::mut caches` | `Vec<Box<dyn ReplayCache>>` | Keep |
| [410](benchmarks/memory/src/main.rs#L410) | Local | `main::report` | — | Keep |
| [432](benchmarks/memory/src/main.rs#L432) | Local | `trace_workload::mut requests` | — | Rename from `mut accesses` |
| [447](benchmarks/memory/src/main.rs#L447) | Local | `trace_workload::expanded_downloads` | — | Keep |
| [466](benchmarks/memory/src/main.rs#L466) | Local | `replay_cache::mut warmup_traffic` | — | Keep |
| [470](benchmarks/memory/src/main.rs#L470) | Local | `replay_cache::mut traffic` | — | Keep |
| [471](benchmarks/memory/src/main.rs#L471) | Local | `replay_cache::started` | — | Keep |
| [473](benchmarks/memory/src/main.rs#L473) | Local | `replay_cache::elapsed` | — | Keep |
| [500](benchmarks/memory/src/main.rs#L500) | Local | `expanded_download_ranges::mut by_object` | `HashMap<(&str, u64), Vec<usize>>` | Keep |
| [508](benchmarks/memory/src/main.rs#L508) | Local | `expanded_download_ranges::base_ranges` | `Vec<_>` | Keep |
| [512](benchmarks/memory/src/main.rs#L512) | Local | `expanded_download_ranges::mut ranges` | — | Keep |
| [516](benchmarks/memory/src/main.rs#L516) | Local | `expanded_download_ranges::mut assigned` | — | Keep |
| [523](benchmarks/memory/src/main.rs#L523) | Local | `expanded_download_ranges::deadline` | — | Keep |
| [526](benchmarks/memory/src/main.rs#L526) | Local | `expanded_download_ranges::pending` | — | Keep |
| [537](benchmarks/memory/src/main.rs#L537) | Local | `expanded_download_ranges::download` | — | Keep |
| [579](benchmarks/memory/src/main.rs#L579) | Local | `execute_pass::downloaded_range` | — | Rename from `downloaded` |
| [585](benchmarks/memory/src/main.rs#L585) | Local | `execute_pass::hit` | — | Keep |
| [591](benchmarks/memory/src/main.rs#L591) | Local | `execute_pass::payload` | — | Keep |
| [612](benchmarks/memory/src/main.rs#L612) | Local | `coalesced_downloads::mut downloads` | `Vec<CoalescedDownload<'_>>` | Keep |
| [614](benchmarks/memory/src/main.rs#L614) | Local | `coalesced_downloads::merge` | — | Keep |
| [615](benchmarks/memory/src/main.rs#L615) | Local | `coalesced_downloads::merge::representative` | — | Keep |
| [616](benchmarks/memory/src/main.rs#L616) | Local | `coalesced_downloads::merge::gap` | — | Keep |
| [651](benchmarks/memory/src/main.rs#L651) | Local | `source_payload_slice::payload_len` | — | Keep |
| [661](benchmarks/memory/src/main.rs#L661) | Local | `requested_payload::start` | — | Keep |
| [663](benchmarks/memory/src/main.rs#L663) | Local | `requested_payload::end` | — | Keep |
| [669](benchmarks/memory/src/main.rs#L669) | Local | `print_human_header::shards` | — | Keep |
| [772](benchmarks/memory/src/main.rs#L772) | Local | `load_trace::path` | — | Keep |
| [773](benchmarks/memory/src/main.rs#L773) | Local | `load_trace::content` | — | Keep |
| [785](benchmarks/memory/src/main.rs#L785) | Local | `parse_trace_line::object_key` | — | Keep |
| [786](benchmarks/memory/src/main.rs#L786) | Local | `parse_trace_line::object_size` | — | Keep |
| [787](benchmarks/memory/src/main.rs#L787) | Local | `parse_trace_line::start` | — | Keep |
| [788](benchmarks/memory/src/main.rs#L788) | Local | `parse_trace_line::end` | — | Keep |
| [789](benchmarks/memory/src/main.rs#L789) | Local | `parse_trace_line::requested_range` | — | Rename from `requested` |
| [790](benchmarks/memory/src/main.rs#L790) | Local | `parse_trace_line::timestamp` | — | Keep |
| [791](benchmarks/memory/src/main.rs#L791) | Local | `parse_trace_line::timestamp_millis` | — | Keep |
| [801](benchmarks/memory/src/main.rs#L801) | Local | `parse_timestamp_millis::bytes` | — | Keep |
| [802](benchmarks/memory/src/main.rs#L802) | Local | `parse_timestamp_millis::valid_suffix` | — | Keep |
| [814](benchmarks/memory/src/main.rs#L814) | Local | `parse_timestamp_millis::component` | — | Keep |
| [819](benchmarks/memory/src/main.rs#L819) | Local | `parse_timestamp_millis::year` | — | Keep |
| [820](benchmarks/memory/src/main.rs#L820) | Local | `parse_timestamp_millis::month` | — | Keep |
| [821](benchmarks/memory/src/main.rs#L821) | Local | `parse_timestamp_millis::day` | — | Keep |
| [822](benchmarks/memory/src/main.rs#L822) | Local | `parse_timestamp_millis::hour` | — | Keep |
| [823](benchmarks/memory/src/main.rs#L823) | Local | `parse_timestamp_millis::minute` | — | Keep |
| [824](benchmarks/memory/src/main.rs#L824) | Local | `parse_timestamp_millis::second` | — | Keep |
| [825](benchmarks/memory/src/main.rs#L825) | Local | `parse_timestamp_millis::millis` | — | Keep |
| [834](benchmarks/memory/src/main.rs#L834) | Local | `parse_timestamp_millis::leap_year` | — | Keep |
| [835](benchmarks/memory/src/main.rs#L835) | Local | `parse_timestamp_millis::month_lengths` | — | Keep |
| [849](benchmarks/memory/src/main.rs#L849) | Local | `parse_timestamp_millis::month_index` | — | Keep |
| [854](benchmarks/memory/src/main.rs#L854) | Local | `parse_timestamp_millis::previous_year` | — | Keep |
| [855](benchmarks/memory/src/main.rs#L855) | Local | `parse_timestamp_millis::days_before_year` | — | Keep |
| [856](benchmarks/memory/src/main.rs#L856) | Local | `parse_timestamp_millis::days_before_month` | `u64` | Keep |
| [857](benchmarks/memory/src/main.rs#L857) | Local | `parse_timestamp_millis::days` | — | Keep |
| [862](benchmarks/memory/src/main.rs#L862) | Local | `find_json_string::value` | — | Keep |
| [863](benchmarks/memory/src/main.rs#L863) | Local | `find_json_string::value` | — | Keep |
| [866](benchmarks/memory/src/main.rs#L866) | Local | `find_json_string::end` | — | Keep |
| [873](benchmarks/memory/src/main.rs#L873) | Local | `find_json_u64::value` | — | Keep |
| [874](benchmarks/memory/src/main.rs#L874) | Local | `find_json_u64::end` | — | Keep |
| [886](benchmarks/memory/src/main.rs#L886) | Local | `field_value::needle` | — | Keep |
| [887](benchmarks/memory/src/main.rs#L887) | Local | `field_value::after_field` | — | Keep |
| [891](benchmarks/memory/src/main.rs#L891) | Local | `field_value::after_colon` | — | Keep |
| [907](benchmarks/memory/src/main.rs#L907) | Local | `byte_count_from_env::value` | — | Keep |
| [912](benchmarks/memory/src/main.rs#L912) | Local | `byte_count_from_env::bytes` | — | Keep |
| [917](benchmarks/memory/src/main.rs#L917) | Local | `parse_byte_count_usize::bytes` | — | Keep |
| [922](benchmarks/memory/src/main.rs#L922) | Local | `parse_byte_count::value` | — | Keep |
| [923](benchmarks/memory/src/main.rs#L923) | Local | `parse_byte_count::split` | — | Keep |
| [926](benchmarks/memory/src/main.rs#L926) | Local | `parse_byte_count::number` | `u128` | Keep |
| [929](benchmarks/memory/src/main.rs#L929) | Local | `parse_byte_count::suffix` | — | Keep |
| [930](benchmarks/memory/src/main.rs#L930) | Local | `parse_byte_count::multiplier` | — | Keep |
| [1011](benchmarks/memory/src/main.rs#L1011) | Local | `tests::warmup_preserves_cache_state_but_not_reported_traffic::workload` | — | Keep |
| [1025](benchmarks/memory/src/main.rs#L1025) | Local | `tests::warmup_preserves_cache_state_but_not_reported_traffic::report` | — | Keep |
| [1044](benchmarks/memory/src/main.rs#L1044) | Local | `tests::foyer_expanded_key_reuses_identical_expansions_for_distinct_requests::request` | — | Rename from `access` |
| [1050](benchmarks/memory/src/main.rs#L1050) | Local | `tests::foyer_expanded_key_reuses_identical_expansions_for_distinct_requests::first` | — | Keep |
| [1051](benchmarks/memory/src/main.rs#L1051) | Local | `tests::foyer_expanded_key_reuses_identical_expansions_for_distinct_requests::second` | — | Keep |
| [1052](benchmarks/memory/src/main.rs#L1052) | Local | `tests::foyer_expanded_key_reuses_identical_expansions_for_distinct_requests::expanded` | — | Keep |
| [1054](benchmarks/memory/src/main.rs#L1054) | Local | `tests::foyer_expanded_key_reuses_identical_expansions_for_distinct_requests::mut expanded_key` | — | Keep |
| [1062](benchmarks/memory/src/main.rs#L1062) | Local | `tests::foyer_expanded_key_reuses_identical_expansions_for_distinct_requests::mut exact_key` | — | Keep |
| [1069](benchmarks/memory/src/main.rs#L1069) | Local | `tests::expanded_downloader_assigns_one_coalesced_range_to_all_batch_members::request` | — | Rename from `access` |
| [1075](benchmarks/memory/src/main.rs#L1075) | Local | `tests::expanded_downloader_assigns_one_coalesced_range_to_all_batch_members::requests` | — | Rename from `accesses` |
| [1081](benchmarks/memory/src/main.rs#L1081) | Local | `tests::expanded_downloader_assigns_one_coalesced_range_to_all_batch_members::download_config` | — | Keep |
| [1085](benchmarks/memory/src/main.rs#L1085) | Local | `tests::expanded_downloader_assigns_one_coalesced_range_to_all_batch_members::expanded_downloads` | — | Keep |
| [1086](benchmarks/memory/src/main.rs#L1086) | Local | `tests::expanded_downloader_assigns_one_coalesced_range_to_all_batch_members::workload` | — | Keep |
| [1094](benchmarks/memory/src/main.rs#L1094) | Local | `tests::expanded_downloader_assigns_one_coalesced_range_to_all_batch_members::report` | — | Keep |
| [1113](benchmarks/memory/src/main.rs#L1113) | Local | `tests::coalescing_distance_parameter_is_a_strict_upper_bound::distance` | — | Keep |
| [1114](benchmarks/memory/src/main.rs#L1114) | Local | `tests::coalescing_distance_parameter_is_a_strict_upper_bound::ranges_for_gap` | — | Keep |
| [1115](benchmarks/memory/src/main.rs#L1115) | Local | `tests::coalescing_distance_parameter_is_a_strict_upper_bound::ranges_for_gap::requests` | — | Rename from `accesses` |
| [1147](benchmarks/memory/src/main.rs#L1147) | Local | `tests::source_cost_baseline_uses_requested_bytes_not_downloaded_bytes::report` | — | Keep |
| [1169](benchmarks/memory/src/main.rs#L1169) | Local | `tests::parses_the_captured_trace_shape::request` | — | Rename from `access` |
| [1184](benchmarks/memory/src/main.rs#L1184) | Local | `tests::expanded_downloader_honors_the_whole_split_threshold::config` | — | Keep |
| [1188](benchmarks/memory/src/main.rs#L1188) | Local | `tests::expanded_downloader_honors_the_whole_split_threshold::small_split` | — | Keep |
| [1203](benchmarks/memory/src/main.rs#L1203) | Local | `tests::expanded_downloader_honors_the_whole_split_threshold::threshold_split` | — | Keep |
| [1217](benchmarks/memory/src/main.rs#L1217) | Local | `tests::timestamp_parser_handles_day_boundaries::before` | — | Keep |
| [1218](benchmarks/memory/src/main.rs#L1218) | Local | `tests::timestamp_parser_handles_day_boundaries::after` | — | Keep |

</details>

## feuer-storage

### `feuer-storage/examples/direct_io.rs`

| Line | Kind | Name / source parent | Signature or type | Review decision |
| ---: | --- | --- | --- | --- |
| [13](feuer-storage/examples/direct_io.rs#L13) | Const | `MIB` | `usize` | Keep |
| [14](feuer-storage/examples/direct_io.rs#L14) | Const | `CAPACITY` | `u64` | Keep |
| [15](feuer-storage/examples/direct_io.rs#L15) | Const | `READ_SPACE_BYTES` | `u64` | Rename from `READ_SPACE` |
| [19](feuer-storage/examples/direct_io.rs#L19) | Struct | `IoMeasurements` | — | Keep |
| [20](feuer-storage/examples/direct_io.rs#L20) | Field | `IoMeasurements::operations` | `u64` | Keep |
| [21](feuer-storage/examples/direct_io.rs#L21) | Field | `IoMeasurements::bytes` | `u64` | Keep |
| [22](feuer-storage/examples/direct_io.rs#L22) | Field | `IoMeasurements::latency_samples_micros` | `Vec<u64>` | Keep |
| [26](feuer-storage/examples/direct_io.rs#L26) | Function | `main` | `fn() -> Result<(), Box<dyn std::error::Error>>` | Keep |
| [66](feuer-storage/examples/direct_io.rs#L66) | Function | `run_case` | `fn(file: &DataFile, read_size: usize, readers: usize, writers: usize, seconds: u64)` | Rename from `run` |
| [145](feuer-storage/examples/direct_io.rs#L145) | Function | `cpu_seconds` | `fn() -> f64` | Keep |

<details>
<summary>Local bindings (35)</summary>

| Line | Kind | Name / source parent | Signature or type | Review decision |
| ---: | --- | --- | --- | --- |
| [27](feuer-storage/examples/direct_io.rs#L27) | Local | `main::mut args` | — | Keep |
| [28](feuer-storage/examples/direct_io.rs#L28) | Local | `main::root` | — | Keep |
| [29](feuer-storage/examples/direct_io.rs#L29) | Local | `main::seconds` | `u64` | Keep |
| [31](feuer-storage/examples/direct_io.rs#L31) | Local | `main::write_callers` | `Vec<usize>` | Keep |
| [40](feuer-storage/examples/direct_io.rs#L40) | Local | `main::temp` | — | Keep |
| [43](feuer-storage/examples/direct_io.rs#L43) | Local | `main::registry` | `mixtrics::metrics::BoxedRegistry` | Keep |
| [44](feuer-storage/examples/direct_io.rs#L44) | Local | `main::file` | — | Keep |
| [46](feuer-storage/examples/direct_io.rs#L46) | Local | `main::payload` | — | Keep |
| [67](feuer-storage/examples/direct_io.rs#L67) | Local | `run_case::measure_start` | — | Keep |
| [68](feuer-storage/examples/direct_io.rs#L68) | Local | `run_case::deadline` | — | Keep |
| [69](feuer-storage/examples/direct_io.rs#L69) | Local | `run_case::mut tasks` | — | Keep |
| [72](feuer-storage/examples/direct_io.rs#L72) | Local | `run_case::payload` | — | Keep |
| [74](feuer-storage/examples/direct_io.rs#L74) | Local | `run_case::file` | — | Keep |
| [75](feuer-storage/examples/direct_io.rs#L75) | Local | `run_case::payload` | — | Keep |
| [77](feuer-storage/examples/direct_io.rs#L77) | Local | `run_case::reading` | — | Keep |
| [78](feuer-storage/examples/direct_io.rs#L78) | Local | `run_case::mut random` | — | Keep |
| [79](feuer-storage/examples/direct_io.rs#L79) | Local | `run_case::mut measurements` | — | Keep |
| [80](feuer-storage/examples/direct_io.rs#L80) | Local | `run_case::mut write_offset` | — | Keep |
| [81](feuer-storage/examples/direct_io.rs#L81) | Local | `run_case::lane_size` | — | Keep |
| [83](feuer-storage/examples/direct_io.rs#L83) | Local | `run_case::started` | — | Keep |
| [84](feuer-storage/examples/direct_io.rs#L84) | Local | `run_case::bytes` | — | Keep |
| [88](feuer-storage/examples/direct_io.rs#L88) | Local | `run_case::bytes::offset` | — | Keep |
| [89](feuer-storage/examples/direct_io.rs#L89) | Local | `run_case::bytes::value` | — | Keep |
| [95](feuer-storage/examples/direct_io.rs#L95) | Local | `run_case::bytes::offset` | — | Keep |
| [100](feuer-storage/examples/direct_io.rs#L100) | Local | `run_case::finished` | — | Keep |
| [116](feuer-storage/examples/direct_io.rs#L116) | Local | `run_case::cpu_start` | — | Keep |
| [117](feuer-storage/examples/direct_io.rs#L117) | Local | `run_case::mut read` | — | Keep |
| [118](feuer-storage/examples/direct_io.rs#L118) | Local | `run_case::mut write` | — | Keep |
| [120](feuer-storage/examples/direct_io.rs#L120) | Local | `run_case::(reading, measurements)` | — | Keep |
| [121](feuer-storage/examples/direct_io.rs#L121) | Local | `run_case::total` | — | Keep |
| [126](feuer-storage/examples/direct_io.rs#L126) | Local | `run_case::cpu_percent` | — | Rename from `cpu` |
| [128](feuer-storage/examples/direct_io.rs#L128) | Local | `run_case::percentile` | — | Keep |
| [146](feuer-storage/examples/direct_io.rs#L146) | Local | `cpu_seconds::mut usage` | — | Keep |
| [150](feuer-storage/examples/direct_io.rs#L150) | Local | `cpu_seconds::usage` | — | Keep |
| [151](feuer-storage/examples/direct_io.rs#L151) | Local | `cpu_seconds::seconds` | — | Keep |

</details>

### `feuer-storage/src/error.rs`

| Line | Kind | Name / source parent | Signature or type | Review decision |
| ---: | --- | --- | --- | --- |
| [5](feuer-storage/src/error.rs#L5) | Enum | `IoOperation` | — | Keep |
| [7](feuer-storage/src/error.rs#L7) | Variant | `IoOperation::CreateDirectory` | — | Keep |
| [9](feuer-storage/src/error.rs#L9) | Variant | `IoOperation::OpenLockFile` | — | Keep |
| [11](feuer-storage/src/error.rs#L11) | Variant | `IoOperation::LockDirectory` | — | Keep |
| [13](feuer-storage/src/error.rs#L13) | Variant | `IoOperation::OpenDataFile` | — | Keep |
| [15](feuer-storage/src/error.rs#L15) | Variant | `IoOperation::InspectDataFile` | — | Keep |
| [17](feuer-storage/src/error.rs#L17) | Variant | `IoOperation::ResizeDataFile` | — | Keep |
| [19](feuer-storage/src/error.rs#L19) | Variant | `IoOperation::Read` | — | Keep |
| [21](feuer-storage/src/error.rs#L21) | Variant | `IoOperation::Write` | — | Keep |
| [24](feuer-storage/src/error.rs#L24) | Impl | `impl IoOperation` | — | Keep |
| [26](feuer-storage/src/error.rs#L26) | Method | `impl IoOperation::as_str` | `fn(self) -> &'static str` | Keep |
| [40](feuer-storage/src/error.rs#L40) | Impl | `impl fmt::Display for IoOperation` | — | Keep |
| [41](feuer-storage/src/error.rs#L41) | Method | `impl fmt::Display for IoOperation::fmt` | `fn(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result` | Keep |
| [48](feuer-storage/src/error.rs#L48) | Enum | `DataFileErrorKind` | — | Rename from `ErrorKind` |
| [50](feuer-storage/src/error.rs#L50) | Variant | `DataFileErrorKind::InvalidConfiguration` | — | Keep |
| [52](feuer-storage/src/error.rs#L52) | Variant | `DataFileErrorKind::AlreadyOpen` | — | Keep |
| [54](feuer-storage/src/error.rs#L54) | Variant | `DataFileErrorKind::OutOfBounds` | — | Keep |
| [56](feuer-storage/src/error.rs#L56) | Variant | `DataFileErrorKind::Allocation` | — | Keep |
| [58](feuer-storage/src/error.rs#L58) | Variant | `DataFileErrorKind::Io` | — | Keep |
| [60](feuer-storage/src/error.rs#L60) | Variant | `DataFileErrorKind::Task` | — | Keep |
| [63](feuer-storage/src/error.rs#L63) | Impl | `impl DataFileErrorKind` | — | Updated type references |
| [65](feuer-storage/src/error.rs#L65) | Method | `impl DataFileErrorKind::as_str` | `fn(self) -> &'static str` | Keep |
| [77](feuer-storage/src/error.rs#L77) | Impl | `impl fmt::Display for DataFileErrorKind` | — | Updated type references |
| [78](feuer-storage/src/error.rs#L78) | Method | `impl fmt::Display for DataFileErrorKind::fmt` | `fn(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result` | Keep |
| [86](feuer-storage/src/error.rs#L86) | Enum | `DataFileError` | — | Rename from `Error` |
| [89](feuer-storage/src/error.rs#L89) | Variant | `DataFileError::InvalidCapacity` | — | Keep |
| [92](feuer-storage/src/error.rs#L92) | Variant | `DataFileError::AlreadyOpen` | — | Keep |
| [94](feuer-storage/src/error.rs#L94) | Field | `DataFileError::AlreadyOpen::directory` | `PathBuf` | Keep |
| [98](feuer-storage/src/error.rs#L98) | Variant | `DataFileError::InvalidDataFile` | — | Keep |
| [100](feuer-storage/src/error.rs#L100) | Field | `DataFileError::InvalidDataFile::path` | `PathBuf` | Keep |
| [104](feuer-storage/src/error.rs#L104) | Variant | `DataFileError::OutOfBounds` | — | Keep |
| [106](feuer-storage/src/error.rs#L106) | Field | `DataFileError::OutOfBounds::operation` | `IoOperation` | Keep |
| [108](feuer-storage/src/error.rs#L108) | Field | `DataFileError::OutOfBounds::offset` | `u64` | Keep |
| [110](feuer-storage/src/error.rs#L110) | Field | `DataFileError::OutOfBounds::length` | `u64` | Keep |
| [112](feuer-storage/src/error.rs#L112) | Field | `DataFileError::OutOfBounds::capacity` | `u64` | Keep |
| [116](feuer-storage/src/error.rs#L116) | Variant | `DataFileError::LengthOverflow` | — | Keep |
| [118](feuer-storage/src/error.rs#L118) | Field | `DataFileError::LengthOverflow::operation` | `IoOperation` | Keep |
| [120](feuer-storage/src/error.rs#L120) | Field | `DataFileError::LengthOverflow::length` | `usize` | Keep |
| [124](feuer-storage/src/error.rs#L124) | Variant | `DataFileError::Allocation` | — | Keep |
| [126](feuer-storage/src/error.rs#L126) | Field | `DataFileError::Allocation::length` | `usize` | Keep |
| [129](feuer-storage/src/error.rs#L129) | Field | `DataFileError::Allocation::source` | `TryReserveError` | Keep |
| [133](feuer-storage/src/error.rs#L133) | Variant | `DataFileError::Io` | — | Keep |
| [135](feuer-storage/src/error.rs#L135) | Field | `DataFileError::Io::operation` | `IoOperation` | Keep |
| [137](feuer-storage/src/error.rs#L137) | Field | `DataFileError::Io::path` | `PathBuf` | Keep |
| [140](feuer-storage/src/error.rs#L140) | Field | `DataFileError::Io::source` | `io::Error` | Keep |
| [144](feuer-storage/src/error.rs#L144) | Variant | `DataFileError::RuntimeUnavailable` | — | Keep |
| [147](feuer-storage/src/error.rs#L147) | Variant | `DataFileError::Task` | — | Keep |
| [149](feuer-storage/src/error.rs#L149) | Field | `DataFileError::Task::operation` | `IoOperation` | Keep |
| [152](feuer-storage/src/error.rs#L152) | Field | `DataFileError::Task::source` | `Box<dyn std::error::Error + Send + Sync>` | Keep |
| [156](feuer-storage/src/error.rs#L156) | Impl | `impl DataFileError` | — | Updated type references |
| [158](feuer-storage/src/error.rs#L158) | Method | `impl DataFileError::kind` | `fn(&self) -> DataFileErrorKind` | Keep |
| [170](feuer-storage/src/error.rs#L170) | Method | `impl DataFileError::operation` | `fn(&self) -> IoOperation` | Keep |
| [186](feuer-storage/src/error.rs#L186) | TypeAlias | `DataFileResult` | `std::result::Result<T, DataFileError>` | Rename from `Result` |

### `feuer-storage/src/file.rs`

| Line | Kind | Name / source parent | Signature or type | Review decision |
| ---: | --- | --- | --- | --- |
| [18](feuer-storage/src/file.rs#L18) | Const | `DATA_FILE_NAME` | `&str` | Keep |
| [19](feuer-storage/src/file.rs#L19) | Const | `LOCK_FILE_NAME` | `&str` | Keep |
| [22](feuer-storage/src/file.rs#L22) | Struct | `DataFileState` | — | Keep |
| [23](feuer-storage/src/file.rs#L23) | Field | `DataFileState::queue` | `uring::IoQueueHandle` | Keep |
| [24](feuer-storage/src/file.rs#L24) | Field | `DataFileState::data_path` | `PathBuf` | Keep |
| [25](feuer-storage/src/file.rs#L25) | Field | `DataFileState::capacity` | `u64` | Keep |
| [64](feuer-storage/src/file.rs#L64) | Struct | `DataFile` | — | Keep |
| [65](feuer-storage/src/file.rs#L65) | Field | `DataFile::state` | `Arc<DataFileState>` | Keep |
| [66](feuer-storage/src/file.rs#L66) | Field | `DataFile::metrics` | `Arc<IoMetrics>` | Keep |
| [69](feuer-storage/src/file.rs#L69) | Impl | `impl fmt::Debug for DataFile` | — | Keep |
| [70](feuer-storage/src/file.rs#L70) | Method | `impl fmt::Debug for DataFile::fmt` | `fn(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result` | Keep |
| [77](feuer-storage/src/file.rs#L77) | Impl | `impl DataFile` | — | Keep |
| [83](feuer-storage/src/file.rs#L83) | Function | `impl DataFile::open` | `fn(directory: impl AsRef<Path>, capacity: u64, metrics: Arc<IoMetrics>) -> DataFileResult<Self>` | Keep |
| [125](feuer-storage/src/file.rs#L125) | Method | `impl DataFile::capacity` | `fn(&self) -> u64` | Keep |
| [133](feuer-storage/src/file.rs#L133) | Method | `impl DataFile::read_at` | `fn(&self, offset: u64, length: usize) -> DataFileResult<Bytes>` | Keep |
| [144](feuer-storage/src/file.rs#L144) | Method | `impl DataFile::write_at` | `fn(&self, offset: u64, bytes: &Bytes) -> DataFileResult<()>` | Keep |
| [150](feuer-storage/src/file.rs#L150) | Method | `impl DataFile::execute_measured` | `fn( &self, operation: IoOperation, offset: u64, length: usize, payload: &[u8], ) -> DataFileResult<Bytes>` | Rename from `execute` |
| [179](feuer-storage/src/file.rs#L179) | Method | `impl DataFile::execute_chunks` | `fn( &self, operation: IoOperation, offset: u64, length: usize, payload: &[u8], ) -> DataFileResult<Bytes>` | Rename from `execute_inner` |
| [229](feuer-storage/src/file.rs#L229) | Function | `open_file_state` | `fn(directory: PathBuf, capacity: u64) -> DataFileResult<DataFileState>` | Keep |
| [297](feuer-storage/src/file.rs#L297) | Function | `check_direct_io_alignment` | `fn(file: &File) -> io::Result<()>` | Rename from `check_direct_alignment` |
| [328](feuer-storage/src/file.rs#L328) | Function | `check_range` | `fn(operation: IoOperation, offset: u64, length: u64, capacity: u64) -> DataFileResult<()>` | Keep |
| [340](feuer-storage/src/file.rs#L340) | Function | `record_span_outcome` | `fn<T>(span: &Span, elapsed: std::time::Duration, result: &DataFileResult<T>)` | Keep |
| [354](feuer-storage/src/file.rs#L354) | Module | `tests` | — | Keep |
| [359](feuer-storage/src/file.rs#L359) | Const | `tests::CAPACITY` | `u64` | Keep |
| [362](feuer-storage/src/file.rs#L362) | Function | `tests::public_io_types_are_send_sync_static` | `fn()` | Keep |
| [363](feuer-storage/src/file.rs#L363) | Function | `tests::public_io_types_are_send_sync_static::assert_send_sync_static` | `fn<T: Send + Sync + 'static>()` | Keep |
| [369](feuer-storage/src/file.rs#L369) | Function | `tests::reads_exact_unaligned_ranges_and_preserves_neighbors` | `fn()` | Keep |
| [396](feuer-storage/src/file.rs#L396) | Function | `tests::rejects_out_of_bounds_and_accepts_empty_ranges` | `fn()` | Keep |
| [415](feuer-storage/src/file.rs#L415) | Function | `tests::fails_short_reads_instead_of_returning_uncertain_bytes` | `fn()` | Keep |
| [431](feuer-storage/src/file.rs#L431) | Function | `tests::holds_exclusive_ownership_and_reopens_after_shutdown` | `fn()` | Keep |
| [453](feuer-storage/src/file.rs#L453) | Function | `tests::concurrent_mixed_io_with_caller_serialized_same_page_rmw` | `fn()` | Storage update (see above) |
| [488](feuer-storage/src/file.rs#L488) | Function | `tests::canceled_callers_do_not_release_submitted_buffers_or_lock_early` | `fn()` | Keep |
| [517](feuer-storage/src/file.rs#L517) | Function | `tests::reports_missing_runtime` | `fn()` | Keep |
| [532](feuer-storage/src/file.rs#L532) | Function | `tests::rejects_unverified_direct_io_instead_of_falling_back` | `fn()` | Keep |
| [548](feuer-storage/src/file.rs#L548) | Function | `tests::validates_capacity_and_omits_paths_from_debug` | `fn()` | Keep |

<details>
<summary>Local bindings (68)</summary>

| Line | Kind | Name / source parent | Signature or type | Review decision |
| ---: | --- | --- | --- | --- |
| [84](feuer-storage/src/file.rs#L84) | Local | `impl DataFile::open::directory` | — | Keep |
| [85](feuer-storage/src/file.rs#L85) | Local | `impl DataFile::open::runtime` | — | Keep |
| [86](feuer-storage/src/file.rs#L86) | Local | `impl DataFile::open::started` | — | Keep |
| [87](feuer-storage/src/file.rs#L87) | Local | `impl DataFile::open::span` | — | Keep |
| [95](feuer-storage/src/file.rs#L95) | Local | `impl DataFile::open::result` | — | Keep |
| [96](feuer-storage/src/file.rs#L96) | Local | `impl DataFile::open::result::state` | — | Keep |
| [157](feuer-storage/src/file.rs#L157) | Local | `impl DataFile::execute_measured::started` | — | Keep |
| [158](feuer-storage/src/file.rs#L158) | Local | `impl DataFile::execute_measured::observed_bytes` | — | Keep |
| [159](feuer-storage/src/file.rs#L159) | Local | `impl DataFile::execute_measured::span` | — | Keep |
| [169](feuer-storage/src/file.rs#L169) | Local | `impl DataFile::execute_measured::result` | — | Keep |
| [173](feuer-storage/src/file.rs#L173) | Local | `impl DataFile::execute_measured::elapsed` | — | Keep |
| [186](feuer-storage/src/file.rs#L186) | Local | `impl DataFile::execute_chunks::length_u64` | — | Keep |
| [188](feuer-storage/src/file.rs#L188) | Local | `impl DataFile::execute_chunks::io_error` | — | Keep |
| [193](feuer-storage/src/file.rs#L193) | Local | `impl DataFile::execute_chunks::mut read_bytes` | — | Rename from `mut result` |
| [201](feuer-storage/src/file.rs#L201) | Local | `impl DataFile::execute_chunks::mut completed_bytes` | — | Keep |
| [203](feuer-storage/src/file.rs#L203) | Local | `impl DataFile::execute_chunks::chunk_offset` | — | Keep |
| [204](feuer-storage/src/file.rs#L204) | Local | `impl DataFile::execute_chunks::chunk_length` | — | Keep |
| [206](feuer-storage/src/file.rs#L206) | Local | `impl DataFile::execute_chunks::chunk_payload` | — | Rename from `input` |
| [211](feuer-storage/src/file.rs#L211) | Local | `impl DataFile::execute_chunks::bytes` | — | Keep |
| [239](feuer-storage/src/file.rs#L239) | Local | `open_file_state::lock_path` | — | Keep |
| [240](feuer-storage/src/file.rs#L240) | Local | `open_file_state::lock_file` | — | Keep |
| [251](feuer-storage/src/file.rs#L251) | Local | `open_file_state::locked` | — | Keep |
| [259](feuer-storage/src/file.rs#L259) | Local | `open_file_state::data_path` | — | Keep |
| [260](feuer-storage/src/file.rs#L260) | Local | `open_file_state::error` | — | Keep |
| [265](feuer-storage/src/file.rs#L265) | Local | `open_file_state::file` | — | Keep |
| [282](feuer-storage/src/file.rs#L282) | Local | `open_file_state::resize_file` | — | Keep |
| [285](feuer-storage/src/file.rs#L285) | Local | `open_file_state::queue` | — | Keep |
| [298](feuer-storage/src/file.rs#L298) | Local | `check_direct_io_alignment::mut stat` | — | Keep |
| [300](feuer-storage/src/file.rs#L300) | Local | `check_direct_io_alignment::result` | — | Keep |
| [313](feuer-storage/src/file.rs#L313) | Local | `check_direct_io_alignment::stat` | — | Keep |
| [370](feuer-storage/src/file.rs#L370) | Local | `tests::reads_exact_unaligned_ranges_and_preserves_neighbors::temp` | — | Keep |
| [371](feuer-storage/src/file.rs#L371) | Local | `tests::reads_exact_unaligned_ranges_and_preserves_neighbors::directory` | — | Keep |
| [372](feuer-storage/src/file.rs#L372) | Local | `tests::reads_exact_unaligned_ranges_and_preserves_neighbors::file` | — | Keep |
| [373](feuer-storage/src/file.rs#L373) | Local | `tests::reads_exact_unaligned_ranges_and_preserves_neighbors::original` | — | Keep |
| [375](feuer-storage/src/file.rs#L375) | Local | `tests::reads_exact_unaligned_ranges_and_preserves_neighbors::payload` | — | Keep |
| [384](feuer-storage/src/file.rs#L384) | Local | `tests::reads_exact_unaligned_ranges_and_preserves_neighbors::payload` | — | Keep |
| [397](feuer-storage/src/file.rs#L397) | Local | `tests::rejects_out_of_bounds_and_accepts_empty_ranges::temp` | — | Keep |
| [398](feuer-storage/src/file.rs#L398) | Local | `tests::rejects_out_of_bounds_and_accepts_empty_ranges::file` | — | Keep |
| [416](feuer-storage/src/file.rs#L416) | Local | `tests::fails_short_reads_instead_of_returning_uncertain_bytes::temp` | — | Keep |
| [417](feuer-storage/src/file.rs#L417) | Local | `tests::fails_short_reads_instead_of_returning_uncertain_bytes::file` | — | Keep |
| [432](feuer-storage/src/file.rs#L432) | Local | `tests::holds_exclusive_ownership_and_reopens_after_shutdown::temp` | — | Keep |
| [433](feuer-storage/src/file.rs#L433) | Local | `tests::holds_exclusive_ownership_and_reopens_after_shutdown::file` | — | Keep |
| [434](feuer-storage/src/file.rs#L434) | Local | `tests::holds_exclusive_ownership_and_reopens_after_shutdown::clone` | — | Keep |
| [445](feuer-storage/src/file.rs#L445) | Local | `tests::holds_exclusive_ownership_and_reopens_after_shutdown::file` | — | Keep |
| [454](feuer-storage/src/file.rs#L454) | Local | `tests::concurrent_mixed_io_with_caller_serialized_same_page_rmw::temp` | — | Storage update (see above) |
| [455](feuer-storage/src/file.rs#L455) | Local | `tests::concurrent_mixed_io_with_caller_serialized_same_page_rmw::file` | — | Storage update (see above) |
| [456](feuer-storage/src/file.rs#L456) | Local | `tests::concurrent_mixed_io_with_caller_serialized_same_page_rmw::mut tasks` | — | Storage update (see above) |
| [457](feuer-storage/src/file.rs#L457) | Local | `tests::concurrent_mixed_io_with_caller_serialized_same_page_rmw::shared_pages` | — | Storage update (see above) |
| [460](feuer-storage/src/file.rs#L460) | Local | `tests::concurrent_mixed_io_with_caller_serialized_same_page_rmw::file` | — | Storage update (see above) |
| [461](feuer-storage/src/file.rs#L461) | Local | `tests::concurrent_mixed_io_with_caller_serialized_same_page_rmw::shared_pages` | — | Storage update (see above) |
| [464](feuer-storage/src/file.rs#L464) | Local | `tests::concurrent_mixed_io_with_caller_serialized_same_page_rmw::_guard` | — | Storage update (see above) |
| [465](feuer-storage/src/file.rs#L465) | Local | `tests::concurrent_mixed_io_with_caller_serialized_same_page_rmw::value` | — | Storage update (see above) |
| [469](feuer-storage/src/file.rs#L469) | Local | `tests::concurrent_mixed_io_with_caller_serialized_same_page_rmw::value` | — | Storage update (see above) |
| [470](feuer-storage/src/file.rs#L470) | Local | `tests::concurrent_mixed_io_with_caller_serialized_same_page_rmw::offset` | — | Storage update (see above) |
| [489](feuer-storage/src/file.rs#L489) | Local | `tests::canceled_callers_do_not_release_submitted_buffers_or_lock_early::temp` | — | Keep |
| [490](feuer-storage/src/file.rs#L490) | Local | `tests::canceled_callers_do_not_release_submitted_buffers_or_lock_early::file` | — | Keep |
| [491](feuer-storage/src/file.rs#L491) | Local | `tests::canceled_callers_do_not_release_submitted_buffers_or_lock_early::mut tasks` | — | Keep |
| [493](feuer-storage/src/file.rs#L493) | Local | `tests::canceled_callers_do_not_release_submitted_buffers_or_lock_early::file` | — | Keep |
| [503](feuer-storage/src/file.rs#L503) | Local | `tests::canceled_callers_do_not_release_submitted_buffers_or_lock_early::_` | — | Keep |
| [506](feuer-storage/src/file.rs#L506) | Local | `tests::canceled_callers_do_not_release_submitted_buffers_or_lock_early::file` | — | Keep |
| [522](feuer-storage/src/file.rs#L522) | Local | `tests::reports_missing_runtime::temp` | — | Keep |
| [523](feuer-storage/src/file.rs#L523) | Local | `tests::reports_missing_runtime::mut open` | — | Keep |
| [524](feuer-storage/src/file.rs#L524) | Local | `tests::reports_missing_runtime::mut context` | — | Keep |
| [525](feuer-storage/src/file.rs#L525) | Local | `tests::reports_missing_runtime::Poll::Ready(result)` | — | Keep |
| [537](feuer-storage/src/file.rs#L537) | Local | `tests::rejects_unverified_direct_io_instead_of_falling_back::fd` | — | Keep |
| [540](feuer-storage/src/file.rs#L540) | Local | `tests::rejects_unverified_direct_io_instead_of_falling_back::file` | — | Keep |
| [549](feuer-storage/src/file.rs#L549) | Local | `tests::validates_capacity_and_omits_paths_from_debug::temp` | — | Keep |
| [559](feuer-storage/src/file.rs#L559) | Local | `tests::validates_capacity_and_omits_paths_from_debug::file` | — | Keep |

</details>

### `feuer-storage/src/lib.rs`

| Line | Kind | Name / source parent | Signature or type | Review decision |
| ---: | --- | --- | --- | --- |
| [9](feuer-storage/src/lib.rs#L9) | Module | `error` | — | Keep |
| [11](feuer-storage/src/lib.rs#L11) | Module | `file` | — | Keep |
| [12](feuer-storage/src/lib.rs#L12) | Module | `metrics` | — | Keep |
| [14](feuer-storage/src/lib.rs#L14) | Module | `uring` | — | Keep |

### `feuer-storage/src/metrics.rs`

| Line | Kind | Name / source parent | Signature or type | Review decision |
| ---: | --- | --- | --- | --- |
| [7](feuer-storage/src/metrics.rs#L7) | Struct | `IoOperationMetrics` | — | Rename from `OperationMetrics` |
| [8](feuer-storage/src/metrics.rs#L8) | Field | `IoOperationMetrics::success` | `BoxedCounter` | Keep |
| [9](feuer-storage/src/metrics.rs#L9) | Field | `IoOperationMetrics::error` | `BoxedCounter` | Keep |
| [10](feuer-storage/src/metrics.rs#L10) | Field | `IoOperationMetrics::bytes` | `BoxedCounter` | Keep |
| [11](feuer-storage/src/metrics.rs#L11) | Field | `IoOperationMetrics::success_duration` | `BoxedHistogram` | Keep |
| [12](feuer-storage/src/metrics.rs#L12) | Field | `IoOperationMetrics::error_duration` | `BoxedHistogram` | Keep |
| [15](feuer-storage/src/metrics.rs#L15) | Impl | `impl fmt::Debug for IoOperationMetrics` | — | Updated type references |
| [16](feuer-storage/src/metrics.rs#L16) | Method | `impl fmt::Debug for IoOperationMetrics::fmt` | `fn(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result` | Keep |
| [27](feuer-storage/src/metrics.rs#L27) | Struct | `IoMetrics` | — | Keep |
| [28](feuer-storage/src/metrics.rs#L28) | Field | `IoMetrics::read` | `IoOperationMetrics` | Keep |
| [29](feuer-storage/src/metrics.rs#L29) | Field | `IoMetrics::write` | `IoOperationMetrics` | Keep |
| [32](feuer-storage/src/metrics.rs#L32) | Impl | `impl IoMetrics` | — | Keep |
| [34](feuer-storage/src/metrics.rs#L34) | Function | `impl IoMetrics::new` | `fn(registry: &BoxedRegistry) -> Arc<Self>` | Keep |
| [66](feuer-storage/src/metrics.rs#L66) | Method | `impl IoMetrics::record` | `fn(&self, operation: IoOperation, bytes: u64, elapsed: Duration, success: bool)` | Keep |
| [84](feuer-storage/src/metrics.rs#L84) | Function | `impl IoMetrics::noop` | `fn() -> Arc<Self>` | Keep |
| [91](feuer-storage/src/metrics.rs#L91) | Module | `tests` | — | Keep |
| [95](feuer-storage/src/metrics.rs#L95) | Function | `tests::registers_with_the_normal_registry_boundary` | `fn()` | Keep |

<details>
<summary>Local bindings (7)</summary>

| Line | Kind | Name / source parent | Signature or type | Review decision |
| ---: | --- | --- | --- | --- |
| [35](feuer-storage/src/metrics.rs#L35) | Local | `impl IoMetrics::new::operations` | — | Keep |
| [40](feuer-storage/src/metrics.rs#L40) | Local | `impl IoMetrics::new::bytes` | — | Keep |
| [45](feuer-storage/src/metrics.rs#L45) | Local | `impl IoMetrics::new::duration` | — | Keep |
| [52](feuer-storage/src/metrics.rs#L52) | Local | `impl IoMetrics::new::operation` | — | Keep |
| [67](feuer-storage/src/metrics.rs#L67) | Local | `impl IoMetrics::record::metrics` | — | Keep |
| [85](feuer-storage/src/metrics.rs#L85) | Local | `impl IoMetrics::noop::registry` | `BoxedRegistry` | Keep |
| [96](feuer-storage/src/metrics.rs#L96) | Local | `tests::registers_with_the_normal_registry_boundary::metrics` | — | Keep |

</details>

### `feuer-storage/src/uring/tests.rs`

| Line | Kind | Name / source parent | Signature or type | Review decision |
| ---: | --- | --- | --- | --- |
| [5](feuer-storage/src/uring/tests.rs#L5) | TypeAlias | `IoResultReceiver` | `oneshot::Receiver<io::Result<Bytes>>` | Keep |
| [7](feuer-storage/src/uring/tests.rs#L7) | Function | `request` | `fn(queue: &IoQueue, operation: IoOperation, offset: u64, length: usize) -> (IoRequest, IoResultReceiver)` | Keep |
| [37](feuer-storage/src/uring/tests.rs#L37) | Function | `queue` | `fn() -> IoQueue` | Rename from `driver` |
| [65](feuer-storage/src/uring/tests.rs#L65) | Function | `fills_qd64_with_simultaneous_reads_and_writes_and_bounds_admission` | `fn()` | Keep |
| [132](feuer-storage/src/uring/tests.rs#L132) | Function | `complete_one` | `fn(queue: &mut IoQueue)` | Keep |
| [141](feuer-storage/src/uring/tests.rs#L141) | Function | `write_only_fills_the_ring_but_an_older_queued_read_gets_the_next_slot` | `fn()` | Keep |
| [195](feuer-storage/src/uring/tests.rs#L195) | Function | `active_read_does_not_throttle_rmw_writes` | `fn()` | Keep |
| [230](feuer-storage/src/uring/tests.rs#L230) | Function | `full_write_admission_and_buffers_leave_a_full_read_ring_available` | `fn()` | Keep |
| [262](feuer-storage/src/uring/tests.rs#L262) | Function | `canceled_submitted_rmw_retains_resources_and_still_writes` | `fn()` | Storage update (see above) |
| [296](feuer-storage/src/uring/tests.rs#L296) | Function | `discarded_queued_requests_never_reach_the_ring` | `fn()` | Keep |
| [315](feuer-storage/src/uring/tests.rs#L315) | Function | `completion_state_handles_short_io_errors_and_rmw` | `fn()` | Keep |
| [342](feuer-storage/src/uring/tests.rs#L342) | Function | `byte_budget_bounds_rmw_requests_and_releases_on_cancel` | `fn()` | Keep |

<details>
<summary>Local bindings (41)</summary>

| Line | Kind | Name / source parent | Signature or type | Review decision |
| ---: | --- | --- | --- | --- |
| [8](feuer-storage/src/uring/tests.rs#L8) | Local | `request::(reply, receive)` | — | Keep |
| [9](feuer-storage/src/uring/tests.rs#L9) | Local | `request::class` | — | Keep |
| [10](feuer-storage/src/uring/tests.rs#L10) | Local | `request::request_permit` | — | Keep |
| [14](feuer-storage/src/uring/tests.rs#L14) | Local | `request::staging_pages_permit` | — | Rename from `staging_permit` |
| [18](feuer-storage/src/uring/tests.rs#L18) | Local | `request::payload` | — | Keep |
| [38](feuer-storage/src/uring/tests.rs#L38) | Local | `queue::temporary` | — | Keep |
| [39](feuer-storage/src/uring/tests.rs#L39) | Local | `queue::file` | — | Keep |
| [47](feuer-storage/src/uring/tests.rs#L47) | Local | `queue::fd` | — | Keep |
| [50](feuer-storage/src/uring/tests.rs#L50) | Local | `queue::wake_fd` | — | Rename from `wake` |
| [51](feuer-storage/src/uring/tests.rs#L51) | Local | `queue::(_, receiver)` | — | Keep |
| [66](feuer-storage/src/uring/tests.rs#L66) | Local | `fills_qd64_with_simultaneous_reads_and_writes_and_bounds_admission::mut queue` | — | Rename from `mut driver` |
| [67](feuer-storage/src/uring/tests.rs#L67) | Local | `fills_qd64_with_simultaneous_reads_and_writes_and_bounds_admission::mut replies` | — | Keep |
| [69](feuer-storage/src/uring/tests.rs#L69) | Local | `fills_qd64_with_simultaneous_reads_and_writes_and_bounds_admission::operation` | — | Keep |
| [74](feuer-storage/src/uring/tests.rs#L74) | Local | `fills_qd64_with_simultaneous_reads_and_writes_and_bounds_admission::(request, reply)` | — | Keep |
| [134](feuer-storage/src/uring/tests.rs#L134) | Local | `complete_one::cqe` | — | Keep |
| [135](feuer-storage/src/uring/tests.rs#L135) | Local | `complete_one::mut request` | — | Keep |
| [142](feuer-storage/src/uring/tests.rs#L142) | Local | `write_only_fills_the_ring_but_an_older_queued_read_gets_the_next_slot::mut queue` | — | Rename from `mut driver` |
| [143](feuer-storage/src/uring/tests.rs#L143) | Local | `write_only_fills_the_ring_but_an_older_queued_read_gets_the_next_slot::mut replies` | — | Keep |
| [145](feuer-storage/src/uring/tests.rs#L145) | Local | `write_only_fills_the_ring_but_an_older_queued_read_gets_the_next_slot::(request, reply)` | — | Keep |
| [157](feuer-storage/src/uring/tests.rs#L157) | Local | `write_only_fills_the_ring_but_an_older_queued_read_gets_the_next_slot::(read, read_reply)` | — | Keep |
| [168](feuer-storage/src/uring/tests.rs#L168) | Local | `write_only_fills_the_ring_but_an_older_queued_read_gets_the_next_slot::(write, reply)` | — | Keep |
| [196](feuer-storage/src/uring/tests.rs#L196) | Local | `active_read_does_not_throttle_rmw_writes::mut queue` | — | Rename from `mut driver` |
| [197](feuer-storage/src/uring/tests.rs#L197) | Local | `active_read_does_not_throttle_rmw_writes::(read, reply)` | — | Keep |
| [199](feuer-storage/src/uring/tests.rs#L199) | Local | `active_read_does_not_throttle_rmw_writes::mut replies` | — | Keep |
| [202](feuer-storage/src/uring/tests.rs#L202) | Local | `active_read_does_not_throttle_rmw_writes::(request, reply)` | — | Keep |
| [231](feuer-storage/src/uring/tests.rs#L231) | Local | `full_write_admission_and_buffers_leave_a_full_read_ring_available::queue` | — | Rename from `driver` |
| [232](feuer-storage/src/uring/tests.rs#L232) | Local | `full_write_admission_and_buffers_leave_a_full_read_ring_available::mut writes` | — | Keep |
| [233](feuer-storage/src/uring/tests.rs#L233) | Local | `full_write_admission_and_buffers_leave_a_full_read_ring_available::mut reads` | — | Keep |
| [263](feuer-storage/src/uring/tests.rs#L263) | Local | `canceled_submitted_rmw_retains_resources_and_still_writes::mut queue` | — | Storage update (see above) |
| [264](feuer-storage/src/uring/tests.rs#L264) | Local | `canceled_submitted_rmw_retains_resources_and_still_writes::(write, reply)` | — | Storage update (see above) |
| [289](feuer-storage/src/uring/tests.rs#L289) | Local | `canceled_submitted_rmw_retains_resources_and_still_writes::(read, mut reply)` | — | Storage update (see above) |
| [297](feuer-storage/src/uring/tests.rs#L297) | Local | `discarded_queued_requests_never_reach_the_ring::mut queue` | — | Rename from `mut driver` |
| [298](feuer-storage/src/uring/tests.rs#L298) | Local | `discarded_queued_requests_never_reach_the_ring::(request, reply)` | — | Keep |
| [316](feuer-storage/src/uring/tests.rs#L316) | Local | `completion_state_handles_short_io_errors_and_rmw::queue` | — | Rename from `driver` |
| [317](feuer-storage/src/uring/tests.rs#L317) | Local | `completion_state_handles_short_io_errors_and_rmw::(mut read, _reply)` | — | Keep |
| [323](feuer-storage/src/uring/tests.rs#L323) | Local | `completion_state_handles_short_io_errors_and_rmw::(mut read, _reply)` | — | Keep |
| [325](feuer-storage/src/uring/tests.rs#L325) | Local | `completion_state_handles_short_io_errors_and_rmw::(mut write, _reply)` | — | Keep |
| [332](feuer-storage/src/uring/tests.rs#L332) | Local | `completion_state_handles_short_io_errors_and_rmw::(mut rmw, _reply)` | — | Keep |
| [343](feuer-storage/src/uring/tests.rs#L343) | Local | `byte_budget_bounds_rmw_requests_and_releases_on_cancel::queue` | — | Rename from `driver` |
| [344](feuer-storage/src/uring/tests.rs#L344) | Local | `byte_budget_bounds_rmw_requests_and_releases_on_cancel::mut requests` | — | Keep |
| [352](feuer-storage/src/uring/tests.rs#L352) | Local | `byte_budget_bounds_rmw_requests_and_releases_on_cancel::_read` | — | Keep |

</details>

### `feuer-storage/src/uring.rs`

| Line | Kind | Name / source parent | Signature or type | Review decision |
| ---: | --- | --- | --- | --- |
| [22](feuer-storage/src/uring.rs#L22) | Const | `DIRECT_IO_ALIGNMENT_BYTES` | `usize` | Keep |
| [25](feuer-storage/src/uring.rs#L25) | Const | `MAX_IO_CHUNK_BYTES` | `usize` | Keep |
| [27](feuer-storage/src/uring.rs#L27) | Const | `MAX_IN_FLIGHT_IO` | `usize` | Keep |
| [30](feuer-storage/src/uring.rs#L30) | Const | `MAX_ADMITTED_REQUESTS` | `usize` | Keep |
| [34](feuer-storage/src/uring.rs#L34) | Const | `MAX_STAGING_BUFFER_BYTES` | `usize` | Keep |
| [37](feuer-storage/src/uring.rs#L37) | Module | `tests` | — | Keep |
| [40](feuer-storage/src/uring.rs#L40) | Struct | `IoQueueHandle` | — | Keep |
| [42](feuer-storage/src/uring.rs#L42) | Field | `IoQueueHandle::sender` | `Option<mpsc::SyncSender<IoRequest>>` | Keep |
| [44](feuer-storage/src/uring.rs#L44) | Field | `IoQueueHandle::wake_fd` | `Arc<OwnedFd>` | Rename from `wake` |
| [46](feuer-storage/src/uring.rs#L46) | Field | `IoQueueHandle::thread` | `Option<JoinHandle<()>>` | Keep |
| [48](feuer-storage/src/uring.rs#L48) | Field | `IoQueueHandle::admission` | `Arc<IoAdmissionBudgets>` | Keep |
| [53](feuer-storage/src/uring.rs#L53) | Struct | `IoAdmissionBudgets` | — | Keep |
| [55](feuer-storage/src/uring.rs#L55) | Field | `IoAdmissionBudgets::request_slots` | `[Arc<Semaphore>; 2]` | Rename from `requests` |
| [57](feuer-storage/src/uring.rs#L57) | Field | `IoAdmissionBudgets::staging_pages` | `[Arc<Semaphore>; 2]` | Rename from `buffers` |
| [60](feuer-storage/src/uring.rs#L60) | Impl | `impl IoAdmissionBudgets` | — | Keep |
| [61](feuer-storage/src/uring.rs#L61) | Function | `impl IoAdmissionBudgets::new` | `fn() -> Self` | Keep |
| [71](feuer-storage/src/uring.rs#L71) | Function | `staging_pages_for` | `fn(operation: IoOperation, offset: u64, length: usize) -> u32` | Keep |
| [80](feuer-storage/src/uring.rs#L80) | Impl | `impl IoQueueHandle` | — | Keep |
| [81](feuer-storage/src/uring.rs#L81) | Function | `impl IoQueueHandle::new` | `fn(file: File, directory_lock: File) -> io::Result<Self>` | Keep |
| [115](feuer-storage/src/uring.rs#L115) | Method | `impl IoQueueHandle::execute` | `fn( &self, operation: IoOperation, offset: u64, length: usize, payload: &[u8], ) -> io::Result<Bytes>` | Keep |
| [153](feuer-storage/src/uring.rs#L153) | Impl | `impl Drop for IoQueueHandle` | — | Keep |
| [154](feuer-storage/src/uring.rs#L154) | Method | `impl Drop for IoQueueHandle::drop` | `fn(&mut self)` | Keep |
| [164](feuer-storage/src/uring.rs#L164) | Function | `queue_stopped_error` | `fn() -> io::Error` | Rename from `stopped` |
| [168](feuer-storage/src/uring.rs#L168) | Struct | `AlignedIoBuffer` | — | Rename from `AlignedBuffer` |
| [170](feuer-storage/src/uring.rs#L170) | Field | `AlignedIoBuffer::ptr` | `NonNull<u8>` | Keep |
| [172](feuer-storage/src/uring.rs#L172) | Field | `AlignedIoBuffer::layout` | `Layout` | Keep |
| [175](feuer-storage/src/uring.rs#L175) | Impl | `impl AlignedIoBuffer` | — | Updated type references |
| [176](feuer-storage/src/uring.rs#L176) | Function | `impl AlignedIoBuffer::new` | `fn(length: usize) -> io::Result<Self>` | Keep |
| [184](feuer-storage/src/uring.rs#L184) | Method | `impl AlignedIoBuffer::as_mut_slice` | `fn(&mut self) -> &mut [u8]` | Keep |
| [192](feuer-storage/src/uring.rs#L192) | Impl | `impl Send for AlignedIoBuffer` | — | Updated type references |
| [194](feuer-storage/src/uring.rs#L194) | Impl | `impl Drop for AlignedIoBuffer` | — | Updated type references |
| [195](feuer-storage/src/uring.rs#L195) | Method | `impl Drop for AlignedIoBuffer::drop` | `fn(&mut self)` | Keep |
| [202](feuer-storage/src/uring.rs#L202) | Struct | `IoRequest` | — | Keep |
| [204](feuer-storage/src/uring.rs#L204) | Field | `IoRequest::operation` | `IoOperation` | Keep |
| [206](feuer-storage/src/uring.rs#L206) | Field | `IoRequest::aligned_offset` | `u64` | Keep |
| [208](feuer-storage/src/uring.rs#L208) | Field | `IoRequest::data_offset_in_buffer` | `usize` | Keep |
| [210](feuer-storage/src/uring.rs#L210) | Field | `IoRequest::length` | `usize` | Keep |
| [212](feuer-storage/src/uring.rs#L212) | Field | `IoRequest::aligned_length` | `usize` | Keep |
| [214](feuer-storage/src/uring.rs#L214) | Field | `IoRequest::io_buffer` | `AlignedIoBuffer` | Keep |
| [216](feuer-storage/src/uring.rs#L216) | Field | `IoRequest::read_modify_write_payload` | `Option<Bytes>` | Keep |
| [218](feuer-storage/src/uring.rs#L218) | Field | `IoRequest::reading_phase` | `bool` | Rename from `reading` |
| [220](feuer-storage/src/uring.rs#L220) | Field | `IoRequest::completed_bytes` | `usize` | Keep |
| [222](feuer-storage/src/uring.rs#L222) | Field | `IoRequest::reply` | `Option<oneshot::Sender<io::Result<Bytes>>>` | Keep |
| [224](feuer-storage/src/uring.rs#L224) | Field | `IoRequest::_request_permit` | `OwnedSemaphorePermit` | Keep |
| [226](feuer-storage/src/uring.rs#L226) | Field | `IoRequest::_staging_pages_permit` | `OwnedSemaphorePermit` | Rename from `_staging_permit` |
| [229](feuer-storage/src/uring.rs#L229) | Impl | `impl IoRequest` | — | Keep |
| [230](feuer-storage/src/uring.rs#L230) | Function | `impl IoRequest::new` | `fn( operation: IoOperation, offset: u64, length: usize, payload: &[u8], reply: oneshot::Sender<io::Result<Bytes>>, permits: (OwnedSemaphorePermit, OwnedSemaphorePermit), ) -> io::Result<Self>` | Keep |
| [267](feuer-storage/src/uring.rs#L267) | Method | `impl IoRequest::submission_entry` | `fn(&mut self, fd: i32, slot: usize) -> squeue::Entry` | Keep |
| [283](feuer-storage/src/uring.rs#L283) | Method | `impl IoRequest::complete` | `fn(&mut self, result: i32) -> io::Result<bool>` | Keep |
| [312](feuer-storage/src/uring.rs#L312) | Method | `impl IoRequest::incomplete_io_error` | `fn(&self) -> io::Error` | Rename from `short_error` |
| [323](feuer-storage/src/uring.rs#L323) | Method | `impl IoRequest::finish` | `fn(mut self, result: io::Result<()>)` | Keep |
| [338](feuer-storage/src/uring.rs#L338) | Struct | `IoQueue` | — | Keep |
| [340](feuer-storage/src/uring.rs#L340) | Field | `IoQueue::admission` | `Arc<IoAdmissionBudgets>` | Keep |
| [342](feuer-storage/src/uring.rs#L342) | Field | `IoQueue::ring` | `IoUring` | Keep |
| [344](feuer-storage/src/uring.rs#L344) | Field | `IoQueue::file` | `Option<File>` | Keep |
| [346](feuer-storage/src/uring.rs#L346) | Field | `IoQueue::directory_lock` | `Option<File>` | Rename from `lock` |
| [348](feuer-storage/src/uring.rs#L348) | Field | `IoQueue::wake_fd` | `Arc<OwnedFd>` | Rename from `wake` |
| [350](feuer-storage/src/uring.rs#L350) | Field | `IoQueue::receiver` | `mpsc::Receiver<IoRequest>` | Keep |
| [352](feuer-storage/src/uring.rs#L352) | Field | `IoQueue::pending` | `VecDeque<IoRequest>` | Keep |
| [354](feuer-storage/src/uring.rs#L354) | Field | `IoQueue::active` | `Vec<Option<IoRequest>>` | Keep |
| [357](feuer-storage/src/uring.rs#L357) | Impl | `impl IoQueue` | — | Keep |
| [358](feuer-storage/src/uring.rs#L358) | Method | `impl IoQueue::run` | `fn(&mut self) -> io::Result<()>` | Keep |
| [405](feuer-storage/src/uring.rs#L405) | Method | `impl IoQueue::schedule` | `fn(&mut self)` | Keep |
| [425](feuer-storage/src/uring.rs#L425) | Method | `impl IoQueue::submit_slot` | `fn(&mut self, slot: usize)` | Keep |
| [440](feuer-storage/src/uring.rs#L440) | Method | `impl IoQueue::wait` | `fn(&self) -> io::Result<()>` | Keep |
| [478](feuer-storage/src/uring.rs#L478) | Impl | `impl Drop for IoQueue` | — | Keep |
| [479](feuer-storage/src/uring.rs#L479) | Method | `impl Drop for IoQueue::drop` | `fn(&mut self)` | Keep |
| [501](feuer-storage/src/uring.rs#L501) | Function | `notify` | `fn(wake_fd: &OwnedFd)` | Keep |

<details>
<summary>Local bindings (45)</summary>

| Line | Kind | Name / source parent | Signature or type | Review decision |
| ---: | --- | --- | --- | --- |
| [72](feuer-storage/src/uring.rs#L72) | Local | `staging_pages_for::data_offset_in_buffer` | — | Keep |
| [73](feuer-storage/src/uring.rs#L73) | Local | `staging_pages_for::aligned_length` | — | Keep |
| [74](feuer-storage/src/uring.rs#L74) | Local | `staging_pages_for::needs_read_modify_write` | — | Keep |
| [82](feuer-storage/src/uring.rs#L82) | Local | `impl IoQueueHandle::new::ring` | — | Keep |
| [84](feuer-storage/src/uring.rs#L84) | Local | `impl IoQueueHandle::new::fd` | — | Keep |
| [89](feuer-storage/src/uring.rs#L89) | Local | `impl IoQueueHandle::new::wake_fd` | — | Rename from `wake` |
| [90](feuer-storage/src/uring.rs#L90) | Local | `impl IoQueueHandle::new::(sender, receiver)` | — | Keep |
| [91](feuer-storage/src/uring.rs#L91) | Local | `impl IoQueueHandle::new::admission` | — | Keep |
| [92](feuer-storage/src/uring.rs#L92) | Local | `impl IoQueueHandle::new::mut queue` | — | Keep |
| [102](feuer-storage/src/uring.rs#L102) | Local | `impl IoQueueHandle::new::thread` | — | Keep |
| [122](feuer-storage/src/uring.rs#L122) | Local | `impl IoQueueHandle::execute::class` | — | Keep |
| [123](feuer-storage/src/uring.rs#L123) | Local | `impl IoQueueHandle::execute::request_permit` | — | Keep |
| [128](feuer-storage/src/uring.rs#L128) | Local | `impl IoQueueHandle::execute::staging_pages_permit` | — | Rename from `staging_permit` |
| [133](feuer-storage/src/uring.rs#L133) | Local | `impl IoQueueHandle::execute::(reply, receive)` | — | Keep |
| [134](feuer-storage/src/uring.rs#L134) | Local | `impl IoQueueHandle::execute::request` | — | Keep |
| [159](feuer-storage/src/uring.rs#L159) | Local | `impl Drop for IoQueueHandle::drop::_` | — | Keep |
| [177](feuer-storage/src/uring.rs#L177) | Local | `impl AlignedIoBuffer::new::layout` | — | Keep |
| [179](feuer-storage/src/uring.rs#L179) | Local | `impl AlignedIoBuffer::new::ptr` | — | Keep |
| [238](feuer-storage/src/uring.rs#L238) | Local | `impl IoRequest::new::data_offset_in_buffer` | — | Keep |
| [239](feuer-storage/src/uring.rs#L239) | Local | `impl IoRequest::new::aligned_length` | — | Keep |
| [240](feuer-storage/src/uring.rs#L240) | Local | `impl IoRequest::new::mut io_buffer` | — | Keep |
| [241](feuer-storage/src/uring.rs#L241) | Local | `impl IoRequest::new::needs_read_modify_write` | — | Keep |
| [243](feuer-storage/src/uring.rs#L243) | Local | `impl IoRequest::new::payload` | — | Keep |
| [268](feuer-storage/src/uring.rs#L268) | Local | `impl IoRequest::submission_entry::fd` | — | Keep |
| [271](feuer-storage/src/uring.rs#L271) | Local | `impl IoRequest::submission_entry::ptr` | — | Keep |
| [272](feuer-storage/src/uring.rs#L272) | Local | `impl IoRequest::submission_entry::length` | — | Keep |
| [273](feuer-storage/src/uring.rs#L273) | Local | `impl IoRequest::submission_entry::offset` | — | Keep |
| [274](feuer-storage/src/uring.rs#L274) | Local | `impl IoRequest::submission_entry::entry` | — | Keep |
| [290](feuer-storage/src/uring.rs#L290) | Local | `impl IoRequest::complete::count` | — | Keep |
| [324](feuer-storage/src/uring.rs#L324) | Local | `impl IoRequest::finish::result` | — | Keep |
| [334](feuer-storage/src/uring.rs#L334) | Local | `impl IoRequest::finish::_` | — | Keep |
| [359](feuer-storage/src/uring.rs#L359) | Local | `impl IoQueue::run::mut disconnected` | — | Keep |
| [361](feuer-storage/src/uring.rs#L361) | Local | `impl IoQueue::run::completions` | `Vec<_>` | Keep |
| [367](feuer-storage/src/uring.rs#L367) | Local | `impl IoQueue::run::slot` | — | Keep |
| [368](feuer-storage/src/uring.rs#L368) | Local | `impl IoQueue::run::request` | — | Keep |
| [385](feuer-storage/src/uring.rs#L385) | Local | `impl IoQueue::run::active` | — | Keep |
| [417](feuer-storage/src/uring.rs#L417) | Local | `impl IoQueue::schedule::Some(request)` | — | Storage update (see above) |
| [426](feuer-storage/src/uring.rs#L426) | Local | `impl IoQueue::submit_slot::entry` | — | Keep |
| [441](feuer-storage/src/uring.rs#L441) | Local | `impl IoQueue::wait::mut fds` | — | Keep |
| [454](feuer-storage/src/uring.rs#L454) | Local | `impl IoQueue::wait::result` | — | Keep |
| [456](feuer-storage/src/uring.rs#L456) | Local | `impl IoQueue::wait::error` | — | Keep |
| [470](feuer-storage/src/uring.rs#L470) | Local | `impl IoQueue::wait::mut value` | — | Keep |
| [491](feuer-storage/src/uring.rs#L491) | Local | `impl Drop for IoQueue::drop::_` | — | Keep |
| [502](feuer-storage/src/uring.rs#L502) | Local | `notify::value` | — | Keep |
| [505](feuer-storage/src/uring.rs#L505) | Local | `notify::result` | — | Keep |

</details>

## feuer-tokio

### `feuer-tokio/src/lib.rs`

No source symbols reported (for example, a re-export-only module).

## feuer-types

### `feuer-types/src/download.rs`

| Line | Kind | Name / source parent | Signature or type | Review decision |
| ---: | --- | --- | --- | --- |
| [15](feuer-types/src/download.rs#L15) | Struct | `Download` | — | Keep |
| [16](feuer-types/src/download.rs#L16) | Field | `Download::downloaded_start` | `u64` | Keep |
| [17](feuer-types/src/download.rs#L17) | Field | `Download::bytes` | `Bytes` | Keep |
| [20](feuer-types/src/download.rs#L20) | Impl | `impl Download` | — | Keep |
| [22](feuer-types/src/download.rs#L22) | Function | `impl Download::new` | `fn(downloaded_start: u64, bytes: Bytes) -> Result<Self, DownloadError>` | Keep |
| [40](feuer-types/src/download.rs#L40) | Method | `impl Download::downloaded_start` | `fn(&self) -> u64` | Keep |
| [45](feuer-types/src/download.rs#L45) | Method | `impl Download::downloaded_range` | `fn(&self) -> ByteRange` | Keep |
| [51](feuer-types/src/download.rs#L51) | Method | `impl Download::bytes` | `fn(&self) -> &Bytes` | Keep |
| [56](feuer-types/src/download.rs#L56) | Method | `impl Download::into_parts` | `fn(self) -> (ByteRange, Bytes)` | Keep |
| [62](feuer-types/src/download.rs#L62) | Impl | `impl fmt::Debug for Download` | — | Keep |
| [63](feuer-types/src/download.rs#L63) | Method | `impl fmt::Debug for Download::fmt` | `fn(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result` | Keep |
| [73](feuer-types/src/download.rs#L73) | Enum | `DownloadError` | — | Keep |
| [76](feuer-types/src/download.rs#L76) | Variant | `DownloadError::EmptyPayload` | — | Keep |
| [79](feuer-types/src/download.rs#L79) | Variant | `DownloadError::RangeOverflow` | — | Keep |
| [81](feuer-types/src/download.rs#L81) | Field | `DownloadError::RangeOverflow::downloaded_start` | `u64` | Keep |
| [83](feuer-types/src/download.rs#L83) | Field | `DownloadError::RangeOverflow::payload_bytes` | `u64` | Keep |
| [88](feuer-types/src/download.rs#L88) | Module | `tests` | — | Keep |
| [91](feuer-types/src/download.rs#L91) | Function | `tests::range` | `fn(start: u64, end: u64) -> ByteRange` | Keep |
| [96](feuer-types/src/download.rs#L96) | Function | `tests::derives_the_exact_range_from_the_start_and_payload` | `fn()` | Keep |
| [106](feuer-types/src/download.rs#L106) | Function | `tests::rejects_empty_or_unrepresentable_ranges` | `fn()` | Keep |
| [118](feuer-types/src/download.rs#L118) | Function | `tests::debug_output_omits_payload_bytes` | `fn()` | Keep |

<details>
<summary>Local bindings (6)</summary>

| Line | Kind | Name / source parent | Signature or type | Review decision |
| ---: | --- | --- | --- | --- |
| [23](feuer-types/src/download.rs#L23) | Local | `impl Download::new::payload_bytes` | — | Keep |
| [57](feuer-types/src/download.rs#L57) | Local | `impl Download::into_parts::range` | — | Keep |
| [97](feuer-types/src/download.rs#L97) | Local | `tests::derives_the_exact_range_from_the_start_and_payload::payload` | — | Keep |
| [98](feuer-types/src/download.rs#L98) | Local | `tests::derives_the_exact_range_from_the_start_and_payload::download` | — | Keep |
| [119](feuer-types/src/download.rs#L119) | Local | `tests::debug_output_omits_payload_bytes::download` | — | Keep |
| [120](feuer-types/src/download.rs#L120) | Local | `tests::debug_output_omits_payload_bytes::output` | — | Keep |

</details>

### `feuer-types/src/lib.rs`

| Line | Kind | Name / source parent | Signature or type | Review decision |
| ---: | --- | --- | --- | --- |
| [8](feuer-types/src/lib.rs#L8) | Module | `download` | — | Keep |
| [9](feuer-types/src/lib.rs#L9) | Module | `range` | — | Keep |
| [15](feuer-types/src/lib.rs#L15) | TypeAlias | `ObjectKey` | `String` | Keep |

### `feuer-types/src/range.rs`

| Line | Kind | Name / source parent | Signature or type | Review decision |
| ---: | --- | --- | --- | --- |
| [9](feuer-types/src/range.rs#L9) | Struct | `ByteRange` | — | Keep |
| [10](feuer-types/src/range.rs#L10) | Field | `ByteRange::start` | `u64` | Keep |
| [11](feuer-types/src/range.rs#L11) | Field | `ByteRange::end` | `u64` | Keep |
| [14](feuer-types/src/range.rs#L14) | Impl | `impl ByteRange` | — | Keep |
| [16](feuer-types/src/range.rs#L16) | Function | `impl ByteRange::new` | `fn(start: u64, end: u64) -> Result<Self, InvalidByteRange>` | Keep |
| [24](feuer-types/src/range.rs#L24) | Method | `impl ByteRange::start` | `fn(self) -> u64` | Keep |
| [29](feuer-types/src/range.rs#L29) | Method | `impl ByteRange::end` | `fn(self) -> u64` | Keep |
| [34](feuer-types/src/range.rs#L34) | Method | `impl ByteRange::len` | `fn(self) -> u64` | Keep |
| [39](feuer-types/src/range.rs#L39) | Method | `impl ByteRange::is_empty` | `fn(self) -> bool` | Keep |
| [44](feuer-types/src/range.rs#L44) | Method | `impl ByteRange::contains` | `fn(self, other: Self) -> bool` | Keep |
| [49](feuer-types/src/range.rs#L49) | Method | `impl ByteRange::overlaps` | `fn(self, other: Self) -> bool` | Keep |
| [54](feuer-types/src/range.rs#L54) | Impl | `impl TryFrom<Range<u64>> for ByteRange` | — | Keep |
| [55](feuer-types/src/range.rs#L55) | TypeAlias | `impl TryFrom<Range<u64>> for ByteRange::Error` | `InvalidByteRange` | Keep |
| [57](feuer-types/src/range.rs#L57) | Function | `impl TryFrom<Range<u64>> for ByteRange::try_from` | `fn(range: Range<u64>) -> Result<Self, Self::Error>` | Keep |
| [62](feuer-types/src/range.rs#L62) | Impl | `impl From<ByteRange> for Range<u64>` | — | Keep |
| [63](feuer-types/src/range.rs#L63) | Function | `impl From<ByteRange> for Range<u64>::from` | `fn(range: ByteRange) -> Self` | Keep |
| [71](feuer-types/src/range.rs#L71) | Struct | `InvalidByteRange` | — | Rename from `InvalidRange` |
| [72](feuer-types/src/range.rs#L72) | Field | `InvalidByteRange::start` | `u64` | Keep |
| [73](feuer-types/src/range.rs#L73) | Field | `InvalidByteRange::end` | `u64` | Keep |
| [76](feuer-types/src/range.rs#L76) | Impl | `impl InvalidByteRange` | — | Updated type references |
| [78](feuer-types/src/range.rs#L78) | Method | `impl InvalidByteRange::start` | `fn(self) -> u64` | Keep |
| [83](feuer-types/src/range.rs#L83) | Method | `impl InvalidByteRange::end` | `fn(self) -> u64` | Keep |
| [89](feuer-types/src/range.rs#L89) | Module | `tests` | — | Keep |
| [93](feuer-types/src/range.rs#L93) | Function | `tests::preserves_arbitrary_unaligned_endpoints` | `fn()` | Keep |
| [102](feuer-types/src/range.rs#L102) | Function | `tests::rejects_empty_and_reversed_ranges` | `fn()` | Keep |
| [108](feuer-types/src/range.rs#L108) | Function | `tests::containment_and_overlap_use_exact_boundaries` | `fn()` | Keep |

<details>
<summary>Local bindings (2)</summary>

| Line | Kind | Name / source parent | Signature or type | Review decision |
| ---: | --- | --- | --- | --- |
| [94](feuer-types/src/range.rs#L94) | Local | `tests::preserves_arbitrary_unaligned_endpoints::range` | — | Keep |
| [109](feuer-types/src/range.rs#L109) | Local | `tests::containment_and_overlap_use_exact_boundaries::outer` | — | Keep |

</details>
