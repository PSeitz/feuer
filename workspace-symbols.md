# Workspace symbol inventory

Generated with `rust-analyzer 0.3.2929-standalone (7ea2b259ca 2026-06-07)` using `rust-analyzer symbols` on each workspace source file.

## Scope

- Cargo workspace: **6 packages**, **22 Rust source files**.
- Includes source declarations, fields, variants, implementation blocks, and local bindings reported by rust-analyzer.
- Includes tests, examples, the memory benchmark, and inactive `cfg` branches (including Linux-only storage code).
- Excludes the separate `foyer/` workspace, external dependencies, and generated build output.
- Syntax inventory, not an LSP `workspace/symbol` search or a type-check. Macro expansions and import/re-export aliases are not expanded.
- Function parameters appear in signatures; this is not a complete list of every identifier or pattern binding.
- Names are qualified by their source parent where rust-analyzer provides one; locations distinguish duplicate names.
- This is a snapshot of the current source symbols for naming review.

## Naming review

Reviewed declarations, fields, variants, methods, parameters, and local bindings against `AGENTS.md`, using the source to check what each name represents. The inventory below reflects the resulting names.

| What it represents | Name |
| --- | --- |
| Configuration and memory-cache state shared by cache handles | `CacheState` |
| Queue, path, and capacity state shared by data-file handles | `DataFileState` |
| One retained downloaded range and its bytes | `CachedRange` |
| One object's cached ranges and access history | `ObjectCachedRanges` |
| Cached ranges superseded by a larger download | `SupersededRanges` |
| Payload-byte and entry usage removed during admission | `RemovedUsage` |
| A cached range's identity: object key, start offset, and entry ID | `CachedRangeIdentity` |
| Cached-range candidates sampled under memory pressure | `PressureCandidates` |
| A cached range selected for compaction or eviction under memory pressure | `PressureCandidate` |
| Source bytes, identity, and plan for compaction outside the shard lock | `CompactionSource` |
| Copied compaction replacement payload awaiting revalidated publication | `CompactionReplacement` |
| One memory-cache shard's range indexes, access history, and accounting | `MemoryShard` |
| A handle that submits I/O requests and owns the queue thread's lifetime | `IoQueueHandle` |
| Per-operation-class request and staging-buffer admission budgets | `IoAdmissionBudgets` |
| An admitted I/O request owning its buffers and permits through completion | `IoRequest` |
| A cache populated and queried during workload replay | `ReplayCache` |
| Feuer and Foyer caches used for workload replay | `FeuerReplayCache`, `FoyerReplayCache` |
| Test caches for warmup and range-coverage replay | `WarmupTestCache`, `RangeTestCache` |
| Completed I/O counts and sampled latencies for a measurement interval | `IoMeasurements` |
| The receiver for an I/O request's result in tests | `IoResultReceiver` |

- Clarified physical versus logical offsets (`aligned_offset`), progress (`completed_bytes`), read-modify-write input (`read_modify_write_payload`), admission permits, retained-byte counts, and latency-sample units.
- Named I/O limits by their purpose and units: `DIRECT_IO_ALIGNMENT_BYTES`, `MAX_ADMITTED_REQUESTS`, and `MAX_STAGING_BUFFER_BYTES`. `staging_pages_for` returns alignment-page units, not bytes.
- Kept names that are already concrete in context, public APIs, Rust trait-required names, and conventional short local names where their meaning is unambiguous. Benchmark CLI flags, CSV columns, metric names, and tracing labels are unchanged.
- No reserved disk-region or read-guard types exist yet. When implemented, use `DiskRegion` and `DiskRegionReadGuard` as required by `AGENTS.md`; the I/O request's semaphore permits are not disk-region read guards.
- The separate `foyer/` workspace was not edited.

## Counts

| Kind | Count |
| --- | ---: |
| Const | 28 |
| Enum | 10 |
| Field | 178 |
| Function | 164 |
| Impl | 51 |
| Local | 544 |
| Method | 135 |
| Module | 25 |
| Struct | 44 |
| Trait | 1 |
| TypeAlias | 5 |
| Variant | 38 |
| **Total** | **1223** |

## Packages

| Package | Files | Items / fields / variants | Implementation blocks | Local bindings |
| --- | ---: | ---: | ---: | ---: |
| `feuer` | 3 | 42 | 3 | 51 |
| `feuer-memory` | 7 | 211 | 12 | 180 |
| `feuer-memory-bench` | 1 | 149 | 12 | 109 |
| `feuer-storage` | 7 | 182 | 18 | 196 |
| `feuer-tokio` | 1 | 0 | 0 | 0 |
| `feuer-types` | 3 | 44 | 6 | 8 |

## feuer

### `feuer/src/cache.rs`

