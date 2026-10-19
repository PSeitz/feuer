# Observed objective leaders

Exploratory, one observation per configuration. Small differences are not established improvements.
All configurations and captured summary fields are in [results.csv](results.csv).
Source downloaded bytes equal callback-returned bytes because downloads are independent exact ranges.

## range

36 completed configurations. Disk eviction observed: False.

| Objective | Baseline | Best observed | Case | Memory cost / disk cost / half-life |
| --- | ---: | ---: | --- | --- |
| Memory request hits | 67.722615% | 67.762698% | case-054 | [1000000000, 256000.0, 262144] |
| Memory byte hits | 80.648088% | 80.654748% | case-022 | [230400.0, 256000.0, 288358] |
| Conditional disk request hits | 0.212502% | 0.665689% | case-052 | [0, 256000.0, 262144] |
| Conditional disk byte hits | 0.746526% | 1.767879% | case-054 | [1000000000, 256000.0, 262144] |
| Combined request hits | 67.791205% | 67.792698% | case-031 | [186624.0, 256000.0, 262144] |
| Combined byte hits | 80.792555% | 80.793642% | case-031 | [186624.0, 256000.0, 262144] |
| Disk transfer bytes | 145,609,771,867 | 145,401,618,398 | case-017 | [309760.0, 256000.0, 262144] |

Pareto frontier: case-000, case-001, case-002, case-005, case-006, case-014, case-015, case-017, case-018, case-019, case-020, case-021, case-022, case-030, case-031, case-032, case-033, case-034, case-035, case-036, case-037, case-038, case-039, case-040, case-041, case-042, case-043, case-052, case-053, case-054, case-055, case-056, case-057

## full_objects

32 completed configurations. Disk eviction observed: False.

| Objective | Baseline | Best observed | Case | Memory cost / disk cost / half-life |
| --- | ---: | ---: | --- | --- |
| Memory request hits | 67.703569% | 67.706121% | case-064 | [36533156, 256000.0, 16384] |
| Memory byte hits | 80.780116% | 80.788293% | case-064 | [36533156, 256000.0, 16384] |
| Conditional disk request hits | 0.010407% | 0.010599% | case-058 | [0, 256000.0, 262144] |
| Conditional disk byte hits | 0.052742% | 0.077221% | case-060 | [1000000000, 256000.0, 262144] |
| Combined request hits | 67.706930% | 67.708174% | case-050 | [281600.0, 256000.0, 317194] |
| Combined byte hits | 80.790253% | 80.791601% | case-064 | [36533156, 256000.0, 16384] |
| Disk transfer bytes | 141,877,643,586 | 141,675,056,230 | case-063 | [9133289, 256000.0, 16384] |

Pareto frontier: case-007, case-008, case-009, case-012, case-013, case-023, case-024, case-025, case-026, case-027, case-028, case-029, case-044, case-045, case-046, case-047, case-048, case-049, case-050, case-051, case-058, case-059, case-060, case-061, case-062, case-063, case-064, case-065, case-066, case-067
