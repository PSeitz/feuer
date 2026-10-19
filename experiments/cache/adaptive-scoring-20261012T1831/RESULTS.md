# Captured results

**68 unique measurements completed: 36 range and 32 full-object configurations.** The initial 52-case local search was followed by twelve strong contrasts and four conditional refinements. No measurement was repeated or truncated. All replay processes exited zero and all reports validate. The first 14 reports were recovered offline from untouched captures, retaining their original wrapper exit status 1. The remaining 54 worker reports succeeded directly.

The tables below include both baselines, every distinct observed objective leader with the baseline disk coefficient, and every new contrast/refinement. Disk-only coefficient variants are retained in [results.csv](results.csv), but excluded from leader attribution because no disk eviction occurred. [LEADERS.md](LEADERS.md) gives each objective and the Pareto frontier. Decimal GB = 1,000,000,000 bytes. Caller rates use all 1,606,645 requests. Byte rates use all 2,800,015,397,526 returned bytes. Conditional disk rates exclude memory hits.

## What the larger steps showed

**A strong observed full-object disk-read reduction, not an overall/source-performance win.** The lowest observed read bytes are from **case-065: memory request cost 9,133,289, half-life 4,096 requests, disk request cost 256,000, both per-byte coefficients 1**. Reads fall from **283,838,786 to 80,709,704 bytes**, a reduction of **203,129,082 bytes (71.564949%)** versus baseline. Versus the previous read-byte best, case-049 (memory request cost 281,600, half-life 212,337), this saves **166,115,172 bytes (67.300823%)**. Read calls fall from 54 to 34 (37.037037%). The previous minimum read-call count was 47 in case-028, so the reduction against that separate objective is 13 calls (27.659574%).

This large relative reduction affects a small read component. Writes increase by **2,109,440 bytes (0.001490%)** to **141.595914240 GB**, and total disk traffic falls by only **201,019,642 bytes (0.141685%)** to **141.676623944 GB**. Against case-049, total traffic falls by **164,063,076 bytes (0.115667%)** while writes increase by **2,052,096 bytes (0.001449%)**. Source traffic is effectively unchanged: **518,831 GETs / 537.872828743 GB**, only **4 GETs (0.000771%) / 3,040,041 bytes (0.000565%)** below baseline. Against the previous source best, case-050, it uses **16 more GETs (0.003084%) and 32,989,568 more bytes (0.006134%)**.

The hit shift is principally from disk to memory, not from source to either cache tier. For case-065, memory caller hits rise from 67.703569% to 67.705062% (+0.001494 percentage points), while disk caller hits fall from 0.003361% to 0.002116%. Memory byte hits rise from 80.780116% to 80.787479% (+0.007363 points), and disk byte hits fall from 0.010137% to 0.002882%. Combined caller/byte hits barely change: **67.707179% / 80.790362%**, improvements of just **0.000249 / 0.000109 percentage points**.

There is no meaningful end-to-end latency improvement: case-065 execution/drain are **1,318.432939 / 719.073024 seconds**, both 6.990 ms longer than baseline. Scheduled-to-complete mean is **150.542499 seconds** (1.774 ms lower), and p99 is **719.612275 seconds** (7.385 ms higher). Request-phase CPU is **56 seconds**, unchanged from baseline and one second above case-049. OS maximum RSS is **3.794182144 GB**, 0.259629056 GB (6.404567%) below baseline and 0.228663296 GB (5.684118%) below case-049, but this single-run zero-filled simulation does not establish a real-payload memory benefit. Settled cache charge is **99.158030895 GB**, idle buffers **0.050331648 GB**, and charge minus idle **99.107699247 GB**. These are not payload occupancy. Source wait averages **466.056701 seconds**, with **294,506 waiting requests** at the sampled peak and 48 active. The backlog drained fully.

Other objectives have different settings:

- **Lowest full-object total disk traffic: case-063**, memory request cost **9,133,289**, half-life **16,384**. Read/write/total are **0.082394214 / 141.592662016 / 141.675056230 GB**. Total saves **202,587,356 bytes (0.142790%)** versus baseline and **165,630,790 bytes (0.116772%)** versus previous best case-049. The shorter 4,096-request refinement saves only another **1,684,510 read bytes (2.044452%)** over case-063 and slightly increases total traffic. Do not infer that 4,096 is a proven optimum.
- **Lowest full-object source bytes: case-064**, memory request cost **36,533,156**, half-life **16,384**: **537.838137114 GB**, down **37,731,670 bytes (0.007015%)** from baseline and only **1,702,061 bytes (0.000316%)** from previous best case-050. It uses **518,817 GETs**, two more than case-050. This is far below the predeclared practical source-gain threshold. The lowest source GET count remains **case-050: 518,815**, memory request cost **281,600**, half-life **317,194**.
- **Range has no qualifying strong contrast.** The previous read/total-traffic best remains **case-017**, memory request cost **309,760**, half-life **262,144**, with **3.837841374 GB reads / 145.401618398 GB total**. The previous source best remains **case-031**, memory request cost **186,624**, half-life **262,144**, with **517,457 GETs / 537.780983224 GB**. Request-heavy case-054 (memory request cost **1,000,000,000**, half-life **262,144**) cuts read calls by 621 (56.505914%) to 478, but increases read bytes by **5.634024553 GB (139.287638%)** to **9.678909380 GB**. This tradeoff fails the regression guard. The analogous full-object case-060 cuts calls to 30 but raises read bytes by 46.437807%, also failing the guard.

## Why the search stopped at 68

The twelve contrasts completed at 21:34:36 UTC. Only full-object disk-read bytes qualified for refinement: case-063 improved the previous best by **164,430,662 bytes**, exceeding the unchanged **56,767,757.2-byte** threshold. Its read-call improvement was 10, below the 10.8-call threshold. No source objective qualified. Range’s only regression-acceptable contrast, case-056, did not beat the previous best on any selection metric. The range read-byte threshold stayed **1,055,135,385 bytes** and was not lowered to obtain a winner.

The coordinator then ran exactly four full-object refinements, all at disk request cost 256,000: **(memory cost, half-life) = (36,533,156, 16,384), (9,133,289, 4,096), (2,283,322.25, 16,384), (9,133,289, 65,536)**. They completed at 21:56:59 UTC. The predeclared one-refinement-wave limit then stopped the search at **68 unique cases: 52 original + 12 contrasts + 4 refinements**, rather than spending all 72 permitted evaluations. There were no repeats, threshold changes, additional waves, or truncated drains.

