# Historical fixed-size I/O sweep

Superseded by the [read-trace benchmark and decision](../read-trace/README.md).
This sweep ran separate reads/writes, so it did not test interference between
small and large reads. The old benchmark target has been removed.

One matched pair on `m8g-32cpu-local-ssd-2`, 2026-10-21: production 1-MiB
submissions versus a [32-MiB request limit](whole-payload.patch), which submitted
each tested payload whole. Queue depths were unchanged. Eight-caller throughput:

| Payload | Operation | Chunked MiB/s | Whole MiB/s |
| --- | --- | ---: | ---: |
| 8 MiB | Read | 6548 | 6499 |
| 8 MiB | Write | 2792 | 2726 |
| 32 MiB | Read | 6468 | 6380 |
| 32 MiB | Write | 2736 | 2745 |

No clear throughput benefit. Large-payload changes were −2.4% to +2.3%, while
unchanged 1-MiB control cases varied by up to 3.2%. Both runs exited successfully.

Settings: four Tokio workers, 256-MiB initialized file, aligned writes, unpooled
reads, 1/8/32-MiB payloads, 1/8 callers, one second per case. Storage source:
`0c0f6a1695f511301067ce44fe3d3b88975faf8f`. Raw output and build/source hashes
remain in ignored `*.log` files here. Binaries and historical benchmark source
remain in `/home/ubuntu/feuer-payload-io.FyScLF` on the SSD host.
