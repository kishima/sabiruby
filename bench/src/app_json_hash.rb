# whole-program: building and walking the kind of nested Hash/Array a host
# hands a script (a decoded JSON document), then summing it back.
def build(n)
  rows = []
  i = 0
  while i < n
    rows.push({
      "id" => i,
      "name" => "item-#{i}",
      "tags" => ["a", "b", "c"],
      "meta" => { "score" => i * 3, "ok" => (i % 2 == 0) }
    })
    i += 1
  end
  { "count" => n, "rows" => rows }
end

total = 0
i = 0
while i < 2_300
  doc = build(200)
  doc["rows"].each do |row|
    total += row["meta"]["score"] if row["meta"]["ok"]
    total += row["tags"].size
    total += row["name"].size
  end
  i += 1
end
puts total