The large observed full-object read-byte effect is useful diagnostic evidence. It does not establish statistical significance or justify a universal production setting: only tens of full-object disk reads occurred, the objective was selected adaptively, and each configuration was measured once. The initial source/overall limitations still apply. All exact per-case settings, signed changes against baseline and metric-specific previous best, and broader tradeoffs follow below.


## Predeclared refinement decisions

The thresholds were fixed before the new captures: the greater of a practical reduction (20% of baseline disk reads/calls, 0.1% of baseline source bytes/GETs) and five times the original no-op disk-direction spread. Qualification compares against the best eligible original observation, not merely baseline. Regression guards also apply. These are operational thresholds, not significance tests. `raw/contrast-decisions.json` preserves `contrast_search.assess` output without changing thresholds.

| Mode | Metric | Best acceptable contrast | Improvement over prior best | Minimum required | Qualifies |
| --- | --- | --- | ---: | ---: | --- |
| range | disk_read_bytes | case-056 | -373037776.0 | 1055135385.0 | False |
| range | disk_read_calls | case-056 | -39.0 | 219.8 | False |
| range | source_bytes | case-056 | -56767436.0 | 537811421.7030001 | False |
| range | source_gets | case-056 | -8.0 | 517.481 | False |
| full_objects | disk_read_bytes | case-063 | 164430662.0 | 56767757.2 | True |
| full_objects | disk_read_calls | case-063 | 10.0 | 10.8 | False |
| full_objects | source_bytes | case-062 | -477029.0 | 537875868.784 | False |
| full_objects | source_gets | case-062 | -13.0 | 518.835 | False |

## New-case changes: disk_read_bytes

Signed changes are new minus reference, so negative means reduction. Previous best is the metric-specific minimum among the original 52 cases with disk request cost 256,000. Byte counts are exact bytes, not GB.

| Case | Mode | Value | Change vs baseline (absolute / relative) | Prior best case/value | Change vs prior best (absolute / relative) |
| --- | --- | ---: | ---: | --- | ---: |
| case-052 | range | 3,973,198,648.0 | -71,686,179.0 / -1.772267% | case-017 / 3,837,841,374.0 | +135,357,274.0 / +3.526912% |
| case-053 | range | 6,062,135,740.0 | +2,017,250,913.0 / +49.871653% | case-017 / 3,837,841,374.0 | +2,224,294,366.0 / +57.956912% |
| case-054 | range | 9,678,909,380.0 | +5,634,024,553.0 / +139.287638% | case-017 / 3,837,841,374.0 | +5,841,068,006.0 / +152.196702% |
| case-055 | range | 5,905,807,010.0 | +1,860,922,183.0 / +46.006803% | case-017 / 3,837,841,374.0 | +2,067,965,636.0 / +53.883562% |
| case-056 | range | 4,210,879,150.0 | +165,994,323.0 / +4.103808% | case-017 / 3,837,841,374.0 | +373,037,776.0 / +9.719989% |
| case-057 | range | 5,633,838,882.0 | +1,588,954,055.0 / +39.283048% | case-017 / 3,837,841,374.0 | +1,795,997,508.0 / +46.797075% |
| case-058 | full_objects | 184,057,274.0 | -99,781,512.0 / -35.154291% | case-049 / 246,824,876.0 | -62,767,602.0 / -25.430015% |
| case-059 | full_objects | 288,939,329.0 | +5,100,543.0 / +1.796986% | case-049 / 246,824,876.0 | +42,114,453.0 / +17.062483% |
| case-060 | full_objects | 415,647,294.0 | +131,808,508.0 / +46.437807% | case-049 / 246,824,876.0 | +168,822,418.0 / +68.397651% |
| case-061 | full_objects | 86,020,294.0 | -197,818,492.0 / -69.693961% | case-049 / 246,824,876.0 | -160,804,582.0 / -65.149261% |
| case-062 | full_objects | 254,510,591.0 | -29,328,195.0 / -10.332695% | case-049 / 246,824,876.0 | +7,685,715.0 / +3.113833% |
| case-063 | full_objects | 82,394,214.0 | -201,444,572.0 / -70.971475% | case-049 / 246,824,876.0 | -164,430,662.0 / -66.618351% |
| case-064 | full_objects | 92,628,883.0 | -191,209,903.0 / -67.365671% | case-049 / 246,824,876.0 | -154,195,993.0 / -62.471820% |
| case-065 | full_objects | 80,709,704.0 | -203,129,082.0 / -71.564949% | case-049 / 246,824,876.0 | -166,115,172.0 / -67.300823% |
| case-066 | full_objects | 81,968,879.0 | -201,869,907.0 / -71.121326% | case-049 / 246,824,876.0 | -164,855,997.0 / -66.790673% |
| case-067 | full_objects | 213,269,813.0 | -70,568,973.0 / -24.862343% | case-049 / 246,824,876.0 | -33,555,063.0 / -13.594684% |

## New-case changes: disk_read_calls

Signed changes are new minus reference, so negative means reduction. Previous best is the metric-specific minimum among the original 52 cases with disk request cost 256,000. Byte counts are exact bytes, not GB.

| Case | Mode | Value | Change vs baseline (absolute / relative) | Prior best case/value | Change vs prior best (absolute / relative) |
| --- | --- | ---: | ---: | --- | ---: |
| case-052 | range | 3,464.0 | +2,365.0 / +215.195632% | case-043 / 1,006.0 | +2,458.0 / +244.333996% |
| case-053 | range | 552.0 | -547.0 / -49.772520% | case-043 / 1,006.0 | -454.0 / -45.129225% |
| case-054 | range | 478.0 | -621.0 / -56.505914% | case-043 / 1,006.0 | -528.0 / -52.485089% |
| case-055 | range | 2,921.0 | +1,822.0 / +165.787079% | case-043 / 1,006.0 | +1,915.0 / +190.357853% |
| case-056 | range | 1,045.0 | -54.0 / -4.913558% | case-043 / 1,006.0 | +39.0 / +3.876740% |
| case-057 | range | 1,926.0 | +827.0 / +75.250227% | case-043 / 1,006.0 | +920.0 / +91.451292% |
| case-058 | full_objects | 55.0 | +1.0 / +1.851852% | case-028 / 47.0 | +8.0 / +17.021277% |
| case-059 | full_objects | 41.0 | -13.0 / -24.074074% | case-028 / 47.0 | -6.0 / -12.765957% |
| case-060 | full_objects | 30.0 | -24.0 / -44.444444% | case-028 / 47.0 | -17.0 / -36.170213% |
| case-061 | full_objects | 41.0 | -13.0 / -24.074074% | case-028 / 47.0 | -6.0 / -12.765957% |
| case-062 | full_objects | 48.0 | -6.0 / -11.111111% | case-028 / 47.0 | +1.0 / +2.127660% |
| case-063 | full_objects | 37.0 | -17.0 / -31.481481% | case-028 / 47.0 | -10.0 / -21.276596% |
| case-064 | full_objects | 31.0 | -23.0 / -42.592593% | case-028 / 47.0 | -16.0 / -34.042553% |
| case-065 | full_objects | 34.0 | -20.0 / -37.037037% | case-028 / 47.0 | -13.0 / -27.659574% |
| case-066 | full_objects | 36.0 | -18.0 / -33.333333% | case-028 / 47.0 | -11.0 / -23.404255% |
| case-067 | full_objects | 32.0 | -22.0 / -40.740741% | case-028 / 47.0 | -15.0 / -31.914894% |

