# Geometry for promotion at unchanged allocation charge

Historical range-aware analysis. Full report and local reproduction scripts:

`experiments/cache/partial-read-checksums/raw/static-analysis/promotion/`

Report: **`REPORT.md`**

### Findings
- Validated **1,606,645 requests / 2,800,015,397,526 requested bytes** in the exact 600-second prefix.
- Of **5,188 earlier-covering range pairs**, **652** qualify for nominal no-copy promotion, covering **1,071 request starts**.
- **680** qualifying starts have later additional demand: **1,157 distinct requests / 9,173,470 requested bytes**. These are conditional coverage opportunities—not measured disk savings.
- Every productive geometry uses a **32-KiB allocation**. The largest productive verified range is **27,864 bytes**.
- **None** of the **3,498 >8-MiB covering pairs / 30,021 source starts** qualifies with successful shrinking. Their requested slices are only **2,479–5,238 bytes**.
- Nearest-source reuse is typically not immediate: **8.047 seconds median**, **19,012 global requests median**. Only **88 targets / 469,295 bytes** fall within one second.

Unknown alignment removes two nominal pairs but changes no later-demand counts. Actual reused capacity, checksum selection, publication readiness, and memory eviction are explicitly separated from geometry.

**Recommendation:** Do not prioritize this proposal for reducing this trace’s large-read traffic. The small geometric opportunity is real, but realized benefit remains unestablished.
