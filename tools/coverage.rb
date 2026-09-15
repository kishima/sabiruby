# What classes, modules and methods this VM has. Runs on SabiRuby (`sabiruby run
# tools/coverage.rb`) and on the reference `mruby` unchanged: nothing here is outside
# what mruby-metaprog and mruby-objectspace give a script, and there is no file or
# process access (the VM is no_std, so the comparison is driven from tools/coverage.sh).
#
# Output, one item per line, sorted, tab separated:
#   #engine <RUBY_ENGINE> <RUBY_ENGINE_VERSION> <MRUBY_VERSION> <description>
#   #note   <free text: what the walk could not reach>
#   class   <name>  class|module  <superclass or ->  <ancestors, comma separated>
#   method  <name>#<instance method>   public|private
#   method  <name>.<singleton method>  public|private
# Only methods a module defines *itself* are listed (`instance_methods(false)` and
# `private_instance_methods(false)`), so a method appears once, under the module that owns
# it; `ancestors` says who inherits it. Private methods are listed because that is how
# mruby writes a module function (a public singleton method and a private instance method
# of the same name) and how it hides a good part of Module and Kernel; leaving them out
# makes half of the two VMs' difference an artefact. Protected counts as public here.

# This script's own helpers are `def`ed at the top level, which makes them methods of
# Object (private in the reference, public in SabiRuby), so they would show up as a
# difference between the two. Both sides list Object's own methods as they were before
# this line instead.
COV_OBJECT_METHODS = (Object.instance_methods(false) +
                      Object.private_instance_methods(false)).map { |s| s.to_s }

def cov_name(m)
  n = begin
    m.name
  rescue StandardError, NoMethodError
    nil
  end
  n = m.to_s if n.nil?
  n.to_s
end

# A singleton class ("#<Class:Integer>") has no name of its own; its methods are listed
# as `Integer.foo` through the owner below, so the walk never descends into one.
def cov_skip?(name)
  name.nil? || name.empty? || name.include?("#<") || name.include?(" ")
end

def cov_ask(target, message)
  begin
    target.send(message, false).map { |s| s.to_s }
  rescue StandardError, NoMethodError
    []
  end
end

# [name, "public"|"private"] pairs, sorted, for what `m` defines itself.
def cov_methods(m, singleton)
  target = if singleton
    begin
      m.singleton_class
    rescue StandardError, NoMethodError
      nil
    end
  else
    m
  end
  if target.nil?
    pub = begin
      m.singleton_methods(false).map { |s| s.to_s }
    rescue StandardError, NoMethodError
      []
    end
    priv = []
  else
    pub = cov_ask(target, :instance_methods)
    priv = cov_ask(target, :private_instance_methods)
  end
  if !singleton && m.equal?(Object)
    pub = pub & COV_OBJECT_METHODS
    priv = priv & COV_OBJECT_METHODS
  end
  out = []
  pub.each { |n| out.push([n, "public"]) }
  priv.each { |n| out.push([n, "private"]) unless pub.include?(n) }
  out.sort_by { |pair| pair[0] }
end

lines = []
notes = []
seen = {}

def cov_visit(mod, lines, seen)
  name = cov_name(mod)
  return [] if cov_skip?(name)
  return [] if seen[name]
  seen[name] = true

  kind = mod.is_a?(Class) ? "class" : "module"
  sup = begin
    s = mod.is_a?(Class) ? mod.superclass : nil
    s.nil? ? "-" : cov_name(s)
  rescue StandardError, NoMethodError
    "-"
  end
  anc = begin
    mod.ancestors.map { |a| cov_name(a) }.join(",")
  rescue StandardError, NoMethodError
    ""
  end
  lines.push("class\t#{name}\t#{kind}\t#{sup}\t#{anc}")
  cov_methods(mod, false).each { |m| lines.push("method\t#{name}##{m[0]}\t#{m[1]}") }
  cov_methods(mod, true).each { |m| lines.push("method\t#{name}.#{m[0]}\t#{m[1]}") }

  # nested constants: `Module#constants` answers this module's own constants only
  # (`Integer.constants` is empty on both VMs), so this is a plain tree walk.
  children = []
  consts = begin
    mod.constants
  rescue StandardError, NoMethodError
    []
  end
  consts.sort_by { |c| c.to_s }.each do |c|
    v = begin
      mod.const_get(c)
    rescue StandardError, NoMethodError
      nil
    end
    children.push(v) if v.is_a?(Module)
  end
  children
end

# Breadth-first from Object: every named class and module reachable as a constant.
queue = [Object]
until queue.empty?
  m = queue.shift
  queue.concat(cov_visit(m, lines, seen))
end

# Anything the constant walk missed but that is still alive (mruby-objectspace).
# Singleton classes and anonymous modules are counted, not listed: they have no name
# to compare across the two VMs.
begin
  extra = []
  anon = 0
  ObjectSpace.each_object(Module) do |m|
    n = cov_name(m)
    if cov_skip?(n)
      anon += 1
    elsif !seen[n]
      extra.push(m)
    end
  end
  extra.each { |m| queue.concat(cov_visit(m, lines, seen)) }
  until queue.empty?
    m = queue.shift
    queue.concat(cov_visit(m, lines, seen))
  end
  notes.push("ObjectSpace: #{extra.size} named module(s) not reachable as a constant, #{anon} unnamed (singleton or anonymous)")
rescue StandardError, NoMethodError, NameError
  notes.push("ObjectSpace: not available, the list is what the constant walk reached")
end

desc = begin
  MRUBY_DESCRIPTION
rescue StandardError, NameError
  "-"
end
mver = begin
  MRUBY_VERSION
rescue StandardError, NameError
  "-"
end
eng = begin
  RUBY_ENGINE
rescue StandardError, NameError
  "-"
end
ever = begin
  RUBY_ENGINE_VERSION
rescue StandardError, NameError
  "-"
end
puts "#engine\t#{eng}\t#{ever}\t#{mver}\t#{desc}"
notes.each { |n| puts "#note\t#{n}" }
lines.sort.each { |l| puts l }
