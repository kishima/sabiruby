def kw(a:, b: 2)
  [a, b]
end
p kw(a: 1), kw(b: 3, a: 2)
def mixed(x, y = 1, *r, k: 0, **opts, &blk)
  [x, y, r, k, opts, blk.nil?]
end
p mixed(1), mixed(1, 2, 3, k: 4, z: 5), mixed(1, **{k: 9})
h = {a: 1}
p kw(**h)
p [1, 2, 3].map(&:to_s)
begin
  kw(b: 1)
rescue ArgumentError => e
  p e.message
end
begin
  kw(a: 1, c: 2)
rescue ArgumentError => e
  p e.message
end
def opt_hash(h = {})
  h
end
p opt_hash(a: 1), opt_hash({b: 2}), opt_hash
