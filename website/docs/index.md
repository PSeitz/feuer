---
title: Introduction
slug: /
sidebar_position: 1
---

![Feuer — cost-aware tiered cache](https://raw.githubusercontent.com/PSeitz/feuer/main/full_logo.svg?v=cfeda6a)

Feuer caches immutable objects in memory and on disk. On a cache miss, it calls your function to fetch or compute the object.

## Cache what is expensive to fetch

Eviction considers how often an object is requested, its size, and the cost of fetching it again. Memory and disk can use different retention scorers.

The disk tier uses Linux `io_uring` and direct I/O. Memory-only operation works on every supported platform.

## How lookups work

1. **Memory hit:** return the cached bytes.
2. **Disk hit:** read and checksum-verify the object, then promote it to memory.
3. **Miss:** call your callback, cache the result in memory, and queue a disk write when disk caching is enabled.

Concurrent requests for the same key and callback error type share in-flight work.

## Start here

- [Quick start](quick-start.md): create a cache and supply a fetch callback.
- [API reference](https://docs.rs/feuer): types, builder options, and examples.
- [Eviction](https://github.com/PSeitz/feuer/blob/main/eviction.md): scoring, configuration, and trade-offs.
- [Source code](https://github.com/PSeitz/feuer): development, issues, and contributions.
