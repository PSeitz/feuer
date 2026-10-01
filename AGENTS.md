# Scope

Added code is a liability. Less code is usually better. Every addition must justify its maintenance and complexity cost.

Implement only what the stated requirements need. Do not add features, abstractions, guarantees, or configuration "for completeness" or hypothetical future use. Every addition must solve a concrete, current requirement. If its necessity is unclear, ask before implementing. Prefer the smallest correct solution.

# Linux SSD testing

Machine: `ssh m8g-32cpu-local-ssd`. From the project checkout on that host:

```sh
PATH="$HOME/.cargo/bin:$PATH" TMPDIR=/mnt/local-ssd cargo test --locked -p feuer-storage --lib
```

# Disk chunks

A written chunk is immutable until all entry owners and read guards release it. Explicit batches group small entries into chunks and finalize payload, entry metadata, and chunk metadata before writing. Do not append to written chunks or reuse individual entry holes.

Allocation is a runtime, in-memory construct, not an on-disk structure. Describe the disk layout in terms of chunks, metadata pages, and payload byte ranges, not allocations.

Each entry's payload occupies one contiguous disk byte range, with no metadata gaps. Reserve consecutive whole chunks; never assemble an entry from scattered free chunks. Metadata records one payload address and length. One read guard retains the entire reserved disk region. When an entry spans multiple chunks, metadata appears only before its payload, not at each chunk boundary.

# Naming

- Naming is important. Prefer concrete, descriptive names that explain what something represents or protects.
- Describe what a type represents in plain language before proposing its name. Derive the name from that description, using its concrete terms rather than substituting conventional labels. Check every meaningful word in the proposed name against the description. If it is absent, revise the name, not the description to justify it.
- Do not use "extent" in names or explanations. Use concrete, descriptive terms such as "disk region" or "cached byte range" instead.
- Use `chunk` for a 1-MiB disk chunk, not `unit` or `block`. Use `page` for a 4-KiB metadata page. Chunks are not organized into pages. Payload is plain bytes in 4-KiB-aligned disk byte ranges.
- Use `DiskRegion` for a reserved byte range in the backing file.
- Use `DiskRegionReadGuard` for the guard that prevents a disk region from being overwritten or reused while a read depends on it. Multiple reads may hold guards concurrently. Eviction may remove the cache entry, but reuse must wait until all guards are released.
- Do not call this guard a "lease" or "pin". It has no expiration and is unrelated to Rust's `Pin`. It does not imply a global lock or serialized reads.
