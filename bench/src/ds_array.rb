# data structures: Array#push, Array#[], Array#[]= and Array#each.
s = 0
i = 0
while i < 1_300
  a = []
  j = 0
  while j < 2_000
    a.push(j)
    j += 1
  end
  j = 0
  while j < 2_000
    a[j] = a[j] + 1
    j += 1
  end
  a.each { |x| s += x }
  i += 1
end
puts s