## New-case changes: source_bytes

Signed changes are new minus reference, so negative means reduction. Previous best is the metric-specific minimum among the original 52 cases with disk request cost 256,000. Byte counts are exact bytes, not GB.

| Case | Mode | Value | Change vs baseline (absolute / relative) | Prior best case/value | Change vs prior best (absolute / relative) |
| --- | --- | ---: | ---: | --- | ---: |
| case-052 | range | 537,898,956,716.0 | +87,535,013.0 / +0.016276% | case-031 / 537,780,983,224.0 | +117,973,492.0 / +0.021937% |
| case-053 | range | 537,886,059,750.0 | +74,638,047.0 / +0.013878% | case-031 / 537,780,983,224.0 | +105,076,526.0 / +0.019539% |
| case-054 | range | 537,808,291,309.0 | -3,130,394.0 / -0.000582% | case-031 / 537,780,983,224.0 | +27,308,085.0 / +0.005078% |
| case-055 | range | 537,874,204,546.0 | +62,782,843.0 / +0.011674% | case-031 / 537,780,983,224.0 | +93,221,322.0 / +0.017334% |
| case-056 | range | 537,837,750,660.0 | +26,328,957.0 / +0.004896% | case-031 / 537,780,983,224.0 | +56,767,436.0 / +0.010556% |
| case-057 | range | 537,843,806,135.0 | +32,384,432.0 / +0.006022% | case-031 / 537,780,983,224.0 | +62,822,911.0 / +0.011682% |
| case-058 | full_objects | 537,891,906,204.0 | +16,037,420.0 / +0.002982% | case-050 / 537,839,839,175.0 | +52,067,029.0 / +0.009681% |
| case-059 | full_objects | 537,861,427,793.0 | -14,440,991.0 / -0.002685% | case-050 / 537,839,839,175.0 | +21,588,618.0 / +0.004014% |
| case-060 | full_objects | 537,842,319,890.0 | -33,548,894.0 / -0.006237% | case-050 / 537,839,839,175.0 | +2,480,715.0 / +0.000461% |
| case-061 | full_objects | 537,851,282,262.0 | -24,586,522.0 / -0.004571% | case-050 / 537,839,839,175.0 | +11,443,087.0 / +0.002128% |
| case-062 | full_objects | 537,840,316,204.0 | -35,552,580.0 / -0.006610% | case-050 / 537,839,839,175.0 | +477,029.0 / +0.000089% |
| case-063 | full_objects | 537,865,929,915.0 | -9,938,869.0 / -0.001848% | case-050 / 537,839,839,175.0 | +26,090,740.0 / +0.004851% |
| case-064 | full_objects | 537,838,137,114.0 | -37,731,670.0 / -0.007015% | case-050 / 537,839,839,175.0 | -1,702,061.0 / -0.000316% |
| case-065 | full_objects | 537,872,828,743.0 | -3,040,041.0 / -0.000565% | case-050 / 537,839,839,175.0 | +32,989,568.0 / +0.006134% |
| case-066 | full_objects | 537,875,217,949.0 | -650,835.0 / -0.000121% | case-050 / 537,839,839,175.0 | +35,378,774.0 / +0.006578% |
| case-067 | full_objects | 537,893,570,699.0 | +17,701,915.0 / +0.003291% | case-050 / 537,839,839,175.0 | +53,731,524.0 / +0.009990% |

## New-case changes: source_gets

Signed changes are new minus reference, so negative means reduction. Previous best is the metric-specific minimum among the original 52 cases with disk request cost 256,000. Byte counts are exact bytes, not GB.

| Case | Mode | Value | Change vs baseline (absolute / relative) | Prior best case/value | Change vs prior best (absolute / relative) |
| --- | --- | ---: | ---: | --- | ---: |
| case-052 | range | 517,496.0 | +15.0 / +0.002899% | case-031 / 517,457.0 | +39.0 / +0.007537% |
| case-053 | range | 517,501.0 | +20.0 / +0.003865% | case-031 / 517,457.0 | +44.0 / +0.008503% |
| case-054 | range | 517,461.0 | -20.0 / -0.003865% | case-031 / 517,457.0 | +4.0 / +0.000773% |
| case-055 | range | 517,500.0 | +19.0 / +0.003672% | case-031 / 517,457.0 | +43.0 / +0.008310% |
| case-056 | range | 517,465.0 | -16.0 / -0.003092% | case-031 / 517,457.0 | +8.0 / +0.001546% |
| case-057 | range | 517,469.0 | -12.0 / -0.002319% | case-031 / 517,457.0 | +12.0 / +0.002319% |
| case-058 | full_objects | 518,841.0 | +6.0 / +0.001156% | case-050 / 518,815.0 | +26.0 / +0.005011% |
| case-059 | full_objects | 518,842.0 | +7.0 / +0.001349% | case-050 / 518,815.0 | +27.0 / +0.005204% |
| case-060 | full_objects | 518,832.0 | -3.0 / -0.000578% | case-050 / 518,815.0 | +17.0 / +0.003277% |
| case-061 | full_objects | 518,836.0 | +1.0 / +0.000193% | case-050 / 518,815.0 | +21.0 / +0.004048% |
| case-062 | full_objects | 518,828.0 | -7.0 / -0.001349% | case-050 / 518,815.0 | +13.0 / +0.002506% |
| case-063 | full_objects | 518,844.0 | +9.0 / +0.001735% | case-050 / 518,815.0 | +29.0 / +0.005590% |
| case-064 | full_objects | 518,817.0 | -18.0 / -0.003469% | case-050 / 518,815.0 | +2.0 / +0.000385% |
| case-065 | full_objects | 518,831.0 | -4.0 / -0.000771% | case-050 / 518,815.0 | +16.0 / +0.003084% |
| case-066 | full_objects | 518,833.0 | -2.0 / -0.000385% | case-050 / 518,815.0 | +18.0 / +0.003469% |
| case-067 | full_objects | 518,845.0 | +10.0 / +0.001927% | case-050 / 518,815.0 | +30.0 / +0.005782% |

