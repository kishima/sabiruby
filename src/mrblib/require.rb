# require / load. SabiRuby's own Ruby part (there is none in the reference: mruby has no
# `require`). The shape is picoruby-require's `mrblib/require.rb` (MIT), with the differences
# `docs/plans/eval-require-plan.md` 5 lists: no `extern` (the gems are all there from the start, so
# their names are in `$LOADED_FEATURES` already), no `File` (the VM has no POSIX gems, so a path
# is built as a string and handed to the two natives `__file_exist?` and `__load_file`), and no
# Sandbox (`__exec_file` runs the file in a top-level frame of its own).

class LoadError < StandardError; end

module Kernel
  # Runs `name` once: true the first time, false where it was loaded before. The feature is
  # recorded before the file runs, so a cycle stops instead of repeating (CRuby does the same;
  # picoruby-require records it afterwards).
  def require(name)
    name = __require_str(name)
    return false if $LOADED_FEATURES.include?(name)
    paths = __require_load_paths(name)
    exts = __require_exts(name)
    i = 0
    while i < paths.size
      j = 0
      while j < exts.size
        path = __require_join(paths[i], name + exts[j])
        if __file_exist?(path)
          return false if $LOADED_FEATURES.include?(path)
          return __require_run(path)
        end
        j += 1
      end
      i += 1
    end
    raise LoadError, "cannot load such file -- #{name}"
  end

  # Runs `path` as written, every time it is called, and answers true.
  def load(path)
    path = __require_str(path)
    paths = __require_load_paths(path)
    i = 0
    while i < paths.size
      full = __require_join(paths[i], path)
      if __file_exist?(full)
        __exec_file(__load_file(full), full)
        return true
      end
      i += 1
    end
    raise LoadError, "cannot load such file -- #{path}"
  end

  private

  def __require_str(name)
    return name if name.is_a?(String)
    if name.respond_to?(:to_str)
      converted = name.to_str
      return converted if converted.is_a?(String)
    end
    raise TypeError, "no implicit conversion of #{name.class} into String"
  end

  # A name that says where it lives is not looked for anywhere else.
  def __require_load_paths(name)
    if name.start_with?("/") || name.start_with?("./") || name.start_with?("../")
      [""]
    else
      $LOAD_PATH || []
    end
  end

  # A name that already carries one of the extensions is looked for as written, the way CRuby
  # reads `require "./x.rb"`; a bare name is tried with each in turn.
  def __require_exts(name)
    if name.end_with?(".rb") || name.end_with?(".mrb")
      [""]
    else
      [".mrb", ".rb"]
    end
  end

  def __require_join(dir, name)
    dir.empty? ? name : "#{dir}/#{name}"
  end

  def __require_run(path)
    $LOADED_FEATURES << path
    begin
      __exec_file(__load_file(path), path)
    rescue Exception => e
      # the name is taken back out, so a later require tries the file again (CRuby does too).
      # The exception is named rather than re-raised bare: `$!` is not carried here
      $LOADED_FEATURES.delete(path)
      raise e
    end
    true
  end
end

$LOAD_PATH = []
# The gems are built in rather than loaded, so `require 'fiber'` answers false the way it does
# on a build that has the gem linked (`docs/design/gems.md` lists them in this order).
$LOADED_FEATURES = [
  "fiber", "enumerator", "array-ext", "enum-ext", "hash-ext", "range-ext", "string-ext",
  "sprintf", "metaprog", "proc-ext", "method", "compar-ext", "toplevel-ext", "enum-chain",
  "enum-lazy", "object-ext", "symbol-ext", "kernel-ext", "class-ext", "numeric-ext", "catch",
  "objectspace", "math", "random", "struct", "data", "set", "time", "bigint", "rational",
  "complex", "cmath", "pack", "eval", "binding", "proc-binding", "regexp", "require",
]
