# From mruby's benchmark/vm_optimization_bench.rb: arithmetic, the constant-loading
# sections and the branch-prediction section -- what one instruction costs when the
# operands are immediates. Split out of that one file (see vmo_dispatch.rb).
def measure(name, iterations = 1)
  3.times { yield }
  GC.start
  t0 = Time.now
  iterations.times { yield }
  elapsed = Time.now - t0
  puts "#{name}: #{elapsed * 1000 / iterations} ms"
  elapsed
end

N = 1_000_000
M = 100_000

# 2a. Integer addition (tests OP_ADD fast path)
measure("int_add", 10) do
  x = 0
  i = 0
  while i < N
    x = x + 1
    i += 1
  end
  x
end

# 2b. Integer increment (tests potential OP_INCI fusion)
measure("int_increment", 10) do
  x = 0
  i = 0
  while i < N
    x += 1
    i += 1
  end
  x
end

# 2c. Mixed arithmetic (tests type checking overhead)
measure("mixed_arith", 10) do
  x = 0
  y = 1.5
  i = 0
  while i < M
    x = x + 1
    y = y + 0.5
    i += 1
  end
  x
end

# 2d. Comparison in loop (tests OP_LT + JMPNOT fusion potential)
measure("comparison_loop", 10) do
  x = 0
  while x < N
    x += 1
  end
  x
end

# 2e. Multiple comparisons (branch prediction)
measure("multi_compare", 10) do
  i = 0
  count = 0
  while i < M
    count += 1 if i > 100
    count += 1 if i < 50000
    count += 1 if i == 25000
    i += 1
  end
  count
end

# 6a. Integer literals (tests LOADI optimization)
measure("int_literals", 10) do
  i = 0
  sum = 0
  while i < M
    sum += 1
    sum += 2
    sum += 3
    sum += 42
    sum += 100
    i += 1
  end
  sum
end

# 6b. Large integer literals (tests LOADL from pool)
measure("large_int_literals", 10) do
  i = 0
  sum = 0
  while i < M
    sum += 1000000
    sum += 2000000
    sum += 3000000
    i += 1
  end
  sum
end

# 6c. Float literals
measure("float_literals", 10) do
  i = 0
  sum = 0.0
  while i < M
    sum += 1.5
    sum += 2.5
    sum += 3.5
    i += 1
  end
  sum
end

# 7a. Predictable branch (always true)
measure("predictable_true", 10) do
  i = 0
  count = 0
  while i < N
    count += 1 if true
    i += 1
  end
  count
end

# 7b. Predictable branch (always false)
measure("predictable_false", 10) do
  i = 0
  count = 0
  while i < N
    count += 1 if false
    i += 1
  end
  count
end

# 7c. Unpredictable branch (50/50)
measure("unpredictable_50", 10) do
  i = 0
  count = 0
  while i < M
    count += 1 if i & 1 == 0
    i += 1
  end
  count
end

# 7d. Rare branch (error path simulation)
measure("rare_branch", 10) do
  i = 0
  count = 0
  while i < N
    count += 1 if i == -1  # Never true
    i += 1
  end
  count
end