## Configurations

| Case | Mode | Memory / disk request cost | Half-life | Observed role |
| --- | --- | --- | --- | --- |
| case-000 | range | 256000 / 256000 | 262144 | baseline |
| case-007 | full_objects | 256000 / 256000 | 262144 | baseline |
| case-017 | range | 309760 / 256000 | 262144 | lowest disk transfer, previous best disk_read_bytes |
| case-022 | range | 230400 / 256000 | 288358 | memory byte |
| case-028 | full_objects | 281600 / 256000 | 288358 | previous best disk_read_calls |
| case-031 | range | 186624 / 256000 | 262144 | combined caller, combined byte, previous best source_bytes, previous best source_gets |
| case-043 | range | 309760 / 256000 | 288358 | previous best disk_read_calls |
| case-049 | full_objects | 281600 / 256000 | 212337 | previous best disk_read_bytes |
| case-050 | full_objects | 281600 / 256000 | 317194 | combined caller, previous best source_bytes, previous best source_gets |
| case-052 | range | 0 / 256000 | 262144 | conditional disk caller, contrast |
| case-053 | range | 9133289 / 256000 | 262144 | contrast |
| case-054 | range | 1000000000 / 256000 | 262144 | memory caller, conditional disk byte, contrast |
| case-055 | range | 256000 / 256000 | 16384 | contrast |
| case-056 | range | 256000 / 256000 | 1048576 | contrast |
| case-057 | range | 9133289 / 256000 | 16384 | contrast |
| case-058 | full_objects | 0 / 256000 | 262144 | conditional disk caller, contrast |
| case-059 | full_objects | 9133289 / 256000 | 262144 | contrast |
| case-060 | full_objects | 1000000000 / 256000 | 262144 | conditional disk byte, contrast |
| case-061 | full_objects | 256000 / 256000 | 16384 | contrast |
| case-062 | full_objects | 256000 / 256000 | 1048576 | contrast |
| case-063 | full_objects | 9133289 / 256000 | 16384 | lowest disk transfer, contrast |
| case-064 | full_objects | 36533156 / 256000 | 16384 | memory caller, memory byte, combined byte, refinement |
| case-065 | full_objects | 9133289 / 256000 | 4096 | refinement |
| case-066 | full_objects | 2283322.25 / 256000 | 16384 | refinement |
| case-067 | full_objects | 9133289 / 256000 | 65536 | refinement |

## Hit rates (%)

| Case | Memory caller | Disk caller | Combined caller | Memory byte | Disk byte | Combined byte | Conditional disk caller | Conditional disk byte |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| case-000 | 67.722615 | 0.068590 | 67.791205 | 80.648088 | 0.144467 | 80.792555 | 0.212502 | 0.746526 |
| case-007 | 67.703569 | 0.003361 | 67.706930 | 80.780116 | 0.010137 | 80.790253 | 0.010407 | 0.052742 |
| case-017 | 67.727220 | 0.064482 | 67.791703 | 80.654550 | 0.137064 | 80.791614 | 0.199804 | 0.708509 |
| case-022 | 67.723797 | 0.067843 | 67.791640 | 80.654748 | 0.137823 | 80.792570 | 0.210196 | 0.712436 |
| case-028 | 67.704191 | 0.002925 | 67.707116 | 80.781474 | 0.010054 | 80.791528 | 0.009058 | 0.052312 |
| case-031 | 67.716079 | 0.076619 | 67.792698 | 80.652971 | 0.140671 | 80.793642 | 0.237330 | 0.727092 |
| case-043 | 67.729025 | 0.062739 | 67.791765 | 80.651081 | 0.140533 | 80.791614 | 0.194414 | 0.726311 |
| case-049 | 67.703942 | 0.002988 | 67.706930 | 80.781461 | 0.008815 | 80.790276 | 0.009251 | 0.045868 |
| case-050 | 67.704938 | 0.003237 | 67.708174 | 80.780793 | 0.010747 | 80.791540 | 0.010022 | 0.055919 |
| case-052 | 67.574417 | 0.215854 | 67.790271 | 80.647555 | 0.141874 | 80.789429 | 0.665689 | 0.733105 |
| case-053 | 67.755603 | 0.034357 | 67.789960 | 80.573386 | 0.216504 | 80.789889 | 0.106553 | 1.114469 |
| case-054 | 67.762698 | 0.029751 | 67.792449 | 80.446993 | 0.345673 | 80.792667 | 0.092289 | 1.767879 |
| case-055 | 67.607592 | 0.182430 | 67.790022 | 80.579413 | 0.210900 | 80.790313 | 0.563187 | 1.085961 |
| case-056 | 67.727034 | 0.065167 | 67.792201 | 80.641210 | 0.150405 | 80.791615 | 0.201924 | 0.776933 |
| case-057 | 67.671950 | 0.120002 | 67.791952 | 80.590290 | 0.201109 | 80.791398 | 0.371200 | 1.036124 |
| case-058 | 67.703133 | 0.003423 | 67.706556 | 80.783107 | 0.006573 | 80.789680 | 0.010599 | 0.034207 |
| case-059 | 67.703942 | 0.002552 | 67.706494 | 80.780450 | 0.010319 | 80.790769 | 0.007902 | 0.053691 |
| case-060 | 67.705249 | 0.001867 | 67.707116 | 80.776607 | 0.014844 | 80.791451 | 0.005782 | 0.077221 |
| case-061 | 67.704316 | 0.002552 | 67.706867 | 80.788059 | 0.003072 | 80.791131 | 0.007902 | 0.015991 |
| case-062 | 67.704378 | 0.002988 | 67.707365 | 80.782433 | 0.009090 | 80.791523 | 0.009251 | 0.047298 |
| case-063 | 67.704067 | 0.002303 | 67.706369 | 80.787666 | 0.002943 | 80.790608 | 0.007131 | 0.015316 |
| case-064 | 67.706121 | 0.001929 | 67.708050 | 80.788293 | 0.003308 | 80.791601 | 0.005975 | 0.017219 |
| case-065 | 67.705062 | 0.002116 | 67.707179 | 80.787479 | 0.002882 | 80.790362 | 0.006553 | 0.015003 |
| case-066 | 67.704813 | 0.002241 | 67.707054 | 80.787349 | 0.002927 | 80.790276 | 0.006938 | 0.015237 |
| case-067 | 67.704316 | 0.001992 | 67.706307 | 80.782004 | 0.007617 | 80.789621 | 0.006167 | 0.039633 |

