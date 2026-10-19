# Per-request disk bypass validation

Base revision: `2f381acdb13e114593e98d1078f29b341279380d`, plus the disk-bypass working-tree changes.
Linux host: `m8g-32cpu-local-ssd-2`.
Remote workspace: `/home/ubuntu/feuer-disk-bypass.4WkA0D`.
No replay or performance benchmark was run.

## Commands and results

Linux commands used `PATH="$HOME/.cargo/bin:$PATH"`, `TMPDIR=/mnt/local-ssd`, and
`CARGO_TARGET_DIR="$HOME/feuer-simplify-target"`.

- Initial `cargo test --locked -p feuer-storage -p feuer --lib`: Feuer 31 passed.
  Storage 146 passed, 1 failed, 1 ignored. The failure is described below.
- After adding queue-wide sweeps, `cargo test --locked -p feuer-storage -p feuer --lib -- --skip overflowing_fields_preserve_chunk_ownership`:
  Feuer 31 passed. Storage 150 passed, 1 ignored, 1 filtered out.
  All nine bypass/admission tests passed.
- `cargo clippy --locked -p feuer-storage -p feuer --all-targets --all-features -- -D warnings`: passed after the queue-wide change.
- Local macOS `cargo test --locked -p feuer --lib`: 14 passed.
- `cargo fmt --all --check` and `git diff --check`: passed.

The storage failure is `disk_cache::adversarial_tests::overflowing_fields_preserve_chunk_ownership`.
An isolated `git archive HEAD` baseline, with the generated proptest regression seed,
reproduces the same overflow. Its minimal metadata entry has length `u64::MAX`.
The baseline command was `cargo test --locked -p feuer-storage --lib overflowing_fields_preserve_chunk_ownership`.
This pre-existing failure was not changed.

## Coverage

Payload read permits allow 64 reads to run at once, shared through
`IoQueues`. At 64 waiting reads, every unchecked waiter is scheduled to consider
bypass. Callbacks run in their own tasks, outside the queue lock, at most once.
A scheduled check still runs if earlier bypasses or completions relieve pressure.
I/O pieces retain the existing io_uring limits and are not canceled by bypass.

Tests hold read permits to produce deterministic queue pressure without
depending on SSD speed. A 64-read cache test verifies that reaching the bypass threshold
checks earlier requests without issuing I/O. Admission tests cover repeated checks,
fewer waiting reads, newly available permits, and cancellation cleanup.
Other tests check accepted and declined bypass, entry preservation, separate bypass
metrics, buffered and absent entries, uncongested reads, shared-read waiters, and
takeover after initializer cancellation. A bypass does not initialize the shared
read result. Tiered API tests cover range and whole-object calls, memory hits, disk
hits and disabled disk. Existing cache tests cover source errors and invalid downloads.

Raw test and Clippy logs are local and Git-ignored under `raw/`.
`sweep-test.log` and `sweep-clippy.log` record the queue-wide implementation.

## Review validation

Workspace: `/home/ubuntu/feuer-bypass-review.pUd1aO`, with the same environment as above.
Shared-read tests now use cache lookups instead of initializing shared results directly.
Duplicate no-I/O, entry-preservation, and download-validation assertions were removed.

- Full Linux library run: Feuer 31 passed, storage 150 passed, 1 failed, 1 ignored.
  The same recovery-overflow test failed. A clean `git archive HEAD` baseline with
  the generated seed reproduced it with `payload_address = u64::MAX` and length zero.
- Final Linux library run with that test excluded: Feuer 31 passed, storage 150 passed,
  1 ignored, 1 filtered out. All nine bypass/admission tests passed.
- Linux Clippy with `--all-targets --all-features -- -D warnings`: passed.
- macOS Feuer library tests: 14 passed. Formatting and whitespace checks passed.

Logs are under `raw/review-*.log`. No replay or performance benchmark was run.

## Fallthrough rename validation

Renamed the current APIs, internal names, test modules, and metric outcome from
bypass to fallthrough without changing behavior. Historical experiment paths remain unchanged.
Validated the working tree based on `ada2d89` in
`/home/ubuntu/feuer-fallthrough-rename.mjLvga` on the same Linux host and environment.

- Linux library tests: Feuer 36 passed, storage 150 passed, 1 ignored.
  The known `overflowing_fields_preserve_chunk_ownership` failure was excluded.
- Linux Clippy with `--all-targets --all-features -- -D warnings`: passed.
- macOS Feuer library tests: 20 passed. Formatting and whitespace checks passed.

Output, including two initial incomplete-workspace setup failures, is retained in
`raw/fallthrough-rename-validation.log`. No replay or benchmark was run.
