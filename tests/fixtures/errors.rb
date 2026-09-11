def ensure_order
  yield
  :normal
ensure
  puts "ensure ran"
end
p ensure_order { 1 }
def early
  return :early
ensure
  puts "ensure on return"
end
p early
def with_rescue
  yield
rescue TypeError => e
  "rescued #{e.message}"
else
  "no error"
ensure
  puts "always"
end
p with_rescue { 1 }
p with_rescue { raise TypeError, "boom" }
begin
  with_rescue { raise "other" }
rescue => e
  p e.class, e.message
end
class MyError < StandardError
  def initialize(msg = "my default")
    super
  end
end
begin
  raise MyError
rescue MyError => e
  p e.message, e.class.ancestors.include?(StandardError)
end
begin
  raise MyError, "custom"
rescue StandardError => e
  p e.message, e.is_a?(MyError)
end
def retrying
  tries = 0
  begin
    tries += 1
    raise "fail" if tries < 3
    tries
  rescue
    retry if tries < 3
    :gave_up
  end
end
p retrying
x = [1, 2, 3].each do |i|
  begin
    raise ArgumentError if i == 2
  rescue ArgumentError
    next
  ensure
    print i
  end
end
puts
p x
begin
  begin
    raise "inner"
  ensure
    puts "inner ensure"
  end
rescue => e
  p e.message
end
begin
  Integer("abc")
rescue ArgumentError => e
  p e.message
end
begin
  [].fetch(3)
rescue IndexError => e
  p e.class, e.message
end
begin
  {a: 1}.fetch(:b)
rescue KeyError => e
  p e.class, e.message
end
begin
  nil + 1
rescue NoMethodError => e
  p e.message
end
begin
  1 + nil
rescue TypeError => e
  p e.message
end
begin
  "a" + 1
rescue TypeError => e
  p e.message
end
begin
  def two(a, b); end
  two(1)
rescue ArgumentError => e
  p e.message
end
begin
  raise Exception, "base"
rescue StandardError
  p :wrong
rescue Exception => e
  p e.message
end
e = RuntimeError.new("made")
p e.message, e.inspect, e.backtrace
p (raise "x" rescue "inline rescue")
