# The three natives that copied the elements twice (Slot -> Value -> Slot):
# `dup`, `initialize_copy` (what `clone` and `Array.new(other)` reach) and `replace`.
a = Array.new(200) { |i| i }
b = []
i = 0
while i < 50_000
  a.dup
  a.clone
  b.replace(a)
  i += 1
end
