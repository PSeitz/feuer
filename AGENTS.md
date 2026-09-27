# Scope

Added code is a liability. Less code is usually better; every addition must justify its maintenance and complexity cost.

Implement only what the stated requirements need. Do not add features, abstractions, guarantees, or configuration "for completeness" or hypothetical future use. Every addition must solve a concrete, current requirement. If its necessity is unclear, ask before implementing. Prefer the smallest correct solution.

# Disk chunks

A written chunk is immutable until all entry owners and read guards release it. Explicit batches group small entries into chunks and finalize payload, metadata, and discovery bitmaps before writing. Do not append to written chunks or reuse individual entry holes.

# Naming

- Naming is important. Prefer concrete, descriptive names that explain what something represents or protects.
- Describe what a type represents in plain language before proposing its name. Derive the name from that description, using its concrete terms rather than substituting conventional labels. Check every meaningful word in the proposed name against the description; if it is absent, revise the name, not the description to justify it.
- Do not use "extent" in names or explanations. Use concrete, descriptive terms such as "disk region" or "cached byte range" instead.
- Use `chunk` for a 1-MiB disk allocation chunk, not `unit` or `block`. Use `page` for a 4-KiB metadata page. Chunks are not organized into pages; payload is plain bytes in 4-KiB-aligned allocations.
- Use `DiskRegion` for a reserved byte range in the backing file.
- Use `DiskRegionReadGuard` for the guard that prevents a disk region from being overwritten or reused while a read depends on it. Multiple reads may hold guards concurrently; eviction may remove the cache entry, but reuse must wait until all guards are released.
- Do not call this guard a "lease" or "pin". It has no expiration and is unrelated to Rust's `Pin`; it does not imply a global lock or serialized reads.
