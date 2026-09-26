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

## Counts

| Kind | Count |
| --- | ---: |
| Const | 29 |
| Enum | 10 |
| Field | 183 |
| Function | 166 |
| Impl | 53 |
| Local | 549 |
| Method | 138 |
| Module | 25 |
| Struct | 45 |
| Trait | 1 |
| TypeAlias | 5 |
| Variant | 40 |
| **Total** | **1244** |

## Packages

| Package | Files | Items / fields / variants | Implementation blocks | Local bindings |
| --- | ---: | ---: | ---: | ---: |
| `feuer` | 3 | 42 | 3 | 51 |
| `feuer-memory` | 7 | 211 | 12 | 180 |
| `feuer-memory-bench` | 1 | 149 | 12 | 109 |
| `feuer-storage` | 7 | 196 | 20 | 201 |
| `feuer-tokio` | 1 | 0 | 0 | 0 |
| `feuer-types` | 3 | 44 | 6 | 8 |

## feuer

### `feuer/src/cache.rs`

| Line | Kind | Name / source parent | Signature or type |
| ---: | --- | --- | --- |
| [10](feuer/src/cache.rs#L10) | Struct | `Inner` | — |
| [11](feuer/src/cache.rs#L11) | Field | `Inner::config` | `Config` |
| [12](feuer/src/cache.rs#L12) | Field | `Inner::memory` | `MemoryCache` |
| [22](feuer/src/cache.rs#L22) | Struct | `Cache` | — |
| [23](feuer/src/cache.rs#L23) | Field | `Cache::inner` | `Arc<Inner>` |
| [26](feuer/src/cache.rs#L26) | Impl | `impl fmt::Debug for Cache` | — |
| [27](feuer/src/cache.rs#L27) | Method | `impl fmt::Debug for Cache::fmt` | `fn(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result` |
| [35](feuer/src/cache.rs#L35) | Impl | `impl Cache` | — |
| [37](feuer/src/cache.rs#L37) | Function | `impl Cache::new` | `fn(config: Config) -> Self` |
| [45](feuer/src/cache.rs#L45) | Method | `impl Cache::config` | `fn(&self) -> &Config` |
| [61](feuer/src/cache.rs#L61) | Method | `impl Cache::get_or_fetch` | `fn<F, Fut, E>( &self, object_key: ObjectKey, requested_range: ByteRange, callback: F, ) -> Result<Bytes, GetOrFetchError<E>>` |
| [95](feuer/src/cache.rs#L95) | Enum | `GetOrFetchError` | — |
| [98](feuer/src/cache.rs#L98) | Variant | `GetOrFetchError::Callback` | — |
| [101](feuer/src/cache.rs#L101) | Variant | `GetOrFetchError::DownloadDoesNotCover` | — |
| [103](feuer/src/cache.rs#L103) | Field | `GetOrFetchError::DownloadDoesNotCover::requested_range` | `ByteRange` |
| [105](feuer/src/cache.rs#L105) | Field | `GetOrFetchError::DownloadDoesNotCover::downloaded_range` | `ByteRange` |
| [109](feuer/src/cache.rs#L109) | Function | `requested_slice` | `fn(bytes: &Bytes, downloaded_range: ByteRange, requested_range: ByteRange) -> Bytes` |
| [119](feuer/src/cache.rs#L119) | Module | `tests` | — |
| [132](feuer/src/cache.rs#L132) | Function | `tests::range` | `fn(start: u64, end: u64) -> ByteRange` |
| [136](feuer/src/cache.rs#L136) | Function | `tests::cache` | `fn(memory_capacity: u64) -> Cache` |
| [141](feuer/src/cache.rs#L141) | Function | `tests::cache_handle_is_send_sync_static` | `fn()` |
| [142](feuer/src/cache.rs#L142) | Function | `tests::cache_handle_is_send_sync_static::assert_send_sync_static` | `fn<T: Send + Sync + 'static>()` |
| [147](feuer/src/cache.rs#L147) | Function | `tests::callback_result_and_covering_memory_hit_return_the_exact_request` | `fn()` |
| [178](feuer/src/cache.rs#L178) | Function | `tests::every_concurrent_miss_invokes_its_own_callback` | `fn()` |
| [210](feuer/src/cache.rs#L210) | Function | `tests::callback_errors_are_returned_without_retry_or_population` | `fn()` |
| [231](feuer/src/cache.rs#L231) | Function | `tests::rejects_noncovering_but_retains_oversized_callback_results` | `fn()` |
| [271](feuer/src/cache.rs#L271) | Function | `tests::a_racing_contained_download_is_discarded_but_returns_its_own_bytes` | `fn()` |

<details>
<summary>Local bindings (48)</summary>

| Line | Kind | Name / source parent | Signature or type |
| ---: | --- | --- | --- |
| [38](feuer/src/cache.rs#L38) | Local | `impl Cache::new::memory` | — |
| [75](feuer/src/cache.rs#L75) | Local | `impl Cache::get_or_fetch::download` | — |
| [76](feuer/src/cache.rs#L76) | Local | `impl Cache::get_or_fetch::downloaded_range` | — |
| [84](feuer/src/cache.rs#L84) | Local | `impl Cache::get_or_fetch::requested_bytes` | — |
| [111](feuer/src/cache.rs#L111) | Local | `requested_slice::start` | — |
| [113](feuer/src/cache.rs#L113) | Local | `requested_slice::end` | — |
| [148](feuer/src/cache.rs#L148) | Local | `tests::callback_result_and_covering_memory_hit_return_the_exact_request::cache` | — |
| [149](feuer/src/cache.rs#L149) | Local | `tests::callback_result_and_covering_memory_hit_return_the_exact_request::key` | — |
| [150](feuer/src/cache.rs#L150) | Local | `tests::callback_result_and_covering_memory_hit_return_the_exact_request::payload` | — |
| [151](feuer/src/cache.rs#L151) | Local | `tests::callback_result_and_covering_memory_hit_return_the_exact_request::callback_count` | — |
| [153](feuer/src/cache.rs#L153) | Local | `tests::callback_result_and_covering_memory_hit_return_the_exact_request::count` | — |
| [154](feuer/src/cache.rs#L154) | Local | `tests::callback_result_and_covering_memory_hit_return_the_exact_request::callback_payload` | — |
| [155](feuer/src/cache.rs#L155) | Local | `tests::callback_result_and_covering_memory_hit_return_the_exact_request::result` | — |
| [165](feuer/src/cache.rs#L165) | Local | `tests::callback_result_and_covering_memory_hit_return_the_exact_request::count` | — |
| [166](feuer/src/cache.rs#L166) | Local | `tests::callback_result_and_covering_memory_hit_return_the_exact_request::result` | — |
| [179](feuer/src/cache.rs#L179) | Local | `tests::every_concurrent_miss_invokes_its_own_callback::cache` | — |
| [180](feuer/src/cache.rs#L180) | Local | `tests::every_concurrent_miss_invokes_its_own_callback::key` | — |
| [181](feuer/src/cache.rs#L181) | Local | `tests::every_concurrent_miss_invokes_its_own_callback::barrier` | — |
| [182](feuer/src/cache.rs#L182) | Local | `tests::every_concurrent_miss_invokes_its_own_callback::callback_count` | — |
| [183](feuer/src/cache.rs#L183) | Local | `tests::every_concurrent_miss_invokes_its_own_callback::mut tasks` | — |
| [186](feuer/src/cache.rs#L186) | Local | `tests::every_concurrent_miss_invokes_its_own_callback::cache` | — |
| [187](feuer/src/cache.rs#L187) | Local | `tests::every_concurrent_miss_invokes_its_own_callback::key` | — |
| [188](feuer/src/cache.rs#L188) | Local | `tests::every_concurrent_miss_invokes_its_own_callback::barrier` | — |
| [189](feuer/src/cache.rs#L189) | Local | `tests::every_concurrent_miss_invokes_its_own_callback::callback_count` | — |
| [211](feuer/src/cache.rs#L211) | Local | `tests::callback_errors_are_returned_without_retry_or_population::cache` | — |
| [212](feuer/src/cache.rs#L212) | Local | `tests::callback_errors_are_returned_without_retry_or_population::key` | — |
| [213](feuer/src/cache.rs#L213) | Local | `tests::callback_errors_are_returned_without_retry_or_population::callback_count` | — |
| [216](feuer/src/cache.rs#L216) | Local | `tests::callback_errors_are_returned_without_retry_or_population::invocation_count` | — |
| [217](feuer/src/cache.rs#L217) | Local | `tests::callback_errors_are_returned_without_retry_or_population::error` | — |
| [232](feuer/src/cache.rs#L232) | Local | `tests::rejects_noncovering_but_retains_oversized_callback_results::cache` | — |
| [233](feuer/src/cache.rs#L233) | Local | `tests::rejects_noncovering_but_retains_oversized_callback_results::key` | — |
| [235](feuer/src/cache.rs#L235) | Local | `tests::rejects_noncovering_but_retains_oversized_callback_results::error` | — |
| [249](feuer/src/cache.rs#L249) | Local | `tests::rejects_noncovering_but_retains_oversized_callback_results::result` | — |
| [257](feuer/src/cache.rs#L257) | Local | `tests::rejects_noncovering_but_retains_oversized_callback_results::unexpected_callback_count` | — |
| [258](feuer/src/cache.rs#L258) | Local | `tests::rejects_noncovering_but_retains_oversized_callback_results::count` | — |
| [259](feuer/src/cache.rs#L259) | Local | `tests::rejects_noncovering_but_retains_oversized_callback_results::result` | — |
| [272](feuer/src/cache.rs#L272) | Local | `tests::a_racing_contained_download_is_discarded_but_returns_its_own_bytes::cache` | — |
| [273](feuer/src/cache.rs#L273) | Local | `tests::a_racing_contained_download_is_discarded_but_returns_its_own_bytes::key` | — |
| [274](feuer/src/cache.rs#L274) | Local | `tests::a_racing_contained_download_is_discarded_but_returns_its_own_bytes::callback_entered` | — |
| [275](feuer/src/cache.rs#L275) | Local | `tests::a_racing_contained_download_is_discarded_but_returns_its_own_bytes::release_callback` | — |
| [277](feuer/src/cache.rs#L277) | Local | `tests::a_racing_contained_download_is_discarded_but_returns_its_own_bytes::pending` | — |
| [278](feuer/src/cache.rs#L278) | Local | `tests::a_racing_contained_download_is_discarded_but_returns_its_own_bytes::pending::cache` | — |
| [279](feuer/src/cache.rs#L279) | Local | `tests::a_racing_contained_download_is_discarded_but_returns_its_own_bytes::pending::key` | — |
| [280](feuer/src/cache.rs#L280) | Local | `tests::a_racing_contained_download_is_discarded_but_returns_its_own_bytes::pending::callback_entered` | — |
| [281](feuer/src/cache.rs#L281) | Local | `tests::a_racing_contained_download_is_discarded_but_returns_its_own_bytes::pending::release_callback` | — |
| [304](feuer/src/cache.rs#L304) | Local | `tests::a_racing_contained_download_is_discarded_but_returns_its_own_bytes::unexpected_callback_count` | — |
| [305](feuer/src/cache.rs#L305) | Local | `tests::a_racing_contained_download_is_discarded_but_returns_its_own_bytes::count` | — |
| [306](feuer/src/cache.rs#L306) | Local | `tests::a_racing_contained_download_is_discarded_but_returns_its_own_bytes::cached` | — |

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
| [16](feuer-memory/src/store/shard.rs#L16) | Const | `POLICY_SAMPLE_SIZE` | `usize` |
| [19](feuer-memory/src/store/shard.rs#L19) | Struct | `Entry` | — |
| [21](feuer-memory/src/store/shard.rs#L21) | Field | `Entry::id` | `u64` |
| [23](feuer-memory/src/store/shard.rs#L23) | Field | `Entry::range` | `ByteRange` |
| [25](feuer-memory/src/store/shard.rs#L25) | Field | `Entry::bytes` | `Bytes` |
| [27](feuer-memory/src/store/shard.rs#L27) | Field | `Entry::candidate_slot` | `usize` |
| [29](feuer-memory/src/store/shard.rs#L29) | Field | `Entry::admitted_at` | `u64` |
| [32](feuer-memory/src/store/shard.rs#L32) | Impl | `impl Entry` | — |
| [33](feuer-memory/src/store/shard.rs#L33) | Method | `impl Entry::requested_bytes` | `fn(&self, requested_range: ByteRange) -> Bytes` |
| [49](feuer-memory/src/store/shard.rs#L49) | Struct | `CachedRanges` | — |
| [51](feuer-memory/src/store/shard.rs#L51) | Field | `CachedRanges::by_start` | `BTreeMap<u64, Entry>` |
| [53](feuer-memory/src/store/shard.rs#L53) | Field | `CachedRanges::accesses` | `AccessHistory` |
| [55](feuer-memory/src/store/shard.rs#L55) | Field | `CachedRanges::generation` | `u64` |
| [58](feuer-memory/src/store/shard.rs#L58) | Impl | `impl CachedRanges` | — |
| [59](feuer-memory/src/store/shard.rs#L59) | Method | `impl CachedRanges::covering` | `fn(&self, range: ByteRange) -> Option<&Entry>` |
| [64](feuer-memory/src/store/shard.rs#L64) | Method | `impl CachedRanges::observe_covering` | `fn<R>( &mut self, requested: ByteRange, access_clock: u64, project: impl FnOnce(&Entry) -> R, ) -> Option<R>` |
| [82](feuer-memory/src/store/shard.rs#L82) | Method | `impl CachedRanges::superseded_by` | `fn(&self, range: ByteRange) -> Superseded` |
| [96](feuer-memory/src/store/shard.rs#L96) | Struct | `Superseded` | — |
| [97](feuer-memory/src/store/shard.rs#L97) | Field | `Superseded::ranges` | `Vec<ByteRange>` |
| [98](feuer-memory/src/store/shard.rs#L98) | Field | `Superseded::bytes` | `u64` |
| [103](feuer-memory/src/store/shard.rs#L103) | Struct | `Removal` | — |
| [104](feuer-memory/src/store/shard.rs#L104) | Field | `Removal::bytes` | `u64` |
| [105](feuer-memory/src/store/shard.rs#L105) | Field | `Removal::entries` | `u64` |
| [109](feuer-memory/src/store/shard.rs#L109) | Struct | `CandidateRef` | — |
| [110](feuer-memory/src/store/shard.rs#L110) | Field | `CandidateRef::object_key` | `ObjectKey` |
| [111](feuer-memory/src/store/shard.rs#L111) | Field | `CandidateRef::start` | `u64` |
| [112](feuer-memory/src/store/shard.rs#L112) | Field | `CandidateRef::id` | `u64` |
| [117](feuer-memory/src/store/shard.rs#L117) | Struct | `PolicyCandidates` | — |
| [118](feuer-memory/src/store/shard.rs#L118) | Field | `PolicyCandidates::entries` | `Vec<CandidateRef>` |
| [119](feuer-memory/src/store/shard.rs#L119) | Field | `PolicyCandidates::cursor` | `usize` |
| [122](feuer-memory/src/store/shard.rs#L122) | Impl | `impl PolicyCandidates` | — |
| [123](feuer-memory/src/store/shard.rs#L123) | Method | `impl PolicyCandidates::register` | `fn(&mut self, candidate: CandidateRef) -> usize` |
| [130](feuer-memory/src/store/shard.rs#L130) | Method | `impl PolicyCandidates::remove` | `fn(&mut self, slot: usize, expected_id: u64) -> Option<CandidateRef>` |
| [143](feuer-memory/src/store/shard.rs#L143) | Method | `impl PolicyCandidates::sample` | `fn(&mut self) -> (usize, usize)` |
| [155](feuer-memory/src/store/shard.rs#L155) | Struct | `Victim` | — |
| [156](feuer-memory/src/store/shard.rs#L156) | Field | `Victim::object_key` | `ObjectKey` |
| [157](feuer-memory/src/store/shard.rs#L157) | Field | `Victim::range` | `ByteRange` |
| [158](feuer-memory/src/store/shard.rs#L158) | Field | `Victim::id` | `u64` |
| [159](feuer-memory/src/store/shard.rs#L159) | Field | `Victim::bytes` | `u64` |
| [160](feuer-memory/src/store/shard.rs#L160) | Field | `Victim::retrieval_value` | `u64` |
| [164](feuer-memory/src/store/shard.rs#L164) | Struct | `CompactionWork` | — |
| [165](feuer-memory/src/store/shard.rs#L165) | Field | `CompactionWork::object_key` | `ObjectKey` |
| [166](feuer-memory/src/store/shard.rs#L166) | Field | `CompactionWork::start` | `u64` |
| [167](feuer-memory/src/store/shard.rs#L167) | Field | `CompactionWork::id` | `u64` |
| [168](feuer-memory/src/store/shard.rs#L168) | Field | `CompactionWork::generation` | `u64` |
| [169](feuer-memory/src/store/shard.rs#L169) | Field | `CompactionWork::plan` | `CompactionPlan` |
| [170](feuer-memory/src/store/shard.rs#L170) | Field | `CompactionWork::source_bytes` | `Bytes` |
| [173](feuer-memory/src/store/shard.rs#L173) | Impl | `impl CompactionWork` | — |
| [175](feuer-memory/src/store/shard.rs#L175) | Method | `impl CompactionWork::copy_payload` | `fn(self) -> PreparedCompaction` |
| [200](feuer-memory/src/store/shard.rs#L200) | Struct | `PreparedCompaction` | — |
| [201](feuer-memory/src/store/shard.rs#L201) | Field | `PreparedCompaction::object_key` | `ObjectKey` |
| [202](feuer-memory/src/store/shard.rs#L202) | Field | `PreparedCompaction::start` | `u64` |
| [203](feuer-memory/src/store/shard.rs#L203) | Field | `PreparedCompaction::id` | `u64` |
| [204](feuer-memory/src/store/shard.rs#L204) | Field | `PreparedCompaction::generation` | `u64` |
| [205](feuer-memory/src/store/shard.rs#L205) | Field | `PreparedCompaction::plan` | `CompactionPlan` |
| [206](feuer-memory/src/store/shard.rs#L206) | Field | `PreparedCompaction::retained` | `Vec<(ByteRange, Bytes)>` |
| [210](feuer-memory/src/store/shard.rs#L210) | Enum | `AdmissionStep` | — |
| [211](feuer-memory/src/store/shard.rs#L211) | Variant | `AdmissionStep::Complete` | — |
| [212](feuer-memory/src/store/shard.rs#L212) | Variant | `AdmissionStep::Retry` | — |
| [213](feuer-memory/src/store/shard.rs#L213) | Variant | `AdmissionStep::Compact` | — |
| [217](feuer-memory/src/store/shard.rs#L217) | Struct | `Shard` | — |
| [218](feuer-memory/src/store/shard.rs#L218) | Field | `Shard::capacity` | `u64` |
| [219](feuer-memory/src/store/shard.rs#L219) | Field | `Shard::used_bytes` | `u64` |
| [220](feuer-memory/src/store/shard.rs#L220) | Field | `Shard::ranges` | `FxHashMap<ObjectKey, CachedRanges>` |
| [221](feuer-memory/src/store/shard.rs#L221) | Field | `Shard::access_clock` | `u64` |
| [222](feuer-memory/src/store/shard.rs#L222) | Field | `Shard::next_entry_id` | `u64` |
| [223](feuer-memory/src/store/shard.rs#L223) | Field | `Shard::candidates` | `PolicyCandidates` |
| [224](feuer-memory/src/store/shard.rs#L224) | Field | `Shard::metrics` | `Arc<MemoryMetrics>` |
| [227](feuer-memory/src/store/shard.rs#L227) | Impl | `impl Shard` | — |
| [228](feuer-memory/src/store/shard.rs#L228) | Function | `impl Shard::new` | `fn(capacity: u64, metrics: Arc<MemoryMetrics>) -> Self` |
| [240](feuer-memory/src/store/shard.rs#L240) | Method | `impl Shard::used_bytes` | `fn(&self) -> u64` |
| [244](feuer-memory/src/store/shard.rs#L244) | Method | `impl Shard::get` | `fn(&mut self, object_key: &ObjectKey, requested_range: ByteRange) -> Option<Bytes>` |
| [262](feuer-memory/src/store/shard.rs#L262) | Method | `impl Shard::record_access` | `fn(&mut self, object_key: &ObjectKey, requested_range: ByteRange)` |
| [266](feuer-memory/src/store/shard.rs#L266) | Method | `impl Shard::record_successful_access` | `fn(&mut self, object_key: &ObjectKey, requested_range: ByteRange)` |
| [275](feuer-memory/src/store/shard.rs#L275) | Method | `impl Shard::admission_step` | `fn( &mut self, object_key: &ObjectKey, range: ByteRange, bytes: &Bytes, requested_range: Option<ByteRange>, allow_compaction: bool, ) -> AdmissionStep` |
| [337](feuer-memory/src/store/shard.rs#L337) | Method | `impl Shard::insert_admission` | `fn(&mut self, object_key: ObjectKey, range: ByteRange, bytes: Bytes)` |
| [359](feuer-memory/src/store/shard.rs#L359) | Method | `impl Shard::insert_compacted` | `fn(&mut self, object_key: &ObjectKey, range: ByteRange, bytes: Bytes)` |
| [379](feuer-memory/src/store/shard.rs#L379) | Method | `impl Shard::allocate_entry_id` | `fn(&mut self) -> u64` |
| [387](feuer-memory/src/store/shard.rs#L387) | Method | `impl Shard::remove_superseded` | `fn(&mut self, object_key: &ObjectKey, ranges: &[ByteRange]) -> Removal` |
| [399](feuer-memory/src/store/shard.rs#L399) | Method | `impl Shard::remove` | `fn(&mut self, object_key: &ObjectKey, range: ByteRange) -> bool` |
| [408](feuer-memory/src/store/shard.rs#L408) | Method | `impl Shard::detach_entry` | `fn( &mut self, object_key: &ObjectKey, range: ByteRange, expected_id: Option<u64>, preserve_access: bool, ) -> Option<u64>` |
| [439](feuer-memory/src/store/shard.rs#L439) | Method | `impl Shard::unregister_candidate` | `fn(&mut self, slot: usize, expected_id: u64)` |
| [454](feuer-memory/src/store/shard.rs#L454) | Method | `impl Shard::pressure_candidate` | `fn(&mut self, admitting_key: &ObjectKey, admitting_range: ByteRange) -> Option<Victim>` |
| [492](feuer-memory/src/store/shard.rs#L492) | Method | `impl Shard::compaction_work` | `fn(&self, victim: &Victim) -> Option<CompactionWork>` |
| [517](feuer-memory/src/store/shard.rs#L517) | Method | `impl Shard::publish_compaction` | `fn(&mut self, prepared: PreparedCompaction) -> bool` |
| [560](feuer-memory/src/store/shard.rs#L560) | Method | `impl Shard::entry_count` | `fn(&self) -> usize` |
| [565](feuer-memory/src/store/shard.rs#L565) | Method | `impl Shard::accessed_ranges` | `fn(&self, object_key: &ObjectKey) -> Vec<ByteRange>` |
| [572](feuer-memory/src/store/shard.rs#L572) | Method | `impl Shard::access_history_len` | `fn(&self, object_key: &ObjectKey) -> usize` |
| [577](feuer-memory/src/store/shard.rs#L577) | Method | `impl Shard::candidate_count` | `fn(&self) -> usize` |
| [583](feuer-memory/src/store/shard.rs#L583) | Function | `compare_retention` | `fn(left: &Victim, right: &Victim) -> Ordering` |
| [590](feuer-memory/src/store/shard.rs#L590) | Function | `compare_value_density` | `fn(left: u64, left_bytes: u64, right: u64, right_bytes: u64) -> Ordering` |
| [594](feuer-memory/src/store/shard.rs#L594) | Impl | `impl Drop for Shard` | — |
| [595](feuer-memory/src/store/shard.rs#L595) | Method | `impl Drop for Shard::drop` | `fn(&mut self)` |

<details>
<summary>Local bindings (62)</summary>

| Line | Kind | Name / source parent | Signature or type |
| ---: | --- | --- | --- |
| [35](feuer-memory/src/store/shard.rs#L35) | Local | `impl Entry::requested_bytes::start` | — |
| [37](feuer-memory/src/store/shard.rs#L37) | Local | `impl Entry::requested_bytes::end` | — |
| [60](feuer-memory/src/store/shard.rs#L60) | Local | `impl CachedRanges::covering::(_, entry)` | — |
| [70](feuer-memory/src/store/shard.rs#L70) | Local | `impl CachedRanges::observe_covering::projected` | — |
| [71](feuer-memory/src/store/shard.rs#L71) | Local | `impl CachedRanges::observe_covering::projected::(_, entry)` | — |
| [83](feuer-memory/src/store/shard.rs#L83) | Local | `impl CachedRanges::superseded_by::mut superseded` | — |
| [124](feuer-memory/src/store/shard.rs#L124) | Local | `impl PolicyCandidates::register::slot` | — |
| [132](feuer-memory/src/store/shard.rs#L132) | Local | `impl PolicyCandidates::remove::last` | — |
| [134](feuer-memory/src/store/shard.rs#L134) | Local | `impl PolicyCandidates::remove::moved` | — |
| [144](feuer-memory/src/store/shard.rs#L144) | Local | `impl PolicyCandidates::sample::count` | — |
| [148](feuer-memory/src/store/shard.rs#L148) | Local | `impl PolicyCandidates::sample::start` | — |
| [176](feuer-memory/src/store/shard.rs#L176) | Local | `impl CompactionWork::copy_payload::retained` | — |
| [181](feuer-memory/src/store/shard.rs#L181) | Local | `impl CompactionWork::copy_payload::retained::start` | — |
| [183](feuer-memory/src/store/shard.rs#L183) | Local | `impl CompactionWork::copy_payload::retained::end` | — |
| [245](feuer-memory/src/store/shard.rs#L245) | Local | `impl Shard::get::access_clock` | — |
| [246](feuer-memory/src/store/shard.rs#L246) | Local | `impl Shard::get::accessed` | — |
| [251](feuer-memory/src/store/shard.rs#L251) | Local | `impl Shard::get::Some(bytes)` | — |
| [283](feuer-memory/src/store/shard.rs#L283) | Local | `impl Shard::admission_step::superseded` | — |
| [294](feuer-memory/src/store/shard.rs#L294) | Local | `impl Shard::admission_step::added_bytes` | — |
| [295](feuer-memory/src/store/shard.rs#L295) | Local | `impl Shard::admission_step::effective_used` | — |
| [296](feuer-memory/src/store/shard.rs#L296) | Local | `impl Shard::admission_step::target` | — |
| [299](feuer-memory/src/store/shard.rs#L299) | Local | `impl Shard::admission_step::removal` | — |
| [314](feuer-memory/src/store/shard.rs#L314) | Local | `impl Shard::admission_step::Some(victim)` | — |
| [324](feuer-memory/src/store/shard.rs#L324) | Local | `impl Shard::admission_step::removed` | — |
| [339](feuer-memory/src/store/shard.rs#L339) | Local | `impl Shard::insert_admission::id` | — |
| [340](feuer-memory/src/store/shard.rs#L340) | Local | `impl Shard::insert_admission::candidate_slot` | — |
| [345](feuer-memory/src/store/shard.rs#L345) | Local | `impl Shard::insert_admission::entry` | — |
| [353](feuer-memory/src/store/shard.rs#L353) | Local | `impl Shard::insert_admission::entries` | — |
| [355](feuer-memory/src/store/shard.rs#L355) | Local | `impl Shard::insert_admission::replaced` | — |
| [360](feuer-memory/src/store/shard.rs#L360) | Local | `impl Shard::insert_compacted::id` | — |
| [361](feuer-memory/src/store/shard.rs#L361) | Local | `impl Shard::insert_compacted::candidate_slot` | — |
| [366](feuer-memory/src/store/shard.rs#L366) | Local | `impl Shard::insert_compacted::entry` | — |
| [373](feuer-memory/src/store/shard.rs#L373) | Local | `impl Shard::insert_compacted::entries` | — |
| [375](feuer-memory/src/store/shard.rs#L375) | Local | `impl Shard::insert_compacted::replaced` | — |
| [388](feuer-memory/src/store/shard.rs#L388) | Local | `impl Shard::remove_superseded::mut removal` | — |
| [390](feuer-memory/src/store/shard.rs#L390) | Local | `impl Shard::remove_superseded::bytes` | — |
| [400](feuer-memory/src/store/shard.rs#L400) | Local | `impl Shard::remove::Some(bytes)` | — |
| [415](feuer-memory/src/store/shard.rs#L415) | Local | `impl Shard::detach_entry::(entry, object_is_empty)` | — |
| [416](feuer-memory/src/store/shard.rs#L416) | Local | `impl Shard::detach_entry::(entry, object_is_empty)::entries` | — |
| [417](feuer-memory/src/store/shard.rs#L417) | Local | `impl Shard::detach_entry::(entry, object_is_empty)::current` | — |
| [421](feuer-memory/src/store/shard.rs#L421) | Local | `impl Shard::detach_entry::(entry, object_is_empty)::entry` | — |
| [426](feuer-memory/src/store/shard.rs#L426) | Local | `impl Shard::detach_entry::(entry, object_is_empty)::object_is_empty` | — |
| [434](feuer-memory/src/store/shard.rs#L434) | Local | `impl Shard::detach_entry::bytes` | — |
| [440](feuer-memory/src/store/shard.rs#L440) | Local | `impl Shard::unregister_candidate::moved` | — |
| [441](feuer-memory/src/store/shard.rs#L441) | Local | `impl Shard::unregister_candidate::Some(moved)` | — |
| [444](feuer-memory/src/store/shard.rs#L444) | Local | `impl Shard::unregister_candidate::entry` | — |
| [455](feuer-memory/src/store/shard.rs#L455) | Local | `impl Shard::pressure_candidate::(sample_start, sample_count)` | — |
| [456](feuer-memory/src/store/shard.rs#L456) | Local | `impl Shard::pressure_candidate::candidate_count` | — |
| [457](feuer-memory/src/store/shard.rs#L457) | Local | `impl Shard::pressure_candidate::mut victim` | `Option<Victim>` |
| [460](feuer-memory/src/store/shard.rs#L460) | Local | `impl Shard::pressure_candidate::candidate` | — |
| [461](feuer-memory/src/store/shard.rs#L461) | Local | `impl Shard::pressure_candidate::entries` | — |
| [465](feuer-memory/src/store/shard.rs#L465) | Local | `impl Shard::pressure_candidate::entry` | — |
| [474](feuer-memory/src/store/shard.rs#L474) | Local | `impl Shard::pressure_candidate::candidate_victim` | — |
| [493](feuer-memory/src/store/shard.rs#L493) | Local | `impl Shard::compaction_work::entries` | — |
| [497](feuer-memory/src/store/shard.rs#L497) | Local | `impl Shard::compaction_work::entry` | — |
| [505](feuer-memory/src/store/shard.rs#L505) | Local | `impl Shard::compaction_work::plan` | — |
| [518](feuer-memory/src/store/shard.rs#L518) | Local | `impl Shard::publish_compaction::valid` | — |
| [529](feuer-memory/src/store/shard.rs#L529) | Local | `impl Shard::publish_compaction::source_bytes` | — |
| [532](feuer-memory/src/store/shard.rs#L532) | Local | `impl Shard::publish_compaction::mut retained_bytes` | — |
| [533](feuer-memory/src/store/shard.rs#L533) | Local | `impl Shard::publish_compaction::mut retained_entries` | — |
| [550](feuer-memory/src/store/shard.rs#L550) | Local | `impl Shard::publish_compaction::reclaimed` | — |
| [596](feuer-memory/src/store/shard.rs#L596) | Local | `impl Drop for Shard::drop::entries` | — |

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
| [521](feuer-memory/src/store/tests.rs#L521) | Local | `copied_compaction_is_revalidated_before_publication_and_can_fall_back::prepared` | — |
| [522](feuer-memory/src/store/tests.rs#L522) | Local | `copied_compaction_is_revalidated_before_publication_and_can_fall_back::prepared::mut shard` | — |
| [523](feuer-memory/src/store/tests.rs#L523) | Local | `copied_compaction_is_revalidated_before_publication_and_can_fall_back::prepared::AdmissionStep::Compact(work)` | — |
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
| [45](feuer-memory/src/store.rs#L45) | Field | `MemoryCache::shards` | `Box<[Mutex<Shard>]>` |
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
| [150](feuer-memory/src/store.rs#L150) | Local | `impl MemoryCache::insert_inner::prepared` | — |
| [167](feuer-memory/src/store.rs#L167) | Local | `impl MemoryCache::record_access::shard_index` | — |
| [175](feuer-memory/src/store.rs#L175) | Local | `impl MemoryCache::remove::shard_index` | — |
| [183](feuer-memory/src/store.rs#L183) | Local | `impl MemoryCache::shard_index::mut hasher` | — |
| [195](feuer-memory/src/store.rs#L195) | Local | `shard_capacity_for::shards` | — |

</details>

## feuer-memory-bench

### `benchmarks/memory/src/main.rs`

| Line | Kind | Name / source parent | Signature or type |
| ---: | --- | --- | --- |
| [17](benchmarks/memory/src/main.rs#L17) | Const | `FOYER_BASE_REVISION` | `&str` |
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
| [141](benchmarks/memory/src/main.rs#L141) | Trait | `Engine` | — |
| [142](benchmarks/memory/src/main.rs#L142) | Method | `Engine::name` | `fn(&self) -> &'static str` |
| [145](benchmarks/memory/src/main.rs#L145) | Method | `Engine::get` | `fn(&mut self, access: &Access, downloaded: ByteRange) -> bool` |
| [147](benchmarks/memory/src/main.rs#L147) | Method | `Engine::populate` | `fn(&mut self, access: &Access, downloaded: ByteRange, payload: Bytes) -> Result<(), String>` |
| [149](benchmarks/memory/src/main.rs#L149) | Method | `Engine::used_payload_bytes` | `fn(&self) -> u64` |
| [152](benchmarks/memory/src/main.rs#L152) | Struct | `FeuerEngine` | — |
| [153](benchmarks/memory/src/main.rs#L153) | Field | `FeuerEngine::cache` | `MemoryCache` |
| [156](benchmarks/memory/src/main.rs#L156) | Impl | `impl FeuerEngine` | — |
| [157](benchmarks/memory/src/main.rs#L157) | Function | `impl FeuerEngine::new` | `fn(capacity: usize, shards: usize) -> Self` |
| [164](benchmarks/memory/src/main.rs#L164) | Impl | `impl Engine for FeuerEngine` | — |
| [165](benchmarks/memory/src/main.rs#L165) | Method | `impl Engine for FeuerEngine::name` | `fn(&self) -> &'static str` |
| [169](benchmarks/memory/src/main.rs#L169) | Method | `impl Engine for FeuerEngine::get` | `fn(&mut self, access: &Access, _downloaded: ByteRange) -> bool` |
| [177](benchmarks/memory/src/main.rs#L177) | Method | `impl Engine for FeuerEngine::populate` | `fn(&mut self, access: &Access, downloaded: ByteRange, payload: Bytes) -> Result<(), String>` |
| [185](benchmarks/memory/src/main.rs#L185) | Method | `impl Engine for FeuerEngine::used_payload_bytes` | `fn(&self) -> u64` |
| [190](benchmarks/memory/src/main.rs#L190) | TypeAlias | `NativeFoyerKey` | `(ObjectKey, ByteRange)` |
| [193](benchmarks/memory/src/main.rs#L193) | Struct | `NativeFoyerValue` | — |
| [194](benchmarks/memory/src/main.rs#L194) | Field | `NativeFoyerValue::downloaded` | `ByteRange` |
| [195](benchmarks/memory/src/main.rs#L195) | Field | `NativeFoyerValue::payload` | `Bytes` |
| [199](benchmarks/memory/src/main.rs#L199) | Enum | `NativeFoyerKeyMode` | — |
| [201](benchmarks/memory/src/main.rs#L201) | Variant | `NativeFoyerKeyMode::ExactRequest` | — |
| [203](benchmarks/memory/src/main.rs#L203) | Variant | `NativeFoyerKeyMode::ExpandedDownload` | — |
| [206](benchmarks/memory/src/main.rs#L206) | Impl | `impl NativeFoyerKeyMode` | — |
| [207](benchmarks/memory/src/main.rs#L207) | Method | `impl NativeFoyerKeyMode::range` | `fn(self, access: &Access, downloaded: ByteRange) -> ByteRange` |
| [216](benchmarks/memory/src/main.rs#L216) | Enum | `NativeFoyerPolicy` | — |
| [217](benchmarks/memory/src/main.rs#L217) | Variant | `NativeFoyerPolicy::S3Fifo` | — |
| [218](benchmarks/memory/src/main.rs#L218) | Variant | `NativeFoyerPolicy::CostAware` | — |
| [221](benchmarks/memory/src/main.rs#L221) | Impl | `impl NativeFoyerPolicy` | — |
| [222](benchmarks/memory/src/main.rs#L222) | Method | `impl NativeFoyerPolicy::engine_name` | `fn(self, key_mode: NativeFoyerKeyMode) -> &'static str` |
| [232](benchmarks/memory/src/main.rs#L232) | Struct | `NativeFoyerEngine` | — |
| [233](benchmarks/memory/src/main.rs#L233) | Field | `NativeFoyerEngine::cache` | `FoyerCache<NativeFoyerKey, NativeFoyerValue>` |
| [234](benchmarks/memory/src/main.rs#L234) | Field | `NativeFoyerEngine::key_mode` | `NativeFoyerKeyMode` |
| [235](benchmarks/memory/src/main.rs#L235) | Field | `NativeFoyerEngine::policy` | `NativeFoyerPolicy` |
| [238](benchmarks/memory/src/main.rs#L238) | Impl | `impl NativeFoyerEngine` | — |
| [239](benchmarks/memory/src/main.rs#L239) | Function | `impl NativeFoyerEngine::new` | `fn(capacity: usize, shards: usize, key_mode: NativeFoyerKeyMode, policy: NativeFoyerPolicy) -> Self` |
| [247](benchmarks/memory/src/main.rs#L247) | Method | `impl NativeFoyerEngine::key` | `fn(&self, access: &Access, downloaded: ByteRange) -> NativeFoyerKey` |
| [252](benchmarks/memory/src/main.rs#L252) | Impl | `impl Engine for NativeFoyerEngine` | — |
| [253](benchmarks/memory/src/main.rs#L253) | Method | `impl Engine for NativeFoyerEngine::name` | `fn(&self) -> &'static str` |
| [257](benchmarks/memory/src/main.rs#L257) | Method | `impl Engine for NativeFoyerEngine::get` | `fn(&mut self, access: &Access, downloaded: ByteRange) -> bool` |
| [269](benchmarks/memory/src/main.rs#L269) | Method | `impl Engine for NativeFoyerEngine::populate` | `fn(&mut self, access: &Access, downloaded: ByteRange, payload: Bytes) -> Result<(), String>` |
| [275](benchmarks/memory/src/main.rs#L275) | Method | `impl Engine for NativeFoyerEngine::used_payload_bytes` | `fn(&self) -> u64` |
| [280](benchmarks/memory/src/main.rs#L280) | Function | `foyer_cache` | `fn( capacity: usize, shards: usize, policy: NativeFoyerPolicy, ) -> FoyerCache<NativeFoyerKey, NativeFoyerValue>` |
| [300](benchmarks/memory/src/main.rs#L300) | Struct | `Traffic` | — |
| [301](benchmarks/memory/src/main.rs#L301) | Field | `Traffic::requests` | `u64` |
| [302](benchmarks/memory/src/main.rs#L302) | Field | `Traffic::requested_bytes` | `u64` |
| [303](benchmarks/memory/src/main.rs#L303) | Field | `Traffic::hits` | `u64` |
| [304](benchmarks/memory/src/main.rs#L304) | Field | `Traffic::hit_bytes` | `u64` |
| [305](benchmarks/memory/src/main.rs#L305) | Field | `Traffic::source_requests` | `u64` |
| [306](benchmarks/memory/src/main.rs#L306) | Field | `Traffic::source_bytes` | `u64` |
| [309](benchmarks/memory/src/main.rs#L309) | Struct | `Report` | — |
| [310](benchmarks/memory/src/main.rs#L310) | Field | `Report::workload` | `&'static str` |
| [311](benchmarks/memory/src/main.rs#L311) | Field | `Report::downloader` | `&'static str` |
| [312](benchmarks/memory/src/main.rs#L312) | Field | `Report::shards` | `usize` |
| [313](benchmarks/memory/src/main.rs#L313) | Field | `Report::capacity` | `usize` |
| [314](benchmarks/memory/src/main.rs#L314) | Field | `Report::engine` | `&'static str` |
| [315](benchmarks/memory/src/main.rs#L315) | Field | `Report::traffic` | `Traffic` |
| [316](benchmarks/memory/src/main.rs#L316) | Field | `Report::used_payload_bytes` | `u64` |
| [317](benchmarks/memory/src/main.rs#L317) | Field | `Report::elapsed` | `Duration` |
| [320](benchmarks/memory/src/main.rs#L320) | Impl | `impl Report` | — |
| [321](benchmarks/memory/src/main.rs#L321) | Method | `impl Report::cache_hit_rate` | `fn(&self) -> f64` |
| [325](benchmarks/memory/src/main.rs#L325) | Method | `impl Report::byte_hit_rate` | `fn(&self) -> f64` |
| [329](benchmarks/memory/src/main.rs#L329) | Method | `impl Report::source_cost_hit_rate` | `fn(&self) -> f64` |
| [340](benchmarks/memory/src/main.rs#L340) | Method | `impl Report::operations_per_second` | `fn(&self) -> f64` |
| [345](benchmarks/memory/src/main.rs#L345) | Function | `main` | `fn() -> Result<(), String>` |
| [419](benchmarks/memory/src/main.rs#L419) | Function | `trace_workload` | `fn(args: &Args, download_config: DownloadConfig) -> Result<Workload, String>` |
| [444](benchmarks/memory/src/main.rs#L444) | Function | `run_engine` | `fn( mut engine: Box<dyn Engine>, workload: &Workload, downloader: DownloadPolicy, shards: usize, capacity: usize, warmup_iterations: usize, source_payload: &Bytes, ) -> Result<Report, String>` |
| [476](benchmarks/memory/src/main.rs#L476) | Struct | `PendingDownload` | — |
| [477](benchmarks/memory/src/main.rs#L477) | Field | `PendingDownload::order` | `usize` |
| [478](benchmarks/memory/src/main.rs#L478) | Field | `PendingDownload::access` | `&'a Access` |
| [479](benchmarks/memory/src/main.rs#L479) | Field | `PendingDownload::downloaded` | `ByteRange` |
| [482](benchmarks/memory/src/main.rs#L482) | Struct | `CoalescedDownload` | — |
| [483](benchmarks/memory/src/main.rs#L483) | Field | `CoalescedDownload::downloaded` | `ByteRange` |
| [484](benchmarks/memory/src/main.rs#L484) | Field | `CoalescedDownload::accesses` | `Vec<(usize, &'a Access)>` |
| [487](benchmarks/memory/src/main.rs#L487) | Function | `expanded_download_ranges` | `fn(workload: &[Access], config: DownloadConfig) -> Vec<ByteRange>` |
| [538](benchmarks/memory/src/main.rs#L538) | Function | `max_download_len` | `fn(workload: &Workload, downloader: DownloadPolicy) -> u64` |
| [559](benchmarks/memory/src/main.rs#L559) | Function | `execute_pass` | `fn<E: Engine + ?Sized>( engine: &mut E, workload: &Workload, downloader: DownloadPolicy, source_payload: &Bytes, traffic: &mut Traffic, ) -> Result<(), String>` |
| [585](benchmarks/memory/src/main.rs#L585) | Function | `coalesced_downloads` | `fn( mut pending: Vec<PendingDownload<'_>>, coalescing_distance_bytes: u64, ) -> Vec<CoalescedDownload<'_>>` |
| [624](benchmarks/memory/src/main.rs#L624) | Function | `record_request` | `fn(traffic: &mut Traffic, access: &Access, hit: bool)` |
| [633](benchmarks/memory/src/main.rs#L633) | Function | `source_payload_slice` | `fn(source_payload: &Bytes, downloaded: ByteRange) -> Result<Bytes, String>` |
| [641](benchmarks/memory/src/main.rs#L641) | Function | `requested_payload` | `fn(bytes: &Bytes, downloaded: ByteRange, requested: ByteRange) -> Bytes` |
| [650](benchmarks/memory/src/main.rs#L650) | Function | `print_human_header` | `fn(args: &Args, workload: &Workload)` |
| [669](benchmarks/memory/src/main.rs#L669) | Function | `print_human_report` | `fn(report: &Report)` |
| [684](benchmarks/memory/src/main.rs#L684) | Function | `human_engine_name` | `fn(engine: &str) -> &str` |
| [695](benchmarks/memory/src/main.rs#L695) | Function | `format_decimal_bytes` | `fn(bytes: u64) -> String` |
| [707](benchmarks/memory/src/main.rs#L707) | Function | `format_bytes` | `fn(bytes: u64) -> String` |
| [719](benchmarks/memory/src/main.rs#L719) | Function | `format_rate` | `fn(operations_per_second: f64) -> String` |
| [729](benchmarks/memory/src/main.rs#L729) | Function | `print_csv_report` | `fn(report: &Report)` |
| [753](benchmarks/memory/src/main.rs#L753) | Function | `load_trace` | `fn() -> Result<Vec<Access>, String>` |
| [766](benchmarks/memory/src/main.rs#L766) | Function | `parse_trace_line` | `fn(line: &str) -> Result<Access, String>` |
| [782](benchmarks/memory/src/main.rs#L782) | Function | `parse_timestamp_millis` | `fn(value: &str) -> Result<u64, String>` |
| [843](benchmarks/memory/src/main.rs#L843) | Function | `find_json_string` | `fn(line: &str, field: &str) -> Result<String, String>` |
| [854](benchmarks/memory/src/main.rs#L854) | Function | `find_json_u64` | `fn(line: &str, field: &str) -> Result<u64, String>` |
| [867](benchmarks/memory/src/main.rs#L867) | Function | `field_value` | `fn<'a>(line: &'a str, field: &str) -> Result<&'a str, String>` |
| [880](benchmarks/memory/src/main.rs#L880) | Function | `ratio` | `fn(numerator: u64, denominator: u64) -> f64` |
| [888](benchmarks/memory/src/main.rs#L888) | Function | `byte_count_from_env` | `fn(name: &str, default: u64) -> Result<u64, String>` |
| [898](benchmarks/memory/src/main.rs#L898) | Function | `parse_bytes` | `fn(value: &str) -> Result<usize, String>` |
| [903](benchmarks/memory/src/main.rs#L903) | Function | `parse_byte_count` | `fn(value: &str) -> Result<u128, String>` |
| [928](benchmarks/memory/src/main.rs#L928) | Module | `tests` | — |
| [931](benchmarks/memory/src/main.rs#L931) | Struct | `tests::WarmupEngine` | — |
| [932](benchmarks/memory/src/main.rs#L932) | Field | `tests::WarmupEngine::populated` | `bool` |
| [935](benchmarks/memory/src/main.rs#L935) | Impl | `tests::impl Engine for WarmupEngine` | — |
| [936](benchmarks/memory/src/main.rs#L936) | Method | `tests::impl Engine for WarmupEngine::name` | `fn(&self) -> &'static str` |
| [940](benchmarks/memory/src/main.rs#L940) | Method | `tests::impl Engine for WarmupEngine::get` | `fn(&mut self, _access: &Access, _downloaded: ByteRange) -> bool` |
| [944](benchmarks/memory/src/main.rs#L944) | Method | `tests::impl Engine for WarmupEngine::populate` | `fn(&mut self, _access: &Access, _downloaded: ByteRange, _payload: Bytes) -> Result<(), String>` |
| [949](benchmarks/memory/src/main.rs#L949) | Method | `tests::impl Engine for WarmupEngine::used_payload_bytes` | `fn(&self) -> u64` |
| [955](benchmarks/memory/src/main.rs#L955) | Struct | `tests::RangeEngine` | — |
| [956](benchmarks/memory/src/main.rs#L956) | Field | `tests::RangeEngine::entries` | `Vec<(ObjectKey, ByteRange)>` |
| [959](benchmarks/memory/src/main.rs#L959) | Impl | `tests::impl Engine for RangeEngine` | — |
| [960](benchmarks/memory/src/main.rs#L960) | Method | `tests::impl Engine for RangeEngine::name` | `fn(&self) -> &'static str` |
| [964](benchmarks/memory/src/main.rs#L964) | Method | `tests::impl Engine for RangeEngine::get` | `fn(&mut self, access: &Access, _downloaded: ByteRange) -> bool` |
| [970](benchmarks/memory/src/main.rs#L970) | Method | `tests::impl Engine for RangeEngine::populate` | `fn(&mut self, access: &Access, downloaded: ByteRange, _payload: Bytes) -> Result<(), String>` |
| [976](benchmarks/memory/src/main.rs#L976) | Method | `tests::impl Engine for RangeEngine::used_payload_bytes` | `fn(&self) -> u64` |
| [982](benchmarks/memory/src/main.rs#L982) | Function | `tests::warmup_preserves_cache_state_but_not_reported_traffic` | `fn()` |
| [1015](benchmarks/memory/src/main.rs#L1015) | Function | `tests::foyer_expanded_key_reuses_identical_expansions_for_distinct_requests` | `fn()` |
| [1045](benchmarks/memory/src/main.rs#L1045) | Function | `tests::expanded_downloader_assigns_one_coalesced_range_to_all_batch_members` | `fn()` |
| [1089](benchmarks/memory/src/main.rs#L1089) | Function | `tests::coalescing_distance_parameter_is_a_strict_upper_bound` | `fn()` |
| [1123](benchmarks/memory/src/main.rs#L1123) | Function | `tests::source_cost_baseline_uses_requested_bytes_not_downloaded_bytes` | `fn()` |
| [1145](benchmarks/memory/src/main.rs#L1145) | Function | `tests::parses_the_captured_trace_shape` | `fn()` |
| [1160](benchmarks/memory/src/main.rs#L1160) | Function | `tests::expanded_downloader_honors_the_whole_split_threshold` | `fn()` |
| [1193](benchmarks/memory/src/main.rs#L1193) | Function | `tests::timestamp_parser_handles_day_boundaries` | `fn()` |
| [1205](benchmarks/memory/src/main.rs#L1205) | Function | `tests::environment_byte_counts_accept_documented_units` | `fn()` |
| [1211](benchmarks/memory/src/main.rs#L1211) | Function | `tests::human_output_is_default_and_csv_is_opt_in` | `fn()` |

<details>
<summary>Local bindings (109)</summary>

| Line | Kind | Name / source parent | Signature or type |
| ---: | --- | --- | --- |
| [170](benchmarks/memory/src/main.rs#L170) | Local | `impl Engine for FeuerEngine::get::Some(bytes)` | — |
| [178](benchmarks/memory/src/main.rs#L178) | Local | `impl Engine for FeuerEngine::populate::download` | — |
| [258](benchmarks/memory/src/main.rs#L258) | Local | `impl Engine for NativeFoyerEngine::get::key` | — |
| [259](benchmarks/memory/src/main.rs#L259) | Local | `impl Engine for NativeFoyerEngine::get::Some(entry)` | — |
| [262](benchmarks/memory/src/main.rs#L262) | Local | `impl Engine for NativeFoyerEngine::get::value` | — |
| [264](benchmarks/memory/src/main.rs#L264) | Local | `impl Engine for NativeFoyerEngine::get::result` | — |
| [270](benchmarks/memory/src/main.rs#L270) | Local | `impl Engine for NativeFoyerEngine::populate::key` | — |
| [285](benchmarks/memory/src/main.rs#L285) | Local | `foyer_cache::builder` | — |
| [330](benchmarks/memory/src/main.rs#L330) | Local | `impl Report::source_cost_hit_rate::fixed_cost` | — |
| [331](benchmarks/memory/src/main.rs#L331) | Local | `impl Report::source_cost_hit_rate::baseline` | — |
| [332](benchmarks/memory/src/main.rs#L332) | Local | `impl Report::source_cost_hit_rate::actual` | — |
| [346](benchmarks/memory/src/main.rs#L346) | Local | `main::args` | — |
| [356](benchmarks/memory/src/main.rs#L356) | Local | `main::download_config` | — |
| [357](benchmarks/memory/src/main.rs#L357) | Local | `main::workload` | — |
| [374](benchmarks/memory/src/main.rs#L374) | Local | `main::max_download` | — |
| [375](benchmarks/memory/src/main.rs#L375) | Local | `main::max_download` | — |
| [376](benchmarks/memory/src/main.rs#L376) | Local | `main::source_payload` | — |
| [380](benchmarks/memory/src/main.rs#L380) | Local | `main::mut engines` | `Vec<Box<dyn Engine>>` |
| [398](benchmarks/memory/src/main.rs#L398) | Local | `main::report` | — |
| [420](benchmarks/memory/src/main.rs#L420) | Local | `trace_workload::mut accesses` | — |
| [435](benchmarks/memory/src/main.rs#L435) | Local | `trace_workload::expanded_downloads` | — |
| [454](benchmarks/memory/src/main.rs#L454) | Local | `run_engine::mut warmup_traffic` | — |
| [458](benchmarks/memory/src/main.rs#L458) | Local | `run_engine::mut traffic` | — |
| [459](benchmarks/memory/src/main.rs#L459) | Local | `run_engine::started` | — |
| [461](benchmarks/memory/src/main.rs#L461) | Local | `run_engine::elapsed` | — |
| [488](benchmarks/memory/src/main.rs#L488) | Local | `expanded_download_ranges::mut by_object` | `HashMap<(&str, u64), Vec<usize>>` |
| [496](benchmarks/memory/src/main.rs#L496) | Local | `expanded_download_ranges::base_ranges` | `Vec<_>` |
| [500](benchmarks/memory/src/main.rs#L500) | Local | `expanded_download_ranges::mut ranges` | — |
| [504](benchmarks/memory/src/main.rs#L504) | Local | `expanded_download_ranges::mut assigned` | — |
| [511](benchmarks/memory/src/main.rs#L511) | Local | `expanded_download_ranges::deadline` | — |
| [514](benchmarks/memory/src/main.rs#L514) | Local | `expanded_download_ranges::pending` | — |
| [525](benchmarks/memory/src/main.rs#L525) | Local | `expanded_download_ranges::download` | — |
| [567](benchmarks/memory/src/main.rs#L567) | Local | `execute_pass::downloaded` | — |
| [571](benchmarks/memory/src/main.rs#L571) | Local | `execute_pass::hit` | — |
| [577](benchmarks/memory/src/main.rs#L577) | Local | `execute_pass::payload` | — |
| [598](benchmarks/memory/src/main.rs#L598) | Local | `coalesced_downloads::mut downloads` | `Vec<CoalescedDownload<'_>>` |
| [600](benchmarks/memory/src/main.rs#L600) | Local | `coalesced_downloads::merge` | — |
| [601](benchmarks/memory/src/main.rs#L601) | Local | `coalesced_downloads::merge::representative` | — |
| [602](benchmarks/memory/src/main.rs#L602) | Local | `coalesced_downloads::merge::gap` | — |
| [634](benchmarks/memory/src/main.rs#L634) | Local | `source_payload_slice::payload_len` | — |
| [643](benchmarks/memory/src/main.rs#L643) | Local | `requested_payload::start` | — |
| [645](benchmarks/memory/src/main.rs#L645) | Local | `requested_payload::end` | — |
| [651](benchmarks/memory/src/main.rs#L651) | Local | `print_human_header::shards` | — |
| [754](benchmarks/memory/src/main.rs#L754) | Local | `load_trace::path` | — |
| [755](benchmarks/memory/src/main.rs#L755) | Local | `load_trace::content` | — |
| [767](benchmarks/memory/src/main.rs#L767) | Local | `parse_trace_line::object_key` | — |
| [768](benchmarks/memory/src/main.rs#L768) | Local | `parse_trace_line::object_size` | — |
| [769](benchmarks/memory/src/main.rs#L769) | Local | `parse_trace_line::start` | — |
| [770](benchmarks/memory/src/main.rs#L770) | Local | `parse_trace_line::end` | — |
| [771](benchmarks/memory/src/main.rs#L771) | Local | `parse_trace_line::requested` | — |
| [772](benchmarks/memory/src/main.rs#L772) | Local | `parse_trace_line::timestamp` | — |
| [773](benchmarks/memory/src/main.rs#L773) | Local | `parse_trace_line::timestamp_millis` | — |
| [783](benchmarks/memory/src/main.rs#L783) | Local | `parse_timestamp_millis::bytes` | — |
| [784](benchmarks/memory/src/main.rs#L784) | Local | `parse_timestamp_millis::valid_suffix` | — |
| [796](benchmarks/memory/src/main.rs#L796) | Local | `parse_timestamp_millis::component` | — |
| [801](benchmarks/memory/src/main.rs#L801) | Local | `parse_timestamp_millis::year` | — |
| [802](benchmarks/memory/src/main.rs#L802) | Local | `parse_timestamp_millis::month` | — |
| [803](benchmarks/memory/src/main.rs#L803) | Local | `parse_timestamp_millis::day` | — |
| [804](benchmarks/memory/src/main.rs#L804) | Local | `parse_timestamp_millis::hour` | — |
| [805](benchmarks/memory/src/main.rs#L805) | Local | `parse_timestamp_millis::minute` | — |
| [806](benchmarks/memory/src/main.rs#L806) | Local | `parse_timestamp_millis::second` | — |
| [807](benchmarks/memory/src/main.rs#L807) | Local | `parse_timestamp_millis::millis` | — |
| [816](benchmarks/memory/src/main.rs#L816) | Local | `parse_timestamp_millis::leap_year` | — |
| [817](benchmarks/memory/src/main.rs#L817) | Local | `parse_timestamp_millis::month_lengths` | — |
| [831](benchmarks/memory/src/main.rs#L831) | Local | `parse_timestamp_millis::month_index` | — |
| [836](benchmarks/memory/src/main.rs#L836) | Local | `parse_timestamp_millis::previous_year` | — |
| [837](benchmarks/memory/src/main.rs#L837) | Local | `parse_timestamp_millis::days_before_year` | — |
| [838](benchmarks/memory/src/main.rs#L838) | Local | `parse_timestamp_millis::days_before_month` | `u64` |
| [839](benchmarks/memory/src/main.rs#L839) | Local | `parse_timestamp_millis::days` | — |
| [844](benchmarks/memory/src/main.rs#L844) | Local | `find_json_string::value` | — |
| [845](benchmarks/memory/src/main.rs#L845) | Local | `find_json_string::value` | — |
| [848](benchmarks/memory/src/main.rs#L848) | Local | `find_json_string::end` | — |
| [855](benchmarks/memory/src/main.rs#L855) | Local | `find_json_u64::value` | — |
| [856](benchmarks/memory/src/main.rs#L856) | Local | `find_json_u64::end` | — |
| [868](benchmarks/memory/src/main.rs#L868) | Local | `field_value::needle` | — |
| [869](benchmarks/memory/src/main.rs#L869) | Local | `field_value::after_field` | — |
| [873](benchmarks/memory/src/main.rs#L873) | Local | `field_value::after_colon` | — |
| [889](benchmarks/memory/src/main.rs#L889) | Local | `byte_count_from_env::value` | — |
| [894](benchmarks/memory/src/main.rs#L894) | Local | `byte_count_from_env::bytes` | — |
| [899](benchmarks/memory/src/main.rs#L899) | Local | `parse_bytes::bytes` | — |
| [904](benchmarks/memory/src/main.rs#L904) | Local | `parse_byte_count::value` | — |
| [905](benchmarks/memory/src/main.rs#L905) | Local | `parse_byte_count::split` | — |
| [908](benchmarks/memory/src/main.rs#L908) | Local | `parse_byte_count::number` | `u128` |
| [911](benchmarks/memory/src/main.rs#L911) | Local | `parse_byte_count::suffix` | — |
| [912](benchmarks/memory/src/main.rs#L912) | Local | `parse_byte_count::multiplier` | — |
| [983](benchmarks/memory/src/main.rs#L983) | Local | `tests::warmup_preserves_cache_state_but_not_reported_traffic::workload` | — |
| [997](benchmarks/memory/src/main.rs#L997) | Local | `tests::warmup_preserves_cache_state_but_not_reported_traffic::report` | — |
| [1016](benchmarks/memory/src/main.rs#L1016) | Local | `tests::foyer_expanded_key_reuses_identical_expansions_for_distinct_requests::access` | — |
| [1022](benchmarks/memory/src/main.rs#L1022) | Local | `tests::foyer_expanded_key_reuses_identical_expansions_for_distinct_requests::first` | — |
| [1023](benchmarks/memory/src/main.rs#L1023) | Local | `tests::foyer_expanded_key_reuses_identical_expansions_for_distinct_requests::second` | — |
| [1024](benchmarks/memory/src/main.rs#L1024) | Local | `tests::foyer_expanded_key_reuses_identical_expansions_for_distinct_requests::expanded` | — |
| [1026](benchmarks/memory/src/main.rs#L1026) | Local | `tests::foyer_expanded_key_reuses_identical_expansions_for_distinct_requests::mut expanded_key` | — |
| [1038](benchmarks/memory/src/main.rs#L1038) | Local | `tests::foyer_expanded_key_reuses_identical_expansions_for_distinct_requests::mut exact_key` | — |
| [1046](benchmarks/memory/src/main.rs#L1046) | Local | `tests::expanded_downloader_assigns_one_coalesced_range_to_all_batch_members::access` | — |
| [1052](benchmarks/memory/src/main.rs#L1052) | Local | `tests::expanded_downloader_assigns_one_coalesced_range_to_all_batch_members::accesses` | — |
| [1058](benchmarks/memory/src/main.rs#L1058) | Local | `tests::expanded_downloader_assigns_one_coalesced_range_to_all_batch_members::download_config` | — |
| [1062](benchmarks/memory/src/main.rs#L1062) | Local | `tests::expanded_downloader_assigns_one_coalesced_range_to_all_batch_members::expanded_downloads` | — |
| [1063](benchmarks/memory/src/main.rs#L1063) | Local | `tests::expanded_downloader_assigns_one_coalesced_range_to_all_batch_members::workload` | — |
| [1071](benchmarks/memory/src/main.rs#L1071) | Local | `tests::expanded_downloader_assigns_one_coalesced_range_to_all_batch_members::report` | — |
| [1090](benchmarks/memory/src/main.rs#L1090) | Local | `tests::coalescing_distance_parameter_is_a_strict_upper_bound::distance` | — |
| [1091](benchmarks/memory/src/main.rs#L1091) | Local | `tests::coalescing_distance_parameter_is_a_strict_upper_bound::ranges_for_gap` | — |
| [1092](benchmarks/memory/src/main.rs#L1092) | Local | `tests::coalescing_distance_parameter_is_a_strict_upper_bound::ranges_for_gap::accesses` | — |
| [1124](benchmarks/memory/src/main.rs#L1124) | Local | `tests::source_cost_baseline_uses_requested_bytes_not_downloaded_bytes::report` | — |
| [1146](benchmarks/memory/src/main.rs#L1146) | Local | `tests::parses_the_captured_trace_shape::access` | — |
| [1161](benchmarks/memory/src/main.rs#L1161) | Local | `tests::expanded_downloader_honors_the_whole_split_threshold::config` | — |
| [1165](benchmarks/memory/src/main.rs#L1165) | Local | `tests::expanded_downloader_honors_the_whole_split_threshold::small_split` | — |
| [1180](benchmarks/memory/src/main.rs#L1180) | Local | `tests::expanded_downloader_honors_the_whole_split_threshold::threshold_split` | — |
| [1194](benchmarks/memory/src/main.rs#L1194) | Local | `tests::timestamp_parser_handles_day_boundaries::before` | — |
| [1195](benchmarks/memory/src/main.rs#L1195) | Local | `tests::timestamp_parser_handles_day_boundaries::after` | — |

</details>

## feuer-storage

### `feuer-storage/examples/direct_io.rs`

| Line | Kind | Name / source parent | Signature or type |
| ---: | --- | --- | --- |
| [13](feuer-storage/examples/direct_io.rs#L13) | Const | `MIB` | `usize` |
| [14](feuer-storage/examples/direct_io.rs#L14) | Const | `CAPACITY` | `u64` |
| [15](feuer-storage/examples/direct_io.rs#L15) | Const | `READ_SPACE` | `u64` |
| [18](feuer-storage/examples/direct_io.rs#L18) | Struct | `Counts` | — |
| [19](feuer-storage/examples/direct_io.rs#L19) | Field | `Counts::operations` | `u64` |
| [20](feuer-storage/examples/direct_io.rs#L20) | Field | `Counts::bytes` | `u64` |
| [21](feuer-storage/examples/direct_io.rs#L21) | Field | `Counts::samples` | `Vec<u64>` |
| [25](feuer-storage/examples/direct_io.rs#L25) | Function | `main` | `fn() -> Result<(), Box<dyn std::error::Error>>` |
| [66](feuer-storage/examples/direct_io.rs#L66) | Function | `run` | `fn(file: &DataFile, size: usize, readers: usize, writers: usize, seconds: u64)` |
| [138](feuer-storage/examples/direct_io.rs#L138) | Function | `cpu_seconds` | `fn() -> f64` |

<details>
<summary>Local bindings (35)</summary>

| Line | Kind | Name / source parent | Signature or type |
| ---: | --- | --- | --- |
| [26](feuer-storage/examples/direct_io.rs#L26) | Local | `main::mut args` | — |
| [27](feuer-storage/examples/direct_io.rs#L27) | Local | `main::root` | — |
| [28](feuer-storage/examples/direct_io.rs#L28) | Local | `main::seconds` | `u64` |
| [30](feuer-storage/examples/direct_io.rs#L30) | Local | `main::write_callers` | `Vec<usize>` |
| [39](feuer-storage/examples/direct_io.rs#L39) | Local | `main::temp` | — |
| [42](feuer-storage/examples/direct_io.rs#L42) | Local | `main::registry` | `mixtrics::metrics::BoxedRegistry` |
| [43](feuer-storage/examples/direct_io.rs#L43) | Local | `main::file` | — |
| [45](feuer-storage/examples/direct_io.rs#L45) | Local | `main::payload` | — |
| [67](feuer-storage/examples/direct_io.rs#L67) | Local | `run::measure_start` | — |
| [68](feuer-storage/examples/direct_io.rs#L68) | Local | `run::deadline` | — |
| [69](feuer-storage/examples/direct_io.rs#L69) | Local | `run::mut tasks` | — |
| [72](feuer-storage/examples/direct_io.rs#L72) | Local | `run::payload` | — |
| [74](feuer-storage/examples/direct_io.rs#L74) | Local | `run::file` | — |
| [75](feuer-storage/examples/direct_io.rs#L75) | Local | `run::payload` | — |
| [77](feuer-storage/examples/direct_io.rs#L77) | Local | `run::reading` | — |
| [78](feuer-storage/examples/direct_io.rs#L78) | Local | `run::mut random` | — |
| [79](feuer-storage/examples/direct_io.rs#L79) | Local | `run::mut counts` | — |
| [80](feuer-storage/examples/direct_io.rs#L80) | Local | `run::mut write_offset` | — |
| [81](feuer-storage/examples/direct_io.rs#L81) | Local | `run::lane_size` | — |
| [83](feuer-storage/examples/direct_io.rs#L83) | Local | `run::started` | — |
| [84](feuer-storage/examples/direct_io.rs#L84) | Local | `run::bytes` | — |
| [88](feuer-storage/examples/direct_io.rs#L88) | Local | `run::bytes::offset` | — |
| [89](feuer-storage/examples/direct_io.rs#L89) | Local | `run::bytes::value` | — |
| [95](feuer-storage/examples/direct_io.rs#L95) | Local | `run::bytes::offset` | — |
| [100](feuer-storage/examples/direct_io.rs#L100) | Local | `run::finished` | — |
| [114](feuer-storage/examples/direct_io.rs#L114) | Local | `run::cpu_start` | — |
| [115](feuer-storage/examples/direct_io.rs#L115) | Local | `run::mut read` | — |
| [116](feuer-storage/examples/direct_io.rs#L116) | Local | `run::mut write` | — |
| [118](feuer-storage/examples/direct_io.rs#L118) | Local | `run::(reading, counts)` | — |
| [119](feuer-storage/examples/direct_io.rs#L119) | Local | `run::total` | — |
| [124](feuer-storage/examples/direct_io.rs#L124) | Local | `run::cpu` | — |
| [126](feuer-storage/examples/direct_io.rs#L126) | Local | `run::percentile` | — |
| [139](feuer-storage/examples/direct_io.rs#L139) | Local | `cpu_seconds::mut usage` | — |
| [143](feuer-storage/examples/direct_io.rs#L143) | Local | `cpu_seconds::usage` | — |
| [144](feuer-storage/examples/direct_io.rs#L144) | Local | `cpu_seconds::seconds` | — |

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
| [23](feuer-storage/src/error.rs#L23) | Variant | `IoOperation::SyncData` | — |
| [25](feuer-storage/src/error.rs#L25) | Variant | `IoOperation::SyncAll` | — |
| [28](feuer-storage/src/error.rs#L28) | Impl | `impl IoOperation` | — |
| [30](feuer-storage/src/error.rs#L30) | Method | `impl IoOperation::as_str` | `fn(self) -> &'static str` |
| [46](feuer-storage/src/error.rs#L46) | Impl | `impl fmt::Display for IoOperation` | — |
| [47](feuer-storage/src/error.rs#L47) | Method | `impl fmt::Display for IoOperation::fmt` | `fn(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result` |
| [54](feuer-storage/src/error.rs#L54) | Enum | `ErrorKind` | — |
| [56](feuer-storage/src/error.rs#L56) | Variant | `ErrorKind::InvalidConfiguration` | — |
| [58](feuer-storage/src/error.rs#L58) | Variant | `ErrorKind::AlreadyOpen` | — |
| [60](feuer-storage/src/error.rs#L60) | Variant | `ErrorKind::OutOfBounds` | — |
| [62](feuer-storage/src/error.rs#L62) | Variant | `ErrorKind::Allocation` | — |
| [64](feuer-storage/src/error.rs#L64) | Variant | `ErrorKind::Io` | — |
| [66](feuer-storage/src/error.rs#L66) | Variant | `ErrorKind::Task` | — |
| [69](feuer-storage/src/error.rs#L69) | Impl | `impl ErrorKind` | — |
| [71](feuer-storage/src/error.rs#L71) | Method | `impl ErrorKind::as_str` | `fn(self) -> &'static str` |
| [83](feuer-storage/src/error.rs#L83) | Impl | `impl fmt::Display for ErrorKind` | — |
| [84](feuer-storage/src/error.rs#L84) | Method | `impl fmt::Display for ErrorKind::fmt` | `fn(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result` |
| [92](feuer-storage/src/error.rs#L92) | Enum | `Error` | — |
| [95](feuer-storage/src/error.rs#L95) | Variant | `Error::InvalidCapacity` | — |
| [98](feuer-storage/src/error.rs#L98) | Variant | `Error::AlreadyOpen` | — |
| [100](feuer-storage/src/error.rs#L100) | Field | `Error::AlreadyOpen::directory` | `PathBuf` |
| [104](feuer-storage/src/error.rs#L104) | Variant | `Error::InvalidDataFile` | — |
| [106](feuer-storage/src/error.rs#L106) | Field | `Error::InvalidDataFile::path` | `PathBuf` |
| [110](feuer-storage/src/error.rs#L110) | Variant | `Error::OutOfBounds` | — |
| [112](feuer-storage/src/error.rs#L112) | Field | `Error::OutOfBounds::operation` | `IoOperation` |
| [114](feuer-storage/src/error.rs#L114) | Field | `Error::OutOfBounds::offset` | `u64` |
| [116](feuer-storage/src/error.rs#L116) | Field | `Error::OutOfBounds::length` | `u64` |
| [118](feuer-storage/src/error.rs#L118) | Field | `Error::OutOfBounds::capacity` | `u64` |
| [122](feuer-storage/src/error.rs#L122) | Variant | `Error::LengthOverflow` | — |
| [124](feuer-storage/src/error.rs#L124) | Field | `Error::LengthOverflow::operation` | `IoOperation` |
| [126](feuer-storage/src/error.rs#L126) | Field | `Error::LengthOverflow::length` | `usize` |
| [130](feuer-storage/src/error.rs#L130) | Variant | `Error::Allocation` | — |
| [132](feuer-storage/src/error.rs#L132) | Field | `Error::Allocation::length` | `usize` |
| [135](feuer-storage/src/error.rs#L135) | Field | `Error::Allocation::source` | `TryReserveError` |
| [139](feuer-storage/src/error.rs#L139) | Variant | `Error::Io` | — |
| [141](feuer-storage/src/error.rs#L141) | Field | `Error::Io::operation` | `IoOperation` |
| [143](feuer-storage/src/error.rs#L143) | Field | `Error::Io::path` | `PathBuf` |
| [146](feuer-storage/src/error.rs#L146) | Field | `Error::Io::source` | `io::Error` |
| [150](feuer-storage/src/error.rs#L150) | Variant | `Error::RuntimeUnavailable` | — |
| [153](feuer-storage/src/error.rs#L153) | Variant | `Error::Task` | — |
| [155](feuer-storage/src/error.rs#L155) | Field | `Error::Task::operation` | `IoOperation` |
| [158](feuer-storage/src/error.rs#L158) | Field | `Error::Task::source` | `Box<dyn std::error::Error + Send + Sync>` |
| [162](feuer-storage/src/error.rs#L162) | Impl | `impl Error` | — |
| [164](feuer-storage/src/error.rs#L164) | Method | `impl Error::kind` | `fn(&self) -> ErrorKind` |
| [176](feuer-storage/src/error.rs#L176) | Method | `impl Error::operation` | `fn(&self) -> IoOperation` |
| [192](feuer-storage/src/error.rs#L192) | TypeAlias | `Result` | `std::result::Result<T, Error>` |

### `feuer-storage/src/file.rs`

| Line | Kind | Name / source parent | Signature or type |
| ---: | --- | --- | --- |
| [18](feuer-storage/src/file.rs#L18) | Const | `DATA_FILE_NAME` | `&str` |
| [19](feuer-storage/src/file.rs#L19) | Const | `LOCK_FILE_NAME` | `&str` |
| [21](feuer-storage/src/file.rs#L21) | Struct | `Inner` | — |
| [22](feuer-storage/src/file.rs#L22) | Field | `Inner::driver` | `uring::Handle` |
| [23](feuer-storage/src/file.rs#L23) | Field | `Inner::data_path` | `PathBuf` |
| [24](feuer-storage/src/file.rs#L24) | Field | `Inner::capacity` | `u64` |
| [44](feuer-storage/src/file.rs#L44) | Struct | `DataFile` | — |
| [45](feuer-storage/src/file.rs#L45) | Field | `DataFile::inner` | `Arc<Inner>` |
| [46](feuer-storage/src/file.rs#L46) | Field | `DataFile::metrics` | `Arc<IoMetrics>` |
| [49](feuer-storage/src/file.rs#L49) | Impl | `impl fmt::Debug for DataFile` | — |
| [50](feuer-storage/src/file.rs#L50) | Method | `impl fmt::Debug for DataFile::fmt` | `fn(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result` |
| [57](feuer-storage/src/file.rs#L57) | Impl | `impl DataFile` | — |
| [63](feuer-storage/src/file.rs#L63) | Function | `impl DataFile::open` | `fn(directory: impl AsRef<Path>, capacity: u64, metrics: Arc<IoMetrics>) -> Result<Self>` |
| [105](feuer-storage/src/file.rs#L105) | Method | `impl DataFile::capacity` | `fn(&self) -> u64` |
| [111](feuer-storage/src/file.rs#L111) | Method | `impl DataFile::read_at` | `fn(&self, offset: u64, length: usize) -> Result<Bytes>` |
| [120](feuer-storage/src/file.rs#L120) | Method | `impl DataFile::write_at` | `fn(&self, offset: u64, bytes: &Bytes) -> Result<()>` |
| [128](feuer-storage/src/file.rs#L128) | Method | `impl DataFile::sync_data` | `fn(&self) -> Result<()>` |
| [134](feuer-storage/src/file.rs#L134) | Method | `impl DataFile::sync_all` | `fn(&self) -> Result<()>` |
| [138](feuer-storage/src/file.rs#L138) | Method | `impl DataFile::execute` | `fn(&self, operation: IoOperation, offset: u64, length: usize, payload: &[u8]) -> Result<Bytes>` |
| [161](feuer-storage/src/file.rs#L161) | Method | `impl DataFile::execute_inner` | `fn(&self, operation: IoOperation, offset: u64, length: usize, payload: &[u8]) -> Result<Bytes>` |
| [205](feuer-storage/src/file.rs#L205) | Function | `open_inner` | `fn(directory: PathBuf, capacity: u64) -> Result<Inner>` |
| [271](feuer-storage/src/file.rs#L271) | Function | `check_direct_alignment` | `fn(file: &File) -> io::Result<()>` |
| [302](feuer-storage/src/file.rs#L302) | Function | `check_range` | `fn(operation: IoOperation, offset: u64, length: u64, capacity: u64) -> Result<()>` |
| [314](feuer-storage/src/file.rs#L314) | Function | `record_span_outcome` | `fn<T>(span: &Span, elapsed: std::time::Duration, result: &Result<T>)` |
| [328](feuer-storage/src/file.rs#L328) | Module | `tests` | — |
| [333](feuer-storage/src/file.rs#L333) | Const | `tests::CAPACITY` | `u64` |
| [336](feuer-storage/src/file.rs#L336) | Function | `tests::public_io_types_are_send_sync_static` | `fn()` |
| [337](feuer-storage/src/file.rs#L337) | Function | `tests::public_io_types_are_send_sync_static::assert_send_sync_static` | `fn<T: Send + Sync + 'static>()` |
| [343](feuer-storage/src/file.rs#L343) | Function | `tests::reads_exact_unaligned_ranges_and_preserves_neighbors` | `fn()` |
| [372](feuer-storage/src/file.rs#L372) | Function | `tests::rejects_out_of_bounds_and_accepts_empty_ranges` | `fn()` |
| [391](feuer-storage/src/file.rs#L391) | Function | `tests::fails_short_reads_instead_of_returning_uncertain_bytes` | `fn()` |
| [407](feuer-storage/src/file.rs#L407) | Function | `tests::holds_exclusive_ownership_and_reopens_after_shutdown` | `fn()` |
| [430](feuer-storage/src/file.rs#L430) | Function | `tests::concurrent_mixed_io_and_same_page_rmw_do_not_lose_updates` | `fn()` |
| [461](feuer-storage/src/file.rs#L461) | Function | `tests::canceled_callers_do_not_release_submitted_buffers_or_lock_early` | `fn()` |
| [490](feuer-storage/src/file.rs#L490) | Function | `tests::reports_missing_runtime` | `fn()` |
| [505](feuer-storage/src/file.rs#L505) | Function | `tests::rejects_unverified_direct_io_instead_of_falling_back` | `fn()` |
| [521](feuer-storage/src/file.rs#L521) | Function | `tests::validates_capacity_and_omits_paths_from_debug` | `fn()` |

<details>
<summary>Local bindings (65)</summary>

| Line | Kind | Name / source parent | Signature or type |
| ---: | --- | --- | --- |
| [64](feuer-storage/src/file.rs#L64) | Local | `impl DataFile::open::directory` | — |
| [65](feuer-storage/src/file.rs#L65) | Local | `impl DataFile::open::runtime` | — |
| [66](feuer-storage/src/file.rs#L66) | Local | `impl DataFile::open::started` | — |
| [67](feuer-storage/src/file.rs#L67) | Local | `impl DataFile::open::span` | — |
| [75](feuer-storage/src/file.rs#L75) | Local | `impl DataFile::open::result` | — |
| [76](feuer-storage/src/file.rs#L76) | Local | `impl DataFile::open::result::inner` | — |
| [139](feuer-storage/src/file.rs#L139) | Local | `impl DataFile::execute::started` | — |
| [140](feuer-storage/src/file.rs#L140) | Local | `impl DataFile::execute::observed_bytes` | — |
| [141](feuer-storage/src/file.rs#L141) | Local | `impl DataFile::execute::span` | — |
| [151](feuer-storage/src/file.rs#L151) | Local | `impl DataFile::execute::result` | — |
| [155](feuer-storage/src/file.rs#L155) | Local | `impl DataFile::execute::elapsed` | — |
| [162](feuer-storage/src/file.rs#L162) | Local | `impl DataFile::execute_inner::length_u64` | — |
| [164](feuer-storage/src/file.rs#L164) | Local | `impl DataFile::execute_inner::io_error` | — |
| [172](feuer-storage/src/file.rs#L172) | Local | `impl DataFile::execute_inner::mut result` | — |
| [178](feuer-storage/src/file.rs#L178) | Local | `impl DataFile::execute_inner::mut completed` | — |
| [180](feuer-storage/src/file.rs#L180) | Local | `impl DataFile::execute_inner::at` | — |
| [181](feuer-storage/src/file.rs#L181) | Local | `impl DataFile::execute_inner::chunk` | — |
| [182](feuer-storage/src/file.rs#L182) | Local | `impl DataFile::execute_inner::input` | — |
| [187](feuer-storage/src/file.rs#L187) | Local | `impl DataFile::execute_inner::bytes` | — |
| [214](feuer-storage/src/file.rs#L214) | Local | `open_inner::lock_path` | — |
| [215](feuer-storage/src/file.rs#L215) | Local | `open_inner::lock_file` | — |
| [226](feuer-storage/src/file.rs#L226) | Local | `open_inner::locked` | — |
| [234](feuer-storage/src/file.rs#L234) | Local | `open_inner::data_path` | — |
| [235](feuer-storage/src/file.rs#L235) | Local | `open_inner::error` | — |
| [240](feuer-storage/src/file.rs#L240) | Local | `open_inner::file` | — |
| [257](feuer-storage/src/file.rs#L257) | Local | `open_inner::resize` | — |
| [260](feuer-storage/src/file.rs#L260) | Local | `open_inner::driver` | — |
| [272](feuer-storage/src/file.rs#L272) | Local | `check_direct_alignment::mut stat` | — |
| [274](feuer-storage/src/file.rs#L274) | Local | `check_direct_alignment::result` | — |
| [287](feuer-storage/src/file.rs#L287) | Local | `check_direct_alignment::stat` | — |
| [344](feuer-storage/src/file.rs#L344) | Local | `tests::reads_exact_unaligned_ranges_and_preserves_neighbors::temp` | — |
| [345](feuer-storage/src/file.rs#L345) | Local | `tests::reads_exact_unaligned_ranges_and_preserves_neighbors::directory` | — |
| [346](feuer-storage/src/file.rs#L346) | Local | `tests::reads_exact_unaligned_ranges_and_preserves_neighbors::file` | — |
| [347](feuer-storage/src/file.rs#L347) | Local | `tests::reads_exact_unaligned_ranges_and_preserves_neighbors::original` | — |
| [349](feuer-storage/src/file.rs#L349) | Local | `tests::reads_exact_unaligned_ranges_and_preserves_neighbors::payload` | — |
| [358](feuer-storage/src/file.rs#L358) | Local | `tests::reads_exact_unaligned_ranges_and_preserves_neighbors::payload` | — |
| [373](feuer-storage/src/file.rs#L373) | Local | `tests::rejects_out_of_bounds_and_accepts_empty_ranges::temp` | — |
| [374](feuer-storage/src/file.rs#L374) | Local | `tests::rejects_out_of_bounds_and_accepts_empty_ranges::file` | — |
| [392](feuer-storage/src/file.rs#L392) | Local | `tests::fails_short_reads_instead_of_returning_uncertain_bytes::temp` | — |
| [393](feuer-storage/src/file.rs#L393) | Local | `tests::fails_short_reads_instead_of_returning_uncertain_bytes::file` | — |
| [408](feuer-storage/src/file.rs#L408) | Local | `tests::holds_exclusive_ownership_and_reopens_after_shutdown::temp` | — |
| [409](feuer-storage/src/file.rs#L409) | Local | `tests::holds_exclusive_ownership_and_reopens_after_shutdown::file` | — |
| [410](feuer-storage/src/file.rs#L410) | Local | `tests::holds_exclusive_ownership_and_reopens_after_shutdown::clone` | — |
| [422](feuer-storage/src/file.rs#L422) | Local | `tests::holds_exclusive_ownership_and_reopens_after_shutdown::file` | — |
| [431](feuer-storage/src/file.rs#L431) | Local | `tests::concurrent_mixed_io_and_same_page_rmw_do_not_lose_updates::temp` | — |
| [432](feuer-storage/src/file.rs#L432) | Local | `tests::concurrent_mixed_io_and_same_page_rmw_do_not_lose_updates::file` | — |
| [433](feuer-storage/src/file.rs#L433) | Local | `tests::concurrent_mixed_io_and_same_page_rmw_do_not_lose_updates::mut tasks` | — |
| [437](feuer-storage/src/file.rs#L437) | Local | `tests::concurrent_mixed_io_and_same_page_rmw_do_not_lose_updates::file` | — |
| [439](feuer-storage/src/file.rs#L439) | Local | `tests::concurrent_mixed_io_and_same_page_rmw_do_not_lose_updates::value` | — |
| [442](feuer-storage/src/file.rs#L442) | Local | `tests::concurrent_mixed_io_and_same_page_rmw_do_not_lose_updates::value` | — |
| [443](feuer-storage/src/file.rs#L443) | Local | `tests::concurrent_mixed_io_and_same_page_rmw_do_not_lose_updates::offset` | — |
| [462](feuer-storage/src/file.rs#L462) | Local | `tests::canceled_callers_do_not_release_submitted_buffers_or_lock_early::temp` | — |
| [463](feuer-storage/src/file.rs#L463) | Local | `tests::canceled_callers_do_not_release_submitted_buffers_or_lock_early::file` | — |
| [464](feuer-storage/src/file.rs#L464) | Local | `tests::canceled_callers_do_not_release_submitted_buffers_or_lock_early::mut tasks` | — |
| [466](feuer-storage/src/file.rs#L466) | Local | `tests::canceled_callers_do_not_release_submitted_buffers_or_lock_early::file` | — |
| [476](feuer-storage/src/file.rs#L476) | Local | `tests::canceled_callers_do_not_release_submitted_buffers_or_lock_early::_` | — |
| [479](feuer-storage/src/file.rs#L479) | Local | `tests::canceled_callers_do_not_release_submitted_buffers_or_lock_early::file` | — |
| [495](feuer-storage/src/file.rs#L495) | Local | `tests::reports_missing_runtime::temp` | — |
| [496](feuer-storage/src/file.rs#L496) | Local | `tests::reports_missing_runtime::mut open` | — |
| [497](feuer-storage/src/file.rs#L497) | Local | `tests::reports_missing_runtime::mut context` | — |
| [498](feuer-storage/src/file.rs#L498) | Local | `tests::reports_missing_runtime::Poll::Ready(result)` | — |
| [510](feuer-storage/src/file.rs#L510) | Local | `tests::rejects_unverified_direct_io_instead_of_falling_back::fd` | — |
| [513](feuer-storage/src/file.rs#L513) | Local | `tests::rejects_unverified_direct_io_instead_of_falling_back::file` | — |
| [522](feuer-storage/src/file.rs#L522) | Local | `tests::validates_capacity_and_omits_paths_from_debug::temp` | — |
| [532](feuer-storage/src/file.rs#L532) | Local | `tests::validates_capacity_and_omits_paths_from_debug::file` | — |

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
| [30](feuer-storage/src/metrics.rs#L30) | Field | `IoMetrics::sync_data` | `OperationMetrics` |
| [31](feuer-storage/src/metrics.rs#L31) | Field | `IoMetrics::sync_all` | `OperationMetrics` |
| [34](feuer-storage/src/metrics.rs#L34) | Impl | `impl IoMetrics` | — |
| [36](feuer-storage/src/metrics.rs#L36) | Function | `impl IoMetrics::new` | `fn(registry: &BoxedRegistry) -> Arc<Self>` |
| [70](feuer-storage/src/metrics.rs#L70) | Method | `impl IoMetrics::record` | `fn(&self, operation: IoOperation, bytes: u64, elapsed: Duration, success: bool)` |
| [90](feuer-storage/src/metrics.rs#L90) | Function | `impl IoMetrics::noop` | `fn() -> Arc<Self>` |
| [97](feuer-storage/src/metrics.rs#L97) | Module | `tests` | — |
| [101](feuer-storage/src/metrics.rs#L101) | Function | `tests::registers_with_the_normal_registry_boundary` | `fn()` |

<details>
<summary>Local bindings (7)</summary>

| Line | Kind | Name / source parent | Signature or type |
| ---: | --- | --- | --- |
| [37](feuer-storage/src/metrics.rs#L37) | Local | `impl IoMetrics::new::operations` | — |
| [42](feuer-storage/src/metrics.rs#L42) | Local | `impl IoMetrics::new::bytes` | — |
| [47](feuer-storage/src/metrics.rs#L47) | Local | `impl IoMetrics::new::duration` | — |
| [54](feuer-storage/src/metrics.rs#L54) | Local | `impl IoMetrics::new::operation` | — |
| [71](feuer-storage/src/metrics.rs#L71) | Local | `impl IoMetrics::record::metrics` | — |
| [91](feuer-storage/src/metrics.rs#L91) | Local | `impl IoMetrics::noop::registry` | `BoxedRegistry` |
| [102](feuer-storage/src/metrics.rs#L102) | Local | `tests::registers_with_the_normal_registry_boundary::metrics` | — |

</details>

### `feuer-storage/src/uring/tests.rs`

| Line | Kind | Name / source parent | Signature or type |
| ---: | --- | --- | --- |
| [5](feuer-storage/src/uring/tests.rs#L5) | TypeAlias | `Reply` | `oneshot::Receiver<io::Result<Bytes>>` |
| [7](feuer-storage/src/uring/tests.rs#L7) | Function | `request` | `fn(driver: &Driver, operation: IoOperation, offset: u64, length: usize) -> (Request, Reply)` |
| [26](feuer-storage/src/uring/tests.rs#L26) | Function | `driver` | `fn() -> Driver` |
| [54](feuer-storage/src/uring/tests.rs#L54) | Function | `fills_qd64_with_simultaneous_reads_and_writes_and_bounds_admission` | `fn()` |
| [113](feuer-storage/src/uring/tests.rs#L113) | Function | `complete_one` | `fn(driver: &mut Driver)` |
| [122](feuer-storage/src/uring/tests.rs#L122) | Function | `write_only_fills_the_ring_but_an_arriving_read_gets_the_next_slot` | `fn()` |
| [166](feuer-storage/src/uring/tests.rs#L166) | Function | `read_demand_before_admission_throttles_writes_and_cancellation_restores_full_speed` | `fn()` |
| [188](feuer-storage/src/uring/tests.rs#L188) | Function | `full_write_admission_and_buffers_leave_a_full_read_ring_available` | `fn()` |
| [220](feuer-storage/src/uring/tests.rs#L220) | Function | `overlap_and_sync_block_only_the_requests_they_must` | `fn()` |
| [245](feuer-storage/src/uring/tests.rs#L245) | Function | `discarded_queued_requests_never_reach_the_ring` | `fn()` |
| [264](feuer-storage/src/uring/tests.rs#L264) | Function | `completion_state_handles_short_io_errors_and_rmw` | `fn()` |
| [291](feuer-storage/src/uring/tests.rs#L291) | Function | `byte_budget_bounds_rmw_requests_and_releases_on_cancel` | `fn()` |

<details>
<summary>Local bindings (41)</summary>

| Line | Kind | Name / source parent | Signature or type |
| ---: | --- | --- | --- |
| [8](feuer-storage/src/uring/tests.rs#L8) | Local | `request::(reply, receive)` | — |
| [9](feuer-storage/src/uring/tests.rs#L9) | Local | `request::class` | — |
| [10](feuer-storage/src/uring/tests.rs#L10) | Local | `request::slots` | — |
| [11](feuer-storage/src/uring/tests.rs#L11) | Local | `request::bytes` | — |
| [15](feuer-storage/src/uring/tests.rs#L15) | Local | `request::payload` | — |
| [27](feuer-storage/src/uring/tests.rs#L27) | Local | `driver::temporary` | — |
| [28](feuer-storage/src/uring/tests.rs#L28) | Local | `driver::file` | — |
| [36](feuer-storage/src/uring/tests.rs#L36) | Local | `driver::fd` | — |
| [39](feuer-storage/src/uring/tests.rs#L39) | Local | `driver::wake` | — |
| [40](feuer-storage/src/uring/tests.rs#L40) | Local | `driver::(_, receiver)` | — |
| [55](feuer-storage/src/uring/tests.rs#L55) | Local | `fills_qd64_with_simultaneous_reads_and_writes_and_bounds_admission::mut driver` | — |
| [56](feuer-storage/src/uring/tests.rs#L56) | Local | `fills_qd64_with_simultaneous_reads_and_writes_and_bounds_admission::mut replies` | — |
| [58](feuer-storage/src/uring/tests.rs#L58) | Local | `fills_qd64_with_simultaneous_reads_and_writes_and_bounds_admission::operation` | — |
| [63](feuer-storage/src/uring/tests.rs#L63) | Local | `fills_qd64_with_simultaneous_reads_and_writes_and_bounds_admission::(request, reply)` | — |
| [115](feuer-storage/src/uring/tests.rs#L115) | Local | `complete_one::cqe` | — |
| [116](feuer-storage/src/uring/tests.rs#L116) | Local | `complete_one::mut request` | — |
| [123](feuer-storage/src/uring/tests.rs#L123) | Local | `write_only_fills_the_ring_but_an_arriving_read_gets_the_next_slot::mut driver` | — |
| [124](feuer-storage/src/uring/tests.rs#L124) | Local | `write_only_fills_the_ring_but_an_arriving_read_gets_the_next_slot::mut replies` | — |
| [126](feuer-storage/src/uring/tests.rs#L126) | Local | `write_only_fills_the_ring_but_an_arriving_read_gets_the_next_slot::(request, reply)` | — |
| [133](feuer-storage/src/uring/tests.rs#L133) | Local | `write_only_fills_the_ring_but_an_arriving_read_gets_the_next_slot::(read, read_reply)` | — |
| [144](feuer-storage/src/uring/tests.rs#L144) | Local | `write_only_fills_the_ring_but_an_arriving_read_gets_the_next_slot::(write, reply)` | — |
| [167](feuer-storage/src/uring/tests.rs#L167) | Local | `read_demand_before_admission_throttles_writes_and_cancellation_restores_full_speed::mut driver` | — |
| [168](feuer-storage/src/uring/tests.rs#L168) | Local | `read_demand_before_admission_throttles_writes_and_cancellation_restores_full_speed::mut replies` | — |
| [171](feuer-storage/src/uring/tests.rs#L171) | Local | `read_demand_before_admission_throttles_writes_and_cancellation_restores_full_speed::(request, reply)` | — |
| [175](feuer-storage/src/uring/tests.rs#L175) | Local | `read_demand_before_admission_throttles_writes_and_cancellation_restores_full_speed::read` | — |
| [189](feuer-storage/src/uring/tests.rs#L189) | Local | `full_write_admission_and_buffers_leave_a_full_read_ring_available::driver` | — |
| [190](feuer-storage/src/uring/tests.rs#L190) | Local | `full_write_admission_and_buffers_leave_a_full_read_ring_available::mut writes` | — |
| [191](feuer-storage/src/uring/tests.rs#L191) | Local | `full_write_admission_and_buffers_leave_a_full_read_ring_available::mut reads` | — |
| [221](feuer-storage/src/uring/tests.rs#L221) | Local | `overlap_and_sync_block_only_the_requests_they_must::mut driver` | — |
| [222](feuer-storage/src/uring/tests.rs#L222) | Local | `overlap_and_sync_block_only_the_requests_they_must::mut replies` | — |
| [231](feuer-storage/src/uring/tests.rs#L231) | Local | `overlap_and_sync_block_only_the_requests_they_must::(request, reply)` | — |
| [246](feuer-storage/src/uring/tests.rs#L246) | Local | `discarded_queued_requests_never_reach_the_ring::mut driver` | — |
| [247](feuer-storage/src/uring/tests.rs#L247) | Local | `discarded_queued_requests_never_reach_the_ring::(request, reply)` | — |
| [265](feuer-storage/src/uring/tests.rs#L265) | Local | `completion_state_handles_short_io_errors_and_rmw::driver` | — |
| [266](feuer-storage/src/uring/tests.rs#L266) | Local | `completion_state_handles_short_io_errors_and_rmw::(mut read, _reply)` | — |
| [272](feuer-storage/src/uring/tests.rs#L272) | Local | `completion_state_handles_short_io_errors_and_rmw::(mut read, _reply)` | — |
| [274](feuer-storage/src/uring/tests.rs#L274) | Local | `completion_state_handles_short_io_errors_and_rmw::(mut write, _reply)` | — |
| [281](feuer-storage/src/uring/tests.rs#L281) | Local | `completion_state_handles_short_io_errors_and_rmw::(mut rmw, _reply)` | — |
| [292](feuer-storage/src/uring/tests.rs#L292) | Local | `byte_budget_bounds_rmw_requests_and_releases_on_cancel::driver` | — |
| [293](feuer-storage/src/uring/tests.rs#L293) | Local | `byte_budget_bounds_rmw_requests_and_releases_on_cancel::mut requests` | — |
| [301](feuer-storage/src/uring/tests.rs#L301) | Local | `byte_budget_bounds_rmw_requests_and_releases_on_cancel::_read` | — |

</details>

### `feuer-storage/src/uring.rs`

| Line | Kind | Name / source parent | Signature or type |
| ---: | --- | --- | --- |
| [26](feuer-storage/src/uring.rs#L26) | Const | `ALIGN` | `usize` |
| [29](feuer-storage/src/uring.rs#L29) | Const | `MAX_IO_CHUNK_BYTES` | `usize` |
| [31](feuer-storage/src/uring.rs#L31) | Const | `MAX_IN_FLIGHT_IO` | `usize` |
| [34](feuer-storage/src/uring.rs#L34) | Const | `WRITES_WITH_READS` | `usize` |
| [37](feuer-storage/src/uring.rs#L37) | Const | `REQUESTS` | `usize` |
| [41](feuer-storage/src/uring.rs#L41) | Const | `BUFFER_BYTES` | `usize` |
| [44](feuer-storage/src/uring.rs#L44) | Module | `tests` | — |
| [46](feuer-storage/src/uring.rs#L46) | Struct | `Handle` | — |
| [47](feuer-storage/src/uring.rs#L47) | Field | `Handle::sender` | `Option<mpsc::SyncSender<Request>>` |
| [48](feuer-storage/src/uring.rs#L48) | Field | `Handle::wake` | `Arc<OwnedFd>` |
| [49](feuer-storage/src/uring.rs#L49) | Field | `Handle::thread` | `Option<JoinHandle<()>>` |
| [50](feuer-storage/src/uring.rs#L50) | Field | `Handle::admission` | `Arc<Admission>` |
| [55](feuer-storage/src/uring.rs#L55) | Struct | `Admission` | — |
| [56](feuer-storage/src/uring.rs#L56) | Field | `Admission::requests` | `[Arc<Semaphore>; 2]` |
| [57](feuer-storage/src/uring.rs#L57) | Field | `Admission::buffers` | `[Arc<Semaphore>; 2]` |
| [58](feuer-storage/src/uring.rs#L58) | Field | `Admission::reads` | `AtomicUsize` |
| [61](feuer-storage/src/uring.rs#L61) | Impl | `impl Admission` | — |
| [62](feuer-storage/src/uring.rs#L62) | Function | `impl Admission::new` | `fn() -> Self` |
| [74](feuer-storage/src/uring.rs#L74) | Struct | `ReadDemandGuard` | — |
| [75](feuer-storage/src/uring.rs#L75) | Field | `ReadDemandGuard::admission` | `Arc<Admission>` |
| [76](feuer-storage/src/uring.rs#L76) | Field | `ReadDemandGuard::wake` | `Arc<OwnedFd>` |
| [79](feuer-storage/src/uring.rs#L79) | Impl | `impl ReadDemandGuard` | — |
| [80](feuer-storage/src/uring.rs#L80) | Function | `impl ReadDemandGuard::new` | `fn(admission: &Arc<Admission>, wake: &Arc<OwnedFd>) -> Self` |
| [91](feuer-storage/src/uring.rs#L91) | Impl | `impl Drop for ReadDemandGuard` | — |
| [92](feuer-storage/src/uring.rs#L92) | Method | `impl Drop for ReadDemandGuard::drop` | `fn(&mut self)` |
| [99](feuer-storage/src/uring.rs#L99) | Function | `buffer_charge` | `fn(operation: IoOperation, offset: u64, length: usize) -> u32` |
| [106](feuer-storage/src/uring.rs#L106) | Impl | `impl Handle` | — |
| [107](feuer-storage/src/uring.rs#L107) | Function | `impl Handle::new` | `fn(file: File, lock: File) -> io::Result<Self>` |
| [141](feuer-storage/src/uring.rs#L141) | Method | `impl Handle::execute` | `fn( &self, operation: IoOperation, offset: u64, length: usize, payload: &[u8], ) -> io::Result<Bytes>` |
| [169](feuer-storage/src/uring.rs#L169) | Impl | `impl Drop for Handle` | — |
| [170](feuer-storage/src/uring.rs#L170) | Method | `impl Drop for Handle::drop` | `fn(&mut self)` |
| [180](feuer-storage/src/uring.rs#L180) | Function | `stopped` | `fn() -> io::Error` |
| [184](feuer-storage/src/uring.rs#L184) | Function | `is_sync` | `fn(operation: IoOperation) -> bool` |
| [188](feuer-storage/src/uring.rs#L188) | Struct | `AlignedBuffer` | — |
| [189](feuer-storage/src/uring.rs#L189) | Field | `AlignedBuffer::ptr` | `NonNull<u8>` |
| [190](feuer-storage/src/uring.rs#L190) | Field | `AlignedBuffer::layout` | `Layout` |
| [193](feuer-storage/src/uring.rs#L193) | Impl | `impl AlignedBuffer` | — |
| [194](feuer-storage/src/uring.rs#L194) | Function | `impl AlignedBuffer::new` | `fn(length: usize) -> io::Result<Self>` |
| [202](feuer-storage/src/uring.rs#L202) | Method | `impl AlignedBuffer::bytes` | `fn(&mut self) -> &mut [u8]` |
| [210](feuer-storage/src/uring.rs#L210) | Impl | `impl Send for AlignedBuffer` | — |
| [212](feuer-storage/src/uring.rs#L212) | Impl | `impl Drop for AlignedBuffer` | — |
| [213](feuer-storage/src/uring.rs#L213) | Method | `impl Drop for AlignedBuffer::drop` | `fn(&mut self)` |
| [219](feuer-storage/src/uring.rs#L219) | Struct | `Request` | — |
| [220](feuer-storage/src/uring.rs#L220) | Field | `Request::operation` | `IoOperation` |
| [221](feuer-storage/src/uring.rs#L221) | Field | `Request::offset` | `u64` |
| [222](feuer-storage/src/uring.rs#L222) | Field | `Request::prefix` | `usize` |
| [223](feuer-storage/src/uring.rs#L223) | Field | `Request::length` | `usize` |
| [224](feuer-storage/src/uring.rs#L224) | Field | `Request::aligned_length` | `usize` |
| [225](feuer-storage/src/uring.rs#L225) | Field | `Request::buffer` | `AlignedBuffer` |
| [226](feuer-storage/src/uring.rs#L226) | Field | `Request::payload` | `Option<Bytes>` |
| [227](feuer-storage/src/uring.rs#L227) | Field | `Request::reading` | `bool` |
| [228](feuer-storage/src/uring.rs#L228) | Field | `Request::completed` | `usize` |
| [229](feuer-storage/src/uring.rs#L229) | Field | `Request::reply` | `Option<oneshot::Sender<io::Result<Bytes>>>` |
| [230](feuer-storage/src/uring.rs#L230) | Field | `Request::_slot` | `OwnedSemaphorePermit` |
| [231](feuer-storage/src/uring.rs#L231) | Field | `Request::_bytes` | `OwnedSemaphorePermit` |
| [234](feuer-storage/src/uring.rs#L234) | Impl | `impl Request` | — |
| [235](feuer-storage/src/uring.rs#L235) | Function | `impl Request::new` | `fn( operation: IoOperation, offset: u64, length: usize, payload: &[u8], reply: oneshot::Sender<io::Result<Bytes>>, permits: (OwnedSemaphorePermit, OwnedSemaphorePermit), ) -> io::Result<Self>` |
| [271](feuer-storage/src/uring.rs#L271) | Method | `impl Request::conflicts` | `fn(&self, other: &Self) -> bool` |
| [279](feuer-storage/src/uring.rs#L279) | Method | `impl Request::entry` | `fn(&mut self, fd: i32, slot: usize) -> squeue::Entry` |
| [304](feuer-storage/src/uring.rs#L304) | Method | `impl Request::complete` | `fn(&mut self, result: i32) -> io::Result<bool>` |
| [335](feuer-storage/src/uring.rs#L335) | Method | `impl Request::short_error` | `fn(&self) -> io::Error` |
| [346](feuer-storage/src/uring.rs#L346) | Method | `impl Request::finish` | `fn(mut self, result: io::Result<()>)` |
| [358](feuer-storage/src/uring.rs#L358) | Struct | `Driver` | — |
| [359](feuer-storage/src/uring.rs#L359) | Field | `Driver::admission` | `Arc<Admission>` |
| [360](feuer-storage/src/uring.rs#L360) | Field | `Driver::ring` | `IoUring` |
| [361](feuer-storage/src/uring.rs#L361) | Field | `Driver::file` | `Option<File>` |
| [362](feuer-storage/src/uring.rs#L362) | Field | `Driver::lock` | `Option<File>` |
| [363](feuer-storage/src/uring.rs#L363) | Field | `Driver::wake` | `Arc<OwnedFd>` |
| [364](feuer-storage/src/uring.rs#L364) | Field | `Driver::receiver` | `mpsc::Receiver<Request>` |
| [365](feuer-storage/src/uring.rs#L365) | Field | `Driver::pending` | `VecDeque<Request>` |
| [366](feuer-storage/src/uring.rs#L366) | Field | `Driver::active` | `Vec<Option<Request>>` |
| [369](feuer-storage/src/uring.rs#L369) | Impl | `impl Driver` | — |
| [370](feuer-storage/src/uring.rs#L370) | Method | `impl Driver::run` | `fn(&mut self) -> io::Result<()>` |
| [417](feuer-storage/src/uring.rs#L417) | Method | `impl Driver::schedule` | `fn(&mut self)` |
| [461](feuer-storage/src/uring.rs#L461) | Method | `impl Driver::submit_slot` | `fn(&mut self, slot: usize)` |
| [476](feuer-storage/src/uring.rs#L476) | Method | `impl Driver::wait` | `fn(&self) -> io::Result<()>` |
| [514](feuer-storage/src/uring.rs#L514) | Impl | `impl Drop for Driver` | — |
| [515](feuer-storage/src/uring.rs#L515) | Method | `impl Drop for Driver::drop` | `fn(&mut self)` |
| [537](feuer-storage/src/uring.rs#L537) | Function | `notify` | `fn(wake: &OwnedFd)` |

<details>
<summary>Local bindings (53)</summary>

| Line | Kind | Name / source parent | Signature or type |
| ---: | --- | --- | --- |
| [100](feuer-storage/src/uring.rs#L100) | Local | `buffer_charge::prefix` | — |
| [101](feuer-storage/src/uring.rs#L101) | Local | `buffer_charge::aligned_length` | — |
| [102](feuer-storage/src/uring.rs#L102) | Local | `buffer_charge::rmw` | — |
| [108](feuer-storage/src/uring.rs#L108) | Local | `impl Handle::new::ring` | — |
| [110](feuer-storage/src/uring.rs#L110) | Local | `impl Handle::new::fd` | — |
| [115](feuer-storage/src/uring.rs#L115) | Local | `impl Handle::new::wake` | — |
| [116](feuer-storage/src/uring.rs#L116) | Local | `impl Handle::new::(sender, receiver)` | — |
| [117](feuer-storage/src/uring.rs#L117) | Local | `impl Handle::new::admission` | — |
| [118](feuer-storage/src/uring.rs#L118) | Local | `impl Handle::new::mut driver` | — |
| [128](feuer-storage/src/uring.rs#L128) | Local | `impl Handle::new::thread` | — |
| [148](feuer-storage/src/uring.rs#L148) | Local | `impl Handle::execute::_read` | — |
| [149](feuer-storage/src/uring.rs#L149) | Local | `impl Handle::execute::class` | — |
| [150](feuer-storage/src/uring.rs#L150) | Local | `impl Handle::execute::slot` | — |
| [155](feuer-storage/src/uring.rs#L155) | Local | `impl Handle::execute::bytes` | — |
| [160](feuer-storage/src/uring.rs#L160) | Local | `impl Handle::execute::(reply, receive)` | — |
| [161](feuer-storage/src/uring.rs#L161) | Local | `impl Handle::execute::request` | — |
| [175](feuer-storage/src/uring.rs#L175) | Local | `impl Drop for Handle::drop::_` | — |
| [195](feuer-storage/src/uring.rs#L195) | Local | `impl AlignedBuffer::new::layout` | — |
| [197](feuer-storage/src/uring.rs#L197) | Local | `impl AlignedBuffer::new::ptr` | — |
| [243](feuer-storage/src/uring.rs#L243) | Local | `impl Request::new::prefix` | — |
| [244](feuer-storage/src/uring.rs#L244) | Local | `impl Request::new::aligned_length` | — |
| [245](feuer-storage/src/uring.rs#L245) | Local | `impl Request::new::mut buffer` | — |
| [246](feuer-storage/src/uring.rs#L246) | Local | `impl Request::new::rmw` | — |
| [247](feuer-storage/src/uring.rs#L247) | Local | `impl Request::new::payload` | — |
| [280](feuer-storage/src/uring.rs#L280) | Local | `impl Request::entry::fd` | — |
| [281](feuer-storage/src/uring.rs#L281) | Local | `impl Request::entry::entry` | — |
| [282](feuer-storage/src/uring.rs#L282) | Local | `impl Request::entry::entry::flags` | — |
| [291](feuer-storage/src/uring.rs#L291) | Local | `impl Request::entry::entry::ptr` | — |
| [292](feuer-storage/src/uring.rs#L292) | Local | `impl Request::entry::entry::length` | — |
| [293](feuer-storage/src/uring.rs#L293) | Local | `impl Request::entry::entry::offset` | — |
| [314](feuer-storage/src/uring.rs#L314) | Local | `impl Request::complete::count` | — |
| [347](feuer-storage/src/uring.rs#L347) | Local | `impl Request::finish::result` | — |
| [354](feuer-storage/src/uring.rs#L354) | Local | `impl Request::finish::_` | — |
| [371](feuer-storage/src/uring.rs#L371) | Local | `impl Driver::run::mut disconnected` | — |
| [373](feuer-storage/src/uring.rs#L373) | Local | `impl Driver::run::completions` | `Vec<_>` |
| [379](feuer-storage/src/uring.rs#L379) | Local | `impl Driver::run::slot` | — |
| [380](feuer-storage/src/uring.rs#L380) | Local | `impl Driver::run::request` | — |
| [397](feuer-storage/src/uring.rs#L397) | Local | `impl Driver::run::active` | — |
| [420](feuer-storage/src/uring.rs#L420) | Local | `impl Driver::schedule::has_pending_or_active_reads` | — |
| [422](feuer-storage/src/uring.rs#L422) | Local | `impl Driver::schedule::mut writes` | — |
| [432](feuer-storage/src/uring.rs#L432) | Local | `impl Driver::schedule::mut index` | — |
| [434](feuer-storage/src/uring.rs#L434) | Local | `impl Driver::schedule::Some(slot)` | — |
| [438](feuer-storage/src/uring.rs#L438) | Local | `impl Driver::schedule::write_limit` | — |
| [446](feuer-storage/src/uring.rs#L446) | Local | `impl Driver::schedule::request` | — |
| [447](feuer-storage/src/uring.rs#L447) | Local | `impl Driver::schedule::blocked` | — |
| [462](feuer-storage/src/uring.rs#L462) | Local | `impl Driver::submit_slot::entry` | — |
| [477](feuer-storage/src/uring.rs#L477) | Local | `impl Driver::wait::mut fds` | — |
| [490](feuer-storage/src/uring.rs#L490) | Local | `impl Driver::wait::result` | — |
| [492](feuer-storage/src/uring.rs#L492) | Local | `impl Driver::wait::error` | — |
| [506](feuer-storage/src/uring.rs#L506) | Local | `impl Driver::wait::mut value` | — |
| [527](feuer-storage/src/uring.rs#L527) | Local | `impl Drop for Driver::drop::_` | — |
| [538](feuer-storage/src/uring.rs#L538) | Local | `notify::value` | — |
| [541](feuer-storage/src/uring.rs#L541) | Local | `notify::result` | — |

</details>

## feuer-tokio

### `feuer-tokio/src/lib.rs`

No source symbols reported (for example, a re-export-only module).

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
