def add(a, b)
  a + b
end
def fib(n)
  n < 2 ? n : fib(n - 1) + fib(n - 2)
end
def opt(a, b = 10, *rest)
  [a, b, rest]
end
p add(1, 2)
p fib(15)
p opt(1)
p opt(1, 2)
p opt(1, 2, 3, 4)
