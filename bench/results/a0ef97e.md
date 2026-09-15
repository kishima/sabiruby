# Benchmarks by category: a0ef97e

SabiRuby against mruby 4.1.0-rc, best and median of the runs `tools/bench.sh` made.
The reference runs inside the Docker image on the same machine, so read the ratio and not
the milliseconds. Empty reference columns mean Docker was not there when this was measured.
Source: `bench/results/a0ef97e.tsv`.

## Summary

`ratio (sum)` weighs every benchmark by how long it runs, so one long benchmark speaks for the
whole category; `ratio (median)` is the middle of the per-benchmark ratios, which one does not.

| category | mruby ms | SabiRuby ms | ratio (sum) | ratio (median) |
|---|---:|---:|---:|---:|
| whole program | 8902 | 25033 | 2.81x | 2.76x |
| data structures | 1109 | 3572 | 3.22x | 3.18x |
| instruction loop | 3382 | 9869 | 2.92x | 3.25x |
| calls | 2004 | 6671 | 3.33x | 3.20x |
| memory | 2259 | 2109 | 0.93x | 2.13x |
| **all** | 17656 | 47254 | 2.68x | 3.05x |

## whole program

| benchmark | mruby ms (best) | mruby ms (median) | SabiRuby ms (best) | SabiRuby ms (median) | ratio | instructions | ns/instruction |
|---|---:|---:|---:|---:|---:|---:|---:|
| app_json_hash | 444 | 448 | 1300.003 | 1302.995 | 2.93x | 31828141 | 40.8 |
| app_robot | 568 | 581 | 1163.310 | 1178.611 | 2.05x | 78076105 | 14.9 |
| app_tak | 405 | 408 | 1118.714 | 1128.497 | 2.76x | 174127313 | 6.4 |
| bm_ao_render | 2494 | 2538 | 5407.529 | 5437.388 | 2.17x | 472066695 | 11.5 |
| bm_fib | 1699 | 1791 | 5179.682 | 5203.512 | 3.05x | 742675934 | 7.0 |
| bm_mandel_term | 11 | 11 | 20.957 | 21.297 | 1.91x | 4745006 | 4.4 |
| vm_optimization_bench | 3281 | 3366 | 10842.976 | 10918.077 | 3.30x | 1796631487 | 6.0 |
| **subtotal** | 8902 | | 25033 | | 2.81x | | |

## data structures

| benchmark | mruby ms (best) | mruby ms (median) | SabiRuby ms (best) | SabiRuby ms (median) | ratio | instructions | ns/instruction |
|---|---:|---:|---:|---:|---:|---:|---:|
| bm_so_lists | 246 | 249 | 988.293 | 998.387 | 4.02x | 57014842 | 17.3 |
| ds_array | 249 | 253 | 840.008 | 847.148 | 3.37x | 106641040 | 7.9 |
| ds_hash | 87 | 88 | 220.120 | 221.600 | 2.53x | 27019240 | 8.1 |
| ds_string | 102 | 104 | 384.116 | 386.792 | 3.77x | 13685740 | 28.1 |
| vmo_index | 163 | 167 | 517.944 | 520.866 | 3.18x | 72821218 | 7.1 |
| vmo_objects | 262 | 343 | 621.730 | 626.770 | 2.37x | 30349025 | 20.5 |
| **subtotal** | 1109 | | 3572 | | 3.22x | | |

## instruction loop

| benchmark | mruby ms (best) | mruby ms (median) | SabiRuby ms (best) | SabiRuby ms (median) | ratio | instructions | ns/instruction |
|---|---:|---:|---:|---:|---:|---:|---:|
| bm_so_mandelbrot | 897 | 900 | 1482.693 | 1488.623 | 1.65x | 341836245 | 4.3 |
| loop_if_branch | 257 | 263 | 884.099 | 885.549 | 3.44x | 135334074 | 6.5 |
| loop_times | 268 | 274 | 869.921 | 870.569 | 3.25x | 144000748 | 6.0 |
| loop_while_add | 315 | 316 | 848.913 | 853.272 | 2.69x | 220000740 | 3.9 |
| vmo_arith | 1046 | 1048 | 3657.410 | 3684.451 | 3.50x | 761803553 | 4.8 |
| vmo_dispatch | 599 | 606 | 2126.015 | 2146.262 | 3.55x | 453859315 | 4.7 |
| **subtotal** | 3382 | | 9869 | | 2.92x | | |

## calls

| benchmark | mruby ms (best) | mruby ms (median) | SabiRuby ms (best) | SabiRuby ms (median) | ratio | instructions | ns/instruction |
|---|---:|---:|---:|---:|---:|---:|---:|
| call_args | 303 | 307 | 875.640 | 884.493 | 2.89x | 150000752 | 5.8 |
| call_block_yield | 329 | 332 | 1051.558 | 1054.142 | 3.20x | 120000749 | 8.8 |
| call_fiber | 370 | 372 | 994.228 | 1000.989 | 2.69x | 108000742 | 9.2 |
| call_kwargs | 233 | 310 | 866.800 | 871.844 | 3.72x | 86000749 | 10.1 |
| vmo_calls | 769 | 838 | 2882.313 | 2903.368 | 3.75x | 320128166 | 9.0 |
| **subtotal** | 2004 | | 6671 | | 3.33x | | |

## memory

| benchmark | mruby ms (best) | mruby ms (median) | SabiRuby ms (best) | SabiRuby ms (median) | ratio | instructions | ns/instruction |
|---|---:|---:|---:|---:|---:|---:|---:|
| gc_churn | 91 | 94 | 393.169 | 396.108 | 4.32x | 15000742 | 26.2 |
| mem_retained | 1710 | 1731 | 740.931 | 746.266 | 0.43x | 46740755 | 15.9 |
| mem_short_lived | 458 | 461 | 974.941 | 980.387 | 2.13x | 78000749 | 12.5 |
| **subtotal** | 2259 | | 2109 | | 0.93x | | |