## Disk and source transfers

| Case | Disk read GB | Read calls | Disk write GB | Write calls | Disk total GB | Source GB | Source GETs | Tail write bytes |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| case-000 | 4.044885 | 1099 | 141.564887 | 47549 | 145.609772 | 537.811422 | 517481 | 66084864 |
| case-007 | 0.283839 | 54 | 141.593805 | 48999 | 141.877644 | 537.875869 | 518835 | 66375680 |
| case-017 | 3.837841 | 1035 | 141.563777 | 47533 | 145.401618 | 537.837768 | 517473 | 66084864 |
| case-022 | 3.859550 | 1090 | 141.565067 | 47593 | 145.424617 | 537.810996 | 517474 | 66084864 |
| case-028 | 0.281504 | 47 | 141.592846 | 49020 | 141.874350 | 537.840176 | 518832 | 66375680 |
| case-031 | 3.939526 | 1231 | 141.568336 | 47626 | 145.507862 | 537.780983 | 517457 | 66084864 |
| case-043 | 3.934736 | 1006 | 141.565944 | 47552 | 145.500679 | 537.837760 | 517472 | 66084864 |
| case-049 | 0.246825 | 48 | 141.593862 | 49013 | 141.840687 | 537.875224 | 518835 | 66375680 |
| case-050 | 0.300922 | 52 | 141.592838 | 49018 | 141.893760 | 537.839839 | 518815 | 66371584 |
| case-052 | 3.973199 | 3464 | 141.571117 | 47540 | 145.544316 | 537.898957 | 517496 | 66088960 |
| case-053 | 6.062136 | 552 | 141.566923 | 47536 | 147.629058 | 537.886060 | 517501 | 66088960 |
| case-054 | 9.678909 | 478 | 141.570433 | 47628 | 151.249342 | 537.808291 | 517461 | 66084864 |
| case-055 | 5.905807 | 2921 | 141.566939 | 47540 | 147.472746 | 537.874205 | 517500 | 66088960 |
| case-056 | 4.210879 | 1045 | 141.564998 | 47576 | 145.775877 | 537.837751 | 517465 | 66084864 |
| case-057 | 5.633839 | 1926 | 141.569126 | 47564 | 147.202965 | 537.843806 | 517469 | 66084864 |
| case-058 | 0.184057 | 55 | 141.592687 | 48981 | 141.776744 | 537.891906 | 518841 | 66383872 |
| case-059 | 0.288939 | 41 | 141.591642 | 48981 | 141.880581 | 537.861428 | 518842 | 66383872 |
| case-060 | 0.415647 | 30 | 141.593825 | 49004 | 142.009473 | 537.842320 | 518832 | 66375680 |
| case-061 | 0.086020 | 41 | 141.593813 | 49001 | 141.679833 | 537.851282 | 518836 | 66375680 |
| case-062 | 0.254511 | 48 | 141.592773 | 49002 | 141.847283 | 537.840316 | 518828 | 66375680 |
| case-063 | 0.082394 | 37 | 141.592662 | 48975 | 141.675056 | 537.865930 | 518844 | 66383872 |
| case-064 | 0.092629 | 31 | 141.592785 | 49005 | 141.685414 | 537.838137 | 518817 | 66375680 |
| case-065 | 0.080710 | 34 | 141.595914 | 49004 | 141.676624 | 537.872829 | 518831 | 66375680 |
| case-066 | 0.081969 | 36 | 141.593891 | 49020 | 141.675860 | 537.875218 | 518833 | 66379776 |
| case-067 | 0.213270 | 32 | 141.592678 | 48979 | 141.805948 | 537.893571 | 518845 | 66383872 |

Disk traffic is measured DataFile I/O, subtracting the before snapshot and including the five-second observation tail. It differs from caller disk-returned bytes, which can include buffered hits. Whole-device counters, read/write call counts and iostat metrics are retained in the CSV and per-case inventory. Source GETs and bytes match independent callback counts and returned bytes exactly.

## CPU and RSS

| Case | Request-phase CPU s | Mean cores | OS user/system s | Sampled peak RSS GB | OS maximum RSS GB |
| --- | --- | --- | --- | --- | --- |
| case-000 | 59.000000 | 0.044855 | 39.84/27.05 | 6.178333 | 6.179074 |
| case-007 | 56.000000 | 0.042475 | 39.40/24.51 | 4.051603 | 4.053811 |
| case-017 | 57.000000 | 0.043335 | 39.25/25.75 | 6.143455 | 6.143750 |
| case-022 | 57.000000 | 0.043336 | 38.73/25.97 | 6.075335 | 6.075793 |
| case-028 | 55.000000 | 0.041717 | 39.87/24.11 | 4.026962 | 4.027691 |
| case-031 | 56.000000 | 0.042577 | 40.02/25.36 | 6.049796 | 6.050910 |
| case-043 | 56.000000 | 0.042575 | 39.05/25.48 | 6.219383 | 6.220202 |
| case-049 | 55.000000 | 0.041716 | 38.32/24.82 | 4.019995 | 4.022845 |
| case-050 | 55.000000 | 0.041717 | 39.34/24.84 | 4.104905 | 4.107457 |
| case-052 | 59.000000 | 0.044853 | 40.62/25.87 | 5.899489 | 5.900616 |
| case-053 | 56.000000 | 0.042572 | 38.69/25.65 | 7.339061 | 7.340659 |
| case-054 | 58.000000 | 0.044097 | 39.56/27.05 | 8.883479 | 8.886448 |
| case-055 | 58.000000 | 0.044092 | 41.09/26.28 | 8.782889 | 8.789975 |
| case-056 | 57.000000 | 0.043336 | 39.09/25.54 | 5.820199 | 5.820609 |
| case-057 | 59.000000 | 0.044855 | 40.99/27.31 | 8.693957 | 8.697901 |
| case-058 | 56.000000 | 0.042473 | 39.44/24.05 | 3.908870 | 3.910656 |
| case-059 | 55.000000 | 0.041715 | 39.47/24.42 | 4.028609 | 4.041413 |
| case-060 | 55.000000 | 0.041717 | 39.18/24.39 | 4.256322 | 4.267758 |
| case-061 | 57.000000 | 0.043233 | 40.31/24.15 | 3.793666 | 3.796652 |
| case-062 | 55.000000 | 0.041717 | 39.35/24.84 | 3.964887 | 3.968168 |
| case-063 | 55.000000 | 0.041715 | 39.34/23.70 | 3.769831 | 3.771085 |
| case-064 | 57.000000 | 0.043234 | 39.55/24.87 | 3.831009 | 3.833565 |
| case-065 | 56.000000 | 0.042475 | 39.51/23.69 | 3.791770 | 3.794182 |
| case-066 | 56.000000 | 0.042474 | 39.03/25.58 | 3.818127 | 3.822432 |
| case-067 | 56.000000 | 0.042473 | 39.71/24.88 | 3.946058 | 3.949904 |

