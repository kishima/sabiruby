# Benchmarks by category: 9fa5b0b

SabiRuby against mruby 4.1.0-rc, best and median of the runs `tools/bench.sh` made.
The reference runs inside the Docker image on the same machine, so read the ratio and not
the milliseconds. Empty reference columns mean Docker was not there when this was measured.
Source: `bench/results/9fa5b0b.tsv`.

## Summary

`ratio (sum)` weighs every benchmark by how long it runs, so one long benchmark speaks for the
whole category; `ratio (median)` is the middle of the per-benchmark ratios, which one does not.

| category | mruby ms | SabiRuby ms | ratio (sum) | ratio (median) |
|---|---:|---:|---:|---:|
| whole program | 3120 | 8992 | 2.88x | 2.86x |
| data structures | 665 | 5848 | 8.79x | 4.65x |
| instruction loop | 4947 | 21618 | 4.37x | 3.26x |
| calls | 1280 | 3768 | 2.94x | 2.92x |
| memory | 2167 | 2053 | 0.95x | 2.15x |
| **all** | 12179 | 42279 | 3.47x | 3.15x |

## whole program

| benchmark | mruby ms (best) | mruby ms (median) | SabiRuby ms (best) | SabiRuby ms (median) | ratio | instructions | ns/instruction |
|---|---:|---:|---:|---:|---:|---:|---:|
| app_json_hash | 433 | 437 | 1266.846 | 1271.926 | 2.93x | 31828141 | 39.8 |
| app_robot | 562 | 585 | 1138.499 | 1148.158 | 2.03x | 78076105 | 14.6 |
| app_tak | 403 | 408 | 1154.546 | 1167.851 | 2.86x | 174127313 | 6.6 |
| bm_ao_render | | | fail: undefined method 'printf' for Object (NoMethodError) | | | | |
| bm_fib | 1722 | 1726 | 5432.219 | 5447.359 | 3.15x | 742675934 | 7.3 |
| bm_mandel_term | | | fail: undefined method 'putc' for Object (NoMethodError) | | | | |
| **subtotal** | 3120 | | 8992 | | 2.88x | | |

## data structures

| benchmark | mruby ms (best) | mruby ms (median) | SabiRuby ms (best) | SabiRuby ms (median) | ratio | instructions | ns/instruction |
|---|---:|---:|---:|---:|---:|---:|---:|
| bm_so_lists | 235 | 244 | 3694.667 | 3705.569 | 15.72x | 57014842 | 64.8 |
| ds_array | 245 | 247 | 892.933 | 900.914 | 3.64x | 106641040 | 8.4 |
| ds_hash | 86 | 86 | 799.339 | 802.223 | 9.29x | 27019240 | 29.6 |
| ds_string | 99 | 101 | 460.696 | 461.314 | 4.65x | 13685740 | 33.7 |
| **subtotal** | 665 | | 5848 | | 8.79x | | |

## instruction loop

| benchmark | mruby ms (best) | mruby ms (median) | SabiRuby ms (best) | SabiRuby ms (median) | ratio | instructions | ns/instruction |
|---|---:|---:|---:|---:|---:|---:|---:|
| bm_so_mandelbrot | 862 | 871 | 1379.815 | 1400.621 | 1.60x | 341836245 | 4.0 |
| loop_if_branch | 246 | 250 | 864.950 | 870.459 | 3.52x | 135334074 | 6.4 |
| loop_times | 260 | 262 | 848.144 | 855.277 | 3.26x | 144000748 | 5.9 |
| loop_while_add | 305 | 308 | 841.932 | 842.764 | 2.76x | 220000740 | 3.8 |
| vm_optimization_bench | 3274 | 3298 | 17683.490 | 17717.343 | 5.40x | 1796631487 | 9.8 |
| **subtotal** | 4947 | | 21618 | | 4.37x | | |

## calls

| benchmark | mruby ms (best) | mruby ms (median) | SabiRuby ms (best) | SabiRuby ms (median) | ratio | instructions | ns/instruction |
|---|---:|---:|---:|---:|---:|---:|---:|
| call_args | 299 | 301 | 872.112 | 874.334 | 2.92x | 150000752 | 5.8 |
| call_block_yield | 319 | 323 | 1004.571 | 1011.914 | 3.15x | 120000749 | 8.4 |
| call_fiber | 361 | 363 | 938.211 | 939.924 | 2.60x | 108000742 | 8.7 |
| call_kwargs | 301 | 310 | 953.469 | 962.654 | 3.17x | 86000749 | 11.1 |
| **subtotal** | 1280 | | 3768 | | 2.94x | | |

## memory

| benchmark | mruby ms (best) | mruby ms (median) | SabiRuby ms (best) | SabiRuby ms (median) | ratio | instructions | ns/instruction |
|---|---:|---:|---:|---:|---:|---:|---:|
| gc_churn | 88 | 89 | 375.252 | 379.869 | 4.26x | 15000742 | 25.0 |
| mem_retained | 1634 | 1650 | 720.636 | 725.791 | 0.44x | 46740755 | 15.4 |
| mem_short_lived | 445 | 453 | 956.705 | 961.606 | 2.15x | 78000749 | 12.3 |
| **subtotal** | 2167 | | 2053 | | 0.95x | | |

