# calls: keyword arguments, which go through ENTER and KARG.
class Bench
  def kw(a: 1, b: 2, c: 3)
    a + b + c
  end
end
b = Bench.new
s = 0
i = 0
while i < 1_000_000
  s += b.kw
  s += b.kw(a: 4)
  s += b.kw(a: 4, b: 5, c: 6)
  i += 1
end
puts s
