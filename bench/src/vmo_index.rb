# From mruby's benchmark/vm_optimization_bench.rb: reading and writing an Array and a
# Hash by index, a thousand entries each. Split out of that one file (see
# vmo_dispatch.rb); this is the part of it that answers for the data structures.
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

$ary = Array.new(1000) { |i| i }
$hash = {}
1000.times { |i| $hash[i] = i }

# 4a. Array read (sequential)
measure("array_read_seq", 10) do
  ary = $ary
  i = 0
  sum = 0
  while i < M
    sum += ary[i % 1000]
    i += 1
  end
  sum
end

# 4b. Array read (constant index - tests constant propagation)
measure("array_read_const", 10) do
  ary = $ary
  i = 0
  sum = 0
  while i < M
    sum += ary[500]
    i += 1
  end
  sum
end

# 4c. Array write
measure("array_write", 10) do
  ary = Array.new(1000, 0)
  i = 0
  while i < M
    ary[i % 1000] = i
    i += 1
  end
end

# 4d. Hash read
measure("hash_read", 10) do
  h = $hash
  i = 0
  sum = 0
  while i < M
    sum += h[i % 1000]
    i += 1
  end
  sum
end
