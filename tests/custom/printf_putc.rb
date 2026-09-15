# `Kernel#printf` and `Kernel#putc`. In the reference these live in mruby-io
# (`mrbgems/mruby-io/mrblib/kernel.rb`): `printf(...)` is `$stdout.printf(...)`, which
# `IO#printf` spells `write sprintf(*args)`, and `putc(c)` is `$stdout.putc(c); nil`.
# There is no IO here, so both write where `print` writes; the observable part is the
# bytes and the answer, and both are what the reference answers (its `.rc.out`).
#   printf  -> nil always; the format errors are sprintf's ("too few arguments",
#              "Integer cannot be converted to String")
#   putc    -> nil always (IO#putc answers the argument, Kernel#putc does not)
#              Integer: the one byte `c & 0xff`, so 0x141 is "A" and -1 is "\xff"
#              anything else: `mrb_obj_as_string` then its FIRST character
# The multibyte half of "first character" is in printf_putc_utf8.rb: this file stays
# ASCII so that a byte-read build and a character-read build agree on it.

p printf("a=%d b=%s c=%05.2f\n", 1, "x", 3.14159)
p printf("")
p printf("%s and %s\n", :sym, [1, 2])
p putc(65)
putc(10)
p putc(0x141)
putc(10)
p putc(-1 & 0xff)
putc(10)
p putc("hello")
putc(10)
p putc("")
putc(10)
p putc(nil)
putc(10)
p putc(1.5)
putc(10)

class Z
  def to_s
    "zebra"
  end
end
p putc(Z.new)
putc(10)

p Kernel.printf("K%d\n", 7)
p Kernel.putc(66)
putc(10)

p Kernel.private_instance_methods(false).include?(:printf)
p Kernel.private_instance_methods(false).include?(:putc)
# `respond_to?` is asked with `true` because mruby answers true for a private method and
# SabiRuby answers false — a standing difference of its own (leftovers-plan item 10), not
# something these two methods decide.
p self.respond_to?(:printf, true)
p Object.new.respond_to?(:putc, true)

begin; printf; rescue => e; p e; end
begin; printf(1); rescue => e; p e; end
begin; putc; rescue => e; p e; end
begin; putc(1, 2); rescue => e; p e; end
