# From mruby's benchmark/vm_optimization_bench.rb: the method-call section and the two
# iterators (`times`, `each`), which are calls with a block. Split out of that one file
# (see vmo_dispatch.rb).
def measure(name, iterations = 1)
  3.times { yield }
  GC.start
  t0 = Time.now
  iterations.times { yield }
  elapsed = Time.now - t0
  puts "#{name}: #{elapsed * 1000 / iterations} ms"
  elapsed
end

M = 100_000

class BenchClass
  def empty_method
  end

  def simple_add(a, b)
    a + b
  end

  def self.class_method
  end
end

$obj = BenchClass.new

# 3a. Empty method call (pure dispatch overhead)
measure("empty_method_call", 10) do
  obj = $obj
  i = 0
  while i < M
    obj.empty_method
    i += 1
  end
end

# 3b. Method with arguments
measure("method_with_args", 10) do
  obj = $obj
  i = 0
  while i < M
    obj.simple_add(1, 2)
    i += 1
  end
end

# 3c. Self method call (tests OP_SENDSELF potential)
class SelfCallBench
  def run
    i = 0
    while i < M
      helper
      i += 1
    end
  end

  def helper
  end
end

measure("self_method_call", 10) do
  SelfCallBench.new.run
end

# 3d. Polymorphic call site (tests inline cache invalidation)
class Duck1
  def quack; 1; end
end
class Duck2
  def quack; 2; end
end

$duck1 = Duck1.new
$duck2 = Duck2.new

measure("polymorphic_call", 10) do
  d1, d2 = $duck1, $duck2
  i = 0
  sum = 0
  while i < M
    sum += d1.quack
    sum += d2.quack
    i += 1
  end
  sum
end

# 5b. times iterator (block overhead)
measure("times_iterator", 10) do
  sum = 0
  M.times do |i|
    sum += i
  end
  sum
end

# 5c. each iterator on array
$small_ary = (0...1000).to_a
measure("each_iterator", 10) do
  ary = $small_ary
  total = 0
  1000.times do
    ary.each { |x| total += x }
  end
  total
end
