#!/bin/bash
# Compile tests/fixtures/*.rb with the reference mruby (Docker image kishima/mruby:4.1.0-rc)
# into .mrb and record the reference output (.out) and the verbose dump (.dump).
#   tools/fixtures.sh            # all fixtures
#   tools/fixtures.sh hello      # one fixture
# Also builds mrblib.mrb (the Ruby part of mruby's core library) into src/mrblib.mrb.
set -eu
cd "$(dirname "$0")/.."
IMG=kishima/mruby:4.1.0-rc
MRUBY=${MRUBY_SRC:-../../ref/mruby}
if [ -d "$MRUBY/mrblib" ]; then
  mkdir -p target/mrblib
  cat "$MRUBY"/mrblib/*.rb > target/mrblib/mrblib_all.rb
  docker run --rm -v "$PWD/target/mrblib:/w" $IMG mrbc -o /w/mrblib.mrb /w/mrblib_all.rb
  cp target/mrblib/mrblib.mrb src/mrblib.mrb
fi
for rb in tests/fixtures/${1:-*}.rb; do
  base=${rb%.rb}
  docker run --rm -v "$PWD/tests/fixtures:/w" $IMG /bin/sh -c "
    mrbc -o /w/$(basename $base).mrb /w/$(basename $rb) &&
    mrbc --verbose /w/$(basename $rb) > /w/$(basename $base).dump 2>&1 &&
    mruby /w/$(basename $rb) > /w/$(basename $base).out 2>&1 || true"
  echo "$base: $(wc -c < $base.mrb) bytes, expected $(wc -l < $base.out) lines"
done