Request-phase CPU excludes input loading and startup and has one-second counter resolution. OS time covers the whole process. RSS is not cache charged memory or actual payload occupancy. One-second sampled peaks may miss short spikes.

## Exact scheduled-to-complete latency

| Case | Mean s | p50 ms | p95 s | p99 s | Maximum s |
| --- | --- | --- | --- | --- | --- |
| case-000 | 149.683048 | 0.294161 | 686.542741 | 716.537737 | 719.623179 |
| case-007 | 150.544273 | 0.222850 | 689.337759 | 719.604891 | 722.695362 |
| case-017 | 149.674987 | 0.205887 | 686.509442 | 716.506284 | 719.589294 |
| case-022 | 149.673694 | 0.208099 | 686.502970 | 716.499378 | 719.586375 |
| case-028 | 150.540960 | 0.259230 | 689.326360 | 719.595800 | 722.685715 |
| case-031 | 149.659892 | 0.217046 | 686.452525 | 716.451573 | 719.543279 |
| case-043 | 149.674219 | 0.220938 | 686.504733 | 716.501064 | 719.585322 |
| case-049 | 150.546061 | 0.229036 | 689.349151 | 719.618456 | 722.707414 |
| case-050 | 150.529809 | 0.229511 | 689.297201 | 719.570168 | 722.658473 |
| case-052 | 149.697194 | 0.292363 | 686.595095 | 716.594923 | 719.678544 |
| case-053 | 149.698848 | 0.214665 | 686.595590 | 716.593055 | 719.678135 |
| case-054 | 149.664263 | 0.234681 | 686.467633 | 716.468244 | 719.551487 |
| case-055 | 149.698698 | 0.218651 | 686.598159 | 716.598538 | 719.683467 |
| case-056 | 149.665980 | 0.218602 | 686.473975 | 716.468720 | 719.556072 |
| case-057 | 149.676343 | 0.272586 | 686.522617 | 716.522488 | 719.603505 |
| case-058 | 150.552261 | 0.190396 | 689.377402 | 719.647183 | 722.735704 |
| case-059 | 150.552002 | 0.206762 | 689.373294 | 719.643763 | 722.730655 |
| case-060 | 150.541006 | 0.217391 | 689.324109 | 719.594763 | 722.683858 |
| case-061 | 150.543947 | 0.256304 | 689.338210 | 719.607304 | 722.698454 |
| case-062 | 150.538672 | 0.209998 | 689.323087 | 719.593010 | 722.684367 |
| case-063 | 150.553007 | 0.212231 | 689.374600 | 719.647681 | 722.738579 |
| case-064 | 150.533368 | 0.239015 | 689.314448 | 719.585414 | 722.673104 |
| case-065 | 150.542499 | 0.203935 | 689.339001 | 719.612275 | 722.701993 |
| case-066 | 150.544616 | 0.196861 | 689.349104 | 719.623421 | 722.709485 |
| case-067 | 150.556325 | 0.212612 | 689.392553 | 719.661602 | 722.749520 |

## Replay dispatch and service latency

| Case | Sender lateness mean/p99 ms | Dispatch wait mean/p99 ms | Lookup service mean/p50/p95/p99 s |
| --- | --- | --- | --- |
| case-000 | 0.029493/0.053816 | 0.293501/3.643896 | 149.682725/0.000001/686.542406/716.537382 |
| case-007 | 0.029478/0.053832 | 0.236745/3.003917 | 150.544007/0.000001/689.337709/719.604697 |
| case-017 | 0.029422/0.053750 | 0.227737/3.114111 | 149.674730/0.000001/686.509344/716.506141 |
| case-022 | 0.029512/0.053864 | 0.225774/3.065813 | 149.673439/0.000001/686.502665/716.499268 |
| case-028 | 0.029445/0.053768 | 0.268811/3.430582 | 150.540662/0.000001/689.326300/719.595610 |
| case-031 | 0.029731/0.053737 | 0.238520/3.180880 | 149.659623/0.000001/686.452406/716.451325 |
| case-043 | 0.029872/0.053805 | 0.244555/3.285186 | 149.673944/0.000001/686.504588/716.500771 |
| case-049 | 0.029498/0.053975 | 0.247468/3.299903 | 150.545784/0.000001/689.348803/719.618360 |
| case-050 | 0.029488/0.053806 | 0.245000/3.210443 | 150.529534/0.000001/689.297078/719.570140 |
| case-052 | 0.029503/0.053853 | 0.318128/4.258662 | 149.696847/0.000001/686.594979/716.594804 |
| case-053 | 0.029515/0.053856 | 0.245119/3.307417 | 149.698574/0.000001/686.595505/716.593019 |
| case-054 | 0.029528/0.053921 | 0.259471/3.350423 | 149.663974/0.000001/686.467457/716.468100 |
| case-055 | 0.029535/0.054141 | 0.237808/3.298743 | 149.698431/0.000001/686.597931/716.598478 |
| case-056 | 0.029456/0.053767 | 0.239445/3.247963 | 149.665711/0.000001/686.473853/716.468591 |
| case-057 | 0.029453/0.053794 | 0.282045/3.541935 | 149.676031/0.000001/686.522202/716.522274 |
| case-058 | 0.029549/0.054038 | 0.211490/2.853901 | 150.552020/0.000001/689.377344/719.647100 |
| case-059 | 0.029490/0.053880 | 0.228500/3.029647 | 150.551744/0.000001/689.373191/719.643645 |
| case-060 | 0.029539/0.054127 | 0.232595/3.067950 | 150.540744/0.000001/689.324052/719.594536 |
| case-061 | 0.029482/0.053851 | 0.270764/3.434961 | 150.543647/0.000001/689.337686/719.606941 |
| case-062 | 0.029489/0.053863 | 0.236578/3.106023 | 150.538406/0.000001/689.322679/719.592870 |
| case-063 | 0.029514/0.053889 | 0.234930/3.126457 | 150.552743/0.000001/689.374112/719.647609 |
| case-064 | 0.029475/0.053830 | 0.259336/3.307164 | 150.533079/0.000001/689.314383/719.585162 |
| case-065 | 0.029477/0.053860 | 0.224481/2.995274 | 150.542245/0.000001/689.338830/719.612126 |
| case-066 | 0.029471/0.053808 | 0.221901/2.950684 | 150.544364/0.000001/689.348656/719.623055 |
| case-067 | 0.029499/0.054022 | 0.229041/2.958747 | 150.556066/0.000001/689.392486/719.661484 |

