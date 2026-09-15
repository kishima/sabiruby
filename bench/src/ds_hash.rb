# data structures: Hash#[]=, Hash#[], Hash#key? and Hash#each.
s = 0
i = 0
while i < 500
  h = {}
  j = 0
  while j < 1_000
    h[j] = j * 2
    j += 1
  end
  j = 0
  while j < 1_000
    s += h[j] if h.key?(j)
    j += 1
  end
  h.each { |k, v| s += v - k }
  i += 1
end
puts s
