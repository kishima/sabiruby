# From mruby's benchmark/vm_optimization_bench.rb: the composite benchmarks that build
# things -- an Array grown and mapped, a String grown by concatenation, a Hash of fifty
# thousand entries walked -- and the string-literal section. Split out of that one file
# (see vmo_dispatch.rb). Its other two composites, fib(30) and tak(18,12,6), are what
# `bm_fib` and `app_tak` already measure, so they are not repeated here.
def measure(name, iterations = 1)
  3.times { yield }
  GC.start
  t0 = Time.now
  iterations.times { yield }
  elapsed = Time.now - t0
  puts "#{name}: #{elapsed * 1000 / iterations} ms"
  elapsed
end

# 6d. String literals (allocation vs interning)
measure("string_literals", 5) do
  i = 0
  while i < 100000
    s = "hello"
    s = "world"
    s = "test"
    i += 1
  end
end

# 9c. Array manipulation
measure("array_manipulation", 5) do
  ary = []
  10000.times { |i| ary << i }
  ary.map! { |x| x * 2 }
  ary.select { |x| x % 3 == 0 }.size
end

# 9d. String operations
measure("string_ops", 5) do
  s = ""
  10000.times { |i| s = s + i.to_s }
  s.size
end

# 9e. Hash operations
measure("hash_ops", 5) do
  h = {}
  50000.times { |i| h[i.to_s] = i }
  sum = 0
  h.each { |k, v| sum += v }
  sum
end
