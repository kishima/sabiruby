# From mruby's benchmark/vm_optimization_bench.rb: the sections that measure the
# instruction loop itself -- dispatch, loop shapes and register pressure. Split out of
# that one file so that a category means something (`bench/categories.tsv`); the original
# is kept whole as a whole-program benchmark. The `measure` helper and the bodies are the
# upstream ones, unchanged.
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

# 1a. Empty loop (pure dispatch cost)
measure("empty_loop", 10) do
  i = 0
  while i < N
    i += 1
  end
end

# 1b. NOP-heavy (many instructions, minimal work)
measure("nop_sequence", 10) do
  i = 0
  while i < M
    a = 1; b = 2; c = 3; d = 4; e = 5
    a = 1; b = 2; c = 3; d = 4; e = 5
    a = 1; b = 2; c = 3; d = 4; e = 5
    a = 1; b = 2; c = 3; d = 4; e = 5
    i += 1
  end
end

# 5a. Simple while loop
measure("while_loop", 10) do
  i = 0
  while i < N
    i += 1
  end
end

# 5d. Nested loops
measure("nested_loop", 10) do
  sum = 0
  i = 0
  while i < 1000
    j = 0
    while j < 1000
      sum += 1
      j += 1
    end
    i += 1
  end
  sum
end

# 8a. Few local variables (should fit in registers)
measure("few_locals", 10) do
  i = 0
  a = 0
  while i < N
    a += 1
    i += 1
  end
  a
end

# 8b. Many local variables (register spilling)
measure("many_locals", 10) do
  i = 0
  a = 0; b = 0; c = 0; d = 0; e = 0
  f = 0; g = 0; h = 0; j = 0; k = 0
  l = 0; m = 0; n = 0; o = 0; p = 0
  while i < M
    a += 1; b += 1; c += 1; d += 1; e += 1
    f += 1; g += 1; h += 1; j += 1; k += 1
    l += 1; m += 1; n += 1; o += 1; p += 1
    i += 1
  end
  a + b + c + d + e + f + g + h + j + k + l + m + n + o + p
end
