# Benchmarks by category: 5c3eb6e

SabiRuby against mruby 4.1.0-rc, best and median of the runs `tools/bench.sh` made.
The reference runs inside the Docker image on the same machine, so read the ratio and not
the milliseconds. Empty reference columns mean Docker was not there when this was measured.
Source: `bench/results/5c3eb6e.tsv`.

## Summary

`ratio (sum)` weighs every benchmark by how long it runs, so one long benchmark speaks for the
whole category; `ratio (median)` is the middle of the per-benchmark ratios, which one does not.

| category | mruby ms | SabiRuby ms | ratio (sum) | ratio (median) |
|---|---:|---:|---:|---:|
| whole program | 3185 | 11312 | 3.55x | 2.97x |
| data structures | 678 | 7395 | 10.91x | 11.38x |
| instruction loop | 5076 | 29464 | 5.80x | 3.89x |
| calls | 1304 | 4541 | 3.48x | 3.37x |
| memory | 2265 | 2435 | 1.07x | 2.55x |
| **all** | 12508 | 55147 | 4.41x | 3.85x |

## whole program

| benchmark | mruby ms (best) | mruby ms (median) | SabiRuby ms (best) | SabiRuby ms (median) | ratio | instructions | ns/instruction |
|---|---:|---:|---:|---:|---:|---:|---:|
| app_json_hash | 447 | 453 | 1326.785 | 1335.035 | 2.97x | 31828141 | 41.7 |
| app_robot | 572 | 574 | 1362.733 | 1373.651 | 2.38x | 78076105 | 17.5 |
| app_tak | 404 | 407 | 1561.850 | 1570.434 | 3.87x | 174127313 | 9.0 |
| bm_ao_render | | | fail: undefined method 'printf' for Object (NoMethodError) | | | | |
| bm_fib | 1762 | 1768 | 7060.503 | 7137.197 | 4.01x | 742675934 | 9.5 |
| bm_mandel_term | | | fail: undefined method 'putc' for Object (NoMethodError) | | | | |
| **subtotal** | 3185 | | 11312 | | 3.55x | | |

## data structures

| benchmark | mruby ms (best) | mruby ms (median) | SabiRuby ms (best) | SabiRuby ms (median) | ratio | instructions | ns/instruction |
|---|---:|---:|---:|---:|---:|---:|---:|
| bm_so_lists | 238 | 243 | 3978.878 | 4045.501 | 16.72x | 57014842 | 69.8 |
| ds_array | 251 | 252 | 1139.449 | 1148.130 | 4.54x | 106641040 | 10.7 |
| ds_hash | 87 | 88 | 1115.784 | 1122.967 | 12.83x | 27019240 | 41.3 |
| ds_string | 102 | 103 | 1160.854 | 1178.169 | 11.38x | 13685740 | 84.8 |
| **subtotal** | 678 | | 7395 | | 10.91x | | |

## instruction loop

| benchmark | mruby ms (best) | mruby ms (median) | SabiRuby ms (best) | SabiRuby ms (median) | ratio | instructions | ns/instruction |
|---|---:|---:|---:|---:|---:|---:|---:|
| bm_so_mandelbrot | 883 | 892 | 1836.407 | 1867.272 | 2.08x | 341836245 | 5.4 |
| loop_if_branch | 253 | 257 | 1069.409 | 1079.055 | 4.23x | 135334074 | 7.9 |
| loop_times | 269 | 273 | 1045.073 | 1046.478 | 3.89x | 144000748 | 7.3 |
| loop_while_add | 311 | 314 | 1196.634 | 1204.396 | 3.85x | 220000740 | 5.4 |
| vm_optimization_bench | 3360 | 3371 | 24316.413 | 24403.778 | 7.24x | 1796631487 | 13.5 |
| **subtotal** | 5076 | | 29464 | | 5.80x | | |

## calls

| benchmark | mruby ms (best) | mruby ms (median) | SabiRuby ms (best) | SabiRuby ms (median) | ratio | instructions | ns/instruction |
|---|---:|---:|---:|---:|---:|---:|---:|
| call_args | 306 | 307 | 1165.538 | 1174.346 | 3.81x | 150000752 | 7.8 |
| call_block_yield | 328 | 329 | 1174.768 | 1195.557 | 3.58x | 120000749 | 9.8 |
| call_fiber | 365 | 370 | 1174.419 | 1183.072 | 3.22x | 108000742 | 10.9 |
| call_kwargs | 305 | 315 | 1026.673 | 1031.103 | 3.37x | 86000749 | 11.9 |
| **subtotal** | 1304 | | 4541 | | 3.48x | | |

## memory

| benchmark | mruby ms (best) | mruby ms (median) | SabiRuby ms (best) | SabiRuby ms (median) | ratio | instructions | ns/instruction |
|---|---:|---:|---:|---:|---:|---:|---:|
| gc_churn | 90 | 91 | 398.880 | 404.374 | 4.43x | 15000742 | 26.6 |
| mem_retained | 1717 | 1727 | 868.355 | 881.280 | 0.51x | 46740755 | 18.6 |
| mem_short_lived | 458 | 464 | 1167.617 | 1170.018 | 2.55x | 78000749 | 15.0 |
| **subtotal** | 2265 | | 2435 | | 1.07x | | |

