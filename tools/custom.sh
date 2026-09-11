#!/bin/bash
# SabiRuby's own tests (tests/custom, runner tests/custom.rs). For each
# tests/custom/*.rb: compile it with the reference mrbc (Docker image
# kishima/mruby:4.1.0-rc, with -g) into .mrb and record the reference mruby's
# output as .rc.out. The expected output (.expected) is decided by hand and is
# not touched if it exists; when missing it is created from .rc.out.
#   tools/custom.sh              # all cases
#   tools/custom.sh eval_locals  # one case
set -eu
cd "$(dirname "$0")/.."
IMG=kishima/mruby:4.1.0-rc
for rb in tests/custom/${1:-*}.rb; do
  base=${rb%.rb}; name=$(basename "$base")
  docker run --rm -v "$PWD/tests/custom:/w" $IMG /bin/sh -c "
    mrbc -g -o /w/$name.mrb /w/$name.rb &&
    mruby /w/$name.rb > /w/$name.rc.out 2>&1 || true"
  if [ ! -f "$base.expected" ]; then
    cp "$base.rc.out" "$base.expected"
    echo "$name: .expected created from the reference output; check it"
  fi
  if cmp -s "$base.expected" "$base.rc.out"; then agree=agrees; else agree=DIFFERS; fi
  echo "$name: $(wc -c < "$base.mrb") bytes, reference output $agree with .expected"
done
