# instruction loop: the same counting through Integer#times and a block.
s = 0
9_000_000.times do |i|
  s += i
end
puts s
