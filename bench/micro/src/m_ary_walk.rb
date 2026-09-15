# Walks that call back into Ruby for every element (`==`, `<=>`, a block). The copy is made
# once and then only read, so what a borrow saves here is the one allocation, not an O(n).
a = Array.new(200) { |i| i }
i = 0
while i < 20_000
  a.index(150)
  a.count { |x| x > 100 }
  a.assoc(3)
  i += 1
end
