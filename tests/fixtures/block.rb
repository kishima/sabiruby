def twice
  yield 1
  yield 2
end
twice { |x| p x * 10 }
sum = 0
3.times { |i| sum += i }
p sum
[1, 2, 3].each { |e| p e }
p [1, 2, 3].map { |e| e * 2 }
def mk
  x = 1
  -> { x += 1 }
end
l = mk
p l.call, l.call
