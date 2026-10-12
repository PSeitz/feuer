# Cost-model experiment results

Parent-reviewed seven matched pairs / fourteen cases on five new isolated instances. Each arrow is one sequential same-host control → candidate. Decimal GB throughout. No cross-host matched comparison, repetitions or policy combinations.

Fixed workload: first600 seconds of the same exact immutable whole-slice capture, original pacing, independent zero-filled source callbacks,32 workers,64 shards,100000000000 memory bytes,500000000000 disk bytes,fresh caches,5-second observation tail. Every case completed1606645 requests and2800015397526 returned bytes. All common raw gates and lane-specific gates passed. Source coefficients are historical assumptions, not measured production costs.

## Traffic, CPU and scheduled-completion latency

| Lane / control | SSD reads GB | SSD writes GB | Source GB | Source callbacks | Request CPU s | Completion p99 ms |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| fallback / pair-fixed | 6.354 → 7.716 | 99.384 → 98.792 | 129.932 → 129.697 | 126422 → 125748 | 48 → 56 | 0.271 → 0.580 |
| fallback / pair-assumed | 6.747 → 7.709 | 99.195 → 98.683 | 130.155 → 129.698 | 126936 → 125764 | 58 → 58 | 0.545 → 0.731 |
| read-work / pair-default | 6.429 → 7.658 | 99.302 → 98.708 | 129.972 → 129.695 | 126462 → 125746 | 53 → 60 | 0.507 → 0.682 |
| read-work / pair-assumed | 6.823 → 7.620 | 99.118 → 98.911 | 130.129 → 129.699 | 126903 → 125839 | 62 → 60 | 1.218 → 1.099 |
| reuse-admission / pair | 6.523 → 0.005 | 99.159 → 1.477 | 129.978 → 136.186 | 126755 → 128648 | 45 → 17 | 0.366 → 0.119 |
| tail-32mb / pair | 6.437 → 6.453 | 99.115 → 70.657 | 129.945 → 129.926 | 126490 → 126369 | 46 → 40 | 0.552 → 2.259 |
| tail-64mb / pair | 6.335 → 6.378 | 93.726 → 91.739 | 129.962 → 130.005 | 126562 → 126479 | 42 → 42 | 1.180 → 0.299 |

Read/write values above are whole-device counters, not useful returned payload or DataFile calls. Completion includes sender lateness, caller queue and service. Sender-lateness differences are particularly important in the tail-admission pairs. No causal speedup or slowdown is established by these p99 observations.

## Hits and memory accounting

| Lane / control | Memory request % | Memory byte % | Disk request % | Disk byte % | Combined request % | Combined byte % | Useful memory GB | Resident charge GB | Idle GB | OS max RSS GB |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| fallback / pair-fixed | 91.98124 → 91.77640 | 95.13284 → 95.09288 | 0.15006 → 0.39685 | 0.22675 → 0.27512 | 92.13130 → 92.17326 | 95.35958 → 95.36800 | 97.007 → 96.685 | 99.033 → 98.979 | 0.094 → 0.340 | 11.883 → 12.835 |
| fallback / pair-assumed | 91.79763 → 91.78132 | 95.11100 → 95.09308 | 0.30168 → 0.39094 | 0.24063 → 0.27486 | 92.09931 → 92.17226 | 95.35163 → 95.36794 | 96.844 → 96.705 | 99.019 → 99.028 | 0.139 → 0.380 | 12.140 → 12.948 |
| read-work / pair-default | 91.97956 → 91.78008 | 95.12876 → 95.09503 | 0.14926 → 0.39330 | 0.22943 → 0.27304 | 92.12882 → 92.17338 | 95.35818 → 95.36806 | 97.059 → 96.728 | 99.107 → 99.019 | 0.083 → 0.395 | 11.923 → 12.840 |
| read-work / pair-assumed | 91.80267 → 91.78001 | 95.10923 → 95.09623 | 0.29870 → 0.38758 | 0.24334 → 0.27168 | 92.10137 → 92.16759 | 95.35258 → 95.36791 | 96.823 → 96.840 | 98.985 → 99.101 | 0.245 → 0.340 | 12.162 → 12.680 |
| reuse-admission / pair | 91.95852 → 91.99226 | 95.12516 → 95.13605 | 0.15206 → 0.00050 | 0.23278 → 0.00019 | 92.11058 → 91.99276 | 95.35794 → 95.13624 | 96.886 → 99.170 | 98.941 → 99.172 | 0.121 → 0.054 | 12.109 → 3.498 |
| tail-32mb / pair | 91.97832 → 91.98199 | 95.12943 → 95.12954 | 0.14876 → 0.15262 | 0.22970 → 0.23027 | 92.12707 → 92.13460 | 95.35913 → 95.35981 | 97.044 → 97.089 | 99.094 → 99.140 | 0.094 → 0.092 | 11.937 → 11.992 |
| tail-64mb / pair | 91.97937 → 91.98305 | 95.13245 → 95.12937 | 0.14322 → 0.14471 | 0.22608 → 0.22763 | 92.12259 → 92.12776 | 95.35853 → 95.35700 | 96.973 → 96.947 | 98.990 → 98.981 | 0.073 → 0.093 | 11.815 → 11.914 |