These replay quantiles come from exact per-request durations, not histogram bucket interpolation. Lookup service includes callback source queue and service delay. Disk-lookup, disk-I/O and source queue/service histograms are retained in `histograms-and-windows.json` and `METRICS.md` per case. Their quantiles are bucket intervals, not exact percentiles.

## Source backlog and complete drain

| Case | Execution s | Drain s | Source wait mean s | Source service mean s | Peak waiting | Peak active | Peak pending write GB |
| --- | --- | --- | --- | --- | --- | --- | --- |
| case-000 | 1315.359125 | 715.999210 | 464.604218 | 0.121994 | 293154 | 48 | 2.830828 |
| case-007 | 1318.425949 | 719.066034 | 466.058563 | 0.121960 | 294508 | 48 | 2.772665 |
| case-017 | 1315.324749 | 715.964834 | 464.586584 | 0.121993 | 293150 | 48 | 2.665731 |
| case-022 | 1315.315856 | 715.955941 | 464.581678 | 0.121992 | 293143 | 48 | 3.862484 |
| case-028 | 1318.418378 | 719.058463 | 466.050901 | 0.121960 | 294495 | 48 | 3.882878 |
| case-031 | 1315.271002 | 715.911087 | 464.554052 | 0.121992 | 293120 | 48 | 3.914572 |
| case-043 | 1315.321982 | 715.962067 | 464.585041 | 0.121993 | 293147 | 48 | 2.776529 |
| case-049 | 1318.441294 | 719.081379 | 466.064064 | 0.121962 | 294504 | 48 | 3.882999 |
| case-050 | 1318.392463 | 719.032548 | 466.031714 | 0.121962 | 294484 | 48 | 2.642289 |
| case-052 | 1315.416341 | 716.056426 | 464.634593 | 0.121995 | 293171 | 48 | 2.775539 |
| case-053 | 1315.416134 | 716.056219 | 464.635458 | 0.121995 | 293180 | 48 | 2.606956 |
| case-054 | 1315.288875 | 715.928960 | 464.563962 | 0.121993 | 293130 | 48 | 3.868075 |
| case-055 | 1315.419932 | 716.060017 | 464.635920 | 0.121994 | 293181 | 48 | 2.606885 |
| case-056 | 1315.289245 | 715.929330 | 464.565769 | 0.121992 | 293138 | 48 | 2.705885 |
| case-057 | 1315.342278 | 715.982363 | 464.594216 | 0.121994 | 293148 | 48 | 2.830747 |
| case-058 | 1318.470733 | 719.110818 | 466.077983 | 0.121963 | 294519 | 48 | 2.738571 |
| case-059 | 1318.468508 | 719.108593 | 466.076231 | 0.121963 | 294521 | 48 | 2.628642 |
| case-060 | 1318.418241 | 719.058326 | 466.051153 | 0.121961 | 294506 | 48 | 2.643996 |
| case-061 | 1318.429011 | 719.069096 | 466.056550 | 0.121960 | 294507 | 48 | 2.686414 |
| case-062 | 1318.415243 | 719.055328 | 466.047509 | 0.121961 | 294493 | 48 | 2.642340 |
| case-063 | 1318.468003 | 719.108088 | 466.077527 | 0.121962 | 294525 | 48 | 2.680025 |
| case-064 | 1318.407277 | 719.047362 | 466.040894 | 0.121962 | 294487 | 48 | 2.832155 |
| case-065 | 1318.432939 | 719.073024 | 466.056701 | 0.121961 | 294506 | 48 | 2.616450 |
| case-066 | 1318.447583 | 719.087668 | 466.061465 | 0.121962 | 294509 | 48 | 3.930824 |
| case-067 | 1318.481580 | 719.121665 | 466.086918 | 0.121963 | 294523 | 48 | 2.628605 |

The input prefix has a last scheduled arrival at 599.359914970 seconds. Drain is execution minus that arrival time, not a timeout. All replay queued/active and source waiting/active gauges are zero at request finish and after the tail. Every pending disk-write gauge is zero after the tail. Large expected source queues were allowed to drain fully.

## Admission and eviction

| Case | Queued | Covered at admission | Redundant | Published | Covered terminal | Memory evicting insertions | Disk evicting insertions |
| --- | --- | --- | --- | --- | --- | --- | --- |
| case-000 | 122358 | 394389 | 734 | 122097 | 261 | 2699 | 0 |
| case-007 | 123885 | 394240 | 710 | 123885 | 0 | 1624 | 0 |
| case-017 | 122358 | 394342 | 773 | 122098 | 260 | 2622 | 0 |
| case-022 | 122357 | 394031 | 1086 | 122099 | 258 | 2683 | 0 |
| case-028 | 123885 | 393890 | 1057 | 123885 | 0 | 1444 | 0 |
| case-031 | 122358 | 394047 | 1052 | 122099 | 259 | 2745 | 0 |
| case-043 | 122357 | 394382 | 733 | 122100 | 257 | 2538 | 0 |
| case-049 | 123885 | 393946 | 1004 | 123885 | 0 | 1603 | 0 |
| case-050 | 123885 | 394120 | 810 | 123885 | 0 | 1506 | 0 |
| case-052 | 122356 | 394379 | 761 | 122100 | 256 | 3603 | 0 |
| case-053 | 122358 | 394408 | 735 | 122096 | 262 | 1970 | 0 |
| case-054 | 122357 | 393985 | 1119 | 122101 | 256 | 1647 | 0 |
| case-055 | 122357 | 394309 | 834 | 122100 | 257 | 3312 | 0 |
| case-056 | 122358 | 394334 | 773 | 122100 | 258 | 2494 | 0 |
| case-057 | 122358 | 394316 | 795 | 122097 | 261 | 2980 | 0 |
| case-058 | 123885 | 394230 | 726 | 123885 | 0 | 1712 | 0 |
| case-059 | 123885 | 394144 | 813 | 123885 | 0 | 1163 | 0 |
| case-060 | 123885 | 394155 | 792 | 123885 | 0 | 937 | 0 |
| case-061 | 123885 | 394147 | 804 | 123885 | 0 | 1887 | 0 |
| case-062 | 123885 | 394203 | 740 | 123885 | 0 | 1480 | 0 |
| case-063 | 123885 | 394167 | 792 | 123885 | 0 | 1802 | 0 |
| case-064 | 123885 | 394222 | 710 | 123885 | 0 | 1808 | 0 |
| case-065 | 123885 | 394263 | 683 | 123885 | 0 | 2117 | 0 |
| case-066 | 123885 | 393938 | 1010 | 123885 | 0 | 1899 | 0 |
| case-067 | 123885 | 394174 | 786 | 123885 | 0 | 1431 | 0 |

