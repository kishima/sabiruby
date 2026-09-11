a = [3, 1, 2]
a << 4
a.push(5)
p a, a.size, a[0], a[-1], a[1, 2], a.first, a.last
h = {b: 1, "s" => 2}
h[:c] = 3
p h, h[:b], h["s"], h.size, h.keys
s = "abc"
s << "def"
p s, s.size, s + "!", "x" * 3, s[1], s[1, 2], s.upcase, "a,b".split(",")
p 1..3, (1...3).to_a, :sym, :sym.to_s, "str".to_sym, 42.to_s, "42".to_i, 3.7.to_i
