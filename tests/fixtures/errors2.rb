# exceptions crossing native frames and less common forms
begin
  [3, 1, 2].sort { |a, b| raise "in sort" }
rescue => e
  p e.message
end
begin
  [1, 2].each { |x| raise TypeError, "in each #{x}" }
rescue TypeError, ArgumentError => e
  p e.message
end
r = ([1, 2, 3].select { |x| raise "sel" if x == 2; true } rescue :selected)
p r
def nested
  begin
    begin
      return :inner
    ensure
      puts "e1"
    end
  ensure
    puts "e2"
  end
end
p nested
def reraise
  yield
rescue => e
  puts "reraise #{e.message}"
  raise e
end
begin
  reraise { raise IndexError, "orig" }
rescue IndexError => e
  p e.class, e.message
end
begin
  raise
rescue RuntimeError => e
  p e.message
end
x = 0
begin
  x += 1
  raise "again" if x < 3
rescue
  retry
end
p x
def ensure_value
  :body
ensure
  :ignored
end
p ensure_value
begin
  begin
    raise ArgumentError, "a"
  rescue => e
    raise TypeError, "b"
  end
rescue => e
  p e.class, e.message
end
p (begin; Integer("1"); rescue; :no; else; :else; ensure; puts "ens"; end)
e = ArgumentError.new
p e.message, ArgumentError.new("m").message, StandardError.new(:sym).message
begin
  raise "top level"
rescue Exception => e
  p e.is_a?(StandardError), e.is_a?(Exception)
end
begin
  [1].fetch(5)
rescue IndexError => e
  p :idx
end
def deep(n) = n == 0 ? raise("bottom") : deep(n - 1)
begin
  deep(50)
rescue => e
  p e.message
end
