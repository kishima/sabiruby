# A few elements off one end of a long array: `first(n)`, `last(n)`, `take`, `drop`.
# The answer is three elements; the natives copied all thousand first.
a = Array.new(1000) { |i| i }
i = 0
while i < 100_000
  a.first(3)
  a.last(3)
  a.take(3)
  a.drop(997)
  i += 1
end
