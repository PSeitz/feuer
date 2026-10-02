# Scope

Added code is a liability. Less code is usually better. Every addition must justify its maintenance and complexity cost.

Implement only what the stated requirements need. Do not add features, abstractions, guarantees, or configuration "for completeness" or hypothetical future use. Every addition must solve a concrete, current requirement. If its necessity is unclear, ask before implementing. Prefer the smallest correct solution.

# Linux SSD testing

Machine: `ssh m8g-32cpu-local-ssd`. From the project checkout on that host:

```sh
PATH="$HOME/.cargo/bin:$PATH" TMPDIR=/mnt/local-ssd cargo test --locked -p feuer-storage --lib
```

# Disk chunks

Written payload chunks are immutable until all entry owners and read guards release them. Explicit batches group small payloads into chunks. Do not append to written payload chunks or reuse individual payload holes. Metadata occupies separate 1-MiB chunks and may be updated in place under read/write synchronization. Their last 4-KiB page stores the next metadata chunk address. Invalidate metadata before releasing payload chunks for reuse; allocator ownership and read guards prevent premature reuse.

Allocation is a runtime, in-memory construct, not an on-disk structure. Describe the disk layout in terms of chunks, metadata pages, and payload byte ranges, not allocations.

Each entry's payload occupies one contiguous disk byte range, with no metadata gaps. Reserve consecutive whole chunks; never assemble an entry from scattered free chunks. Metadata records one payload address and length. One read guard retains the entire reserved disk region. Metadata is separate from payload chunks, including when an entry spans multiple chunks.

# Naming

- Naming is important. Prefer concrete, descriptive names that explain what something represents or protects.
- Describe what a type represents in plain language before proposing its name. Derive the name from that description, using its concrete terms rather than substituting conventional labels. Check every meaningful word in the proposed name against the description. If it is absent, revise the name, not the description to justify it.
- Do not use "extent" in names or explanations. Use concrete, descriptive terms such as "disk region" or "cached byte range" instead.
- Do not use "bounded" in names. It doesn't mean anything.
- Use `chunk` for a 1-MiB disk chunk, not `unit` or `block`. Use `page` for a 4-KiB metadata page. Payload chunks are not organized into pages. Payload is plain bytes in 4-KiB-aligned disk byte ranges; metadata chunks contain 4-KiB pages.
- Use `DiskRegion` for a reserved byte range in the backing file.
- Use `ChunkGuard` for the guard that prevents a disk region from being reused while a read depends on it. In-place metadata updates require separate read/write synchronization. Multiple reads may hold guards concurrently. Eviction may remove the cache entry, but reuse must wait until all guards are released.
- Do not call this guard a "lease" or "pin". It has no expiration and is unrelated to Rust's `Pin`. It does not imply a global lock or serialized reads.
