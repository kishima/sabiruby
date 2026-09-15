#!/bin/bash
# Puts two result TSVs of tools/bench.sh side by side: what each benchmark and each
# category did between them.
#   tools/bench_compare.sh bench/results/e9da768.tsv bench/results/354b6bb.tsv
# The column compared is the best of the runs; the median is shown beside it, because a
# change smaller than the gap between the two is this machine's noise and not the commit's.
set -eu
A=$1
B=$2

awk -F'\t' -v fa="$(basename "$A" .tsv)" -v fb="$(basename "$B" .tsv)" '
FNR == 1 { file++; next }
file == 1 { ab[$2] = $3; am[$2] = $4; next }
{
  order[++n] = $2; cat[$2] = $1; bb[$2] = $3; bm[$2] = $4
  if (!($1 in seencat)) { seencat[$1] = 1; catorder[++nc] = $1 }
  if ($3 + 0 > 0 && ab[$2] + 0 > 0) { bsum[$1] += $3; btot += $3; asum[$1] += ab[$2]; atot += ab[$2] }
}
function pct(x, y) { return (x + 0 > 0 && y + 0 > 0) ? sprintf("%+.1f%%", (y - x) * 100 / x) : "" }
END {
  printf "%-24s %12s %12s %9s   %10s %10s\n", "benchmark", fa, fb, "change", "med " fa, "med " fb
  for (i = 1; i <= n; i++) {
    b = order[i]
    if (!(b in ab)) { printf "%-24s %12s %12s %9s   %10s %10s\n", b, "-", bb[b], "", "-", bm[b]; continue }
    printf "%-24s %12s %12s %9s   %10s %10s\n", b, ab[b], bb[b], pct(ab[b], bb[b]), am[b], bm[b]
  }
  printf "\n%-24s %12s %12s %9s\n", "category (sum)", fa, fb, "change"
  for (i = 1; i <= nc; i++) {
    c = catorder[i]
    if (asum[c] + 0 > 0) printf "%-24s %12.0f %12.0f %9s\n", c, asum[c], bsum[c], pct(asum[c], bsum[c])
  }
  printf "%-24s %12.0f %12.0f %9s\n", "all", atot, btot, pct(atot, btot)
}
' "$A" "$B"
