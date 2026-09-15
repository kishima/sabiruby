//! The pattern layer of mruby-regexp (`src/regexp`): what the translation makes of Ruby's syntax
//! and what the automaton under it answers. The Ruby surface is checked by the reference's own
//! tests (`tools/mrbtest.sh`, files `gem_regexp*`); this pins the layer below them, the
//! constructs the engine refuses among it (`docs/design/gems.md`, "Deviations kept").
use sabiruby::regexp::{compile, exec};

fn m(pat: &str, s: &str) -> Option<Vec<i32>> {
    let p = compile(pat.as_bytes(), 0, false).unwrap_or_else(|e| panic!("compile {pat}: {e}"));
    exec(&p, s.as_bytes(), 0, true)
}
fn mf(pat: &str, s: &str, flags: u32) -> Option<Vec<i32>> {
    let p = compile(pat.as_bytes(), flags, false).unwrap_or_else(|e| panic!("compile {pat}: {e}"));
    exec(&p, s.as_bytes(), 0, true)
}
fn err(pat: &str) -> String {
    compile(pat.as_bytes(), 0, false).err().unwrap_or_else(|| panic!("{pat} compiled"))
}

#[test]
fn engine() {
    assert_eq!(m("abc", "xxabc"), Some(vec![2, 5]));
    assert_eq!(m("a+b", "aaab"), Some(vec![0, 4]));
    assert_eq!(m("(a)(b)", "zab"), Some(vec![1, 3, 1, 2, 2, 3]));
    assert_eq!(m("a|bc", "zbc"), Some(vec![1, 3]));
    assert_eq!(m("[a-c]+", "zzabcz"), Some(vec![2, 5]));
    assert_eq!(m("^b$", "a\nb\nc"), Some(vec![2, 3])); // `^`/`$` are line anchors in Ruby
    assert_eq!(m("x*", "yyy"), Some(vec![0, 0]));
    assert_eq!(mf("ABC", "xabc", 1), Some(vec![1, 4]));
    assert_eq!(m("\\d+", "ab123"), Some(vec![2, 5]));
    assert_eq!(m("a{2,3}", "aaaa"), Some(vec![0, 3]));
    assert_eq!(m("(?:ab)+", "ababx"), Some(vec![0, 4]));
    assert_eq!(m("a.*?b", "axxbxxb"), Some(vec![0, 4]));
    assert_eq!(m("(?<n>a)(?<m>b)", "ab"), Some(vec![0, 2, 0, 1, 1, 2]));
    assert_eq!(m("\\bfoo\\b", "a foo b"), Some(vec![2, 5]));
    assert_eq!(m("[[:alpha:]]+", "12abc"), Some(vec![2, 5]));
    assert_eq!(m("(a|b)+", "abab"), Some(vec![0, 4, 3, 4]));
}

/// The translation: what Ruby spells one way and `regex-syntax` another.
#[test]
fn translation() {
    // where a string is characters, a multibyte literal is one atom, so a quantifier repeats the
    // whole character and `.` takes the whole of it; where it is bytes, each byte is an atom
    if cfg!(feature = "utf8") {
        assert_eq!(m("あ+", "xあああ"), Some(vec![1, 10]));
        assert_eq!(m(".", "あ"), Some(vec![0, 3]));
    } else {
        assert_eq!(m("あ+", "xあああ"), Some(vec![1, 4]));
        assert_eq!(m(".", "あ"), Some(vec![0, 1]));
    }
    // Ruby's `m` is the automaton's `s`: `.` matching a newline, not the line anchors
    assert_eq!(m("(?m)a.b", "a\nb"), Some(vec![0, 3]));
    // a byte that spells no character is the byte
    assert_eq!(m("\\xC4x", "\u{c4}x"), None);
    assert_eq!(compile(b"\xC4x", 0, false).map(|p| exec(&p, b"\xC4x", 0, true)).unwrap(), Some(vec![0, 2]));
    // `\h`, the octal escape, `\u`, `{,m}` and a brace that opens no repeat
    assert_eq!(m("\\h+", "zz1fz"), Some(vec![2, 4]));
    assert_eq!(m("\\101", "xA"), Some(vec![1, 2]));
    assert_eq!(m("\\u0041", "xA"), Some(vec![1, 2]));
    assert_eq!(m("a{,2}", "aaa"), Some(vec![0, 2]));
    assert_eq!(m("a{x", "a{x"), Some(vec![0, 3]));
    // free spacing keeps the spaces a class holds
    assert_eq!(mf("a b # c\n[ ]", "ab c", 8), Some(vec![0, 3]));
    // a pattern that names a group numbers no other (Onigmo's DONT_CAPTURE_GROUP), and a name
    // may be given twice
    assert_eq!(m("(a)(?<b>b)", "ab"), Some(vec![0, 2, 1, 2]));
    assert_eq!(m("(?<x>a)|(?<x>b)", "b"), Some(vec![0, 1, -1, -1, 0, 1]));
}

/// What an automaton cannot do, refused by name at compile time.
#[test]
fn refusals() {
    assert!(err("(a)\\1").contains("backreference"));
    assert!(err("(?<a>x)\\k<a>").contains("backreference"));
    assert!(err("a(?=b)").contains("lookahead"));
    assert!(err("a(?!b)").contains("lookahead"));
    assert!(err("(?<=a)b").contains("lookbehind"));
    assert!(err("(?>a*)a").contains("atomic"));
    assert!(err("a*+").contains("possessive"));
    assert!(err("(a)\\g<1>").contains("subexpression call"));
    assert!(err("(?~a)").contains("absent"));
    // the reference is built without the property tables: `\p{…}` raises there too
    assert!(err("\\p{L}").contains("character property"));
    assert!(err("[\\p{Hiragana}]").contains("character property"));
    // the limits the surface reports, and the parser's complaints in the reference's wording
    assert!(err("a{32769}").contains("too big number for repeat range"));
    assert_eq!(err(&"(a)".repeat(32)), "too many capture groups are specified");
    assert_eq!(err("[b-a]"), "empty range in char class");
    assert_eq!(err("("), "end pattern with unmatched parenthesis");
    assert_eq!(err(")"), "unmatched close parenthesis");
    assert_eq!(err("[[:bogus:]]"), "invalid POSIX bracket type");
    assert_eq!(err("(?<a"), "invalid group name <a>");
}
