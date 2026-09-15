# calls: a block argument and yield, which build an environment per call.
class Bench
  def each_twice
    yield 1
    yield 2
  end
end
b = Bench.new
s = 0
i = 0
while i < 4_000_000
  b.each_twice { |x| s += x }
  i += 1
end
puts s
