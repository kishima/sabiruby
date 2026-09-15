# The expected output (json_roundtrip.expected) is CRuby's: `ruby json_roundtrip.rb`
# with ruby 3.2.6 (json 2.6.3). The same file runs on SabiRuby once
# `sabiruby_serde::install_json` has defined JSON, which is why the require is guarded.
#
# Everything printed here is either JSON text or the inspect of an Array, an Integer, a
# Float or a String: Hash#inspect differs between the two engines (Ruby 3.4 changed it) and
# says nothing about JSON, so no Hash is ever printed directly.
require "json" unless defined?(JSON)

# generate
puts JSON.generate({"a" => [1, 2, 3], "b" => nil, "c" => true})
puts JSON.generate([1, -2, 3.5, "x", nil, false])
puts JSON.generate({})
puts JSON.generate([])
puts JSON.generate({"unicode" => "あい"})
# keys are what to_s says, whatever they were
puts JSON.generate({:sym => 1, 2 => "two"})

# to_json on whatever the receiver is
puts({"n" => 1}.to_json)
puts [1, {"k" => "v"}].to_json
puts "a\"b\\c\nd".to_json
puts 42.to_json
puts nil.to_json
puts :sym.to_json

# parse
# the keys are deliberately not in alphabetical order: a JSON object keeps the order it
# was written in, here and in CRuby
h = JSON.parse('{"g": true, "a": [1, 2], "f": 1.5, "b": {"c": "d"}, "e": null}')
puts h["a"].inspect
puts h["b"]["c"]
puts h["e"].inspect
puts h["f"].inspect
puts h["g"].inspect
puts h.keys.inspect
puts JSON.parse("[1,2,3]").inspect
puts JSON.parse('"str"').inspect
puts JSON.parse("17").inspect
puts JSON.parse("-1.25e2").inspect

# text -> Ruby -> text
src = {"list" => [1, 2, {"deep" => [true, nil]}], "name" => "x"}
puts JSON.generate(JSON.parse(JSON.generate(src)))

# pretty_generate
puts JSON.pretty_generate({"a" => [1, 2], "b" => {"c" => 1}})
puts JSON.pretty_generate([])
puts JSON.pretty_generate({})

# errors
begin
  JSON.parse("{oops}")
  puts "no error"
rescue JSON::ParserError
  puts "ParserError"
end
puts JSON::ParserError.ancestors.include?(StandardError)
