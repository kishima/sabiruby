# instruction loop: the bare dispatch cost of a while loop and integer addition.
i = 0
s = 0
while i < 20_000_000
  s = s + i
  i = i + 1
end
puts s
