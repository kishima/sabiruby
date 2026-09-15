# calls: find_method, CallInfo and the frame, at arity 0 to 3.
class Bench
  def a0; 1; end
  def a1(x); x; end
  def a2(x, y); x + y; end
  def a3(x, y, z); x + y + z; end
end
b = Bench.new
s = 0
i = 0
while i < 3_000_000
  s += b.a0
  s += b.a1(1)
  s += b.a2(1, 2)
  s += b.a3(1, 2, 3)
  i += 1
end
puts s
