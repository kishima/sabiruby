s = "Hello, World"
p s.length, s.upcase, s.downcase, s.reverse, s.index("o"), s.include?("World")
p s[0], s[-1], s[0, 5], s[7..], s[0...5], s * 2
p "a-b-c".split("-"), "  x  ".strip, "abc\n".chomp, "abc".chars, "abc".bytes
p "abc" <=> "abd", "abc" == "abc", "abc".eql?("abc"), "abc".equal?("abc")
p "x=#{1 + 1}", 'single #{no}', "tab\tnl\n".inspect
p "abc".start_with?("ab"), "abc".end_with?("bc"), "abc".sub("b", "B"), "aaa".gsub("a", "b")
p "12abc".to_i, "3.5x".to_f, "0x1f".to_i(16), "ff".to_i(16), 255.to_s(2), 255.to_s(16)
p "abc".succ, "az".succ, "zz".succ, "a9".succ
p "abc".to_sym, "with space".to_sym, :"a-b"
t = "mutable"
t << " string"
t[0] = "M"
p t, t.frozen?, "lit".frozen?
puts "multi", ["a", ["b"]], nil, 3.5
print "no newline", "\n"
p 1.0, 1.5e20, 1e-5, 100.0 / 3, 2 ** 62, 7.fdiv(2) rescue p :no_fdiv
p 10.divmod(3), -7.divmod(2), 7.5.floor, 7.5.round, 7.5.ceil, -7 % 3, 7 % -3
