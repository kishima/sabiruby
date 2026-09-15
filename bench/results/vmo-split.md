# Benchmarks by category: vmo-split

SabiRuby against mruby 4.1.0-rc, best and median of the runs `tools/bench.sh` made.
The reference runs inside the Docker image on the same machine, so read the ratio and not
the milliseconds. Empty reference columns mean Docker was not there when this was measured.
Source: `bench/results/vmo-split.tsv`.

## Summary

`ratio (sum)` weighs every benchmark by how long it runs, so one long benchmark speaks for the
whole category; `ratio (median)` is the middle of the per-benchmark ratios, which one does not.

| category | mruby ms | SabiRuby ms | ratio (sum) | ratio (median) |
|---|---:|---:|---:|---:|
| instruction loop | 1524 | 5687 | 3.73x | 3.72x |
| calls | 791 | 2776 | 3.51x | 3.51x |
| data structures | 474 | 1137 | 2.40x | 1.87x |
| **all** | 2789 | 9600 | 3.44x | 3.51x |

## instruction loop

| benchmark | mruby ms (best) | mruby ms (median) | SabiRuby ms (best) | SabiRuby ms (median) | ratio | instructions | ns/instruction |
|---|---:|---:|---:|---:|---:|---:|---:|
| vmo_arith | 960 | 975 | 3590.627 | 3634.812 | 3.74x | 761803553 | 4.7 |
| vmo_dispatch | 564 | 566 | 2096.734 | 2139.835 | 3.72x | 453859315 | 4.6 |
| **subtotal** | 1524 | | 5687 | | 3.73x | | |

## calls

| benchmark | mruby ms (best) | mruby ms (median) | SabiRuby ms (best) | SabiRuby ms (median) | ratio | instructions | ns/instruction |
|---|---:|---:|---:|---:|---:|---:|---:|
| vmo_calls | 791 | 793 | 2776.160 | 2836.411 | 3.51x | 320128166 | 8.7 |
| **subtotal** | 791 | | 2776 | | 3.51x | | |

## data structures

| benchmark | mruby ms (best) | mruby ms (median) | SabiRuby ms (best) | SabiRuby ms (median) | ratio | instructions | ns/instruction |
|---|---:|---:|---:|---:|---:|---:|---:|
| vmo_index | 155 | 156 | 539.493 | 542.552 | 3.48x | 72821218 | 7.4 |
| vmo_objects | 319 | 326 | 597.076 | 600.408 | 1.87x | 30349025 | 19.7 |
| **subtotal** | 474 | | 1137 | | 2.40x | | |

