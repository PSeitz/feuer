# Scope

Added code is a liability. Less code is usually better; every addition must justify its maintenance and complexity cost.

Implement only what the stated requirements need. Do not add features, abstractions, guarantees, or configuration "for completeness" or hypothetical future use. Every addition must solve a concrete, current requirement. If its necessity is unclear, ask before implementing. Prefer the smallest correct solution.

# Naming

- Naming is important. Prefer concrete, descriptive names that explain what something represents or protects.
- Do not use "extent" in names or explanations. Use concrete, descriptive terms such as "disk region" or "cached byte range" instead.
- Use `DiskRegion` for a reserved byte range in the backing file.
- Use `DiskRegionReadGuard` for the guard that prevents a disk region from being overwritten or reused while a read depends on it. Multiple reads may hold guards concurrently; eviction may remove the cache entry, but reuse must wait until all guards are released.
- Do not call this guard a "lease" or "pin". It has no expiration and is unrelated to Rust's `Pin`; it does not imply a global lock or serialized reads.
