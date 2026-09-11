def risky(n)
  raise ArgumentError, "bad #{n}" if n < 0
  n * 2
end
begin
  p risky(1)
  p risky(-1)
rescue ArgumentError => e
  p e.class, e.message
ensure
  p :ensure
end
begin
  1 / 0
rescue ZeroDivisionError => e
  p e.message
end
begin
  nil.foo
rescue NoMethodError => e
  p e.class
end
x = begin
  Integer("zz")
rescue
  :fallback
end
p x
