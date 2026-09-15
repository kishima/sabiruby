# Reading ONE element out of a long array: `fetch`, `at`, `[]` with a default, `values_at`.
# The natives behind these copied the whole array to index into the copy.
a = Array.new(1000) { |i| i }
n = 0
i = 0
while i < 200_000
  n += a.fetch(500)
  n += a.at(500)
  n += a.fetch(5000, 0)
  i += 1
end
