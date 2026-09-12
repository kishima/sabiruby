# Japanese text through the character-indexed String methods. The reference records this
# twice: once built with MRB_UTF8_STRING (utf8.out) and once as it ships (utf8-bytes.out),
# so the fixture says what each build answers rather than only what one of them does.
s = "日本語テキスト"
p s.length, s.size, s.bytesize
p s[0], s[2, 3], s[-1], s[1..3]
p s.index("語"), s.rindex("ト"), s.byteindex("語")
p s.chars
p s.reverse
p s.split("")
p s.each_char.to_a.length
p s.include?("テキ"), s.start_with?("日本"), s.end_with?("スト")
p s.sub("テキスト", "文字"), s.gsub("ト", "-")
p s.center(11, "＊"), s.ljust(9, "・"), s.rjust(9, "・")
p s.inspect
p s.ord, s.chr
# a byte above ASCII comes back signed from the byte-string build (`(mrb_int)*p` over a
# signed `char`), which is the platform's rather than the reference's answer
p s.codepoints.first(3) if __ENCODING__ == "UTF-8"
p "あ".succ, "ｚ".succ, "ア".upto("エ").to_a
p "%s|%5s|%-5s|%.2s" % [s, "あ", "あ", s]
p "%c" % [12354], "%c" % ["a"]

# case and folding above ASCII
p "Straße".upcase, "ÄÖÜ".downcase, "ﬁ".capitalize, "Äö".swapcase
p "ß".casecmp?("SS"), "ä".casecmp("Ä")

# bytes that spell no character
broken = "あ\x80い"
p broken.length, broken.chars.length, broken.bytesize
p broken.scrub, broken.scrub("?")
p broken.index("\x80"), broken.byteindex("\x80")

# a byte-read copy counts its bytes
b = "あ".b
p b.length, b.bytesize, b.index("\x81"), b.inspect
