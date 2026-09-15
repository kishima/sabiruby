# Benchmarks by category: stage2-final

SabiRuby against mruby 4.1.0-rc, best and median of the runs `tools/bench.sh` made.
The reference runs inside the Docker image on the same machine, so read the ratio and not
the milliseconds. Empty reference columns mean Docker was not there when this was measured.
Source: `bench/results/stage2-final.tsv`.

## Summary

`ratio (sum)` weighs every benchmark by how long it runs, so one long benchmark speaks for the
whole category; `ratio (median)` is the middle of the per-benchmark ratios, which one does not.

| category | mruby ms | SabiRuby ms | ratio (sum) | ratio (median) |
|---|---:|---:|---:|---:|
| whole program | 3120 | 8934 | 2.86x | 2.85x |
| data structures | 664 | 5875 | 8.85x | 4.52x |
| instruction loop | 4968 | 22187 | 4.47x | 3.31x |
| calls | 1281 | 3786 | 2.96x | 2.97x |
| memory | 2196 | 2038 | 0.93x | 2.12x |
| **all** | 12229 | 42820 | 3.50x | 3.09x |

## whole program

| benchmark | mruby ms (best) | mruby ms (median) | SabiRuby ms (best) | SabiRuby ms (median) | ratio | instructions | ns/instruction |
|---|---:|---:|---:|---:|---:|---:|---:|
| app_json_hash | 435 | 438 | 1239.682 | 1245.837 | 2.85x | 31828141 | 38.9 |
| app_robot | 559 | 564 | 1140.784 | 1152.615 | 2.04x | 78076105 | 14.6 |
| app_tak | 394 | 395 | 1142.616 | 1145.168 | 2.90x | 174127313 | 6.6 |
| bm_ao_render | | | fail: undefined method 'printf' for Object (NoMethodError) | | | | |
| bm_fib | 1732 | 1743 | 5410.845 | 5420.520 | 3.12x | 742675934 | 7.3 |
| bm_mandel_term | | | fail: undefined method 'putc' for Object (NoMethodError) | | | | |
| **subtotal** | 3120 | | 8934 | | 2.86x | | |

## data structures

| benchmark | mruby ms (best) | mruby ms (median) | SabiRuby ms (best) | SabiRuby ms (median) | ratio | instructions | ns/instruction |
|---|---:|---:|---:|---:|---:|---:|---:|
| bm_so_lists | 237 | 238 | 3726.951 | 3743.882 | 15.73x | 57014842 | 65.4 |
| ds_array | 243 | 245 | 905.793 | 914.685 | 3.73x | 106641040 | 8.5 |
| ds_hash | 85 | 88 | 794.612 | 808.452 | 9.35x | 27019240 | 29.4 |
| ds_string | 99 | 100 | 447.303 | 453.750 | 4.52x | 13685740 | 32.7 |
| **subtotal** | 664 | | 5875 | | 8.85x | | |

## instruction loop

| benchmark | mruby ms (best) | mruby ms (median) | SabiRuby ms (best) | SabiRuby ms (median) | ratio | instructions | ns/instruction |
|---|---:|---:|---:|---:|---:|---:|---:|
| bm_so_mandelbrot | 858 | 862 | 1462.860 | 1477.627 | 1.70x | 341836245 | 4.3 |
| loop_if_branch | 254 | 255 | 937.575 | 942.730 | 3.69x | 135334074 | 6.9 |
| loop_times | 263 | 266 | 871.235 | 881.275 | 3.31x | 144000748 | 6.1 |
| loop_while_add | 309 | 312 | 874.750 | 878.475 | 2.83x | 220000740 | 4.0 |
| vm_optimization_bench | 3284 | 3295 | 18040.810 | 18109.646 | 5.49x | 1796631487 | 10.0 |
| **subtotal** | 4968 | | 22187 | | 4.47x | | |

## calls

| benchmark | mruby ms (best) | mruby ms (median) | SabiRuby ms (best) | SabiRuby ms (median) | ratio | instructions | ns/instruction |
|---|---:|---:|---:|---:|---:|---:|---:|
| call_args | 296 | 301 | 878.778 | 881.608 | 2.97x | 150000752 | 5.9 |
| call_block_yield | 319 | 324 | 1014.038 | 1028.496 | 3.18x | 120000749 | 8.5 |
| call_fiber | 365 | 368 | 962.455 | 965.904 | 2.64x | 108000742 | 8.9 |
| call_kwargs | 301 | 306 | 931.130 | 941.914 | 3.09x | 86000749 | 10.8 |
| **subtotal** | 1281 | | 3786 | | 2.96x | | |

## memory

| benchmark | mruby ms (best) | mruby ms (median) | SabiRuby ms (best) | SabiRuby ms (median) | ratio | instructions | ns/instruction |
|---|---:|---:|---:|---:|---:|---:|---:|
| gc_churn | 88 | 90 | 366.331 | 375.261 | 4.16x | 15000742 | 24.4 |
| mem_retained | 1661 | 1670 | 724.361 | 727.595 | 0.44x | 46740755 | 15.5 |
| mem_short_lived | 447 | 455 | 947.265 | 949.298 | 2.12x | 78000749 | 12.1 |
| **subtotal** | 2196 | | 2038 | | 0.93x | | |

