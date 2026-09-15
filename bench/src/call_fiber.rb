# calls: switching contexts, one resume and one yield per round.
f = Fiber.new do
  s = 0
  while true
    s += Fiber.yield(s)
  end
end
i = 0
while i < 6_000_000
  f.resume(1)
  i += 1
end
puts i
