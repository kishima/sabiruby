# `Numeric#fdiv` (`num_fdiv`, src/numeric.c): the receiver goes through
# `mrb_ensure_float_type`, which converts Integer, Float, Rational, Complex and a wide Integer
# and raises for anything else — it does **not** send `to_f` — and then it is Float division.
# Integer and Float have `fdiv` of their own (`int_fdiv`, `flo_div`); this is what the rest of
# the tower answers with, and what a Numeric subclass inherits.

p 1.fdiv(2)
p 1.0.fdiv(2)
p Rational(1, 2).fdiv(2)
p Rational(3, 4).fdiv(Rational(1, 2))
p (10 ** 30).fdiv(10 ** 29)
# (`Integer#fdiv` with a zero divisor raises ZeroDivisionError there and answers Infinity here;
# that is `int_fdiv`'s own check, not this method's, and is left alone — see the worklog)
p Numeric.instance_method(:fdiv).owner
p 1.method(:fdiv).owner, 1.0.method(:fdiv).owner
p Rational(1, 2).method(:fdiv).owner

# a Numeric that is none of the types `mrb_ensure_float_type` knows
class Odd < Numeric
  def to_f; 3.0; end
end
begin
  p Odd.new.fdiv(2)
rescue TypeError => e
  p e.message
end

# and an argument that cannot be a Float
begin
  Rational(1, 2).fdiv("2")
rescue TypeError => e
  p e.message
end
begin
  Rational(1, 2).fdiv(nil)
rescue TypeError => e
  p e.message
end