## Memory charge, not payload occupancy

| Case | Settled used charge GB | Settled idle buffers GB | Used minus idle GB | Sampled peak used charge GB |
| --- | --- | --- | --- | --- |
| case-000 | 98.480744 | 1.646264 | 96.834479 | 98.597575 |
| case-007 | 98.880076 | 0.138445 | 98.741631 | 99.051149 |
| case-017 | 98.574349 | 1.751646 | 96.822703 | 98.673581 |
| case-022 | 98.546362 | 1.618477 | 96.927885 | 98.596491 |
| case-028 | 99.018840 | 0.248545 | 98.770295 | 99.027907 |
| case-031 | 98.706948 | 1.692238 | 97.014710 | 98.706948 |
| case-043 | 98.612856 | 1.893728 | 96.719128 | 98.734406 |
| case-049 | 98.906396 | 0.075530 | 98.830865 | 99.053542 |
| case-050 | 99.161945 | 0.281051 | 98.880894 | 99.161945 |
| case-052 | 98.504366 | 1.422426 | 97.081940 | 98.504366 |
| case-053 | 99.943829 | 2.904850 | 97.038978 | 99.951475 |
| case-054 | 99.966509 | 3.081798 | 96.884712 | 99.999512 |
| case-055 | 98.695889 | 1.567588 | 97.128300 | 98.695889 |
| case-056 | 98.506899 | 1.689551 | 96.817348 | 98.534385 |
| case-057 | 98.928649 | 1.867481 | 97.061168 | 99.002610 |
| case-058 | 98.889613 | 0.070681 | 98.818933 | 98.900485 |
| case-059 | 99.096885 | 0.221282 | 98.875602 | 99.191240 |
| case-060 | 99.204032 | 0.227574 | 98.976459 | 99.242107 |
| case-061 | 98.537353 | 0.048234 | 98.489118 | 99.047289 |
| case-062 | 99.421304 | 0.271614 | 99.149690 | 99.459534 |
| case-063 | 98.574315 | 0.047186 | 98.527129 | 98.862539 |
| case-064 | 98.730561 | 0.054526 | 98.676035 | 98.937541 |
| case-065 | 99.158031 | 0.050332 | 99.107699 | 99.290121 |
| case-066 | 98.740976 | 0.048234 | 98.692741 | 99.119776 |
| case-067 | 98.774679 | 0.051413 | 98.723266 | 98.847206 |

## Completion and evidence

- Every case completed exactly 1,606,645 requests and returned 2,800,015,397,526 bytes. Replay exit status is zero. The binary checks per-request indices exactly once before successful exit, and captured counters independently cross-check aggregate completion and lookup outcomes.
- Every case has zero lookup errors, disk-I/O errors, queue-full/closed/canceled admissions, no-capacity writes and failed/canceled terminal writes. Admitted writes equal terminal outcomes. No replay/source work or disk-write payload remains after the tail.
- Every actual summary confirms 100,000,000,000 memory bytes, 500,000,000,000 disk bytes, 32 workers, fixed source settings, requested mode and tuning. Every manifest matches the same input and replay binary. Each archive checksum was independently verified. All configurations are unique.
- Full per-case metrics are in [results.csv](results.csv). Raw captures include one-second Prometheus samples, before/request-finish/settled snapshots, exact replay summaries, source and disk histograms, full metric inventories, pidstat, iostat, process time/RSS, device counters, commands and manifests.
- First-wave corrected inventories and reports are under `raw/reported/case-000` through `case-013`. Original captures and failures remain untouched under `raw/cases/` and `raw/collection-recovery/`. Later inventories and successful reports are beside their captures under `raw/cases/case-014` onward.

## Interpretation and limitations

- These are exploratory, adaptive, unreplicated cross-host observations. A large observed read-traffic change is distinct from a source or overall-performance gain and is not statistical proof. Timing noise, multiple candidate selection and different availability zones in the first wave prevent treating tiny differences as causal. The final stop follows the predeclared strong-contrast plan, not demonstrated convergence.
- The original 52 cases produced only tiny combined-hit/source differences. Their historical settings and results remain in `raw/local-search-final/`. The strong contrasts are evaluated separately above against both baseline and those previous best observations. No aggregate score or universal winner is inferred.
- Memory eviction occurred in every case. In the first wave it began around 169 seconds for range and 445 seconds for full objects, before arrivals ended. Disk eviction never occurred, so disk request-cost effects are unidentifiable at this capacity and trace length. No disk-coefficient neighbors were added after the initial wave.
- The source remains a deterministic 48-slot simulation with 108.599 ms plus requested bytes / 84,101,038.08 bytes/s. It uses independent zero-filled downloads, not real S3. Calibration used another host and much larger objects. There is no shared-bandwidth, coalescing, retry, failure or variability model. The expected backlog dominates end-to-end latency.
- Full-object mode treats each exact recorded range as an object, not the original complete source file. All cases use a fresh cache and the same first 600 seconds of input. Capacity and workload were not changed.
- Actual memory-payload occupancy is unavailable because the binary does not emit that metric. Memory charge, idle buffers and RSS are reported separately and must not be substituted for payload occupancy. Snapshot maxima are sampled and may miss short spikes.

Report-recovery details, historical source identity and reproduction requirements are in [README.md](README.md). Operational history remains under `raw/local-search-final/` and `raw/curation-originals/`.
