# The set operations of mruby-array-ext, which walk one array and ask `eql?` against another.
a = Array.new(100) { |i| i }
b = Array.new(100) { |i| i + 50 }
i = 0
while i < 2_000
  a - b
  a | b
  a & b
  a.difference(b)
  i += 1
end
