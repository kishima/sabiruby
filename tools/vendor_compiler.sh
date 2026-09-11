#!/bin/bash
# Copies the reference mruby compiler (mrbgems/mruby-compiler, Prism and its generated
# sources) into compiler/vendor/. The copy is not modified; see compiler/vendor/VENDOR.md.
#   tools/vendor_compiler.sh    # MRUBY_SRC defaults to ../../ref/mruby
# The Prism sources generated from ERB templates are taken from the reference build
# (build/prism/): run `rake` in the reference tree once before vendoring.
set -eu
cd "$(dirname "$0")/.."
MRUBY=${MRUBY_SRC:-../../ref/mruby}
MC=$MRUBY/mrbgems/mruby-compiler
PR=$MC/lib/prism
GEN=$MRUBY/build/prism
V=compiler/vendor
for f in "$MC/src/compile.c" "$PR/src/prism.c" "$GEN/include/prism/ast.h" "$GEN/src/node.c"; do
  [ -f "$f" ] || { echo "missing $f (reference tree not checked out or not built)" >&2; exit 1; }
done
rm -rf "$V/mruby-compiler" "$V/prism" "$V/mrbconf.h"
mkdir -p "$V/mruby-compiler" "$V/prism/generated"
cp -r "$MC/include" "$MC/src" "$MC/LICENSE" "$MC/README.md" "$V/mruby-compiler/"
cp -r "$PR/include" "$PR/src" "$PR/LICENSE.md" "$V/prism/"
cp -r "$GEN/include" "$GEN/src" "$V/prism/generated/"
cp "$MRUBY/include/mrbconf.h" "$V/mrbconf.h"
echo "mruby: $(git -C "$MRUBY" describe --tags --always) ($(git -C "$MRUBY" rev-parse --short HEAD))"
echo "prism: $(grep -o '"[0-9.]*"' "$PR/include/prism/version.h" | tr -d '"') ($(git -C "$PR" rev-parse --short HEAD))"
du -sh "$V"