| Line | Kind | Name / source parent | Signature or type |
| ---: | --- | --- | --- |
| [11](feuer/src/cache.rs#L11) | Struct | `CacheState` | — |
| [12](feuer/src/cache.rs#L12) | Field | `CacheState::config` | `Config` |
| [13](feuer/src/cache.rs#L13) | Field | `CacheState::memory` | `MemoryCache` |
| [23](feuer/src/cache.rs#L23) | Struct | `Cache` | — |
| [24](feuer/src/cache.rs#L24) | Field | `Cache::state` | `Arc<CacheState>` |
| [27](feuer/src/cache.rs#L27) | Impl | `impl fmt::Debug for Cache` | — |
| [28](feuer/src/cache.rs#L28) | Method | `impl fmt::Debug for Cache::fmt` | `fn(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result` |
| [36](feuer/src/cache.rs#L36) | Impl | `impl Cache` | — |
| [38](feuer/src/cache.rs#L38) | Function | `impl Cache::new` | `fn(config: Config) -> Self` |
| [46](feuer/src/cache.rs#L46) | Method | `impl Cache::config` | `fn(&self) -> &Config` |
| [62](feuer/src/cache.rs#L62) | Method | `impl Cache::get_or_fetch` | `fn<F, Fut, E>( &self, object_key: ObjectKey, requested_range: ByteRange, callback: F, ) -> Result<Bytes, GetOrFetchError<E>>` |
| [96](feuer/src/cache.rs#L96) | Enum | `GetOrFetchError` | — |
| [99](feuer/src/cache.rs#L99) | Variant | `GetOrFetchError::Callback` | — |
| [102](feuer/src/cache.rs#L102) | Variant | `GetOrFetchError::DownloadDoesNotCover` | — |
| [104](feuer/src/cache.rs#L104) | Field | `GetOrFetchError::DownloadDoesNotCover::requested_range` | `ByteRange` |
| [106](feuer/src/cache.rs#L106) | Field | `GetOrFetchError::DownloadDoesNotCover::downloaded_range` | `ByteRange` |
| [110](feuer/src/cache.rs#L110) | Function | `requested_slice` | `fn(bytes: &Bytes, downloaded_range: ByteRange, requested_range: ByteRange) -> Bytes` |
| [120](feuer/src/cache.rs#L120) | Module | `tests` | — |
| [133](feuer/src/cache.rs#L133) | Function | `tests::range` | `fn(start: u64, end: u64) -> ByteRange` |
| [137](feuer/src/cache.rs#L137) | Function | `tests::cache` | `fn(memory_capacity: u64) -> Cache` |
| [142](feuer/src/cache.rs#L142) | Function | `tests::cache_handle_is_send_sync_static` | `fn()` |
| [143](feuer/src/cache.rs#L143) | Function | `tests::cache_handle_is_send_sync_static::assert_send_sync_static` | `fn<T: Send + Sync + 'static>()` |
| [148](feuer/src/cache.rs#L148) | Function | `tests::callback_result_and_covering_memory_hit_return_the_exact_request` | `fn()` |
| [179](feuer/src/cache.rs#L179) | Function | `tests::every_concurrent_miss_invokes_its_own_callback` | `fn()` |
| [211](feuer/src/cache.rs#L211) | Function | `tests::callback_errors_are_returned_without_retry_or_population` | `fn()` |
| [232](feuer/src/cache.rs#L232) | Function | `tests::rejects_noncovering_but_retains_oversized_callback_results` | `fn()` |
| [272](feuer/src/cache.rs#L272) | Function | `tests::a_racing_contained_download_is_discarded_but_returns_its_own_bytes` | `fn()` |

<details>
<summary>Local bindings (48)</summary>

| Line | Kind | Name / source parent | Signature or type |
| ---: | --- | --- | --- |
| [39](feuer/src/cache.rs#L39) | Local | `impl Cache::new::memory` | — |
| [76](feuer/src/cache.rs#L76) | Local | `impl Cache::get_or_fetch::download` | — |
| [77](feuer/src/cache.rs#L77) | Local | `impl Cache::get_or_fetch::downloaded_range` | — |
| [85](feuer/src/cache.rs#L85) | Local | `impl Cache::get_or_fetch::requested_bytes` | — |
| [112](feuer/src/cache.rs#L112) | Local | `requested_slice::start` | — |
| [114](feuer/src/cache.rs#L114) | Local | `requested_slice::end` | — |
| [149](feuer/src/cache.rs#L149) | Local | `tests::callback_result_and_covering_memory_hit_return_the_exact_request::cache` | — |
| [150](feuer/src/cache.rs#L150) | Local | `tests::callback_result_and_covering_memory_hit_return_the_exact_request::key` | — |
| [151](feuer/src/cache.rs#L151) | Local | `tests::callback_result_and_covering_memory_hit_return_the_exact_request::payload` | — |
| [152](feuer/src/cache.rs#L152) | Local | `tests::callback_result_and_covering_memory_hit_return_the_exact_request::callback_count` | — |
| [154](feuer/src/cache.rs#L154) | Local | `tests::callback_result_and_covering_memory_hit_return_the_exact_request::count` | — |
| [155](feuer/src/cache.rs#L155) | Local | `tests::callback_result_and_covering_memory_hit_return_the_exact_request::callback_payload` | — |
| [156](feuer/src/cache.rs#L156) | Local | `tests::callback_result_and_covering_memory_hit_return_the_exact_request::result` | — |
| [166](feuer/src/cache.rs#L166) | Local | `tests::callback_result_and_covering_memory_hit_return_the_exact_request::count` | — |
| [167](feuer/src/cache.rs#L167) | Local | `tests::callback_result_and_covering_memory_hit_return_the_exact_request::result` | — |
| [180](feuer/src/cache.rs#L180) | Local | `tests::every_concurrent_miss_invokes_its_own_callback::cache` | — |
| [181](feuer/src/cache.rs#L181) | Local | `tests::every_concurrent_miss_invokes_its_own_callback::key` | — |
| [182](feuer/src/cache.rs#L182) | Local | `tests::every_concurrent_miss_invokes_its_own_callback::barrier` | — |
| [183](feuer/src/cache.rs#L183) | Local | `tests::every_concurrent_miss_invokes_its_own_callback::callback_count` | — |
| [184](feuer/src/cache.rs#L184) | Local | `tests::every_concurrent_miss_invokes_its_own_callback::mut tasks` | — |
| [187](feuer/src/cache.rs#L187) | Local | `tests::every_concurrent_miss_invokes_its_own_callback::cache` | — |
| [188](feuer/src/cache.rs#L188) | Local | `tests::every_concurrent_miss_invokes_its_own_callback::key` | — |
| [189](feuer/src/cache.rs#L189) | Local | `tests::every_concurrent_miss_invokes_its_own_callback::barrier` | — |
| [190](feuer/src/cache.rs#L190) | Local | `tests::every_concurrent_miss_invokes_its_own_callback::callback_count` | — |
| [212](feuer/src/cache.rs#L212) | Local | `tests::callback_errors_are_returned_without_retry_or_population::cache` | — |
| [213](feuer/src/cache.rs#L213) | Local | `tests::callback_errors_are_returned_without_retry_or_population::key` | — |
| [214](feuer/src/cache.rs#L214) | Local | `tests::callback_errors_are_returned_without_retry_or_population::callback_count` | — |
| [217](feuer/src/cache.rs#L217) | Local | `tests::callback_errors_are_returned_without_retry_or_population::invocation_count` | — |
| [218](feuer/src/cache.rs#L218) | Local | `tests::callback_errors_are_returned_without_retry_or_population::error` | — |
| [233](feuer/src/cache.rs#L233) | Local | `tests::rejects_noncovering_but_retains_oversized_callback_results::cache` | — |
| [234](feuer/src/cache.rs#L234) | Local | `tests::rejects_noncovering_but_retains_oversized_callback_results::key` | — |
| [236](feuer/src/cache.rs#L236) | Local | `tests::rejects_noncovering_but_retains_oversized_callback_results::error` | — |
| [250](feuer/src/cache.rs#L250) | Local | `tests::rejects_noncovering_but_retains_oversized_callback_results::result` | — |
| [258](feuer/src/cache.rs#L258) | Local | `tests::rejects_noncovering_but_retains_oversized_callback_results::unexpected_callback_count` | — |
| [259](feuer/src/cache.rs#L259) | Local | `tests::rejects_noncovering_but_retains_oversized_callback_results::count` | — |
| [260](feuer/src/cache.rs#L260) | Local | `tests::rejects_noncovering_but_retains_oversized_callback_results::result` | — |
| [273](feuer/src/cache.rs#L273) | Local | `tests::a_racing_contained_download_is_discarded_but_returns_its_own_bytes::cache` | — |
| [274](feuer/src/cache.rs#L274) | Local | `tests::a_racing_contained_download_is_discarded_but_returns_its_own_bytes::key` | — |
| [275](feuer/src/cache.rs#L275) | Local | `tests::a_racing_contained_download_is_discarded_but_returns_its_own_bytes::callback_entered` | — |
| [276](feuer/src/cache.rs#L276) | Local | `tests::a_racing_contained_download_is_discarded_but_returns_its_own_bytes::release_callback` | — |
| [278](feuer/src/cache.rs#L278) | Local | `tests::a_racing_contained_download_is_discarded_but_returns_its_own_bytes::pending` | — |
| [279](feuer/src/cache.rs#L279) | Local | `tests::a_racing_contained_download_is_discarded_but_returns_its_own_bytes::pending::cache` | — |
| [280](feuer/src/cache.rs#L280) | Local | `tests::a_racing_contained_download_is_discarded_but_returns_its_own_bytes::pending::key` | — |
| [281](feuer/src/cache.rs#L281) | Local | `tests::a_racing_contained_download_is_discarded_but_returns_its_own_bytes::pending::callback_entered` | — |
| [282](feuer/src/cache.rs#L282) | Local | `tests::a_racing_contained_download_is_discarded_but_returns_its_own_bytes::pending::release_callback` | — |
| [305](feuer/src/cache.rs#L305) | Local | `tests::a_racing_contained_download_is_discarded_but_returns_its_own_bytes::unexpected_callback_count` | — |
| [306](feuer/src/cache.rs#L306) | Local | `tests::a_racing_contained_download_is_discarded_but_returns_its_own_bytes::count` | — |
| [307](feuer/src/cache.rs#L307) | Local | `tests::a_racing_contained_download_is_discarded_but_returns_its_own_bytes::cached` | — |

</details>

### `feuer/src/config.rs`

| Line | Kind | Name / source parent | Signature or type |
| ---: | --- | --- | --- |
| [12](feuer/src/config.rs#L12) | Struct | `Config` | — |
| [13](feuer/src/config.rs#L13) | Field | `Config::directory` | `PathBuf` |
| [14](feuer/src/config.rs#L14) | Field | `Config::disk_capacity` | `u64` |
| [15](feuer/src/config.rs#L15) | Field | `Config::memory_capacity` | `u64` |
| [18](feuer/src/config.rs#L18) | Impl | `impl Config` | — |
| [20](feuer/src/config.rs#L20) | Function | `impl Config::new` | `fn(directory: impl Into<PathBuf>, disk_capacity: u64, memory_capacity: u64) -> Result<Self, ConfigError>` |
| [36](feuer/src/config.rs#L36) | Method | `impl Config::directory` | `fn(&self) -> &Path` |
| [41](feuer/src/config.rs#L41) | Method | `impl Config::disk_capacity` | `fn(&self) -> u64` |
| [46](feuer/src/config.rs#L46) | Method | `impl Config::memory_capacity` | `fn(&self) -> u64` |
| [53](feuer/src/config.rs#L53) | Enum | `ConfigError` | — |
| [56](feuer/src/config.rs#L56) | Variant | `ConfigError::InvalidDiskCapacity` | — |
| [59](feuer/src/config.rs#L59) | Variant | `ConfigError::InvalidMemoryCapacity` | — |
| [63](feuer/src/config.rs#L63) | Module | `tests` | — |
| [67](feuer/src/config.rs#L67) | Function | `tests::requires_both_capacity_roles_to_be_explicit` | `fn()` |
| [76](feuer/src/config.rs#L76) | Function | `tests::represents_tib_scale_capacity` | `fn()` |
| [85](feuer/src/config.rs#L85) | Function | `tests::rejects_zero_capacities` | `fn()` |

<details>
<summary>Local bindings (3)</summary>

| Line | Kind | Name / source parent | Signature or type |
| ---: | --- | --- | --- |
| [68](feuer/src/config.rs#L68) | Local | `tests::requires_both_capacity_roles_to_be_explicit::config` | — |
| [77](feuer/src/config.rs#L77) | Local | `tests::represents_tib_scale_capacity::capacity` | — |
| [78](feuer/src/config.rs#L78) | Local | `tests::represents_tib_scale_capacity::config` | — |

</details>

### `feuer/src/lib.rs`

| Line | Kind | Name / source parent | Signature or type |
| ---: | --- | --- | --- |
| [8](feuer/src/lib.rs#L8) | Module | `cache` | — |
| [9](feuer/src/lib.rs#L9) | Module | `config` | — |

## feuer-memory

### `feuer-memory/src/lib.rs`

| Line | Kind | Name / source parent | Signature or type |
| ---: | --- | --- | --- |
| [8](feuer-memory/src/lib.rs#L8) | Module | `metrics` | — |
| [9](feuer-memory/src/lib.rs#L9) | Module | `store` | — |

### `feuer-memory/src/metrics.rs`

| Line | Kind | Name / source parent | Signature or type |
| ---: | --- | --- | --- |
| [9](feuer-memory/src/metrics.rs#L9) | Struct | `MemoryMetrics` | — |
| [10](feuer-memory/src/metrics.rs#L10) | Field | `MemoryMetrics::insert` | `BoxedCounter` |
| [11](feuer-memory/src/metrics.rs#L11) | Field | `MemoryMetrics::replace` | `BoxedCounter` |
| [12](feuer-memory/src/metrics.rs#L12) | Field | `MemoryMetrics::redundant` | `BoxedCounter` |
| [13](feuer-memory/src/metrics.rs#L13) | Field | `MemoryMetrics::access` | `BoxedCounter` |
| [14](feuer-memory/src/metrics.rs#L14) | Field | `MemoryMetrics::hit` | `BoxedCounter` |
| [15](feuer-memory/src/metrics.rs#L15) | Field | `MemoryMetrics::miss` | `BoxedCounter` |
| [16](feuer-memory/src/metrics.rs#L16) | Field | `MemoryMetrics::remove` | `BoxedCounter` |
| [17](feuer-memory/src/metrics.rs#L17) | Field | `MemoryMetrics::evict` | `BoxedCounter` |
| [18](feuer-memory/src/metrics.rs#L18) | Field | `MemoryMetrics::compact` | `BoxedCounter` |
| [19](feuer-memory/src/metrics.rs#L19) | Field | `MemoryMetrics::compacted_payload_bytes` | `BoxedCounter` |
| [20](feuer-memory/src/metrics.rs#L20) | Field | `MemoryMetrics::payload_bytes` | `BoxedGauge` |
| [21](feuer-memory/src/metrics.rs#L21) | Field | `MemoryMetrics::entries` | `BoxedGauge` |
| [24](feuer-memory/src/metrics.rs#L24) | Impl | `impl fmt::Debug for MemoryMetrics` | — |
| [25](feuer-memory/src/metrics.rs#L25) | Method | `impl fmt::Debug for MemoryMetrics::fmt` | `fn(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result` |
| [30](feuer-memory/src/metrics.rs#L30) | Impl | `impl MemoryMetrics` | — |
| [32](feuer-memory/src/metrics.rs#L32) | Function | `impl MemoryMetrics::new` | `fn(registry: &BoxedRegistry) -> Arc<Self>` |
| [71](feuer-memory/src/metrics.rs#L71) | Method | `impl MemoryMetrics::record_insert` | `fn(&self, replaced: bool)` |
| [79](feuer-memory/src/metrics.rs#L79) | Method | `impl MemoryMetrics::record_redundant` | `fn(&self)` |
| [83](feuer-memory/src/metrics.rs#L83) | Method | `impl MemoryMetrics::record_access` | `fn(&self)` |
| [87](feuer-memory/src/metrics.rs#L87) | Method | `impl MemoryMetrics::record_lookup` | `fn(&self, hit: bool)` |
| [95](feuer-memory/src/metrics.rs#L95) | Method | `impl MemoryMetrics::record_remove` | `fn(&self)` |
| [99](feuer-memory/src/metrics.rs#L99) | Method | `impl MemoryMetrics::record_evictions` | `fn(&self, count: u64)` |
| [103](feuer-memory/src/metrics.rs#L103) | Method | `impl MemoryMetrics::record_compaction` | `fn(&self, reclaimed_bytes: u64)` |
| [108](feuer-memory/src/metrics.rs#L108) | Method | `impl MemoryMetrics::increase_usage` | `fn(&self, bytes: u64, entries: u64)` |
| [113](feuer-memory/src/metrics.rs#L113) | Method | `impl MemoryMetrics::decrease_usage` | `fn(&self, bytes: u64, entries: u64)` |
| [118](feuer-memory/src/metrics.rs#L118) | Function | `impl MemoryMetrics::noop` | `fn() -> Arc<Self>` |
| [125](feuer-memory/src/metrics.rs#L125) | Module | `tests` | — |
| [129](feuer-memory/src/metrics.rs#L129) | Function | `tests::registers_and_updates_through_the_normal_registry_boundary` | `fn()` |

<details>
<summary>Local bindings (7)</summary>

| Line | Kind | Name / source parent | Signature or type |
| ---: | --- | --- | --- |
| [33](feuer-memory/src/metrics.rs#L33) | Local | `impl MemoryMetrics::new::operations` | — |
| [38](feuer-memory/src/metrics.rs#L38) | Local | `impl MemoryMetrics::new::compacted_payload_bytes` | — |
| [43](feuer-memory/src/metrics.rs#L43) | Local | `impl MemoryMetrics::new::payload_bytes` | — |
| [48](feuer-memory/src/metrics.rs#L48) | Local | `impl MemoryMetrics::new::entries` | — |
| [53](feuer-memory/src/metrics.rs#L53) | Local | `impl MemoryMetrics::new::operation` | — |
| [119](feuer-memory/src/metrics.rs#L119) | Local | `impl MemoryMetrics::noop::registry` | `BoxedRegistry` |
| [130](feuer-memory/src/metrics.rs#L130) | Local | `tests::registers_and_updates_through_the_normal_registry_boundary::metrics` | — |

</details>

### `feuer-memory/src/store/access_history.rs`

| Line | Kind | Name / source parent | Signature or type |
| ---: | --- | --- | --- |
| [6](feuer-memory/src/store/access_history.rs#L6) | Const | `FIXED_RETRIEVAL_EQUIVALENT_BYTES` | `u64` |
| [8](feuer-memory/src/store/access_history.rs#L8) | Const | `MAX_ACCESS_EVENTS_PER_KEY` | `usize` |
| [10](feuer-memory/src/store/access_history.rs#L10) | Const | `MAX_ACCESS_AGE_ACCESSES` | `u64` |
| [14](feuer-memory/src/store/access_history.rs#L14) | Struct | `AccessEvent` | — |
| [15](feuer-memory/src/store/access_history.rs#L15) | Field | `AccessEvent::range` | `ByteRange` |
| [16](feuer-memory/src/store/access_history.rs#L16) | Field | `AccessEvent::observed_at` | `u64` |
| [24](feuer-memory/src/store/access_history.rs#L24) | Struct | `AccessHistory` | — |
| [25](feuer-memory/src/store/access_history.rs#L25) | Field | `AccessHistory::events` | `VecDeque<AccessEvent>` |
| [28](feuer-memory/src/store/access_history.rs#L28) | Impl | `impl AccessHistory` | — |
| [29](feuer-memory/src/store/access_history.rs#L29) | Method | `impl AccessHistory::record` | `fn(&mut self, range: ByteRange, access_clock: u64)` |
| [41](feuer-memory/src/store/access_history.rs#L41) | Method | `impl AccessHistory::active_ranges` | `fn(&self, access_clock: u64) -> impl Iterator<Item = ByteRange> + '_` |
| [49](feuer-memory/src/store/access_history.rs#L49) | Method | `impl AccessHistory::retention_value` | `fn(&self, cached_range: ByteRange, access_clock: u64) -> u64` |
| [57](feuer-memory/src/store/access_history.rs#L57) | Method | `impl AccessHistory::expire` | `fn(&mut self, access_clock: u64)` |
| [68](feuer-memory/src/store/access_history.rs#L68) | Method | `impl AccessHistory::ranges` | `fn(&self) -> Vec<ByteRange>` |
| [73](feuer-memory/src/store/access_history.rs#L73) | Method | `impl AccessHistory::len` | `fn(&self) -> usize` |
| [78](feuer-memory/src/store/access_history.rs#L78) | Function | `is_active` | `fn(event: AccessEvent, access_clock: u64) -> bool` |
| [83](feuer-memory/src/store/access_history.rs#L83) | Module | `tests` | — |
| [86](feuer-memory/src/store/access_history.rs#L86) | Function | `tests::range` | `fn(start: u64, end: u64) -> ByteRange` |
| [91](feuer-memory/src/store/access_history.rs#L91) | Function | `tests::bounds_events_without_coalescing_repeated_ranges` | `fn()` |
| [109](feuer-memory/src/store/access_history.rs#L109) | Function | `tests::retains_full_retrieval_value_until_expiration` | `fn()` |
| [126](feuer-memory/src/store/access_history.rs#L126) | Function | `tests::credits_only_cached_ranges_covering_the_exact_request` | `fn()` |

<details>
<summary>Local bindings (8)</summary>

| Line | Kind | Name / source parent | Signature or type |
| ---: | --- | --- | --- |
| [92](feuer-memory/src/store/access_history.rs#L92) | Local | `tests::bounds_events_without_coalescing_repeated_ranges::repeated` | — |
| [93](feuer-memory/src/store/access_history.rs#L93) | Local | `tests::bounds_events_without_coalescing_repeated_ranges::mut history` | — |
| [95](feuer-memory/src/store/access_history.rs#L95) | Local | `tests::bounds_events_without_coalescing_repeated_ranges::requested` | — |
| [110](feuer-memory/src/store/access_history.rs#L110) | Local | `tests::retains_full_retrieval_value_until_expiration::requested` | — |
| [111](feuer-memory/src/store/access_history.rs#L111) | Local | `tests::retains_full_retrieval_value_until_expiration::cached_range` | — |
| [112](feuer-memory/src/store/access_history.rs#L112) | Local | `tests::retains_full_retrieval_value_until_expiration::mut history` | — |
| [116](feuer-memory/src/store/access_history.rs#L116) | Local | `tests::retains_full_retrieval_value_until_expiration::expected` | — |
| [127](feuer-memory/src/store/access_history.rs#L127) | Local | `tests::credits_only_cached_ranges_covering_the_exact_request::mut history` | — |

</details>

### `feuer-memory/src/store/compaction.rs`

| Line | Kind | Name / source parent | Signature or type |
| ---: | --- | --- | --- |
| [4](feuer-memory/src/store/compaction.rs#L4) | Const | `MIN_RECLAIM_DIVISOR` | `u64` |
| [8](feuer-memory/src/store/compaction.rs#L8) | Struct | `CompactionPlan` | — |
| [9](feuer-memory/src/store/compaction.rs#L9) | Field | `CompactionPlan::source` | `ByteRange` |
| [10](feuer-memory/src/store/compaction.rs#L10) | Field | `CompactionPlan::retained` | `Vec<ByteRange>` |
| [11](feuer-memory/src/store/compaction.rs#L11) | Field | `CompactionPlan::retained_bytes` | `u64` |
| [14](feuer-memory/src/store/compaction.rs#L14) | Impl | `impl CompactionPlan` | — |
| [15](feuer-memory/src/store/compaction.rs#L15) | Method | `impl CompactionPlan::source` | `fn(&self) -> ByteRange` |
| [19](feuer-memory/src/store/compaction.rs#L19) | Method | `impl CompactionPlan::retained` | `fn(&self) -> &[ByteRange]` |
| [23](feuer-memory/src/store/compaction.rs#L23) | Method | `impl CompactionPlan::reclaimed_bytes` | `fn(&self) -> u64` |
| [34](feuer-memory/src/store/compaction.rs#L34) | Function | `plan_compaction` | `fn( source: ByteRange, requested_ranges: impl IntoIterator<Item = ByteRange>, ) -> Option<CompactionPlan>` |
| [73](feuer-memory/src/store/compaction.rs#L73) | Module | `tests` | — |
| [76](feuer-memory/src/store/compaction.rs#L76) | Function | `tests::range` | `fn(start: u64, end: u64) -> ByteRange` |
| [81](feuer-memory/src/store/compaction.rs#L81) | Function | `tests::projects_and_groups_only_exact_requests_covered_by_the_source` | `fn()` |
| [101](feuer-memory/src/store/compaction.rs#L101) | Function | `tests::merges_adjacent_requests_so_each_original_request_stays_coverable` | `fn()` |
| [108](feuer-memory/src/store/compaction.rs#L108) | Function | `tests::skips_empty_or_low_savings_plans` | `fn()` |

<details>
<summary>Local bindings (6)</summary>

| Line | Kind | Name / source parent | Signature or type |
| ---: | --- | --- | --- |
| [38](feuer-memory/src/store/compaction.rs#L38) | Local | `plan_compaction::mut retained` | `Vec<_>` |
| [44](feuer-memory/src/store/compaction.rs#L44) | Local | `plan_compaction::mut grouped` | `Vec<ByteRange>` |
| [59](feuer-memory/src/store/compaction.rs#L59) | Local | `plan_compaction::retained_bytes` | — |
| [60](feuer-memory/src/store/compaction.rs#L60) | Local | `plan_compaction::reclaimed_bytes` | — |
| [82](feuer-memory/src/store/compaction.rs#L82) | Local | `tests::projects_and_groups_only_exact_requests_covered_by_the_source::plan` | — |
| [102](feuer-memory/src/store/compaction.rs#L102) | Local | `tests::merges_adjacent_requests_so_each_original_request_stays_coverable::plan` | — |

</details>

### `feuer-memory/src/store/shard.rs`

| Line | Kind | Name / source parent | Signature or type |
| ---: | --- | --- | --- |
| [14](feuer-memory/src/store/shard.rs#L14) | Const | `COMPACTION_GRACE_ACCESSES` | `u64` |
| [16](feuer-memory/src/store/shard.rs#L16) | Const | `PRESSURE_SAMPLE_SIZE` | `usize` |
| [19](feuer-memory/src/store/shard.rs#L19) | Struct | `CachedRange` | — |
| [21](feuer-memory/src/store/shard.rs#L21) | Field | `CachedRange::id` | `u64` |
| [23](feuer-memory/src/store/shard.rs#L23) | Field | `CachedRange::range` | `ByteRange` |
| [25](feuer-memory/src/store/shard.rs#L25) | Field | `CachedRange::bytes` | `Bytes` |
| [27](feuer-memory/src/store/shard.rs#L27) | Field | `CachedRange::candidate_slot` | `usize` |
| [29](feuer-memory/src/store/shard.rs#L29) | Field | `CachedRange::admitted_at` | `u64` |
| [32](feuer-memory/src/store/shard.rs#L32) | Impl | `impl CachedRange` | — |
| [33](feuer-memory/src/store/shard.rs#L33) | Method | `impl CachedRange::requested_bytes` | `fn(&self, requested_range: ByteRange) -> Bytes` |
| [49](feuer-memory/src/store/shard.rs#L49) | Struct | `ObjectCachedRanges` | — |
| [51](feuer-memory/src/store/shard.rs#L51) | Field | `ObjectCachedRanges::by_start` | `BTreeMap<u64, CachedRange>` |
| [53](feuer-memory/src/store/shard.rs#L53) | Field | `ObjectCachedRanges::accesses` | `AccessHistory` |
| [55](feuer-memory/src/store/shard.rs#L55) | Field | `ObjectCachedRanges::generation` | `u64` |
| [58](feuer-memory/src/store/shard.rs#L58) | Impl | `impl ObjectCachedRanges` | — |
| [59](feuer-memory/src/store/shard.rs#L59) | Method | `impl ObjectCachedRanges::covering` | `fn(&self, range: ByteRange) -> Option<&CachedRange>` |
| [64](feuer-memory/src/store/shard.rs#L64) | Method | `impl ObjectCachedRanges::observe_covering` | `fn<R>( &mut self, requested: ByteRange, access_clock: u64, project: impl FnOnce(&CachedRange) -> R, ) -> Option<R>` |
| [82](feuer-memory/src/store/shard.rs#L82) | Method | `impl ObjectCachedRanges::superseded_by` | `fn(&self, range: ByteRange) -> SupersededRanges` |
| [96](feuer-memory/src/store/shard.rs#L96) | Struct | `SupersededRanges` | — |
| [97](feuer-memory/src/store/shard.rs#L97) | Field | `SupersededRanges::ranges` | `Vec<ByteRange>` |
| [98](feuer-memory/src/store/shard.rs#L98) | Field | `SupersededRanges::bytes` | `u64` |
| [103](feuer-memory/src/store/shard.rs#L103) | Struct | `RemovedUsage` | — |
| [104](feuer-memory/src/store/shard.rs#L104) | Field | `RemovedUsage::bytes` | `u64` |
| [105](feuer-memory/src/store/shard.rs#L105) | Field | `RemovedUsage::entries` | `u64` |
| [110](feuer-memory/src/store/shard.rs#L110) | Struct | `CachedRangeIdentity` | — |
| [111](feuer-memory/src/store/shard.rs#L111) | Field | `CachedRangeIdentity::object_key` | `ObjectKey` |
| [112](feuer-memory/src/store/shard.rs#L112) | Field | `CachedRangeIdentity::start` | `u64` |
| [113](feuer-memory/src/store/shard.rs#L113) | Field | `CachedRangeIdentity::id` | `u64` |
| [118](feuer-memory/src/store/shard.rs#L118) | Struct | `PressureCandidates` | — |
| [119](feuer-memory/src/store/shard.rs#L119) | Field | `PressureCandidates::entries` | `Vec<CachedRangeIdentity>` |
| [120](feuer-memory/src/store/shard.rs#L120) | Field | `PressureCandidates::cursor` | `usize` |
| [123](feuer-memory/src/store/shard.rs#L123) | Impl | `impl PressureCandidates` | — |
| [124](feuer-memory/src/store/shard.rs#L124) | Method | `impl PressureCandidates::register` | `fn(&mut self, candidate: CachedRangeIdentity) -> usize` |
| [131](feuer-memory/src/store/shard.rs#L131) | Method | `impl PressureCandidates::remove` | `fn(&mut self, slot: usize, expected_id: u64) -> Option<CachedRangeIdentity>` |
| [144](feuer-memory/src/store/shard.rs#L144) | Method | `impl PressureCandidates::sample` | `fn(&mut self) -> (usize, usize)` |
| [156](feuer-memory/src/store/shard.rs#L156) | Struct | `PressureCandidate` | — |
| [157](feuer-memory/src/store/shard.rs#L157) | Field | `PressureCandidate::object_key` | `ObjectKey` |
| [158](feuer-memory/src/store/shard.rs#L158) | Field | `PressureCandidate::range` | `ByteRange` |
| [159](feuer-memory/src/store/shard.rs#L159) | Field | `PressureCandidate::id` | `u64` |
| [160](feuer-memory/src/store/shard.rs#L160) | Field | `PressureCandidate::retained_bytes` | `u64` |
| [161](feuer-memory/src/store/shard.rs#L161) | Field | `PressureCandidate::retrieval_value` | `u64` |
| [165](feuer-memory/src/store/shard.rs#L165) | Struct | `CompactionSource` | — |
| [166](feuer-memory/src/store/shard.rs#L166) | Field | `CompactionSource::object_key` | `ObjectKey` |
| [167](feuer-memory/src/store/shard.rs#L167) | Field | `CompactionSource::start` | `u64` |
| [168](feuer-memory/src/store/shard.rs#L168) | Field | `CompactionSource::id` | `u64` |
| [169](feuer-memory/src/store/shard.rs#L169) | Field | `CompactionSource::generation` | `u64` |
| [170](feuer-memory/src/store/shard.rs#L170) | Field | `CompactionSource::plan` | `CompactionPlan` |
| [171](feuer-memory/src/store/shard.rs#L171) | Field | `CompactionSource::source_bytes` | `Bytes` |
| [174](feuer-memory/src/store/shard.rs#L174) | Impl | `impl CompactionSource` | — |
| [176](feuer-memory/src/store/shard.rs#L176) | Method | `impl CompactionSource::copy_payload` | `fn(self) -> CompactionReplacement` |
| [201](feuer-memory/src/store/shard.rs#L201) | Struct | `CompactionReplacement` | — |
| [202](feuer-memory/src/store/shard.rs#L202) | Field | `CompactionReplacement::object_key` | `ObjectKey` |
| [203](feuer-memory/src/store/shard.rs#L203) | Field | `CompactionReplacement::start` | `u64` |
| [204](feuer-memory/src/store/shard.rs#L204) | Field | `CompactionReplacement::id` | `u64` |
| [205](feuer-memory/src/store/shard.rs#L205) | Field | `CompactionReplacement::generation` | `u64` |
| [206](feuer-memory/src/store/shard.rs#L206) | Field | `CompactionReplacement::plan` | `CompactionPlan` |
| [207](feuer-memory/src/store/shard.rs#L207) | Field | `CompactionReplacement::retained` | `Vec<(ByteRange, Bytes)>` |
| [211](feuer-memory/src/store/shard.rs#L211) | Enum | `AdmissionStep` | — |
| [212](feuer-memory/src/store/shard.rs#L212) | Variant | `AdmissionStep::Complete` | — |
| [213](feuer-memory/src/store/shard.rs#L213) | Variant | `AdmissionStep::Retry` | — |
| [214](feuer-memory/src/store/shard.rs#L214) | Variant | `AdmissionStep::Compact` | — |
| [218](feuer-memory/src/store/shard.rs#L218) | Struct | `MemoryShard` | — |
| [219](feuer-memory/src/store/shard.rs#L219) | Field | `MemoryShard::capacity` | `u64` |
| [220](feuer-memory/src/store/shard.rs#L220) | Field | `MemoryShard::used_bytes` | `u64` |
| [221](feuer-memory/src/store/shard.rs#L221) | Field | `MemoryShard::ranges` | `FxHashMap<ObjectKey, ObjectCachedRanges>` |
| [222](feuer-memory/src/store/shard.rs#L222) | Field | `MemoryShard::access_clock` | `u64` |
| [223](feuer-memory/src/store/shard.rs#L223) | Field | `MemoryShard::next_entry_id` | `u64` |
| [224](feuer-memory/src/store/shard.rs#L224) | Field | `MemoryShard::candidates` | `PressureCandidates` |
| [225](feuer-memory/src/store/shard.rs#L225) | Field | `MemoryShard::metrics` | `Arc<MemoryMetrics>` |
| [228](feuer-memory/src/store/shard.rs#L228) | Impl | `impl MemoryShard` | — |
| [229](feuer-memory/src/store/shard.rs#L229) | Function | `impl MemoryShard::new` | `fn(capacity: u64, metrics: Arc<MemoryMetrics>) -> Self` |
| [241](feuer-memory/src/store/shard.rs#L241) | Method | `impl MemoryShard::used_bytes` | `fn(&self) -> u64` |
| [245](feuer-memory/src/store/shard.rs#L245) | Method | `impl MemoryShard::get` | `fn(&mut self, object_key: &ObjectKey, requested_range: ByteRange) -> Option<Bytes>` |
| [263](feuer-memory/src/store/shard.rs#L263) | Method | `impl MemoryShard::record_access` | `fn(&mut self, object_key: &ObjectKey, requested_range: ByteRange)` |
| [267](feuer-memory/src/store/shard.rs#L267) | Method | `impl MemoryShard::record_successful_access` | `fn(&mut self, object_key: &ObjectKey, requested_range: ByteRange)` |
| [276](feuer-memory/src/store/shard.rs#L276) | Method | `impl MemoryShard::admission_step` | `fn( &mut self, object_key: &ObjectKey, range: ByteRange, bytes: &Bytes, requested_range: Option<ByteRange>, allow_compaction: bool, ) -> AdmissionStep` |
| [338](feuer-memory/src/store/shard.rs#L338) | Method | `impl MemoryShard::insert_admission` | `fn(&mut self, object_key: ObjectKey, range: ByteRange, bytes: Bytes)` |
| [360](feuer-memory/src/store/shard.rs#L360) | Method | `impl MemoryShard::insert_compacted` | `fn(&mut self, object_key: &ObjectKey, range: ByteRange, bytes: Bytes)` |
| [380](feuer-memory/src/store/shard.rs#L380) | Method | `impl MemoryShard::allocate_entry_id` | `fn(&mut self) -> u64` |
| [388](feuer-memory/src/store/shard.rs#L388) | Method | `impl MemoryShard::remove_superseded` | `fn(&mut self, object_key: &ObjectKey, ranges: &[ByteRange]) -> RemovedUsage` |
| [400](feuer-memory/src/store/shard.rs#L400) | Method | `impl MemoryShard::remove` | `fn(&mut self, object_key: &ObjectKey, range: ByteRange) -> bool` |
| [409](feuer-memory/src/store/shard.rs#L409) | Method | `impl MemoryShard::detach_entry` | `fn( &mut self, object_key: &ObjectKey, range: ByteRange, expected_id: Option<u64>, preserve_access: bool, ) -> Option<u64>` |
| [440](feuer-memory/src/store/shard.rs#L440) | Method | `impl MemoryShard::unregister_candidate` | `fn(&mut self, slot: usize, expected_id: u64)` |
| [455](feuer-memory/src/store/shard.rs#L455) | Method | `impl MemoryShard::pressure_candidate` | `fn( &mut self, admitting_key: &ObjectKey, admitting_range: ByteRange, ) -> Option<PressureCandidate>` |
| [497](feuer-memory/src/store/shard.rs#L497) | Method | `impl MemoryShard::compaction_source` | `fn(&self, candidate: &PressureCandidate) -> Option<CompactionSource>` |
| [522](feuer-memory/src/store/shard.rs#L522) | Method | `impl MemoryShard::publish_compaction` | `fn(&mut self, replacement: CompactionReplacement) -> bool` |
| [570](feuer-memory/src/store/shard.rs#L570) | Method | `impl MemoryShard::entry_count` | `fn(&self) -> usize` |
| [575](feuer-memory/src/store/shard.rs#L575) | Method | `impl MemoryShard::accessed_ranges` | `fn(&self, object_key: &ObjectKey) -> Vec<ByteRange>` |
| [582](feuer-memory/src/store/shard.rs#L582) | Method | `impl MemoryShard::access_history_len` | `fn(&self, object_key: &ObjectKey) -> usize` |
| [587](feuer-memory/src/store/shard.rs#L587) | Method | `impl MemoryShard::candidate_count` | `fn(&self) -> usize` |
| [593](feuer-memory/src/store/shard.rs#L593) | Function | `compare_retention` | `fn(left: &PressureCandidate, right: &PressureCandidate) -> Ordering` |
| [605](feuer-memory/src/store/shard.rs#L605) | Function | `compare_value_density` | `fn(left: u64, left_bytes: u64, right: u64, right_bytes: u64) -> Ordering` |
| [609](feuer-memory/src/store/shard.rs#L609) | Impl | `impl Drop for MemoryShard` | — |
| [610](feuer-memory/src/store/shard.rs#L610) | Method | `impl Drop for MemoryShard::drop` | `fn(&mut self)` |

<details>
<summary>Local bindings (62)</summary>

| Line | Kind | Name / source parent | Signature or type |
| ---: | --- | --- | --- |
| [35](feuer-memory/src/store/shard.rs#L35) | Local | `impl CachedRange::requested_bytes::start` | — |
| [37](feuer-memory/src/store/shard.rs#L37) | Local | `impl CachedRange::requested_bytes::end` | — |
| [60](feuer-memory/src/store/shard.rs#L60) | Local | `impl ObjectCachedRanges::covering::(_, entry)` | — |
| [70](feuer-memory/src/store/shard.rs#L70) | Local | `impl ObjectCachedRanges::observe_covering::projected` | — |
| [71](feuer-memory/src/store/shard.rs#L71) | Local | `impl ObjectCachedRanges::observe_covering::projected::(_, entry)` | — |
| [83](feuer-memory/src/store/shard.rs#L83) | Local | `impl ObjectCachedRanges::superseded_by::mut superseded` | — |
| [125](feuer-memory/src/store/shard.rs#L125) | Local | `impl PressureCandidates::register::slot` | — |
| [133](feuer-memory/src/store/shard.rs#L133) | Local | `impl PressureCandidates::remove::last` | — |
| [135](feuer-memory/src/store/shard.rs#L135) | Local | `impl PressureCandidates::remove::moved` | — |
| [145](feuer-memory/src/store/shard.rs#L145) | Local | `impl PressureCandidates::sample::count` | — |
| [149](feuer-memory/src/store/shard.rs#L149) | Local | `impl PressureCandidates::sample::start` | — |
| [177](feuer-memory/src/store/shard.rs#L177) | Local | `impl CompactionSource::copy_payload::retained` | — |
| [182](feuer-memory/src/store/shard.rs#L182) | Local | `impl CompactionSource::copy_payload::retained::start` | — |
| [184](feuer-memory/src/store/shard.rs#L184) | Local | `impl CompactionSource::copy_payload::retained::end` | — |
| [246](feuer-memory/src/store/shard.rs#L246) | Local | `impl MemoryShard::get::access_clock` | — |
| [247](feuer-memory/src/store/shard.rs#L247) | Local | `impl MemoryShard::get::accessed` | — |
| [252](feuer-memory/src/store/shard.rs#L252) | Local | `impl MemoryShard::get::Some(bytes)` | — |
| [284](feuer-memory/src/store/shard.rs#L284) | Local | `impl MemoryShard::admission_step::superseded` | — |
| [295](feuer-memory/src/store/shard.rs#L295) | Local | `impl MemoryShard::admission_step::added_bytes` | — |
| [296](feuer-memory/src/store/shard.rs#L296) | Local | `impl MemoryShard::admission_step::effective_used` | — |
| [297](feuer-memory/src/store/shard.rs#L297) | Local | `impl MemoryShard::admission_step::target` | — |
| [300](feuer-memory/src/store/shard.rs#L300) | Local | `impl MemoryShard::admission_step::removal` | — |
| [315](feuer-memory/src/store/shard.rs#L315) | Local | `impl MemoryShard::admission_step::Some(candidate)` | — |
| [325](feuer-memory/src/store/shard.rs#L325) | Local | `impl MemoryShard::admission_step::removed` | — |
| [340](feuer-memory/src/store/shard.rs#L340) | Local | `impl MemoryShard::insert_admission::id` | — |
| [341](feuer-memory/src/store/shard.rs#L341) | Local | `impl MemoryShard::insert_admission::candidate_slot` | — |
| [346](feuer-memory/src/store/shard.rs#L346) | Local | `impl MemoryShard::insert_admission::entry` | — |
| [354](feuer-memory/src/store/shard.rs#L354) | Local | `impl MemoryShard::insert_admission::entries` | — |
| [356](feuer-memory/src/store/shard.rs#L356) | Local | `impl MemoryShard::insert_admission::replaced` | — |
| [361](feuer-memory/src/store/shard.rs#L361) | Local | `impl MemoryShard::insert_compacted::id` | — |
| [362](feuer-memory/src/store/shard.rs#L362) | Local | `impl MemoryShard::insert_compacted::candidate_slot` | — |
| [367](feuer-memory/src/store/shard.rs#L367) | Local | `impl MemoryShard::insert_compacted::entry` | — |
| [374](feuer-memory/src/store/shard.rs#L374) | Local | `impl MemoryShard::insert_compacted::entries` | — |
| [376](feuer-memory/src/store/shard.rs#L376) | Local | `impl MemoryShard::insert_compacted::replaced` | — |
| [389](feuer-memory/src/store/shard.rs#L389) | Local | `impl MemoryShard::remove_superseded::mut removal` | — |
| [391](feuer-memory/src/store/shard.rs#L391) | Local | `impl MemoryShard::remove_superseded::bytes` | — |
| [401](feuer-memory/src/store/shard.rs#L401) | Local | `impl MemoryShard::remove::Some(bytes)` | — |
| [416](feuer-memory/src/store/shard.rs#L416) | Local | `impl MemoryShard::detach_entry::(entry, object_is_empty)` | — |
| [417](feuer-memory/src/store/shard.rs#L417) | Local | `impl MemoryShard::detach_entry::(entry, object_is_empty)::entries` | — |
| [418](feuer-memory/src/store/shard.rs#L418) | Local | `impl MemoryShard::detach_entry::(entry, object_is_empty)::current` | — |
| [422](feuer-memory/src/store/shard.rs#L422) | Local | `impl MemoryShard::detach_entry::(entry, object_is_empty)::entry` | — |
| [427](feuer-memory/src/store/shard.rs#L427) | Local | `impl MemoryShard::detach_entry::(entry, object_is_empty)::object_is_empty` | — |
| [435](feuer-memory/src/store/shard.rs#L435) | Local | `impl MemoryShard::detach_entry::bytes` | — |
| [441](feuer-memory/src/store/shard.rs#L441) | Local | `impl MemoryShard::unregister_candidate::moved` | — |
| [442](feuer-memory/src/store/shard.rs#L442) | Local | `impl MemoryShard::unregister_candidate::Some(moved)` | — |
| [445](feuer-memory/src/store/shard.rs#L445) | Local | `impl MemoryShard::unregister_candidate::entry` | — |
| [460](feuer-memory/src/store/shard.rs#L460) | Local | `impl MemoryShard::pressure_candidate::(sample_start, sample_count)` | — |
| [461](feuer-memory/src/store/shard.rs#L461) | Local | `impl MemoryShard::pressure_candidate::candidate_count` | — |
| [462](feuer-memory/src/store/shard.rs#L462) | Local | `impl MemoryShard::pressure_candidate::mut selected` | `Option<PressureCandidate>` |
| [465](feuer-memory/src/store/shard.rs#L465) | Local | `impl MemoryShard::pressure_candidate::candidate` | — |
| [466](feuer-memory/src/store/shard.rs#L466) | Local | `impl MemoryShard::pressure_candidate::entries` | — |
| [470](feuer-memory/src/store/shard.rs#L470) | Local | `impl MemoryShard::pressure_candidate::entry` | — |
| [479](feuer-memory/src/store/shard.rs#L479) | Local | `impl MemoryShard::pressure_candidate::sampled` | — |
| [498](feuer-memory/src/store/shard.rs#L498) | Local | `impl MemoryShard::compaction_source::entries` | — |
| [502](feuer-memory/src/store/shard.rs#L502) | Local | `impl MemoryShard::compaction_source::entry` | — |
| [510](feuer-memory/src/store/shard.rs#L510) | Local | `impl MemoryShard::compaction_source::plan` | — |
| [523](feuer-memory/src/store/shard.rs#L523) | Local | `impl MemoryShard::publish_compaction::valid` | — |
| [534](feuer-memory/src/store/shard.rs#L534) | Local | `impl MemoryShard::publish_compaction::removed_bytes` | — |
| [542](feuer-memory/src/store/shard.rs#L542) | Local | `impl MemoryShard::publish_compaction::mut retained_bytes` | — |
| [543](feuer-memory/src/store/shard.rs#L543) | Local | `impl MemoryShard::publish_compaction::mut retained_entries` | — |
| [560](feuer-memory/src/store/shard.rs#L560) | Local | `impl MemoryShard::publish_compaction::reclaimed` | — |
| [611](feuer-memory/src/store/shard.rs#L611) | Local | `impl Drop for MemoryShard::drop::entries` | — |

</details>

### `feuer-memory/src/store/tests.rs`

| Line | Kind | Name / source parent | Signature or type |
| ---: | --- | --- | --- |
| [14](feuer-memory/src/store/tests.rs#L14) | Function | `range` | `fn(start: u64, end: u64) -> ByteRange` |
| [18](feuer-memory/src/store/tests.rs#L18) | Function | `download` | `fn(expected_range: ByteRange, bytes: Bytes) -> Download` |
| [24](feuer-memory/src/store/tests.rs#L24) | Function | `populate` | `fn(cache: &MemoryCache, object_key: ObjectKey, download: Download)` |
| [28](feuer-memory/src/store/tests.rs#L28) | Function | `cache` | `fn(capacity: u64) -> MemoryCache` |
| [32](feuer-memory/src/store/tests.rs#L32) | Function | `accessed_ranges` | `fn(cache: &MemoryCache, key: &ObjectKey) -> Vec<ByteRange>` |
| [36](feuer-memory/src/store/tests.rs#L36) | Function | `access_history_len` | `fn(cache: &MemoryCache, key: &ObjectKey) -> usize` |
| [40](feuer-memory/src/store/tests.rs#L40) | Function | `candidate_count` | `fn(cache: &MemoryCache) -> usize` |
| [45](feuer-memory/src/store/tests.rs#L45) | Function | `covering_lookup_returns_only_requested_bytes_and_shares_the_allocation` | `fn()` |
| [64](feuer-memory/src/store/tests.rs#L64) | Function | `different_identity_or_noncovering_ranges_miss` | `fn()` |
| [76](feuer-memory/src/store/tests.rs#L76) | Function | `adjacent_entries_are_not_assembled_into_a_hit` | `fn()` |
| [86](feuer-memory/src/store/tests.rs#L86) | Function | `population_and_accesses_are_independent` | `fn()` |
| [111](feuer-memory/src/store/tests.rs#L111) | Function | `shared_download_population_is_deduplicated_but_each_waiter_records_an_access` | `fn()` |
| [129](feuer-memory/src/store/tests.rs#L129) | Function | `partially_overlapping_downloads_coexist_and_do_not_form_a_hit` | `fn()` |
| [150](feuer-memory/src/store/tests.rs#L150) | Function | `a_larger_download_replaces_contained_entries_but_not_partial_overlaps` | `fn()` |
| [177](feuer-memory/src/store/tests.rs#L177) | Function | `a_contained_download_is_discarded_without_replacing_cached_bytes` | `fn()` |
| [197](feuer-memory/src/store/tests.rs#L197) | Function | `capacity_is_charged_by_retained_download_payload_bytes` | `fn()` |
| [213](feuer-memory/src/store/tests.rs#L213) | Function | `repeated_redundant_insertions_do_not_change_usage_or_replace_data` | `fn()` |
| [227](feuer-memory/src/store/tests.rs#L227) | Function | `oversized_insertion_empties_its_shard_and_remains_cached` | `fn()` |
| [241](feuer-memory/src/store/tests.rs#L241) | Function | `accessed_ranges_survive_downloaded_range_replacement` | `fn()` |
| [261](feuer-memory/src/store/tests.rs#L261) | Function | `access_history_survives_same_object_eviction_during_replacement` | `fn()` |
| [288](feuer-memory/src/store/tests.rs#L288) | Function | `access_history_is_bounded_and_preserves_repeated_exact_requests` | `fn()` |
| [305](feuer-memory/src/store/tests.rs#L305) | Function | `repeated_requested_intervals_are_retained_over_single_accesses` | `fn()` |
| [328](feuer-memory/src/store/tests.rs#L328) | Function | `requested_bytes_contribute_to_retrieval_value` | `fn()` |
| [356](feuer-memory/src/store/tests.rs#L356) | Function | `retention_credit_is_projected_only_onto_the_requested_interval` | `fn()` |
| [371](feuer-memory/src/store/tests.rs#L371) | Function | `stale_frequency_eventually_expires` | `fn()` |
| [398](feuer-memory/src/store/tests.rs#L398) | Function | `compaction_respects_grace_then_releases_unrequested_payload` | `fn()` |
| [439](feuer-memory/src/store/tests.rs#L439) | Function | `compaction_preserves_disjoint_requested_coverage_without_filling_gaps` | `fn()` |
| [466](feuer-memory/src/store/tests.rs#L466) | Function | `compaction_waits_for_pressure_and_adds_no_access` | `fn()` |
| [494](feuer-memory/src/store/tests.rs#L494) | Function | `candidate_state_tracks_entries_during_oversized_churn` | `fn()` |
| [507](feuer-memory/src/store/tests.rs#L507) | Function | `copied_compaction_is_revalidated_before_publication_and_can_fall_back` | `fn()` |
| [544](feuer-memory/src/store/tests.rs#L544) | Function | `removing_the_last_cached_range_releases_its_access_history` | `fn()` |
| [556](feuer-memory/src/store/tests.rs#L556) | Function | `zero_target_still_retains_the_latest_entry` | `fn()` |
| [569](feuer-memory/src/store/tests.rs#L569) | Function | `configured_target_is_divided_without_losing_remainder_bytes` | `fn()` |
| [581](feuer-memory/src/store/tests.rs#L581) | Function | `shard_targets_can_collectively_exceed_the_configured_capacity` | `fn()` |
| [604](feuer-memory/src/store/tests.rs#L604) | Function | `concurrent_shards_respect_their_targets_for_regular_entries` | `fn()` |

<details>
<summary>Local bindings (85)</summary>

| Line | Kind | Name / source parent | Signature or type |
| ---: | --- | --- | --- |
| [19](feuer-memory/src/store/tests.rs#L19) | Local | `download::download` | — |
| [46](feuer-memory/src/store/tests.rs#L46) | Local | `covering_lookup_returns_only_requested_bytes_and_shares_the_allocation::cache` | — |
| [47](feuer-memory/src/store/tests.rs#L47) | Local | `covering_lookup_returns_only_requested_bytes_and_shares_the_allocation::key` | — |
| [48](feuer-memory/src/store/tests.rs#L48) | Local | `covering_lookup_returns_only_requested_bytes_and_shares_the_allocation::value` | — |
| [51](feuer-memory/src/store/tests.rs#L51) | Local | `covering_lookup_returns_only_requested_bytes_and_shares_the_allocation::result` | — |
| [65](feuer-memory/src/store/tests.rs#L65) | Local | `different_identity_or_noncovering_ranges_miss::cache` | — |
| [66](feuer-memory/src/store/tests.rs#L66) | Local | `different_identity_or_noncovering_ranges_miss::key` | — |
| [77](feuer-memory/src/store/tests.rs#L77) | Local | `adjacent_entries_are_not_assembled_into_a_hit::cache` | — |
| [78](feuer-memory/src/store/tests.rs#L78) | Local | `adjacent_entries_are_not_assembled_into_a_hit::key` | — |
| [87](feuer-memory/src/store/tests.rs#L87) | Local | `population_and_accesses_are_independent::cache` | — |
| [88](feuer-memory/src/store/tests.rs#L88) | Local | `population_and_accesses_are_independent::key` | — |
| [112](feuer-memory/src/store/tests.rs#L112) | Local | `shared_download_population_is_deduplicated_but_each_waiter_records_an_access::cache` | — |
| [113](feuer-memory/src/store/tests.rs#L113) | Local | `shared_download_population_is_deduplicated_but_each_waiter_records_an_access::key` | — |
| [114](feuer-memory/src/store/tests.rs#L114) | Local | `shared_download_population_is_deduplicated_but_each_waiter_records_an_access::shared` | — |
| [130](feuer-memory/src/store/tests.rs#L130) | Local | `partially_overlapping_downloads_coexist_and_do_not_form_a_hit::cache` | — |
| [131](feuer-memory/src/store/tests.rs#L131) | Local | `partially_overlapping_downloads_coexist_and_do_not_form_a_hit::key` | — |
| [151](feuer-memory/src/store/tests.rs#L151) | Local | `a_larger_download_replaces_contained_entries_but_not_partial_overlaps::cache` | — |
| [152](feuer-memory/src/store/tests.rs#L152) | Local | `a_larger_download_replaces_contained_entries_but_not_partial_overlaps::key` | — |
| [178](feuer-memory/src/store/tests.rs#L178) | Local | `a_contained_download_is_discarded_without_replacing_cached_bytes::cache` | — |
| [179](feuer-memory/src/store/tests.rs#L179) | Local | `a_contained_download_is_discarded_without_replacing_cached_bytes::key` | — |
| [198](feuer-memory/src/store/tests.rs#L198) | Local | `capacity_is_charged_by_retained_download_payload_bytes::cache` | — |
| [199](feuer-memory/src/store/tests.rs#L199) | Local | `capacity_is_charged_by_retained_download_payload_bytes::key` | — |
| [214](feuer-memory/src/store/tests.rs#L214) | Local | `repeated_redundant_insertions_do_not_change_usage_or_replace_data::cache` | — |
| [215](feuer-memory/src/store/tests.rs#L215) | Local | `repeated_redundant_insertions_do_not_change_usage_or_replace_data::key` | — |
| [228](feuer-memory/src/store/tests.rs#L228) | Local | `oversized_insertion_empties_its_shard_and_remains_cached::cache` | — |
| [229](feuer-memory/src/store/tests.rs#L229) | Local | `oversized_insertion_empties_its_shard_and_remains_cached::key` | — |
| [242](feuer-memory/src/store/tests.rs#L242) | Local | `accessed_ranges_survive_downloaded_range_replacement::cache` | — |
| [243](feuer-memory/src/store/tests.rs#L243) | Local | `accessed_ranges_survive_downloaded_range_replacement::key` | — |
| [262](feuer-memory/src/store/tests.rs#L262) | Local | `access_history_survives_same_object_eviction_during_replacement::cache` | — |
| [263](feuer-memory/src/store/tests.rs#L263) | Local | `access_history_survives_same_object_eviction_during_replacement::key` | — |
| [289](feuer-memory/src/store/tests.rs#L289) | Local | `access_history_is_bounded_and_preserves_repeated_exact_requests::cache` | — |
| [290](feuer-memory/src/store/tests.rs#L290) | Local | `access_history_is_bounded_and_preserves_repeated_exact_requests::key` | — |
| [306](feuer-memory/src/store/tests.rs#L306) | Local | `repeated_requested_intervals_are_retained_over_single_accesses::cache` | — |
| [307](feuer-memory/src/store/tests.rs#L307) | Local | `repeated_requested_intervals_are_retained_over_single_accesses::hot` | — |
| [308](feuer-memory/src/store/tests.rs#L308) | Local | `repeated_requested_intervals_are_retained_over_single_accesses::cold` | — |
| [309](feuer-memory/src/store/tests.rs#L309) | Local | `repeated_requested_intervals_are_retained_over_single_accesses::incoming` | — |
| [329](feuer-memory/src/store/tests.rs#L329) | Local | `requested_bytes_contribute_to_retrieval_value::cache` | — |
| [330](feuer-memory/src/store/tests.rs#L330) | Local | `requested_bytes_contribute_to_retrieval_value::small` | — |
| [331](feuer-memory/src/store/tests.rs#L331) | Local | `requested_bytes_contribute_to_retrieval_value::large` | — |
| [357](feuer-memory/src/store/tests.rs#L357) | Local | `retention_credit_is_projected_only_onto_the_requested_interval::cache` | — |
| [358](feuer-memory/src/store/tests.rs#L358) | Local | `retention_credit_is_projected_only_onto_the_requested_interval::key` | — |
| [359](feuer-memory/src/store/tests.rs#L359) | Local | `retention_credit_is_projected_only_onto_the_requested_interval::incoming` | — |
| [372](feuer-memory/src/store/tests.rs#L372) | Local | `stale_frequency_eventually_expires::cache` | — |
| [373](feuer-memory/src/store/tests.rs#L373) | Local | `stale_frequency_eventually_expires::stale` | — |
| [374](feuer-memory/src/store/tests.rs#L374) | Local | `stale_frequency_eventually_expires::fresh` | — |
| [375](feuer-memory/src/store/tests.rs#L375) | Local | `stale_frequency_eventually_expires::clock` | — |
| [399](feuer-memory/src/store/tests.rs#L399) | Local | `compaction_respects_grace_then_releases_unrequested_payload::early_pressure` | — |
| [400](feuer-memory/src/store/tests.rs#L400) | Local | `compaction_respects_grace_then_releases_unrequested_payload::early_key` | — |
| [413](feuer-memory/src/store/tests.rs#L413) | Local | `compaction_respects_grace_then_releases_unrequested_payload::cache` | — |
| [414](feuer-memory/src/store/tests.rs#L414) | Local | `compaction_respects_grace_then_releases_unrequested_payload::key` | — |
| [415](feuer-memory/src/store/tests.rs#L415) | Local | `compaction_respects_grace_then_releases_unrequested_payload::incoming` | — |
| [416](feuer-memory/src/store/tests.rs#L416) | Local | `compaction_respects_grace_then_releases_unrequested_payload::original` | — |
| [419](feuer-memory/src/store/tests.rs#L419) | Local | `compaction_respects_grace_then_releases_unrequested_payload::returned` | — |
| [430](feuer-memory/src/store/tests.rs#L430) | Local | `compaction_respects_grace_then_releases_unrequested_payload::retained` | — |
| [440](feuer-memory/src/store/tests.rs#L440) | Local | `compaction_preserves_disjoint_requested_coverage_without_filling_gaps::cache` | — |
| [441](feuer-memory/src/store/tests.rs#L441) | Local | `compaction_preserves_disjoint_requested_coverage_without_filling_gaps::key` | — |
| [467](feuer-memory/src/store/tests.rs#L467) | Local | `compaction_waits_for_pressure_and_adds_no_access::cache` | — |
| [468](feuer-memory/src/store/tests.rs#L468) | Local | `compaction_waits_for_pressure_and_adds_no_access::key` | — |
| [469](feuer-memory/src/store/tests.rs#L469) | Local | `compaction_waits_for_pressure_and_adds_no_access::original` | — |
| [472](feuer-memory/src/store/tests.rs#L472) | Local | `compaction_waits_for_pressure_and_adds_no_access::returned` | — |
| [488](feuer-memory/src/store/tests.rs#L488) | Local | `compaction_waits_for_pressure_and_adds_no_access::retained` | — |
| [495](feuer-memory/src/store/tests.rs#L495) | Local | `candidate_state_tracks_entries_during_oversized_churn::cache` | — |
| [497](feuer-memory/src/store/tests.rs#L497) | Local | `candidate_state_tracks_entries_during_oversized_churn::key` | — |
| [508](feuer-memory/src/store/tests.rs#L508) | Local | `copied_compaction_is_revalidated_before_publication_and_can_fall_back::cache` | — |
| [509](feuer-memory/src/store/tests.rs#L509) | Local | `copied_compaction_is_revalidated_before_publication_and_can_fall_back::key` | — |
| [519](feuer-memory/src/store/tests.rs#L519) | Local | `copied_compaction_is_revalidated_before_publication_and_can_fall_back::incoming` | — |
| [520](feuer-memory/src/store/tests.rs#L520) | Local | `copied_compaction_is_revalidated_before_publication_and_can_fall_back::incoming_bytes` | — |
| [521](feuer-memory/src/store/tests.rs#L521) | Local | `copied_compaction_is_revalidated_before_publication_and_can_fall_back::replacement` | — |
| [522](feuer-memory/src/store/tests.rs#L522) | Local | `copied_compaction_is_revalidated_before_publication_and_can_fall_back::replacement::mut shard` | — |
| [523](feuer-memory/src/store/tests.rs#L523) | Local | `copied_compaction_is_revalidated_before_publication_and_can_fall_back::replacement::AdmissionStep::Compact(source)` | — |
| [536](feuer-memory/src/store/tests.rs#L536) | Local | `copied_compaction_is_revalidated_before_publication_and_can_fall_back::step` | — |
| [545](feuer-memory/src/store/tests.rs#L545) | Local | `removing_the_last_cached_range_releases_its_access_history::cache` | — |
| [546](feuer-memory/src/store/tests.rs#L546) | Local | `removing_the_last_cached_range_releases_its_access_history::key` | — |
| [557](feuer-memory/src/store/tests.rs#L557) | Local | `zero_target_still_retains_the_latest_entry::cache` | — |
| [558](feuer-memory/src/store/tests.rs#L558) | Local | `zero_target_still_retains_the_latest_entry::key` | — |
| [582](feuer-memory/src/store/tests.rs#L582) | Local | `shard_targets_can_collectively_exceed_the_configured_capacity::cache` | — |
| [583](feuer-memory/src/store/tests.rs#L583) | Local | `shard_targets_can_collectively_exceed_the_configured_capacity::mut keys` | — |
| [585](feuer-memory/src/store/tests.rs#L585) | Local | `shard_targets_can_collectively_exceed_the_configured_capacity::key` | — |
| [586](feuer-memory/src/store/tests.rs#L586) | Local | `shard_targets_can_collectively_exceed_the_configured_capacity::shard` | — |
| [592](feuer-memory/src/store/tests.rs#L592) | Local | `shard_targets_can_collectively_exceed_the_configured_capacity::[Some(first), Some(second)]` | — |
| [605](feuer-memory/src/store/tests.rs#L605) | Local | `concurrent_shards_respect_their_targets_for_regular_entries::cache` | — |
| [606](feuer-memory/src/store/tests.rs#L606) | Local | `concurrent_shards_respect_their_targets_for_regular_entries::mut threads` | — |
| [608](feuer-memory/src/store/tests.rs#L608) | Local | `concurrent_shards_respect_their_targets_for_regular_entries::cache` | — |
| [610](feuer-memory/src/store/tests.rs#L610) | Local | `concurrent_shards_respect_their_targets_for_regular_entries::key` | — |
| [612](feuer-memory/src/store/tests.rs#L612) | Local | `concurrent_shards_respect_their_targets_for_regular_entries::start` | — |

</details>

### `feuer-memory/src/store.rs`

| Line | Kind | Name / source parent | Signature or type |
| ---: | --- | --- | --- |
| [1](feuer-memory/src/store.rs#L1) | Module | `access_history` | — |
| [2](feuer-memory/src/store.rs#L2) | Module | `compaction` | — |
| [3](feuer-memory/src/store.rs#L3) | Module | `shard` | — |
| [5](feuer-memory/src/store.rs#L5) | Module | `tests` | — |
| [22](feuer-memory/src/store.rs#L22) | Const | `MAX_SHARDS` | `usize` |
| [41](feuer-memory/src/store.rs#L41) | Struct | `MemoryCache` | — |
| [43](feuer-memory/src/store.rs#L43) | Field | `MemoryCache::capacity` | `u64` |
| [45](feuer-memory/src/store.rs#L45) | Field | `MemoryCache::shards` | `Box<[Mutex<MemoryShard>]>` |
| [48](feuer-memory/src/store.rs#L48) | Impl | `impl fmt::Debug for MemoryCache` | — |
| [49](feuer-memory/src/store.rs#L49) | Method | `impl fmt::Debug for MemoryCache::fmt` | `fn(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result` |
| [58](feuer-memory/src/store.rs#L58) | Impl | `impl MemoryCache` | — |
| [60](feuer-memory/src/store.rs#L60) | Function | `impl MemoryCache::new` | `fn(capacity: u64) -> Self` |
| [65](feuer-memory/src/store.rs#L65) | Function | `impl MemoryCache::with_metrics` | `fn(capacity: u64, metrics: Arc<MemoryMetrics>) -> Self` |
| [72](feuer-memory/src/store.rs#L72) | Function | `impl MemoryCache::with_shards_for_benchmark` | `fn(capacity: u64, shard_count: usize) -> Self` |
| [76](feuer-memory/src/store.rs#L76) | Function | `impl MemoryCache::with_shard_count` | `fn(capacity: u64, metrics: Arc<MemoryMetrics>, shard_count: usize) -> Self` |
| [90](feuer-memory/src/store.rs#L90) | Method | `impl MemoryCache::capacity` | `fn(&self) -> u64` |
| [95](feuer-memory/src/store.rs#L95) | Method | `impl MemoryCache::used_bytes` | `fn(&self) -> u64` |
| [100](feuer-memory/src/store.rs#L100) | Method | `impl MemoryCache::get` | `fn(&self, object_key: &ObjectKey, requested_range: ByteRange) -> Option<Bytes>` |
| [110](feuer-memory/src/store.rs#L110) | Method | `impl MemoryCache::insert` | `fn(&self, object_key: ObjectKey, download: Download)` |
| [120](feuer-memory/src/store.rs#L120) | Method | `impl MemoryCache::insert_and_record` | `fn(&self, object_key: ObjectKey, download: Download, requested_range: ByteRange)` |
| [126](feuer-memory/src/store.rs#L126) | Method | `impl MemoryCache::insert_inner` | `fn( &self, object_key: ObjectKey, downloaded_range: ByteRange, bytes: Bytes, requested_range: Option<ByteRange>, )` |
| [166](feuer-memory/src/store.rs#L166) | Method | `impl MemoryCache::record_access` | `fn(&self, object_key: &ObjectKey, requested_range: ByteRange)` |
| [174](feuer-memory/src/store.rs#L174) | Method | `impl MemoryCache::remove` | `fn(&self, object_key: &ObjectKey, range: ByteRange) -> bool` |
| [182](feuer-memory/src/store.rs#L182) | Method | `impl MemoryCache::shard_index` | `fn(&self, object_key: &ObjectKey) -> usize` |
| [189](feuer-memory/src/store.rs#L189) | Method | `impl MemoryCache::entry_count` | `fn(&self) -> u64` |
| [194](feuer-memory/src/store.rs#L194) | Function | `shard_capacity_for` | `fn(total: u64, shards: usize, index: usize) -> u64` |
| [199](feuer-memory/src/store.rs#L199) | Function | `default_shard_count` | `fn() -> usize` |

<details>
<summary>Local bindings (12)</summary>

| Line | Kind | Name / source parent | Signature or type |
| ---: | --- | --- | --- |
| [78](feuer-memory/src/store.rs#L78) | Local | `impl MemoryCache::with_shard_count::shards` | — |
| [101](feuer-memory/src/store.rs#L101) | Local | `impl MemoryCache::get::shard_index` | — |
| [111](feuer-memory/src/store.rs#L111) | Local | `impl MemoryCache::insert::(downloaded_range, bytes)` | — |
| [121](feuer-memory/src/store.rs#L121) | Local | `impl MemoryCache::insert_and_record::(downloaded_range, bytes)` | — |
| [133](feuer-memory/src/store.rs#L133) | Local | `impl MemoryCache::insert_inner::shard_index` | — |
| [134](feuer-memory/src/store.rs#L134) | Local | `impl MemoryCache::insert_inner::mut allow_compaction` | — |
| [136](feuer-memory/src/store.rs#L136) | Local | `impl MemoryCache::insert_inner::step` | — |
| [150](feuer-memory/src/store.rs#L150) | Local | `impl MemoryCache::insert_inner::replacement` | — |
| [167](feuer-memory/src/store.rs#L167) | Local | `impl MemoryCache::record_access::shard_index` | — |
| [175](feuer-memory/src/store.rs#L175) | Local | `impl MemoryCache::remove::shard_index` | — |
| [183](feuer-memory/src/store.rs#L183) | Local | `impl MemoryCache::shard_index::mut hasher` | — |
| [195](feuer-memory/src/store.rs#L195) | Local | `shard_capacity_for::shards` | — |

</details>

## feuer-memory-bench

### `benchmarks/memory/src/main.rs`

| Line | Kind | Name / source parent | Signature or type |
| ---: | --- | --- | --- |
| [17](benchmarks/memory/src/main.rs#L17) | Const | `PINNED_FOYER_REVISION` | `&str` |
| [18](benchmarks/memory/src/main.rs#L18) | Const | `SOURCE_FIXED_EQUIVALENT_BYTES` | `u64` |
| [20](benchmarks/memory/src/main.rs#L20) | Const | `FOYER_COST_SAMPLE_SIZE` | `usize` |
| [21](benchmarks/memory/src/main.rs#L21) | Const | `TRACE_FILE` | `&str` |
| [22](benchmarks/memory/src/main.rs#L22) | Const | `COALESCING_DISTANCE_ENV` | `&str` |
| [23](benchmarks/memory/src/main.rs#L23) | Const | `WHOLE_SPLIT_THRESHOLD_ENV` | `&str` |
| [24](benchmarks/memory/src/main.rs#L24) | Const | `DEFAULT_COALESCING_DISTANCE_BYTES` | `u64` |
| [25](benchmarks/memory/src/main.rs#L25) | Const | `DEFAULT_WHOLE_SPLIT_THRESHOLD_BYTES` | `u64` |
| [26](benchmarks/memory/src/main.rs#L26) | Const | `COALESCING_WINDOW_MILLIS` | `u64` |
| [27](benchmarks/memory/src/main.rs#L27) | Const | `CSV_HEADER` | `&str` |
| [31](benchmarks/memory/src/main.rs#L31) | Struct | `Args` | — |
| [39](benchmarks/memory/src/main.rs#L39) | Field | `Args::capacities` | `Vec<usize>` |
| [43](benchmarks/memory/src/main.rs#L43) | Field | `Args::shards` | `Vec<usize>` |
| [52](benchmarks/memory/src/main.rs#L52) | Field | `Args::downloaders` | `Vec<DownloadPolicy>` |
| [56](benchmarks/memory/src/main.rs#L56) | Field | `Args::warmup_iterations` | `usize` |
| [60](benchmarks/memory/src/main.rs#L60) | Field | `Args::operations` | `Option<usize>` |
| [64](benchmarks/memory/src/main.rs#L64) | Field | `Args::csv` | `bool` |
| [68](benchmarks/memory/src/main.rs#L68) | Enum | `DownloadPolicy` | — |
| [69](benchmarks/memory/src/main.rs#L69) | Variant | `DownloadPolicy::Expanded` | — |
| [70](benchmarks/memory/src/main.rs#L70) | Variant | `DownloadPolicy::Exact` | — |
| [73](benchmarks/memory/src/main.rs#L73) | Impl | `impl DownloadPolicy` | — |
| [74](benchmarks/memory/src/main.rs#L74) | Method | `impl DownloadPolicy::name` | `fn(self) -> &'static str` |
| [83](benchmarks/memory/src/main.rs#L83) | Struct | `DownloadConfig` | — |
| [84](benchmarks/memory/src/main.rs#L84) | Field | `DownloadConfig::coalescing_distance_bytes` | `u64` |
| [85](benchmarks/memory/src/main.rs#L85) | Field | `DownloadConfig::whole_split_threshold_bytes` | `u64` |
| [88](benchmarks/memory/src/main.rs#L88) | Impl | `impl DownloadConfig` | — |
| [89](benchmarks/memory/src/main.rs#L89) | Function | `impl DownloadConfig::from_env` | `fn() -> Result<Self, String>` |
| [101](benchmarks/memory/src/main.rs#L101) | Struct | `Access` | — |
| [102](benchmarks/memory/src/main.rs#L102) | Field | `Access::object_key` | `ObjectKey` |
| [103](benchmarks/memory/src/main.rs#L103) | Field | `Access::object_size` | `u64` |
| [104](benchmarks/memory/src/main.rs#L104) | Field | `Access::requested` | `ByteRange` |
| [105](benchmarks/memory/src/main.rs#L105) | Field | `Access::timestamp_millis` | `u64` |
| [108](benchmarks/memory/src/main.rs#L108) | Impl | `impl Access` | — |
| [109](benchmarks/memory/src/main.rs#L109) | Method | `impl Access::validate` | `fn(&self, index: usize) -> Result<(), String>` |
| [121](benchmarks/memory/src/main.rs#L121) | Method | `impl Access::downloaded_range` | `fn(&self, policy: DownloadPolicy, config: DownloadConfig) -> ByteRange` |
| [134](benchmarks/memory/src/main.rs#L134) | Struct | `Workload` | — |
| [135](benchmarks/memory/src/main.rs#L135) | Field | `Workload::name` | `&'static str` |
| [136](benchmarks/memory/src/main.rs#L136) | Field | `Workload::accesses` | `Vec<Access>` |
| [137](benchmarks/memory/src/main.rs#L137) | Field | `Workload::expanded_downloads` | `Vec<ByteRange>` |
| [138](benchmarks/memory/src/main.rs#L138) | Field | `Workload::download_config` | `DownloadConfig` |
| [142](benchmarks/memory/src/main.rs#L142) | Trait | `ReplayCache` | — |
| [143](benchmarks/memory/src/main.rs#L143) | Method | `ReplayCache::name` | `fn(&self) -> &'static str` |
| [146](benchmarks/memory/src/main.rs#L146) | Method | `ReplayCache::get` | `fn(&mut self, access: &Access, downloaded: ByteRange) -> bool` |
| [148](benchmarks/memory/src/main.rs#L148) | Method | `ReplayCache::populate` | `fn(&mut self, access: &Access, downloaded: ByteRange, payload: Bytes) -> Result<(), String>` |
| [150](benchmarks/memory/src/main.rs#L150) | Method | `ReplayCache::used_payload_bytes` | `fn(&self) -> u64` |
| [154](benchmarks/memory/src/main.rs#L154) | Struct | `FeuerReplayCache` | — |
| [155](benchmarks/memory/src/main.rs#L155) | Field | `FeuerReplayCache::cache` | `MemoryCache` |
| [158](benchmarks/memory/src/main.rs#L158) | Impl | `impl FeuerReplayCache` | — |
| [159](benchmarks/memory/src/main.rs#L159) | Function | `impl FeuerReplayCache::new` | `fn(capacity: usize, shards: usize) -> Self` |
| [166](benchmarks/memory/src/main.rs#L166) | Impl | `impl ReplayCache for FeuerReplayCache` | — |
| [167](benchmarks/memory/src/main.rs#L167) | Method | `impl ReplayCache for FeuerReplayCache::name` | `fn(&self) -> &'static str` |
| [171](benchmarks/memory/src/main.rs#L171) | Method | `impl ReplayCache for FeuerReplayCache::get` | `fn(&mut self, access: &Access, _downloaded: ByteRange) -> bool` |
| [179](benchmarks/memory/src/main.rs#L179) | Method | `impl ReplayCache for FeuerReplayCache::populate` | `fn(&mut self, access: &Access, downloaded: ByteRange, payload: Bytes) -> Result<(), String>` |
| [187](benchmarks/memory/src/main.rs#L187) | Method | `impl ReplayCache for FeuerReplayCache::used_payload_bytes` | `fn(&self) -> u64` |
| [192](benchmarks/memory/src/main.rs#L192) | TypeAlias | `NativeFoyerKey` | `(ObjectKey, ByteRange)` |
| [195](benchmarks/memory/src/main.rs#L195) | Struct | `NativeFoyerValue` | — |
| [196](benchmarks/memory/src/main.rs#L196) | Field | `NativeFoyerValue::downloaded` | `ByteRange` |
| [197](benchmarks/memory/src/main.rs#L197) | Field | `NativeFoyerValue::payload` | `Bytes` |
| [201](benchmarks/memory/src/main.rs#L201) | Enum | `NativeFoyerKeyMode` | — |
| [203](benchmarks/memory/src/main.rs#L203) | Variant | `NativeFoyerKeyMode::ExactRequest` | — |
| [205](benchmarks/memory/src/main.rs#L205) | Variant | `NativeFoyerKeyMode::ExpandedDownload` | — |
| [208](benchmarks/memory/src/main.rs#L208) | Impl | `impl NativeFoyerKeyMode` | — |
| [209](benchmarks/memory/src/main.rs#L209) | Method | `impl NativeFoyerKeyMode::range` | `fn(self, access: &Access, downloaded: ByteRange) -> ByteRange` |
| [218](benchmarks/memory/src/main.rs#L218) | Enum | `NativeFoyerPolicy` | — |
| [219](benchmarks/memory/src/main.rs#L219) | Variant | `NativeFoyerPolicy::S3Fifo` | — |
| [220](benchmarks/memory/src/main.rs#L220) | Variant | `NativeFoyerPolicy::CostAware` | — |
| [223](benchmarks/memory/src/main.rs#L223) | Impl | `impl NativeFoyerPolicy` | — |
| [224](benchmarks/memory/src/main.rs#L224) | Method | `impl NativeFoyerPolicy::engine_name` | `fn(self, key_mode: NativeFoyerKeyMode) -> &'static str` |
| [235](benchmarks/memory/src/main.rs#L235) | Struct | `FoyerReplayCache` | — |
| [236](benchmarks/memory/src/main.rs#L236) | Field | `FoyerReplayCache::cache` | `FoyerCache<NativeFoyerKey, NativeFoyerValue>` |
| [237](benchmarks/memory/src/main.rs#L237) | Field | `FoyerReplayCache::key_mode` | `NativeFoyerKeyMode` |
| [238](benchmarks/memory/src/main.rs#L238) | Field | `FoyerReplayCache::policy` | `NativeFoyerPolicy` |
| [241](benchmarks/memory/src/main.rs#L241) | Impl | `impl FoyerReplayCache` | — |
| [242](benchmarks/memory/src/main.rs#L242) | Function | `impl FoyerReplayCache::new` | `fn(capacity: usize, shards: usize, key_mode: NativeFoyerKeyMode, policy: NativeFoyerPolicy) -> Self` |
| [250](benchmarks/memory/src/main.rs#L250) | Method | `impl FoyerReplayCache::key` | `fn(&self, access: &Access, downloaded: ByteRange) -> NativeFoyerKey` |
| [255](benchmarks/memory/src/main.rs#L255) | Impl | `impl ReplayCache for FoyerReplayCache` | — |
| [256](benchmarks/memory/src/main.rs#L256) | Method | `impl ReplayCache for FoyerReplayCache::name` | `fn(&self) -> &'static str` |
| [260](benchmarks/memory/src/main.rs#L260) | Method | `impl ReplayCache for FoyerReplayCache::get` | `fn(&mut self, access: &Access, downloaded: ByteRange) -> bool` |
| [272](benchmarks/memory/src/main.rs#L272) | Method | `impl ReplayCache for FoyerReplayCache::populate` | `fn(&mut self, access: &Access, downloaded: ByteRange, payload: Bytes) -> Result<(), String>` |
| [278](benchmarks/memory/src/main.rs#L278) | Method | `impl ReplayCache for FoyerReplayCache::used_payload_bytes` | `fn(&self) -> u64` |
| [283](benchmarks/memory/src/main.rs#L283) | Function | `foyer_cache` | `fn( capacity: usize, shards: usize, policy: NativeFoyerPolicy, ) -> FoyerCache<NativeFoyerKey, NativeFoyerValue>` |
| [303](benchmarks/memory/src/main.rs#L303) | Struct | `Traffic` | — |
| [304](benchmarks/memory/src/main.rs#L304) | Field | `Traffic::requests` | `u64` |
| [305](benchmarks/memory/src/main.rs#L305) | Field | `Traffic::requested_bytes` | `u64` |
| [306](benchmarks/memory/src/main.rs#L306) | Field | `Traffic::hits` | `u64` |
| [307](benchmarks/memory/src/main.rs#L307) | Field | `Traffic::hit_bytes` | `u64` |
| [308](benchmarks/memory/src/main.rs#L308) | Field | `Traffic::source_requests` | `u64` |
| [309](benchmarks/memory/src/main.rs#L309) | Field | `Traffic::source_bytes` | `u64` |
| [312](benchmarks/memory/src/main.rs#L312) | Struct | `Report` | — |
| [313](benchmarks/memory/src/main.rs#L313) | Field | `Report::workload` | `&'static str` |
| [314](benchmarks/memory/src/main.rs#L314) | Field | `Report::downloader` | `&'static str` |
| [315](benchmarks/memory/src/main.rs#L315) | Field | `Report::shards` | `usize` |
| [316](benchmarks/memory/src/main.rs#L316) | Field | `Report::capacity` | `usize` |
| [317](benchmarks/memory/src/main.rs#L317) | Field | `Report::engine` | `&'static str` |
| [318](benchmarks/memory/src/main.rs#L318) | Field | `Report::traffic` | `Traffic` |
| [319](benchmarks/memory/src/main.rs#L319) | Field | `Report::used_payload_bytes` | `u64` |
| [320](benchmarks/memory/src/main.rs#L320) | Field | `Report::elapsed` | `Duration` |
| [323](benchmarks/memory/src/main.rs#L323) | Impl | `impl Report` | — |
| [324](benchmarks/memory/src/main.rs#L324) | Method | `impl Report::cache_hit_rate` | `fn(&self) -> f64` |
| [328](benchmarks/memory/src/main.rs#L328) | Method | `impl Report::byte_hit_rate` | `fn(&self) -> f64` |
| [332](benchmarks/memory/src/main.rs#L332) | Method | `impl Report::source_cost_hit_rate` | `fn(&self) -> f64` |
| [343](benchmarks/memory/src/main.rs#L343) | Method | `impl Report::operations_per_second` | `fn(&self) -> f64` |
| [348](benchmarks/memory/src/main.rs#L348) | Function | `main` | `fn() -> Result<(), String>` |
| [422](benchmarks/memory/src/main.rs#L422) | Function | `trace_workload` | `fn(args: &Args, download_config: DownloadConfig) -> Result<Workload, String>` |
| [447](benchmarks/memory/src/main.rs#L447) | Function | `replay_cache` | `fn( mut cache: Box<dyn ReplayCache>, workload: &Workload, downloader: DownloadPolicy, shards: usize, capacity: usize, warmup_iterations: usize, source_payload: &Bytes, ) -> Result<Report, String>` |
| [479](benchmarks/memory/src/main.rs#L479) | Struct | `PendingDownload` | — |
| [480](benchmarks/memory/src/main.rs#L480) | Field | `PendingDownload::order` | `usize` |
| [481](benchmarks/memory/src/main.rs#L481) | Field | `PendingDownload::access` | `&'a Access` |
| [482](benchmarks/memory/src/main.rs#L482) | Field | `PendingDownload::downloaded` | `ByteRange` |
| [485](benchmarks/memory/src/main.rs#L485) | Struct | `CoalescedDownload` | — |
| [486](benchmarks/memory/src/main.rs#L486) | Field | `CoalescedDownload::downloaded` | `ByteRange` |
| [487](benchmarks/memory/src/main.rs#L487) | Field | `CoalescedDownload::accesses` | `Vec<(usize, &'a Access)>` |
| [490](benchmarks/memory/src/main.rs#L490) | Function | `expanded_download_ranges` | `fn(workload: &[Access], config: DownloadConfig) -> Vec<ByteRange>` |
| [541](benchmarks/memory/src/main.rs#L541) | Function | `max_download_len` | `fn(workload: &Workload, downloader: DownloadPolicy) -> u64` |
| [562](benchmarks/memory/src/main.rs#L562) | Function | `execute_pass` | `fn<C: ReplayCache + ?Sized>( cache: &mut C, workload: &Workload, downloader: DownloadPolicy, source_payload: &Bytes, traffic: &mut Traffic, ) -> Result<(), String>` |
| [588](benchmarks/memory/src/main.rs#L588) | Function | `coalesced_downloads` | `fn( mut pending: Vec<PendingDownload<'_>>, coalescing_distance_bytes: u64, ) -> Vec<CoalescedDownload<'_>>` |
| [627](benchmarks/memory/src/main.rs#L627) | Function | `record_request` | `fn(traffic: &mut Traffic, access: &Access, hit: bool)` |
| [636](benchmarks/memory/src/main.rs#L636) | Function | `source_payload_slice` | `fn(source_payload: &Bytes, downloaded: ByteRange) -> Result<Bytes, String>` |
| [644](benchmarks/memory/src/main.rs#L644) | Function | `requested_payload` | `fn(bytes: &Bytes, downloaded: ByteRange, requested: ByteRange) -> Bytes` |
| [653](benchmarks/memory/src/main.rs#L653) | Function | `print_human_header` | `fn(args: &Args, workload: &Workload)` |
| [672](benchmarks/memory/src/main.rs#L672) | Function | `print_human_report` | `fn(report: &Report)` |
| [687](benchmarks/memory/src/main.rs#L687) | Function | `human_engine_name` | `fn(engine: &str) -> &str` |
| [698](benchmarks/memory/src/main.rs#L698) | Function | `format_decimal_bytes` | `fn(bytes: u64) -> String` |
| [710](benchmarks/memory/src/main.rs#L710) | Function | `format_bytes` | `fn(bytes: u64) -> String` |
| [722](benchmarks/memory/src/main.rs#L722) | Function | `format_rate` | `fn(operations_per_second: f64) -> String` |
| [732](benchmarks/memory/src/main.rs#L732) | Function | `print_csv_report` | `fn(report: &Report)` |
| [756](benchmarks/memory/src/main.rs#L756) | Function | `load_trace` | `fn() -> Result<Vec<Access>, String>` |
| [769](benchmarks/memory/src/main.rs#L769) | Function | `parse_trace_line` | `fn(line: &str) -> Result<Access, String>` |
| [785](benchmarks/memory/src/main.rs#L785) | Function | `parse_timestamp_millis` | `fn(value: &str) -> Result<u64, String>` |
| [846](benchmarks/memory/src/main.rs#L846) | Function | `find_json_string` | `fn(line: &str, field: &str) -> Result<String, String>` |
| [857](benchmarks/memory/src/main.rs#L857) | Function | `find_json_u64` | `fn(line: &str, field: &str) -> Result<u64, String>` |
| [870](benchmarks/memory/src/main.rs#L870) | Function | `field_value` | `fn<'a>(line: &'a str, field: &str) -> Result<&'a str, String>` |
| [883](benchmarks/memory/src/main.rs#L883) | Function | `ratio` | `fn(numerator: u64, denominator: u64) -> f64` |
| [891](benchmarks/memory/src/main.rs#L891) | Function | `byte_count_from_env` | `fn(name: &str, default: u64) -> Result<u64, String>` |
| [901](benchmarks/memory/src/main.rs#L901) | Function | `parse_bytes` | `fn(value: &str) -> Result<usize, String>` |
| [906](benchmarks/memory/src/main.rs#L906) | Function | `parse_byte_count` | `fn(value: &str) -> Result<u128, String>` |
| [931](benchmarks/memory/src/main.rs#L931) | Module | `tests` | — |
| [934](benchmarks/memory/src/main.rs#L934) | Struct | `tests::WarmupTestCache` | — |
| [935](benchmarks/memory/src/main.rs#L935) | Field | `tests::WarmupTestCache::populated` | `bool` |
| [938](benchmarks/memory/src/main.rs#L938) | Impl | `tests::impl ReplayCache for WarmupTestCache` | — |
| [939](benchmarks/memory/src/main.rs#L939) | Method | `tests::impl ReplayCache for WarmupTestCache::name` | `fn(&self) -> &'static str` |
| [943](benchmarks/memory/src/main.rs#L943) | Method | `tests::impl ReplayCache for WarmupTestCache::get` | `fn(&mut self, _access: &Access, _downloaded: ByteRange) -> bool` |
| [947](benchmarks/memory/src/main.rs#L947) | Method | `tests::impl ReplayCache for WarmupTestCache::populate` | `fn(&mut self, _access: &Access, _downloaded: ByteRange, _payload: Bytes) -> Result<(), String>` |
| [952](benchmarks/memory/src/main.rs#L952) | Method | `tests::impl ReplayCache for WarmupTestCache::used_payload_bytes` | `fn(&self) -> u64` |
| [958](benchmarks/memory/src/main.rs#L958) | Struct | `tests::RangeTestCache` | — |
| [959](benchmarks/memory/src/main.rs#L959) | Field | `tests::RangeTestCache::entries` | `Vec<(ObjectKey, ByteRange)>` |
| [962](benchmarks/memory/src/main.rs#L962) | Impl | `tests::impl ReplayCache for RangeTestCache` | — |
| [963](benchmarks/memory/src/main.rs#L963) | Method | `tests::impl ReplayCache for RangeTestCache::name` | `fn(&self) -> &'static str` |
| [967](benchmarks/memory/src/main.rs#L967) | Method | `tests::impl ReplayCache for RangeTestCache::get` | `fn(&mut self, access: &Access, _downloaded: ByteRange) -> bool` |
| [973](benchmarks/memory/src/main.rs#L973) | Method | `tests::impl ReplayCache for RangeTestCache::populate` | `fn(&mut self, access: &Access, downloaded: ByteRange, _payload: Bytes) -> Result<(), String>` |
| [979](benchmarks/memory/src/main.rs#L979) | Method | `tests::impl ReplayCache for RangeTestCache::used_payload_bytes` | `fn(&self) -> u64` |
| [985](benchmarks/memory/src/main.rs#L985) | Function | `tests::warmup_preserves_cache_state_but_not_reported_traffic` | `fn()` |
| [1018](benchmarks/memory/src/main.rs#L1018) | Function | `tests::foyer_expanded_key_reuses_identical_expansions_for_distinct_requests` | `fn()` |
| [1048](benchmarks/memory/src/main.rs#L1048) | Function | `tests::expanded_downloader_assigns_one_coalesced_range_to_all_batch_members` | `fn()` |
| [1092](benchmarks/memory/src/main.rs#L1092) | Function | `tests::coalescing_distance_parameter_is_a_strict_upper_bound` | `fn()` |
| [1126](benchmarks/memory/src/main.rs#L1126) | Function | `tests::source_cost_baseline_uses_requested_bytes_not_downloaded_bytes` | `fn()` |
| [1148](benchmarks/memory/src/main.rs#L1148) | Function | `tests::parses_the_captured_trace_shape` | `fn()` |
| [1163](benchmarks/memory/src/main.rs#L1163) | Function | `tests::expanded_downloader_honors_the_whole_split_threshold` | `fn()` |
| [1196](benchmarks/memory/src/main.rs#L1196) | Function | `tests::timestamp_parser_handles_day_boundaries` | `fn()` |
| [1208](benchmarks/memory/src/main.rs#L1208) | Function | `tests::environment_byte_counts_accept_documented_units` | `fn()` |
| [1214](benchmarks/memory/src/main.rs#L1214) | Function | `tests::human_output_is_default_and_csv_is_opt_in` | `fn()` |

<details>
<summary>Local bindings (109)</summary>

| Line | Kind | Name / source parent | Signature or type |
| ---: | --- | --- | --- |
| [172](benchmarks/memory/src/main.rs#L172) | Local | `impl ReplayCache for FeuerReplayCache::get::Some(bytes)` | — |
| [180](benchmarks/memory/src/main.rs#L180) | Local | `impl ReplayCache for FeuerReplayCache::populate::download` | — |
| [261](benchmarks/memory/src/main.rs#L261) | Local | `impl ReplayCache for FoyerReplayCache::get::key` | — |
| [262](benchmarks/memory/src/main.rs#L262) | Local | `impl ReplayCache for FoyerReplayCache::get::Some(entry)` | — |
| [265](benchmarks/memory/src/main.rs#L265) | Local | `impl ReplayCache for FoyerReplayCache::get::value` | — |
| [267](benchmarks/memory/src/main.rs#L267) | Local | `impl ReplayCache for FoyerReplayCache::get::result` | — |
| [273](benchmarks/memory/src/main.rs#L273) | Local | `impl ReplayCache for FoyerReplayCache::populate::key` | — |
| [288](benchmarks/memory/src/main.rs#L288) | Local | `foyer_cache::builder` | — |
| [333](benchmarks/memory/src/main.rs#L333) | Local | `impl Report::source_cost_hit_rate::fixed_cost` | — |
| [334](benchmarks/memory/src/main.rs#L334) | Local | `impl Report::source_cost_hit_rate::baseline` | — |
| [335](benchmarks/memory/src/main.rs#L335) | Local | `impl Report::source_cost_hit_rate::actual` | — |
| [349](benchmarks/memory/src/main.rs#L349) | Local | `main::args` | — |
| [359](benchmarks/memory/src/main.rs#L359) | Local | `main::download_config` | — |
| [360](benchmarks/memory/src/main.rs#L360) | Local | `main::workload` | — |
| [377](benchmarks/memory/src/main.rs#L377) | Local | `main::max_download` | — |
| [378](benchmarks/memory/src/main.rs#L378) | Local | `main::max_download` | — |
| [379](benchmarks/memory/src/main.rs#L379) | Local | `main::source_payload` | — |
| [383](benchmarks/memory/src/main.rs#L383) | Local | `main::mut caches` | `Vec<Box<dyn ReplayCache>>` |
| [401](benchmarks/memory/src/main.rs#L401) | Local | `main::report` | — |
| [423](benchmarks/memory/src/main.rs#L423) | Local | `trace_workload::mut accesses` | — |
| [438](benchmarks/memory/src/main.rs#L438) | Local | `trace_workload::expanded_downloads` | — |
| [457](benchmarks/memory/src/main.rs#L457) | Local | `replay_cache::mut warmup_traffic` | — |
| [461](benchmarks/memory/src/main.rs#L461) | Local | `replay_cache::mut traffic` | — |
| [462](benchmarks/memory/src/main.rs#L462) | Local | `replay_cache::started` | — |
| [464](benchmarks/memory/src/main.rs#L464) | Local | `replay_cache::elapsed` | — |
| [491](benchmarks/memory/src/main.rs#L491) | Local | `expanded_download_ranges::mut by_object` | `HashMap<(&str, u64), Vec<usize>>` |
| [499](benchmarks/memory/src/main.rs#L499) | Local | `expanded_download_ranges::base_ranges` | `Vec<_>` |
| [503](benchmarks/memory/src/main.rs#L503) | Local | `expanded_download_ranges::mut ranges` | — |
| [507](benchmarks/memory/src/main.rs#L507) | Local | `expanded_download_ranges::mut assigned` | — |
| [514](benchmarks/memory/src/main.rs#L514) | Local | `expanded_download_ranges::deadline` | — |
| [517](benchmarks/memory/src/main.rs#L517) | Local | `expanded_download_ranges::pending` | — |
| [528](benchmarks/memory/src/main.rs#L528) | Local | `expanded_download_ranges::download` | — |
| [570](benchmarks/memory/src/main.rs#L570) | Local | `execute_pass::downloaded` | — |
| [574](benchmarks/memory/src/main.rs#L574) | Local | `execute_pass::hit` | — |
| [580](benchmarks/memory/src/main.rs#L580) | Local | `execute_pass::payload` | — |
| [601](benchmarks/memory/src/main.rs#L601) | Local | `coalesced_downloads::mut downloads` | `Vec<CoalescedDownload<'_>>` |
| [603](benchmarks/memory/src/main.rs#L603) | Local | `coalesced_downloads::merge` | — |
| [604](benchmarks/memory/src/main.rs#L604) | Local | `coalesced_downloads::merge::representative` | — |
| [605](benchmarks/memory/src/main.rs#L605) | Local | `coalesced_downloads::merge::gap` | — |
| [637](benchmarks/memory/src/main.rs#L637) | Local | `source_payload_slice::payload_len` | — |
| [646](benchmarks/memory/src/main.rs#L646) | Local | `requested_payload::start` | — |
| [648](benchmarks/memory/src/main.rs#L648) | Local | `requested_payload::end` | — |
| [654](benchmarks/memory/src/main.rs#L654) | Local | `print_human_header::shards` | — |
| [757](benchmarks/memory/src/main.rs#L757) | Local | `load_trace::path` | — |
| [758](benchmarks/memory/src/main.rs#L758) | Local | `load_trace::content` | — |
| [770](benchmarks/memory/src/main.rs#L770) | Local | `parse_trace_line::object_key` | — |
| [771](benchmarks/memory/src/main.rs#L771) | Local | `parse_trace_line::object_size` | — |
| [772](benchmarks/memory/src/main.rs#L772) | Local | `parse_trace_line::start` | — |
| [773](benchmarks/memory/src/main.rs#L773) | Local | `parse_trace_line::end` | — |
| [774](benchmarks/memory/src/main.rs#L774) | Local | `parse_trace_line::requested` | — |
| [775](benchmarks/memory/src/main.rs#L775) | Local | `parse_trace_line::timestamp` | — |
| [776](benchmarks/memory/src/main.rs#L776) | Local | `parse_trace_line::timestamp_millis` | — |
| [786](benchmarks/memory/src/main.rs#L786) | Local | `parse_timestamp_millis::bytes` | — |
| [787](benchmarks/memory/src/main.rs#L787) | Local | `parse_timestamp_millis::valid_suffix` | — |
| [799](benchmarks/memory/src/main.rs#L799) | Local | `parse_timestamp_millis::component` | — |
| [804](benchmarks/memory/src/main.rs#L804) | Local | `parse_timestamp_millis::year` | — |
| [805](benchmarks/memory/src/main.rs#L805) | Local | `parse_timestamp_millis::month` | — |
| [806](benchmarks/memory/src/main.rs#L806) | Local | `parse_timestamp_millis::day` | — |
| [807](benchmarks/memory/src/main.rs#L807) | Local | `parse_timestamp_millis::hour` | — |
| [808](benchmarks/memory/src/main.rs#L808) | Local | `parse_timestamp_millis::minute` | — |
| [809](benchmarks/memory/src/main.rs#L809) | Local | `parse_timestamp_millis::second` | — |
| [810](benchmarks/memory/src/main.rs#L810) | Local | `parse_timestamp_millis::millis` | — |
| [819](benchmarks/memory/src/main.rs#L819) | Local | `parse_timestamp_millis::leap_year` | — |
| [820](benchmarks/memory/src/main.rs#L820) | Local | `parse_timestamp_millis::month_lengths` | — |
| [834](benchmarks/memory/src/main.rs#L834) | Local | `parse_timestamp_millis::month_index` | — |
| [839](benchmarks/memory/src/main.rs#L839) | Local | `parse_timestamp_millis::previous_year` | — |
| [840](benchmarks/memory/src/main.rs#L840) | Local | `parse_timestamp_millis::days_before_year` | — |
| [841](benchmarks/memory/src/main.rs#L841) | Local | `parse_timestamp_millis::days_before_month` | `u64` |
| [842](benchmarks/memory/src/main.rs#L842) | Local | `parse_timestamp_millis::days` | — |
| [847](benchmarks/memory/src/main.rs#L847) | Local | `find_json_string::value` | — |
| [848](benchmarks/memory/src/main.rs#L848) | Local | `find_json_string::value` | — |
| [851](benchmarks/memory/src/main.rs#L851) | Local | `find_json_string::end` | — |
| [858](benchmarks/memory/src/main.rs#L858) | Local | `find_json_u64::value` | — |
| [859](benchmarks/memory/src/main.rs#L859) | Local | `find_json_u64::end` | — |
| [871](benchmarks/memory/src/main.rs#L871) | Local | `field_value::needle` | — |
| [872](benchmarks/memory/src/main.rs#L872) | Local | `field_value::after_field` | — |
| [876](benchmarks/memory/src/main.rs#L876) | Local | `field_value::after_colon` | — |
| [892](benchmarks/memory/src/main.rs#L892) | Local | `byte_count_from_env::value` | — |
| [897](benchmarks/memory/src/main.rs#L897) | Local | `byte_count_from_env::bytes` | — |
| [902](benchmarks/memory/src/main.rs#L902) | Local | `parse_bytes::bytes` | — |
| [907](benchmarks/memory/src/main.rs#L907) | Local | `parse_byte_count::value` | — |
| [908](benchmarks/memory/src/main.rs#L908) | Local | `parse_byte_count::split` | — |
| [911](benchmarks/memory/src/main.rs#L911) | Local | `parse_byte_count::number` | `u128` |
| [914](benchmarks/memory/src/main.rs#L914) | Local | `parse_byte_count::suffix` | — |
| [915](benchmarks/memory/src/main.rs#L915) | Local | `parse_byte_count::multiplier` | — |
| [986](benchmarks/memory/src/main.rs#L986) | Local | `tests::warmup_preserves_cache_state_but_not_reported_traffic::workload` | — |
| [1000](benchmarks/memory/src/main.rs#L1000) | Local | `tests::warmup_preserves_cache_state_but_not_reported_traffic::report` | — |
| [1019](benchmarks/memory/src/main.rs#L1019) | Local | `tests::foyer_expanded_key_reuses_identical_expansions_for_distinct_requests::access` | — |
| [1025](benchmarks/memory/src/main.rs#L1025) | Local | `tests::foyer_expanded_key_reuses_identical_expansions_for_distinct_requests::first` | — |
| [1026](benchmarks/memory/src/main.rs#L1026) | Local | `tests::foyer_expanded_key_reuses_identical_expansions_for_distinct_requests::second` | — |
| [1027](benchmarks/memory/src/main.rs#L1027) | Local | `tests::foyer_expanded_key_reuses_identical_expansions_for_distinct_requests::expanded` | — |
| [1029](benchmarks/memory/src/main.rs#L1029) | Local | `tests::foyer_expanded_key_reuses_identical_expansions_for_distinct_requests::mut expanded_key` | — |
| [1041](benchmarks/memory/src/main.rs#L1041) | Local | `tests::foyer_expanded_key_reuses_identical_expansions_for_distinct_requests::mut exact_key` | — |
| [1049](benchmarks/memory/src/main.rs#L1049) | Local | `tests::expanded_downloader_assigns_one_coalesced_range_to_all_batch_members::access` | — |
| [1055](benchmarks/memory/src/main.rs#L1055) | Local | `tests::expanded_downloader_assigns_one_coalesced_range_to_all_batch_members::accesses` | — |
| [1061](benchmarks/memory/src/main.rs#L1061) | Local | `tests::expanded_downloader_assigns_one_coalesced_range_to_all_batch_members::download_config` | — |
| [1065](benchmarks/memory/src/main.rs#L1065) | Local | `tests::expanded_downloader_assigns_one_coalesced_range_to_all_batch_members::expanded_downloads` | — |
| [1066](benchmarks/memory/src/main.rs#L1066) | Local | `tests::expanded_downloader_assigns_one_coalesced_range_to_all_batch_members::workload` | — |
| [1074](benchmarks/memory/src/main.rs#L1074) | Local | `tests::expanded_downloader_assigns_one_coalesced_range_to_all_batch_members::report` | — |
| [1093](benchmarks/memory/src/main.rs#L1093) | Local | `tests::coalescing_distance_parameter_is_a_strict_upper_bound::distance` | — |
| [1094](benchmarks/memory/src/main.rs#L1094) | Local | `tests::coalescing_distance_parameter_is_a_strict_upper_bound::ranges_for_gap` | — |
| [1095](benchmarks/memory/src/main.rs#L1095) | Local | `tests::coalescing_distance_parameter_is_a_strict_upper_bound::ranges_for_gap::accesses` | — |
| [1127](benchmarks/memory/src/main.rs#L1127) | Local | `tests::source_cost_baseline_uses_requested_bytes_not_downloaded_bytes::report` | — |
| [1149](benchmarks/memory/src/main.rs#L1149) | Local | `tests::parses_the_captured_trace_shape::access` | — |
| [1164](benchmarks/memory/src/main.rs#L1164) | Local | `tests::expanded_downloader_honors_the_whole_split_threshold::config` | — |
| [1168](benchmarks/memory/src/main.rs#L1168) | Local | `tests::expanded_downloader_honors_the_whole_split_threshold::small_split` | — |
| [1183](benchmarks/memory/src/main.rs#L1183) | Local | `tests::expanded_downloader_honors_the_whole_split_threshold::threshold_split` | — |
| [1197](benchmarks/memory/src/main.rs#L1197) | Local | `tests::timestamp_parser_handles_day_boundaries::before` | — |
| [1198](benchmarks/memory/src/main.rs#L1198) | Local | `tests::timestamp_parser_handles_day_boundaries::after` | — |

</details>

## feuer-storage

### `feuer-storage/examples/direct_io.rs`

| Line | Kind | Name / source parent | Signature or type |
| ---: | --- | --- | --- |
| [13](feuer-storage/examples/direct_io.rs#L13) | Const | `MIB` | `usize` |
| [14](feuer-storage/examples/direct_io.rs#L14) | Const | `CAPACITY` | `u64` |
| [15](feuer-storage/examples/direct_io.rs#L15) | Const | `READ_SPACE` | `u64` |
| [19](feuer-storage/examples/direct_io.rs#L19) | Struct | `IoMeasurements` | — |
| [20](feuer-storage/examples/direct_io.rs#L20) | Field | `IoMeasurements::operations` | `u64` |
| [21](feuer-storage/examples/direct_io.rs#L21) | Field | `IoMeasurements::bytes` | `u64` |
| [22](feuer-storage/examples/direct_io.rs#L22) | Field | `IoMeasurements::latency_samples_micros` | `Vec<u64>` |
| [26](feuer-storage/examples/direct_io.rs#L26) | Function | `main` | `fn() -> Result<(), Box<dyn std::error::Error>>` |
| [66](feuer-storage/examples/direct_io.rs#L66) | Function | `run` | `fn(file: &DataFile, size: usize, readers: usize, writers: usize, seconds: u64)` |
| [145](feuer-storage/examples/direct_io.rs#L145) | Function | `cpu_seconds` | `fn() -> f64` |

<details>
<summary>Local bindings (35)</summary>

| Line | Kind | Name / source parent | Signature or type |
| ---: | --- | --- | --- |
| [27](feuer-storage/examples/direct_io.rs#L27) | Local | `main::mut args` | — |
| [28](feuer-storage/examples/direct_io.rs#L28) | Local | `main::root` | — |
| [29](feuer-storage/examples/direct_io.rs#L29) | Local | `main::seconds` | `u64` |
| [31](feuer-storage/examples/direct_io.rs#L31) | Local | `main::write_callers` | `Vec<usize>` |
| [40](feuer-storage/examples/direct_io.rs#L40) | Local | `main::temp` | — |
| [43](feuer-storage/examples/direct_io.rs#L43) | Local | `main::registry` | `mixtrics::metrics::BoxedRegistry` |
| [44](feuer-storage/examples/direct_io.rs#L44) | Local | `main::file` | — |
| [46](feuer-storage/examples/direct_io.rs#L46) | Local | `main::payload` | — |
| [67](feuer-storage/examples/direct_io.rs#L67) | Local | `run::measure_start` | — |
| [68](feuer-storage/examples/direct_io.rs#L68) | Local | `run::deadline` | — |
| [69](feuer-storage/examples/direct_io.rs#L69) | Local | `run::mut tasks` | — |
| [72](feuer-storage/examples/direct_io.rs#L72) | Local | `run::payload` | — |
| [74](feuer-storage/examples/direct_io.rs#L74) | Local | `run::file` | — |
| [75](feuer-storage/examples/direct_io.rs#L75) | Local | `run::payload` | — |
| [77](feuer-storage/examples/direct_io.rs#L77) | Local | `run::reading` | — |
| [78](feuer-storage/examples/direct_io.rs#L78) | Local | `run::mut random` | — |
| [79](feuer-storage/examples/direct_io.rs#L79) | Local | `run::mut measurements` | — |
| [80](feuer-storage/examples/direct_io.rs#L80) | Local | `run::mut write_offset` | — |
| [81](feuer-storage/examples/direct_io.rs#L81) | Local | `run::lane_size` | — |
| [83](feuer-storage/examples/direct_io.rs#L83) | Local | `run::started` | — |
| [84](feuer-storage/examples/direct_io.rs#L84) | Local | `run::bytes` | — |
| [88](feuer-storage/examples/direct_io.rs#L88) | Local | `run::bytes::offset` | — |
| [89](feuer-storage/examples/direct_io.rs#L89) | Local | `run::bytes::value` | — |
| [95](feuer-storage/examples/direct_io.rs#L95) | Local | `run::bytes::offset` | — |
| [100](feuer-storage/examples/direct_io.rs#L100) | Local | `run::finished` | — |
| [116](feuer-storage/examples/direct_io.rs#L116) | Local | `run::cpu_start` | — |
| [117](feuer-storage/examples/direct_io.rs#L117) | Local | `run::mut read` | — |
| [118](feuer-storage/examples/direct_io.rs#L118) | Local | `run::mut write` | — |
| [120](feuer-storage/examples/direct_io.rs#L120) | Local | `run::(reading, measurements)` | — |
| [121](feuer-storage/examples/direct_io.rs#L121) | Local | `run::total` | — |
| [126](feuer-storage/examples/direct_io.rs#L126) | Local | `run::cpu` | — |
| [128](feuer-storage/examples/direct_io.rs#L128) | Local | `run::percentile` | — |
| [146](feuer-storage/examples/direct_io.rs#L146) | Local | `cpu_seconds::mut usage` | — |
| [150](feuer-storage/examples/direct_io.rs#L150) | Local | `cpu_seconds::usage` | — |
| [151](feuer-storage/examples/direct_io.rs#L151) | Local | `cpu_seconds::seconds` | — |

</details>

### `feuer-storage/src/error.rs`

| Line | Kind | Name / source parent | Signature or type |
| ---: | --- | --- | --- |
| [5](feuer-storage/src/error.rs#L5) | Enum | `IoOperation` | — |
| [7](feuer-storage/src/error.rs#L7) | Variant | `IoOperation::CreateDirectory` | — |
| [9](feuer-storage/src/error.rs#L9) | Variant | `IoOperation::OpenLockFile` | — |
| [11](feuer-storage/src/error.rs#L11) | Variant | `IoOperation::LockDirectory` | — |
| [13](feuer-storage/src/error.rs#L13) | Variant | `IoOperation::OpenDataFile` | — |
| [15](feuer-storage/src/error.rs#L15) | Variant | `IoOperation::InspectDataFile` | — |
| [17](feuer-storage/src/error.rs#L17) | Variant | `IoOperation::ResizeDataFile` | — |
| [19](feuer-storage/src/error.rs#L19) | Variant | `IoOperation::Read` | — |
| [21](feuer-storage/src/error.rs#L21) | Variant | `IoOperation::Write` | — |
| [24](feuer-storage/src/error.rs#L24) | Impl | `impl IoOperation` | — |
| [26](feuer-storage/src/error.rs#L26) | Method | `impl IoOperation::as_str` | `fn(self) -> &'static str` |
| [40](feuer-storage/src/error.rs#L40) | Impl | `impl fmt::Display for IoOperation` | — |
| [41](feuer-storage/src/error.rs#L41) | Method | `impl fmt::Display for IoOperation::fmt` | `fn(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result` |
| [48](feuer-storage/src/error.rs#L48) | Enum | `ErrorKind` | — |
| [50](feuer-storage/src/error.rs#L50) | Variant | `ErrorKind::InvalidConfiguration` | — |
| [52](feuer-storage/src/error.rs#L52) | Variant | `ErrorKind::AlreadyOpen` | — |
| [54](feuer-storage/src/error.rs#L54) | Variant | `ErrorKind::OutOfBounds` | — |
| [56](feuer-storage/src/error.rs#L56) | Variant | `ErrorKind::Allocation` | — |
| [58](feuer-storage/src/error.rs#L58) | Variant | `ErrorKind::Io` | — |
| [60](feuer-storage/src/error.rs#L60) | Variant | `ErrorKind::Task` | — |
| [63](feuer-storage/src/error.rs#L63) | Impl | `impl ErrorKind` | — |
| [65](feuer-storage/src/error.rs#L65) | Method | `impl ErrorKind::as_str` | `fn(self) -> &'static str` |
| [77](feuer-storage/src/error.rs#L77) | Impl | `impl fmt::Display for ErrorKind` | — |
| [78](feuer-storage/src/error.rs#L78) | Method | `impl fmt::Display for ErrorKind::fmt` | `fn(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result` |
| [86](feuer-storage/src/error.rs#L86) | Enum | `Error` | — |
| [89](feuer-storage/src/error.rs#L89) | Variant | `Error::InvalidCapacity` | — |
| [92](feuer-storage/src/error.rs#L92) | Variant | `Error::AlreadyOpen` | — |
| [94](feuer-storage/src/error.rs#L94) | Field | `Error::AlreadyOpen::directory` | `PathBuf` |
| [98](feuer-storage/src/error.rs#L98) | Variant | `Error::InvalidDataFile` | — |
| [100](feuer-storage/src/error.rs#L100) | Field | `Error::InvalidDataFile::path` | `PathBuf` |
| [104](feuer-storage/src/error.rs#L104) | Variant | `Error::OutOfBounds` | — |
| [106](feuer-storage/src/error.rs#L106) | Field | `Error::OutOfBounds::operation` | `IoOperation` |
| [108](feuer-storage/src/error.rs#L108) | Field | `Error::OutOfBounds::offset` | `u64` |
| [110](feuer-storage/src/error.rs#L110) | Field | `Error::OutOfBounds::length` | `u64` |
| [112](feuer-storage/src/error.rs#L112) | Field | `Error::OutOfBounds::capacity` | `u64` |
| [116](feuer-storage/src/error.rs#L116) | Variant | `Error::LengthOverflow` | — |
| [118](feuer-storage/src/error.rs#L118) | Field | `Error::LengthOverflow::operation` | `IoOperation` |
| [120](feuer-storage/src/error.rs#L120) | Field | `Error::LengthOverflow::length` | `usize` |
| [124](feuer-storage/src/error.rs#L124) | Variant | `Error::Allocation` | — |
| [126](feuer-storage/src/error.rs#L126) | Field | `Error::Allocation::length` | `usize` |
| [129](feuer-storage/src/error.rs#L129) | Field | `Error::Allocation::source` | `TryReserveError` |
| [133](feuer-storage/src/error.rs#L133) | Variant | `Error::Io` | — |
| [135](feuer-storage/src/error.rs#L135) | Field | `Error::Io::operation` | `IoOperation` |
| [137](feuer-storage/src/error.rs#L137) | Field | `Error::Io::path` | `PathBuf` |
| [140](feuer-storage/src/error.rs#L140) | Field | `Error::Io::source` | `io::Error` |
| [144](feuer-storage/src/error.rs#L144) | Variant | `Error::RuntimeUnavailable` | — |
| [147](feuer-storage/src/error.rs#L147) | Variant | `Error::Task` | — |
| [149](feuer-storage/src/error.rs#L149) | Field | `Error::Task::operation` | `IoOperation` |
| [152](feuer-storage/src/error.rs#L152) | Field | `Error::Task::source` | `Box<dyn std::error::Error + Send + Sync>` |
| [156](feuer-storage/src/error.rs#L156) | Impl | `impl Error` | — |
| [158](feuer-storage/src/error.rs#L158) | Method | `impl Error::kind` | `fn(&self) -> ErrorKind` |
| [170](feuer-storage/src/error.rs#L170) | Method | `impl Error::operation` | `fn(&self) -> IoOperation` |
| [186](feuer-storage/src/error.rs#L186) | TypeAlias | `Result` | `std::result::Result<T, Error>` |

### `feuer-storage/src/file.rs`

| Line | Kind | Name / source parent | Signature or type |
| ---: | --- | --- | --- |
| [18](feuer-storage/src/file.rs#L18) | Const | `DATA_FILE_NAME` | `&str` |
| [19](feuer-storage/src/file.rs#L19) | Const | `LOCK_FILE_NAME` | `&str` |
| [22](feuer-storage/src/file.rs#L22) | Struct | `DataFileState` | — |
| [23](feuer-storage/src/file.rs#L23) | Field | `DataFileState::queue` | `uring::IoQueueHandle` |
| [24](feuer-storage/src/file.rs#L24) | Field | `DataFileState::data_path` | `PathBuf` |
| [25](feuer-storage/src/file.rs#L25) | Field | `DataFileState::capacity` | `u64` |
| [46](feuer-storage/src/file.rs#L46) | Struct | `DataFile` | — |
| [47](feuer-storage/src/file.rs#L47) | Field | `DataFile::state` | `Arc<DataFileState>` |
| [48](feuer-storage/src/file.rs#L48) | Field | `DataFile::metrics` | `Arc<IoMetrics>` |
| [51](feuer-storage/src/file.rs#L51) | Impl | `impl fmt::Debug for DataFile` | — |
| [52](feuer-storage/src/file.rs#L52) | Method | `impl fmt::Debug for DataFile::fmt` | `fn(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result` |
| [59](feuer-storage/src/file.rs#L59) | Impl | `impl DataFile` | — |
| [65](feuer-storage/src/file.rs#L65) | Function | `impl DataFile::open` | `fn(directory: impl AsRef<Path>, capacity: u64, metrics: Arc<IoMetrics>) -> Result<Self>` |
| [107](feuer-storage/src/file.rs#L107) | Method | `impl DataFile::capacity` | `fn(&self) -> u64` |
| [113](feuer-storage/src/file.rs#L113) | Method | `impl DataFile::read_at` | `fn(&self, offset: u64, length: usize) -> Result<Bytes>` |
| [122](feuer-storage/src/file.rs#L122) | Method | `impl DataFile::write_at` | `fn(&self, offset: u64, bytes: &Bytes) -> Result<()>` |
| [128](feuer-storage/src/file.rs#L128) | Method | `impl DataFile::execute` | `fn(&self, operation: IoOperation, offset: u64, length: usize, payload: &[u8]) -> Result<Bytes>` |
| [151](feuer-storage/src/file.rs#L151) | Method | `impl DataFile::execute_inner` | `fn(&self, operation: IoOperation, offset: u64, length: usize, payload: &[u8]) -> Result<Bytes>` |
| [195](feuer-storage/src/file.rs#L195) | Function | `open_file_state` | `fn(directory: PathBuf, capacity: u64) -> Result<DataFileState>` |
| [263](feuer-storage/src/file.rs#L263) | Function | `check_direct_alignment` | `fn(file: &File) -> io::Result<()>` |
| [294](feuer-storage/src/file.rs#L294) | Function | `check_range` | `fn(operation: IoOperation, offset: u64, length: u64, capacity: u64) -> Result<()>` |
| [306](feuer-storage/src/file.rs#L306) | Function | `record_span_outcome` | `fn<T>(span: &Span, elapsed: std::time::Duration, result: &Result<T>)` |
| [320](feuer-storage/src/file.rs#L320) | Module | `tests` | — |
| [325](feuer-storage/src/file.rs#L325) | Const | `tests::CAPACITY` | `u64` |
| [328](feuer-storage/src/file.rs#L328) | Function | `tests::public_io_types_are_send_sync_static` | `fn()` |
| [329](feuer-storage/src/file.rs#L329) | Function | `tests::public_io_types_are_send_sync_static::assert_send_sync_static` | `fn<T: Send + Sync + 'static>()` |
| [335](feuer-storage/src/file.rs#L335) | Function | `tests::reads_exact_unaligned_ranges_and_preserves_neighbors` | `fn()` |
| [362](feuer-storage/src/file.rs#L362) | Function | `tests::rejects_out_of_bounds_and_accepts_empty_ranges` | `fn()` |
| [381](feuer-storage/src/file.rs#L381) | Function | `tests::fails_short_reads_instead_of_returning_uncertain_bytes` | `fn()` |
| [397](feuer-storage/src/file.rs#L397) | Function | `tests::holds_exclusive_ownership_and_reopens_after_shutdown` | `fn()` |
| [419](feuer-storage/src/file.rs#L419) | Function | `tests::concurrent_mixed_io_and_same_page_rmw_do_not_lose_updates` | `fn()` |
| [450](feuer-storage/src/file.rs#L450) | Function | `tests::canceled_callers_do_not_release_submitted_buffers_or_lock_early` | `fn()` |
| [479](feuer-storage/src/file.rs#L479) | Function | `tests::reports_missing_runtime` | `fn()` |
| [494](feuer-storage/src/file.rs#L494) | Function | `tests::rejects_unverified_direct_io_instead_of_falling_back` | `fn()` |
| [510](feuer-storage/src/file.rs#L510) | Function | `tests::validates_capacity_and_omits_paths_from_debug` | `fn()` |

<details>
<summary>Local bindings (65)</summary>

| Line | Kind | Name / source parent | Signature or type |
| ---: | --- | --- | --- |
| [66](feuer-storage/src/file.rs#L66) | Local | `impl DataFile::open::directory` | — |
| [67](feuer-storage/src/file.rs#L67) | Local | `impl DataFile::open::runtime` | — |
| [68](feuer-storage/src/file.rs#L68) | Local | `impl DataFile::open::started` | — |
| [69](feuer-storage/src/file.rs#L69) | Local | `impl DataFile::open::span` | — |
| [77](feuer-storage/src/file.rs#L77) | Local | `impl DataFile::open::result` | — |
| [78](feuer-storage/src/file.rs#L78) | Local | `impl DataFile::open::result::state` | — |
| [129](feuer-storage/src/file.rs#L129) | Local | `impl DataFile::execute::started` | — |
| [130](feuer-storage/src/file.rs#L130) | Local | `impl DataFile::execute::observed_bytes` | — |
| [131](feuer-storage/src/file.rs#L131) | Local | `impl DataFile::execute::span` | — |
| [141](feuer-storage/src/file.rs#L141) | Local | `impl DataFile::execute::result` | — |
| [145](feuer-storage/src/file.rs#L145) | Local | `impl DataFile::execute::elapsed` | — |
| [152](feuer-storage/src/file.rs#L152) | Local | `impl DataFile::execute_inner::length_u64` | — |
| [154](feuer-storage/src/file.rs#L154) | Local | `impl DataFile::execute_inner::io_error` | — |
| [159](feuer-storage/src/file.rs#L159) | Local | `impl DataFile::execute_inner::mut result` | — |
| [167](feuer-storage/src/file.rs#L167) | Local | `impl DataFile::execute_inner::mut completed_bytes` | — |
| [169](feuer-storage/src/file.rs#L169) | Local | `impl DataFile::execute_inner::chunk_offset` | — |
| [170](feuer-storage/src/file.rs#L170) | Local | `impl DataFile::execute_inner::chunk_length` | — |
| [172](feuer-storage/src/file.rs#L172) | Local | `impl DataFile::execute_inner::input` | — |
| [177](feuer-storage/src/file.rs#L177) | Local | `impl DataFile::execute_inner::bytes` | — |
| [205](feuer-storage/src/file.rs#L205) | Local | `open_file_state::lock_path` | — |
| [206](feuer-storage/src/file.rs#L206) | Local | `open_file_state::lock_file` | — |
| [217](feuer-storage/src/file.rs#L217) | Local | `open_file_state::locked` | — |
| [225](feuer-storage/src/file.rs#L225) | Local | `open_file_state::data_path` | — |
| [226](feuer-storage/src/file.rs#L226) | Local | `open_file_state::error` | — |
| [231](feuer-storage/src/file.rs#L231) | Local | `open_file_state::file` | — |
| [248](feuer-storage/src/file.rs#L248) | Local | `open_file_state::resize_file` | — |
| [251](feuer-storage/src/file.rs#L251) | Local | `open_file_state::queue` | — |
| [264](feuer-storage/src/file.rs#L264) | Local | `check_direct_alignment::mut stat` | — |
| [266](feuer-storage/src/file.rs#L266) | Local | `check_direct_alignment::result` | — |
| [279](feuer-storage/src/file.rs#L279) | Local | `check_direct_alignment::stat` | — |
| [336](feuer-storage/src/file.rs#L336) | Local | `tests::reads_exact_unaligned_ranges_and_preserves_neighbors::temp` | — |
| [337](feuer-storage/src/file.rs#L337) | Local | `tests::reads_exact_unaligned_ranges_and_preserves_neighbors::directory` | — |
| [338](feuer-storage/src/file.rs#L338) | Local | `tests::reads_exact_unaligned_ranges_and_preserves_neighbors::file` | — |
| [339](feuer-storage/src/file.rs#L339) | Local | `tests::reads_exact_unaligned_ranges_and_preserves_neighbors::original` | — |
| [341](feuer-storage/src/file.rs#L341) | Local | `tests::reads_exact_unaligned_ranges_and_preserves_neighbors::payload` | — |
| [350](feuer-storage/src/file.rs#L350) | Local | `tests::reads_exact_unaligned_ranges_and_preserves_neighbors::payload` | — |
| [363](feuer-storage/src/file.rs#L363) | Local | `tests::rejects_out_of_bounds_and_accepts_empty_ranges::temp` | — |
| [364](feuer-storage/src/file.rs#L364) | Local | `tests::rejects_out_of_bounds_and_accepts_empty_ranges::file` | — |
| [382](feuer-storage/src/file.rs#L382) | Local | `tests::fails_short_reads_instead_of_returning_uncertain_bytes::temp` | — |
| [383](feuer-storage/src/file.rs#L383) | Local | `tests::fails_short_reads_instead_of_returning_uncertain_bytes::file` | — |
| [398](feuer-storage/src/file.rs#L398) | Local | `tests::holds_exclusive_ownership_and_reopens_after_shutdown::temp` | — |
| [399](feuer-storage/src/file.rs#L399) | Local | `tests::holds_exclusive_ownership_and_reopens_after_shutdown::file` | — |
| [400](feuer-storage/src/file.rs#L400) | Local | `tests::holds_exclusive_ownership_and_reopens_after_shutdown::clone` | — |
| [411](feuer-storage/src/file.rs#L411) | Local | `tests::holds_exclusive_ownership_and_reopens_after_shutdown::file` | — |
| [420](feuer-storage/src/file.rs#L420) | Local | `tests::concurrent_mixed_io_and_same_page_rmw_do_not_lose_updates::temp` | — |
| [421](feuer-storage/src/file.rs#L421) | Local | `tests::concurrent_mixed_io_and_same_page_rmw_do_not_lose_updates::file` | — |
| [422](feuer-storage/src/file.rs#L422) | Local | `tests::concurrent_mixed_io_and_same_page_rmw_do_not_lose_updates::mut tasks` | — |
| [426](feuer-storage/src/file.rs#L426) | Local | `tests::concurrent_mixed_io_and_same_page_rmw_do_not_lose_updates::file` | — |
| [428](feuer-storage/src/file.rs#L428) | Local | `tests::concurrent_mixed_io_and_same_page_rmw_do_not_lose_updates::value` | — |
| [431](feuer-storage/src/file.rs#L431) | Local | `tests::concurrent_mixed_io_and_same_page_rmw_do_not_lose_updates::value` | — |
| [432](feuer-storage/src/file.rs#L432) | Local | `tests::concurrent_mixed_io_and_same_page_rmw_do_not_lose_updates::offset` | — |
| [451](feuer-storage/src/file.rs#L451) | Local | `tests::canceled_callers_do_not_release_submitted_buffers_or_lock_early::temp` | — |
| [452](feuer-storage/src/file.rs#L452) | Local | `tests::canceled_callers_do_not_release_submitted_buffers_or_lock_early::file` | — |
| [453](feuer-storage/src/file.rs#L453) | Local | `tests::canceled_callers_do_not_release_submitted_buffers_or_lock_early::mut tasks` | — |
| [455](feuer-storage/src/file.rs#L455) | Local | `tests::canceled_callers_do_not_release_submitted_buffers_or_lock_early::file` | — |
| [465](feuer-storage/src/file.rs#L465) | Local | `tests::canceled_callers_do_not_release_submitted_buffers_or_lock_early::_` | — |
| [468](feuer-storage/src/file.rs#L468) | Local | `tests::canceled_callers_do_not_release_submitted_buffers_or_lock_early::file` | — |
| [484](feuer-storage/src/file.rs#L484) | Local | `tests::reports_missing_runtime::temp` | — |
| [485](feuer-storage/src/file.rs#L485) | Local | `tests::reports_missing_runtime::mut open` | — |
| [486](feuer-storage/src/file.rs#L486) | Local | `tests::reports_missing_runtime::mut context` | — |
| [487](feuer-storage/src/file.rs#L487) | Local | `tests::reports_missing_runtime::Poll::Ready(result)` | — |
| [499](feuer-storage/src/file.rs#L499) | Local | `tests::rejects_unverified_direct_io_instead_of_falling_back::fd` | — |
| [502](feuer-storage/src/file.rs#L502) | Local | `tests::rejects_unverified_direct_io_instead_of_falling_back::file` | — |
| [511](feuer-storage/src/file.rs#L511) | Local | `tests::validates_capacity_and_omits_paths_from_debug::temp` | — |
| [521](feuer-storage/src/file.rs#L521) | Local | `tests::validates_capacity_and_omits_paths_from_debug::file` | — |

</details>

### `feuer-storage/src/lib.rs`

| Line | Kind | Name / source parent | Signature or type |
| ---: | --- | --- | --- |
| [9](feuer-storage/src/lib.rs#L9) | Module | `error` | — |
| [11](feuer-storage/src/lib.rs#L11) | Module | `file` | — |
| [12](feuer-storage/src/lib.rs#L12) | Module | `metrics` | — |
| [14](feuer-storage/src/lib.rs#L14) | Module | `uring` | — |

### `feuer-storage/src/metrics.rs`

| Line | Kind | Name / source parent | Signature or type |
| ---: | --- | --- | --- |
| [7](feuer-storage/src/metrics.rs#L7) | Struct | `OperationMetrics` | — |
| [8](feuer-storage/src/metrics.rs#L8) | Field | `OperationMetrics::success` | `BoxedCounter` |
| [9](feuer-storage/src/metrics.rs#L9) | Field | `OperationMetrics::error` | `BoxedCounter` |
| [10](feuer-storage/src/metrics.rs#L10) | Field | `OperationMetrics::bytes` | `BoxedCounter` |
| [11](feuer-storage/src/metrics.rs#L11) | Field | `OperationMetrics::success_duration` | `BoxedHistogram` |
| [12](feuer-storage/src/metrics.rs#L12) | Field | `OperationMetrics::error_duration` | `BoxedHistogram` |
| [15](feuer-storage/src/metrics.rs#L15) | Impl | `impl fmt::Debug for OperationMetrics` | — |
| [16](feuer-storage/src/metrics.rs#L16) | Method | `impl fmt::Debug for OperationMetrics::fmt` | `fn(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result` |
| [27](feuer-storage/src/metrics.rs#L27) | Struct | `IoMetrics` | — |
| [28](feuer-storage/src/metrics.rs#L28) | Field | `IoMetrics::read` | `OperationMetrics` |
| [29](feuer-storage/src/metrics.rs#L29) | Field | `IoMetrics::write` | `OperationMetrics` |
| [32](feuer-storage/src/metrics.rs#L32) | Impl | `impl IoMetrics` | — |
| [34](feuer-storage/src/metrics.rs#L34) | Function | `impl IoMetrics::new` | `fn(registry: &BoxedRegistry) -> Arc<Self>` |
| [66](feuer-storage/src/metrics.rs#L66) | Method | `impl IoMetrics::record` | `fn(&self, operation: IoOperation, bytes: u64, elapsed: Duration, success: bool)` |
| [84](feuer-storage/src/metrics.rs#L84) | Function | `impl IoMetrics::noop` | `fn() -> Arc<Self>` |
| [91](feuer-storage/src/metrics.rs#L91) | Module | `tests` | — |
| [95](feuer-storage/src/metrics.rs#L95) | Function | `tests::registers_with_the_normal_registry_boundary` | `fn()` |

<details>
<summary>Local bindings (7)</summary>

| Line | Kind | Name / source parent | Signature or type |
| ---: | --- | --- | --- |
| [35](feuer-storage/src/metrics.rs#L35) | Local | `impl IoMetrics::new::operations` | — |
| [40](feuer-storage/src/metrics.rs#L40) | Local | `impl IoMetrics::new::bytes` | — |
| [45](feuer-storage/src/metrics.rs#L45) | Local | `impl IoMetrics::new::duration` | — |
| [52](feuer-storage/src/metrics.rs#L52) | Local | `impl IoMetrics::new::operation` | — |
| [67](feuer-storage/src/metrics.rs#L67) | Local | `impl IoMetrics::record::metrics` | — |
| [85](feuer-storage/src/metrics.rs#L85) | Local | `impl IoMetrics::noop::registry` | `BoxedRegistry` |
| [96](feuer-storage/src/metrics.rs#L96) | Local | `tests::registers_with_the_normal_registry_boundary::metrics` | — |

</details>

### `feuer-storage/src/uring/tests.rs`

| Line | Kind | Name / source parent | Signature or type |
| ---: | --- | --- | --- |
| [5](feuer-storage/src/uring/tests.rs#L5) | TypeAlias | `IoResultReceiver` | `oneshot::Receiver<io::Result<Bytes>>` |
| [7](feuer-storage/src/uring/tests.rs#L7) | Function | `request` | `fn(driver: &IoQueue, operation: IoOperation, offset: u64, length: usize) -> (IoRequest, IoResultReceiver)` |
| [34](feuer-storage/src/uring/tests.rs#L34) | Function | `driver` | `fn() -> IoQueue` |
| [62](feuer-storage/src/uring/tests.rs#L62) | Function | `fills_qd64_with_simultaneous_reads_and_writes_and_bounds_admission` | `fn()` |
| [129](feuer-storage/src/uring/tests.rs#L129) | Function | `complete_one` | `fn(driver: &mut IoQueue)` |
| [138](feuer-storage/src/uring/tests.rs#L138) | Function | `write_only_fills_the_ring_but_an_older_queued_read_gets_the_next_slot` | `fn()` |
| [192](feuer-storage/src/uring/tests.rs#L192) | Function | `active_read_does_not_throttle_rmw_writes` | `fn()` |
| [227](feuer-storage/src/uring/tests.rs#L227) | Function | `full_write_admission_and_buffers_leave_a_full_read_ring_available` | `fn()` |
| [259](feuer-storage/src/uring/tests.rs#L259) | Function | `overlap_blocks_only_the_requests_it_must` | `fn()` |
| [297](feuer-storage/src/uring/tests.rs#L297) | Function | `discarded_queued_requests_never_reach_the_ring` | `fn()` |
| [316](feuer-storage/src/uring/tests.rs#L316) | Function | `completion_state_handles_short_io_errors_and_rmw` | `fn()` |
| [343](feuer-storage/src/uring/tests.rs#L343) | Function | `byte_budget_bounds_rmw_requests_and_releases_on_cancel` | `fn()` |

<details>
<summary>Local bindings (41)</summary>

| Line | Kind | Name / source parent | Signature or type |
| ---: | --- | --- | --- |
| [8](feuer-storage/src/uring/tests.rs#L8) | Local | `request::(reply, receive)` | — |
| [9](feuer-storage/src/uring/tests.rs#L9) | Local | `request::class` | — |
| [10](feuer-storage/src/uring/tests.rs#L10) | Local | `request::request_permit` | — |
| [11](feuer-storage/src/uring/tests.rs#L11) | Local | `request::staging_permit` | — |
| [15](feuer-storage/src/uring/tests.rs#L15) | Local | `request::payload` | — |
| [35](feuer-storage/src/uring/tests.rs#L35) | Local | `driver::temporary` | — |
| [36](feuer-storage/src/uring/tests.rs#L36) | Local | `driver::file` | — |
| [44](feuer-storage/src/uring/tests.rs#L44) | Local | `driver::fd` | — |
| [47](feuer-storage/src/uring/tests.rs#L47) | Local | `driver::wake` | — |
| [48](feuer-storage/src/uring/tests.rs#L48) | Local | `driver::(_, receiver)` | — |
| [63](feuer-storage/src/uring/tests.rs#L63) | Local | `fills_qd64_with_simultaneous_reads_and_writes_and_bounds_admission::mut driver` | — |
| [64](feuer-storage/src/uring/tests.rs#L64) | Local | `fills_qd64_with_simultaneous_reads_and_writes_and_bounds_admission::mut replies` | — |
| [66](feuer-storage/src/uring/tests.rs#L66) | Local | `fills_qd64_with_simultaneous_reads_and_writes_and_bounds_admission::operation` | — |
| [71](feuer-storage/src/uring/tests.rs#L71) | Local | `fills_qd64_with_simultaneous_reads_and_writes_and_bounds_admission::(request, reply)` | — |
| [131](feuer-storage/src/uring/tests.rs#L131) | Local | `complete_one::cqe` | — |
| [132](feuer-storage/src/uring/tests.rs#L132) | Local | `complete_one::mut request` | — |
| [139](feuer-storage/src/uring/tests.rs#L139) | Local | `write_only_fills_the_ring_but_an_older_queued_read_gets_the_next_slot::mut driver` | — |
| [140](feuer-storage/src/uring/tests.rs#L140) | Local | `write_only_fills_the_ring_but_an_older_queued_read_gets_the_next_slot::mut replies` | — |
| [142](feuer-storage/src/uring/tests.rs#L142) | Local | `write_only_fills_the_ring_but_an_older_queued_read_gets_the_next_slot::(request, reply)` | — |
| [154](feuer-storage/src/uring/tests.rs#L154) | Local | `write_only_fills_the_ring_but_an_older_queued_read_gets_the_next_slot::(read, read_reply)` | — |
| [165](feuer-storage/src/uring/tests.rs#L165) | Local | `write_only_fills_the_ring_but_an_older_queued_read_gets_the_next_slot::(write, reply)` | — |
| [193](feuer-storage/src/uring/tests.rs#L193) | Local | `active_read_does_not_throttle_rmw_writes::mut driver` | — |
| [194](feuer-storage/src/uring/tests.rs#L194) | Local | `active_read_does_not_throttle_rmw_writes::(read, reply)` | — |
| [196](feuer-storage/src/uring/tests.rs#L196) | Local | `active_read_does_not_throttle_rmw_writes::mut replies` | — |
| [199](feuer-storage/src/uring/tests.rs#L199) | Local | `active_read_does_not_throttle_rmw_writes::(request, reply)` | — |
| [228](feuer-storage/src/uring/tests.rs#L228) | Local | `full_write_admission_and_buffers_leave_a_full_read_ring_available::driver` | — |
| [229](feuer-storage/src/uring/tests.rs#L229) | Local | `full_write_admission_and_buffers_leave_a_full_read_ring_available::mut writes` | — |
| [230](feuer-storage/src/uring/tests.rs#L230) | Local | `full_write_admission_and_buffers_leave_a_full_read_ring_available::mut reads` | — |
| [260](feuer-storage/src/uring/tests.rs#L260) | Local | `overlap_blocks_only_the_requests_it_must::mut driver` | — |
| [261](feuer-storage/src/uring/tests.rs#L261) | Local | `overlap_blocks_only_the_requests_it_must::mut replies` | — |
| [281](feuer-storage/src/uring/tests.rs#L281) | Local | `overlap_blocks_only_the_requests_it_must::(request, reply)` | — |
| [298](feuer-storage/src/uring/tests.rs#L298) | Local | `discarded_queued_requests_never_reach_the_ring::mut driver` | — |
| [299](feuer-storage/src/uring/tests.rs#L299) | Local | `discarded_queued_requests_never_reach_the_ring::(request, reply)` | — |
| [317](feuer-storage/src/uring/tests.rs#L317) | Local | `completion_state_handles_short_io_errors_and_rmw::driver` | — |
| [318](feuer-storage/src/uring/tests.rs#L318) | Local | `completion_state_handles_short_io_errors_and_rmw::(mut read, _reply)` | — |
| [324](feuer-storage/src/uring/tests.rs#L324) | Local | `completion_state_handles_short_io_errors_and_rmw::(mut read, _reply)` | — |
| [326](feuer-storage/src/uring/tests.rs#L326) | Local | `completion_state_handles_short_io_errors_and_rmw::(mut write, _reply)` | — |
| [333](feuer-storage/src/uring/tests.rs#L333) | Local | `completion_state_handles_short_io_errors_and_rmw::(mut rmw, _reply)` | — |
| [344](feuer-storage/src/uring/tests.rs#L344) | Local | `byte_budget_bounds_rmw_requests_and_releases_on_cancel::driver` | — |
| [345](feuer-storage/src/uring/tests.rs#L345) | Local | `byte_budget_bounds_rmw_requests_and_releases_on_cancel::mut requests` | — |
| [353](feuer-storage/src/uring/tests.rs#L353) | Local | `byte_budget_bounds_rmw_requests_and_releases_on_cancel::_read` | — |

</details>

### `feuer-storage/src/uring.rs`

| Line | Kind | Name / source parent | Signature or type |
| ---: | --- | --- | --- |
| [22](feuer-storage/src/uring.rs#L22) | Const | `DIRECT_IO_ALIGNMENT_BYTES` | `usize` |
| [25](feuer-storage/src/uring.rs#L25) | Const | `MAX_IO_CHUNK_BYTES` | `usize` |
| [27](feuer-storage/src/uring.rs#L27) | Const | `MAX_IN_FLIGHT_IO` | `usize` |
| [30](feuer-storage/src/uring.rs#L30) | Const | `MAX_ADMITTED_REQUESTS` | `usize` |
| [34](feuer-storage/src/uring.rs#L34) | Const | `MAX_STAGING_BUFFER_BYTES` | `usize` |
| [37](feuer-storage/src/uring.rs#L37) | Module | `tests` | — |
| [40](feuer-storage/src/uring.rs#L40) | Struct | `IoQueueHandle` | — |
| [42](feuer-storage/src/uring.rs#L42) | Field | `IoQueueHandle::sender` | `Option<mpsc::SyncSender<IoRequest>>` |
| [44](feuer-storage/src/uring.rs#L44) | Field | `IoQueueHandle::wake` | `Arc<OwnedFd>` |
| [46](feuer-storage/src/uring.rs#L46) | Field | `IoQueueHandle::thread` | `Option<JoinHandle<()>>` |
| [48](feuer-storage/src/uring.rs#L48) | Field | `IoQueueHandle::admission` | `Arc<IoAdmissionBudgets>` |
| [53](feuer-storage/src/uring.rs#L53) | Struct | `IoAdmissionBudgets` | — |
| [55](feuer-storage/src/uring.rs#L55) | Field | `IoAdmissionBudgets::requests` | `[Arc<Semaphore>; 2]` |
| [57](feuer-storage/src/uring.rs#L57) | Field | `IoAdmissionBudgets::buffers` | `[Arc<Semaphore>; 2]` |
| [60](feuer-storage/src/uring.rs#L60) | Impl | `impl IoAdmissionBudgets` | — |
| [61](feuer-storage/src/uring.rs#L61) | Function | `impl IoAdmissionBudgets::new` | `fn() -> Self` |
| [71](feuer-storage/src/uring.rs#L71) | Function | `staging_pages_for` | `fn(operation: IoOperation, offset: u64, length: usize) -> u32` |
| [80](feuer-storage/src/uring.rs#L80) | Impl | `impl IoQueueHandle` | — |
| [81](feuer-storage/src/uring.rs#L81) | Function | `impl IoQueueHandle::new` | `fn(file: File, lock: File) -> io::Result<Self>` |
| [115](feuer-storage/src/uring.rs#L115) | Method | `impl IoQueueHandle::execute` | `fn( &self, operation: IoOperation, offset: u64, length: usize, payload: &[u8], ) -> io::Result<Bytes>` |
| [149](feuer-storage/src/uring.rs#L149) | Impl | `impl Drop for IoQueueHandle` | — |
| [150](feuer-storage/src/uring.rs#L150) | Method | `impl Drop for IoQueueHandle::drop` | `fn(&mut self)` |
| [160](feuer-storage/src/uring.rs#L160) | Function | `stopped` | `fn() -> io::Error` |
| [164](feuer-storage/src/uring.rs#L164) | Struct | `AlignedBuffer` | — |
| [166](feuer-storage/src/uring.rs#L166) | Field | `AlignedBuffer::ptr` | `NonNull<u8>` |
| [168](feuer-storage/src/uring.rs#L168) | Field | `AlignedBuffer::layout` | `Layout` |
| [171](feuer-storage/src/uring.rs#L171) | Impl | `impl AlignedBuffer` | — |
| [172](feuer-storage/src/uring.rs#L172) | Function | `impl AlignedBuffer::new` | `fn(length: usize) -> io::Result<Self>` |
| [180](feuer-storage/src/uring.rs#L180) | Method | `impl AlignedBuffer::as_mut_slice` | `fn(&mut self) -> &mut [u8]` |
| [188](feuer-storage/src/uring.rs#L188) | Impl | `impl Send for AlignedBuffer` | — |
| [190](feuer-storage/src/uring.rs#L190) | Impl | `impl Drop for AlignedBuffer` | — |
| [191](feuer-storage/src/uring.rs#L191) | Method | `impl Drop for AlignedBuffer::drop` | `fn(&mut self)` |
| [198](feuer-storage/src/uring.rs#L198) | Struct | `IoRequest` | — |
| [200](feuer-storage/src/uring.rs#L200) | Field | `IoRequest::operation` | `IoOperation` |
| [202](feuer-storage/src/uring.rs#L202) | Field | `IoRequest::aligned_offset` | `u64` |
| [204](feuer-storage/src/uring.rs#L204) | Field | `IoRequest::data_offset_in_buffer` | `usize` |
| [206](feuer-storage/src/uring.rs#L206) | Field | `IoRequest::length` | `usize` |
| [208](feuer-storage/src/uring.rs#L208) | Field | `IoRequest::aligned_length` | `usize` |
| [210](feuer-storage/src/uring.rs#L210) | Field | `IoRequest::io_buffer` | `AlignedBuffer` |
| [212](feuer-storage/src/uring.rs#L212) | Field | `IoRequest::read_modify_write_payload` | `Option<Bytes>` |
| [214](feuer-storage/src/uring.rs#L214) | Field | `IoRequest::reading` | `bool` |
| [216](feuer-storage/src/uring.rs#L216) | Field | `IoRequest::completed_bytes` | `usize` |
| [218](feuer-storage/src/uring.rs#L218) | Field | `IoRequest::reply` | `Option<oneshot::Sender<io::Result<Bytes>>>` |
| [220](feuer-storage/src/uring.rs#L220) | Field | `IoRequest::_request_permit` | `OwnedSemaphorePermit` |
| [222](feuer-storage/src/uring.rs#L222) | Field | `IoRequest::_staging_permit` | `OwnedSemaphorePermit` |
| [225](feuer-storage/src/uring.rs#L225) | Impl | `impl IoRequest` | — |
| [226](feuer-storage/src/uring.rs#L226) | Function | `impl IoRequest::new` | `fn( operation: IoOperation, offset: u64, length: usize, payload: &[u8], reply: oneshot::Sender<io::Result<Bytes>>, permits: (OwnedSemaphorePermit, OwnedSemaphorePermit), ) -> io::Result<Self>` |
| [263](feuer-storage/src/uring.rs#L263) | Method | `impl IoRequest::conflicts` | `fn(&self, other: &Self) -> bool` |
| [269](feuer-storage/src/uring.rs#L269) | Method | `impl IoRequest::submission_entry` | `fn(&mut self, fd: i32, slot: usize) -> squeue::Entry` |
| [285](feuer-storage/src/uring.rs#L285) | Method | `impl IoRequest::complete` | `fn(&mut self, result: i32) -> io::Result<bool>` |
| [314](feuer-storage/src/uring.rs#L314) | Method | `impl IoRequest::short_error` | `fn(&self) -> io::Error` |
| [325](feuer-storage/src/uring.rs#L325) | Method | `impl IoRequest::finish` | `fn(mut self, result: io::Result<()>)` |
| [340](feuer-storage/src/uring.rs#L340) | Struct | `IoQueue` | — |
| [342](feuer-storage/src/uring.rs#L342) | Field | `IoQueue::admission` | `Arc<IoAdmissionBudgets>` |
| [344](feuer-storage/src/uring.rs#L344) | Field | `IoQueue::ring` | `IoUring` |
| [346](feuer-storage/src/uring.rs#L346) | Field | `IoQueue::file` | `Option<File>` |
| [348](feuer-storage/src/uring.rs#L348) | Field | `IoQueue::lock` | `Option<File>` |
| [350](feuer-storage/src/uring.rs#L350) | Field | `IoQueue::wake` | `Arc<OwnedFd>` |
| [352](feuer-storage/src/uring.rs#L352) | Field | `IoQueue::receiver` | `mpsc::Receiver<IoRequest>` |
| [354](feuer-storage/src/uring.rs#L354) | Field | `IoQueue::pending` | `VecDeque<IoRequest>` |
| [356](feuer-storage/src/uring.rs#L356) | Field | `IoQueue::active` | `Vec<Option<IoRequest>>` |
| [359](feuer-storage/src/uring.rs#L359) | Impl | `impl IoQueue` | — |
| [360](feuer-storage/src/uring.rs#L360) | Method | `impl IoQueue::run` | `fn(&mut self) -> io::Result<()>` |
| [407](feuer-storage/src/uring.rs#L407) | Method | `impl IoQueue::schedule` | `fn(&mut self)` |
| [431](feuer-storage/src/uring.rs#L431) | Method | `impl IoQueue::submit_slot` | `fn(&mut self, slot: usize)` |
| [446](feuer-storage/src/uring.rs#L446) | Method | `impl IoQueue::wait` | `fn(&self) -> io::Result<()>` |
| [484](feuer-storage/src/uring.rs#L484) | Impl | `impl Drop for IoQueue` | — |
| [485](feuer-storage/src/uring.rs#L485) | Method | `impl Drop for IoQueue::drop` | `fn(&mut self)` |
| [507](feuer-storage/src/uring.rs#L507) | Function | `notify` | `fn(wake: &OwnedFd)` |

<details>
<summary>Local bindings (48)</summary>

| Line | Kind | Name / source parent | Signature or type |
| ---: | --- | --- | --- |
| [72](feuer-storage/src/uring.rs#L72) | Local | `staging_pages_for::data_offset_in_buffer` | — |
| [73](feuer-storage/src/uring.rs#L73) | Local | `staging_pages_for::aligned_length` | — |
| [74](feuer-storage/src/uring.rs#L74) | Local | `staging_pages_for::needs_read_modify_write` | — |
| [82](feuer-storage/src/uring.rs#L82) | Local | `impl IoQueueHandle::new::ring` | — |
| [84](feuer-storage/src/uring.rs#L84) | Local | `impl IoQueueHandle::new::fd` | — |
| [89](feuer-storage/src/uring.rs#L89) | Local | `impl IoQueueHandle::new::wake` | — |
| [90](feuer-storage/src/uring.rs#L90) | Local | `impl IoQueueHandle::new::(sender, receiver)` | — |
| [91](feuer-storage/src/uring.rs#L91) | Local | `impl IoQueueHandle::new::admission` | — |
| [92](feuer-storage/src/uring.rs#L92) | Local | `impl IoQueueHandle::new::mut queue` | — |
| [102](feuer-storage/src/uring.rs#L102) | Local | `impl IoQueueHandle::new::thread` | — |
| [122](feuer-storage/src/uring.rs#L122) | Local | `impl IoQueueHandle::execute::class` | — |
| [123](feuer-storage/src/uring.rs#L123) | Local | `impl IoQueueHandle::execute::request_permit` | — |
| [128](feuer-storage/src/uring.rs#L128) | Local | `impl IoQueueHandle::execute::staging_permit` | — |
| [133](feuer-storage/src/uring.rs#L133) | Local | `impl IoQueueHandle::execute::(reply, receive)` | — |
| [134](feuer-storage/src/uring.rs#L134) | Local | `impl IoQueueHandle::execute::request` | — |
| [155](feuer-storage/src/uring.rs#L155) | Local | `impl Drop for IoQueueHandle::drop::_` | — |
| [173](feuer-storage/src/uring.rs#L173) | Local | `impl AlignedBuffer::new::layout` | — |
| [175](feuer-storage/src/uring.rs#L175) | Local | `impl AlignedBuffer::new::ptr` | — |
| [234](feuer-storage/src/uring.rs#L234) | Local | `impl IoRequest::new::data_offset_in_buffer` | — |
| [235](feuer-storage/src/uring.rs#L235) | Local | `impl IoRequest::new::aligned_length` | — |
| [236](feuer-storage/src/uring.rs#L236) | Local | `impl IoRequest::new::mut io_buffer` | — |
| [237](feuer-storage/src/uring.rs#L237) | Local | `impl IoRequest::new::needs_read_modify_write` | — |
| [239](feuer-storage/src/uring.rs#L239) | Local | `impl IoRequest::new::payload` | — |
| [270](feuer-storage/src/uring.rs#L270) | Local | `impl IoRequest::submission_entry::fd` | — |
| [273](feuer-storage/src/uring.rs#L273) | Local | `impl IoRequest::submission_entry::ptr` | — |
| [274](feuer-storage/src/uring.rs#L274) | Local | `impl IoRequest::submission_entry::length` | — |
| [275](feuer-storage/src/uring.rs#L275) | Local | `impl IoRequest::submission_entry::offset` | — |
| [276](feuer-storage/src/uring.rs#L276) | Local | `impl IoRequest::submission_entry::entry` | — |
| [292](feuer-storage/src/uring.rs#L292) | Local | `impl IoRequest::complete::count` | — |
| [326](feuer-storage/src/uring.rs#L326) | Local | `impl IoRequest::finish::result` | — |
| [336](feuer-storage/src/uring.rs#L336) | Local | `impl IoRequest::finish::_` | — |
| [361](feuer-storage/src/uring.rs#L361) | Local | `impl IoQueue::run::mut disconnected` | — |
| [363](feuer-storage/src/uring.rs#L363) | Local | `impl IoQueue::run::completions` | `Vec<_>` |
| [369](feuer-storage/src/uring.rs#L369) | Local | `impl IoQueue::run::slot` | — |
| [370](feuer-storage/src/uring.rs#L370) | Local | `impl IoQueue::run::request` | — |
| [387](feuer-storage/src/uring.rs#L387) | Local | `impl IoQueue::run::active` | — |
| [414](feuer-storage/src/uring.rs#L414) | Local | `impl IoQueue::schedule::mut index` | — |
| [416](feuer-storage/src/uring.rs#L416) | Local | `impl IoQueue::schedule::Some(slot)` | — |
| [419](feuer-storage/src/uring.rs#L419) | Local | `impl IoQueue::schedule::request` | — |
| [420](feuer-storage/src/uring.rs#L420) | Local | `impl IoQueue::schedule::blocked` | — |
| [432](feuer-storage/src/uring.rs#L432) | Local | `impl IoQueue::submit_slot::entry` | — |
| [447](feuer-storage/src/uring.rs#L447) | Local | `impl IoQueue::wait::mut fds` | — |
| [460](feuer-storage/src/uring.rs#L460) | Local | `impl IoQueue::wait::result` | — |
| [462](feuer-storage/src/uring.rs#L462) | Local | `impl IoQueue::wait::error` | — |
| [476](feuer-storage/src/uring.rs#L476) | Local | `impl IoQueue::wait::mut value` | — |
| [497](feuer-storage/src/uring.rs#L497) | Local | `impl Drop for IoQueue::drop::_` | — |
| [508](feuer-storage/src/uring.rs#L508) | Local | `notify::value` | — |
| [511](feuer-storage/src/uring.rs#L511) | Local | `notify::result` | — |

</details>

## feuer-tokio

### `feuer-tokio/src/lib.rs`

No symbols reported by rust-analyzer.

## feuer-types

### `feuer-types/src/download.rs`

| Line | Kind | Name / source parent | Signature or type |
| ---: | --- | --- | --- |
| [15](feuer-types/src/download.rs#L15) | Struct | `Download` | — |
| [16](feuer-types/src/download.rs#L16) | Field | `Download::downloaded_start` | `u64` |
| [17](feuer-types/src/download.rs#L17) | Field | `Download::bytes` | `Bytes` |
| [20](feuer-types/src/download.rs#L20) | Impl | `impl Download` | — |
| [22](feuer-types/src/download.rs#L22) | Function | `impl Download::new` | `fn(downloaded_start: u64, bytes: Bytes) -> Result<Self, DownloadError>` |
| [40](feuer-types/src/download.rs#L40) | Method | `impl Download::downloaded_start` | `fn(&self) -> u64` |
| [45](feuer-types/src/download.rs#L45) | Method | `impl Download::downloaded_range` | `fn(&self) -> ByteRange` |
| [51](feuer-types/src/download.rs#L51) | Method | `impl Download::bytes` | `fn(&self) -> &Bytes` |
| [56](feuer-types/src/download.rs#L56) | Method | `impl Download::into_parts` | `fn(self) -> (ByteRange, Bytes)` |
| [62](feuer-types/src/download.rs#L62) | Impl | `impl fmt::Debug for Download` | — |
| [63](feuer-types/src/download.rs#L63) | Method | `impl fmt::Debug for Download::fmt` | `fn(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result` |
| [73](feuer-types/src/download.rs#L73) | Enum | `DownloadError` | — |
| [76](feuer-types/src/download.rs#L76) | Variant | `DownloadError::EmptyPayload` | — |
| [79](feuer-types/src/download.rs#L79) | Variant | `DownloadError::RangeOverflow` | — |
| [81](feuer-types/src/download.rs#L81) | Field | `DownloadError::RangeOverflow::downloaded_start` | `u64` |
| [83](feuer-types/src/download.rs#L83) | Field | `DownloadError::RangeOverflow::payload_bytes` | `u64` |
| [88](feuer-types/src/download.rs#L88) | Module | `tests` | — |
| [91](feuer-types/src/download.rs#L91) | Function | `tests::range` | `fn(start: u64, end: u64) -> ByteRange` |
| [96](feuer-types/src/download.rs#L96) | Function | `tests::derives_the_exact_range_from_the_start_and_payload` | `fn()` |
| [106](feuer-types/src/download.rs#L106) | Function | `tests::rejects_empty_or_unrepresentable_ranges` | `fn()` |
| [118](feuer-types/src/download.rs#L118) | Function | `tests::debug_output_omits_payload_bytes` | `fn()` |

<details>
<summary>Local bindings (6)</summary>

| Line | Kind | Name / source parent | Signature or type |
| ---: | --- | --- | --- |
| [23](feuer-types/src/download.rs#L23) | Local | `impl Download::new::payload_bytes` | — |
| [57](feuer-types/src/download.rs#L57) | Local | `impl Download::into_parts::range` | — |
| [97](feuer-types/src/download.rs#L97) | Local | `tests::derives_the_exact_range_from_the_start_and_payload::payload` | — |
| [98](feuer-types/src/download.rs#L98) | Local | `tests::derives_the_exact_range_from_the_start_and_payload::download` | — |
| [119](feuer-types/src/download.rs#L119) | Local | `tests::debug_output_omits_payload_bytes::download` | — |
| [120](feuer-types/src/download.rs#L120) | Local | `tests::debug_output_omits_payload_bytes::output` | — |

</details>

### `feuer-types/src/lib.rs`

| Line | Kind | Name / source parent | Signature or type |
| ---: | --- | --- | --- |
| [8](feuer-types/src/lib.rs#L8) | Module | `download` | — |
| [9](feuer-types/src/lib.rs#L9) | Module | `range` | — |
| [15](feuer-types/src/lib.rs#L15) | TypeAlias | `ObjectKey` | `String` |

### `feuer-types/src/range.rs`

| Line | Kind | Name / source parent | Signature or type |
| ---: | --- | --- | --- |
| [9](feuer-types/src/range.rs#L9) | Struct | `ByteRange` | — |
| [10](feuer-types/src/range.rs#L10) | Field | `ByteRange::start` | `u64` |
| [11](feuer-types/src/range.rs#L11) | Field | `ByteRange::end` | `u64` |
| [14](feuer-types/src/range.rs#L14) | Impl | `impl ByteRange` | — |
| [16](feuer-types/src/range.rs#L16) | Function | `impl ByteRange::new` | `fn(start: u64, end: u64) -> Result<Self, InvalidRange>` |
| [24](feuer-types/src/range.rs#L24) | Method | `impl ByteRange::start` | `fn(self) -> u64` |
| [29](feuer-types/src/range.rs#L29) | Method | `impl ByteRange::end` | `fn(self) -> u64` |
| [34](feuer-types/src/range.rs#L34) | Method | `impl ByteRange::len` | `fn(self) -> u64` |
| [39](feuer-types/src/range.rs#L39) | Method | `impl ByteRange::is_empty` | `fn(self) -> bool` |
| [44](feuer-types/src/range.rs#L44) | Method | `impl ByteRange::contains` | `fn(self, other: Self) -> bool` |
| [49](feuer-types/src/range.rs#L49) | Method | `impl ByteRange::overlaps` | `fn(self, other: Self) -> bool` |
| [54](feuer-types/src/range.rs#L54) | Impl | `impl TryFrom<Range<u64>> for ByteRange` | — |
| [55](feuer-types/src/range.rs#L55) | TypeAlias | `impl TryFrom<Range<u64>> for ByteRange::Error` | `InvalidRange` |
| [57](feuer-types/src/range.rs#L57) | Function | `impl TryFrom<Range<u64>> for ByteRange::try_from` | `fn(range: Range<u64>) -> Result<Self, Self::Error>` |
| [62](feuer-types/src/range.rs#L62) | Impl | `impl From<ByteRange> for Range<u64>` | — |
| [63](feuer-types/src/range.rs#L63) | Function | `impl From<ByteRange> for Range<u64>::from` | `fn(range: ByteRange) -> Self` |
| [71](feuer-types/src/range.rs#L71) | Struct | `InvalidRange` | — |
| [72](feuer-types/src/range.rs#L72) | Field | `InvalidRange::start` | `u64` |
| [73](feuer-types/src/range.rs#L73) | Field | `InvalidRange::end` | `u64` |
| [76](feuer-types/src/range.rs#L76) | Impl | `impl InvalidRange` | — |
| [78](feuer-types/src/range.rs#L78) | Method | `impl InvalidRange::start` | `fn(self) -> u64` |
| [83](feuer-types/src/range.rs#L83) | Method | `impl InvalidRange::end` | `fn(self) -> u64` |
| [89](feuer-types/src/range.rs#L89) | Module | `tests` | — |
| [93](feuer-types/src/range.rs#L93) | Function | `tests::preserves_arbitrary_unaligned_endpoints` | `fn()` |
| [102](feuer-types/src/range.rs#L102) | Function | `tests::rejects_empty_and_reversed_ranges` | `fn()` |
| [108](feuer-types/src/range.rs#L108) | Function | `tests::containment_and_overlap_use_exact_boundaries` | `fn()` |

<details>
<summary>Local bindings (2)</summary>

| Line | Kind | Name / source parent | Signature or type |
| ---: | --- | --- | --- |
| [94](feuer-types/src/range.rs#L94) | Local | `tests::preserves_arbitrary_unaligned_endpoints::range` | — |
| [109](feuer-types/src/range.rs#L109) | Local | `tests::containment_and_overlap_use_exact_boundaries::outer` | — |

</details>
