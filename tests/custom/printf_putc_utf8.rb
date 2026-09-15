# utf8-only: strings as characters (the feature `utf8`)
# `Kernel#putc` with a string whose first character is more than one byte. `io_putc`
# (`mrbgems/mruby-io/src/io.c`) writes `mrb_utf8len(ptr, ptr + len)` bytes under
# MRB_UTF8_STRING and one byte without it, so this half of the case belongs to the
# character-read build only; the reference output comes from the -utf8 image.
# It asks the flag of the build and not of the string: `putc("→".b)` writes the
# whole three bytes in the reference too, although `String#b` marks it byte-read.
# A byte that starts no valid sequence is a character of its own (`mrb_utf8len_table`),
# so "\xff\xfe" writes one byte.

putc "→"
putc 10
putc "→abc"
putc 10
putc "→".b
putc 10
putc "\xff\xfe"
putc 10
putc "éx"
putc 10
p putc("→")
