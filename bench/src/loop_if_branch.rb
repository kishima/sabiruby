# instruction loop: conditional jumps, taken and not taken.
i = 0
a = 0
b = 0
while i < 7_000_000
  m = i % 3
  if m == 0
    a = a + 1
  elsif m == 1
    b = b + 1
  else
    a = a - 1
  end
  i = i + 1
end
puts a + b