Logical useful payload, allocation charge, idle pool buffers and physical RSS have different scopes, especially with zero-filled callbacks.

## DataFile operations and observed eviction

| Lane / control | Read calls | Read GB | Write calls | Write GB | Memory eviction-triggering insertions | Disk eviction-triggering insertions | Tail write MB |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| fallback / pair-fixed | 2411 → 6376 | 6.349 → 7.703 | 30170 → 29843 | 99.357 → 98.758 | 2308 → 2691 | 0 → 0 | 66.376 → 66.359 |
| fallback / pair-assumed | 4844 → 6281 | 6.737 → 7.696 | 30362 → 29953 | 99.161 → 98.650 | 2793 → 2608 | 0 → 0 | 66.363 → 66.367 |
| read-work / pair-default | 2398 → 6319 | 6.424 → 7.645 | 30167 → 29690 | 99.274 → 98.674 | 2215 → 2765 | 0 → 0 | 66.355 → 66.363 |
| read-work / pair-assumed | 4799 → 6227 | 6.814 → 7.607 | 30223 → 29812 | 99.087 → 98.877 | 2861 → 2636 | 0 → 0 | 66.372 → 66.359 |
| reuse-admission / pair | 2443 → 8 | 6.518 → 0.005 | 30387 → 2076 | 99.130 → 1.469 | 2209 → 2193 | 0 → 0 | 66.359 → 16.843 |
| tail-32mb / pair | 2390 → 2452 | 6.432 → 6.448 | 30146 → 29677 | 99.087 → 70.625 | 2219 → 2284 | 0 → 0 | 66.359 → 66.376 |
| tail-64mb / pair | 2301 → 2325 | 6.330 → 6.374 | 29157 → 29201 | 93.699 → 91.707 | 2233 → 2310 | 0 → 0 | 66.388 → 66.376 |

DataFile read byte counters are useful payload bytes, excluding final alignment padding. API calls differ from split I/O queue requests and kernel/device requests. Zero disk eviction in every case means no disk victim-scoring conclusion. Tail is observation, not flushing or durability.

## Evidence and interpretation

- [FINDINGS.md](FINDINGS.md) — interpretation and what remains untested.
- [README.md](README.md) — tested settings, model descriptions, source patches, and extraction instructions.
- `measurements.tar.zst` (local, not committed) — raw measurements, commands/settings, baseline source, detailed lane reports, and original audit receipts. Larger data and workspaces are ignored by Git.

All five instances terminated, root volumes were deleted, and isolated Terraform states were empty. All 3649 retrieved files and five original evidence archives were independently hash-verified. Production policies remained unchanged. No production adoption or commit.
