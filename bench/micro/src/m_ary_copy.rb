# Whole-array answers that went through two copies (one to read, one to store):
# `dup`, `compact`, `reverse`, `rotate`, `+`.
a = Array.new(200) { |i| i }
b = [1, 2, 3]
i = 0
while i < 50_000
  a.dup
  a.compact
  a.reverse
  a.rotate
  a + b
  i += 1
end
