//! The pattern engine behind mruby-regexp: Rust's `regex-automata` rather than a port of the
//! reference's NFA (`docs/gems-plan.md` 3.6, author's decision of 2026-09-13). What a pattern
//! *means* is therefore Rust's finite automaton: no backreference, no lookaround, no atomic
//! group, no subexpression call, and those are refused at compile time with the construct named
//! in the message. `docs/gems.md` lists what that costs against the reference.
//!
//! This module is the pattern side alone. The Ruby classes built on it — `Regexp`, `MatchData`
//! and the String methods — are ported from the reference in `crate::builtins::ext_regexp`.

use alloc::{format, string::String, vec::Vec};

use regex_automata::{meta, util::syntax, Input};

/// Regexp flags, as the reference numbers them internally (`RE_FLAG_*`).
pub const FLAG_IGNORECASE: u32 = 1;
pub const FLAG_MULTILINE: u32 = 2;
pub const FLAG_DOTALL: u32 = 4;
pub const FLAG_EXTENDED: u32 = 8;

/// The limits the reference's engine carries, kept as constants because the tests read them
/// (`Regexp::STEP_LIMIT` and friends). This engine has no such limits: it is an automaton, so a
/// search is linear in the subject whatever the pattern.
pub const STEP_LIMIT: u32 = 1_000_000;
pub const STACK_LIMIT: u32 = 2048;
pub const PARSE_DEPTH_LIMIT: u32 = 4096;

/// Maximum captures the surface reports (`RE_MAX_CAPTURES`).
pub const MAX_CAPTURES: usize = 32;

/// The largest repeat count a pattern may write (`RE_MAX_REPEAT`).
pub const MAX_REPEAT: u32 = 32768;

/// A named capture of a compiled pattern (`re_named_capture`).
#[derive(Clone)]
pub struct NamedCapture {
    pub name: Vec<u8>,
    pub group: u16,
}

/// A compiled pattern (`mrb_regexp_pattern`).
pub struct Pattern {
    re: meta::Regex,
    /// number of capture groups, including group 0
    pub num_captures: u16,
    pub named_captures: Vec<NamedCapture>,
    pub flags: u32,
    /// the pattern string is byte-read, which is what the automaton was built for
    pub binary: bool,
}

/// What a compile refused, as the message `RegexpError` carries.
pub type CompileError = String;

/// Bytes of the character at `i`, or one byte where the string is byte-read (`mrb_re_charlen`).
pub fn charlen(b: &[u8], i: usize, binary: bool) -> usize {
    crate::builtins::string::utf8len(b, i, !binary && cfg!(feature = "utf8"))
}

/// Compile a pattern string (`mrb_re_compile`). `binary` says the pattern's bytes spell no
/// characters, which is what the reference asks of the string it is handed.
pub fn compile(pattern: &[u8], flags: u32, binary: bool) -> Result<Pattern, CompileError> {
    let unicode = !binary && cfg!(feature = "utf8");
    let (src, named) = translate(pattern, unicode, flags & FLAG_EXTENDED != 0)?;
    let syn = syntax::Config::new()
        .case_insensitive(flags & FLAG_IGNORECASE != 0)
        // `^` and `$` are line anchors in Ruby whatever the flags; `/m` is `.` matching a newline
        .multi_line(true)
        .dot_matches_new_line(flags & FLAG_DOTALL != 0)
        // `/x` is applied by the translation, where a class still holds the spaces written in
        // it; `regex-syntax`'s own free-spacing mode drops those too
        .unicode(unicode)
        // a pattern may hold a byte that spells no character, which only a build that lets a
        // match land on one can compile; where an empty match may land is `utf8_empty` below
        .utf8(false);
    let cfg = meta::Config::new()
        // an empty match may stand between the bytes of a character only where the subject is
        // read as bytes
        .utf8_empty(unicode)
        // the reference's engine has no automaton cache to bound; this one is asked to keep its
        // own small, since a VM may hold many patterns
        .nfa_size_limit(Some(10 * (1 << 20)));
    let re = meta::Builder::new()
        .configure(cfg)
        .syntax(syn)
        .build(&src)
        .map_err(|e| describe_build_error(&e))?;
    let num_captures = re.captures_len() as u16;
    // group 0 takes the last of the slots the surface reports (`RE_MAX_CAPTURES`)
    if num_captures as usize > MAX_CAPTURES {
        return Err(String::from("too many capture groups are specified"));
    }
    Ok(Pattern { re, num_captures, named_captures: named, flags, binary })
}

/// The message a refused build carries. `regex-automata`'s own text names the construct, so it
/// is quoted as it stands, the way the reference quotes its parser's complaint.
fn describe_build_error(e: &meta::BuildError) -> String {
    // the parser's own text is a caret diagram whose last line names what was wrong, which is
    // what a one-line message wants
    let msg = match e.syntax_error() {
        Some(se) => format!("{se}"),
        None => format!("{e}"),
    };
    let last = msg.split('\n').next_back().map(|l| l.trim()).unwrap_or("");
    let text = match last.strip_prefix("error: ") {
        Some(t) => String::from(t),
        None if !last.is_empty() && msg.contains('\n') => String::from(last),
        _ => msg.replace('\n', " "),
    };
    // the complaints the reference words differently, which its tests read back
    if text.contains("character class range") {
        return String::from("empty range in char class");
    }
    if text.contains("unclosed group") {
        return String::from("end pattern with unmatched parenthesis");
    }
    if text.contains("unopened group") {
        return String::from("unmatched close parenthesis");
    }
    text
}

/// The capture slots a match fills: `[start0, end0, start1, end1, …]` in bytes, `-1` for a group
/// that took no part (`mrb_re_exec`). `None` where the subject holds no match at or after
/// `start`.
pub fn exec(pat: &Pattern, hay: &[u8], start: usize, want_captures: bool) -> Option<Vec<i32>> {
    if start > hay.len() { return None; }
    let input = Input::new(hay).span(start..hay.len());
    if !want_captures {
        // `match?` asks for nothing but the answer, so no capture slots are filled
        return pat.re.search_half(&input).map(|_| alloc::vec![0, 0]);
    }
    let mut caps = pat.re.create_captures();
    pat.re.search_captures(&input, &mut caps);
    if !caps.is_match() { return None; }
    let mut out = alloc::vec![-1i32; pat.num_captures as usize * 2];
    for g in 0..pat.num_captures as usize {
        if let Some(sp) = caps.get_group(g) {
            out[g * 2] = sp.start as i32;
            out[g * 2 + 1] = sp.end as i32;
        }
    }
    Some(out)
}

/// The last match that starts at or before `limit`, which is what `rindex`, `byterindex` and
/// `rpartition` ask for (`mrb_re_rexec`).
pub fn rexec(pat: &Pattern, hay: &[u8], limit: usize, want_captures: bool) -> Option<Vec<i32>> {
    let limit = limit.min(hay.len());
    let mut best: Option<Vec<i32>> = None;
    let mut pos = 0usize;
    while pos <= limit {
        let Some(caps) = exec(pat, hay, pos, want_captures) else { break };
        let beg = caps[0] as usize;
        if beg > limit { break; }
        // each step resumes one byte past the match start, which is what keeps overlapping
        // matches in view (`"aaa"` against `/aa/` answers 1)
        pos = beg + 1;
        best = Some(caps);
    }
    best
}

// ------------------------------------------------------------------ the translation layer

/// What this engine cannot do at all, as the message the compile is refused with. The construct
/// is named, since a pattern that uses one has to be written another way rather than turned on.
fn unsupported<T>(what: &str) -> Result<T, String> {
    Err(format!("{what} is not supported by this engine"))
}

/// Ruby's pattern syntax as `regex-syntax`'s. Three kinds of work: what the two spell differently
/// is rewritten (`\h`, `\uXXXX`, octal escapes, `(?'name'…)`, the ASCII shorthands), what only
/// Ruby has is refused by name, and the rest is passed through, `regex-syntax` reading it the
/// same way (`[[:alpha:]]`, `[a-z&&[^aeiou]]`, `(?<name>…)`, `{n,m}`, `*?`).
struct Tr {
    /// the build reads a string as characters and the pattern is not byte-read
    unicode: bool,
    /// `/x`: whitespace separates tokens and `#` opens a comment
    extended: bool,
    /// how many groups the pattern opens, which is what a digit escape is read against
    ngroups: u32,
    /// the pattern names a group, so a plain one captures nothing and a numbered reference is
    /// refused by number
    named: bool,
}

fn translate(src: &[u8], unicode: bool, extended: bool) -> Result<(String, Vec<NamedCapture>), String> {
    // A pattern that names a group numbers no other: Ruby stops a plain `(...)` from capturing
    // there (Onigmo's DONT_CAPTURE_GROUP), so the group numbers `md[1]` and `captures` report
    // are the named ones. The automaton captures every group, so the plain ones are written as
    // non-capturing instead.
    let named = has_named_group(src, extended);
    // a digit escape is a backreference only where the pattern has that many groups, which the
    // reference settles by counting them before the parse (`re_count_groups`)
    let tr = Tr { unicode, extended, ngroups: count_groups(src, extended), named };
    let mut out = String::new();
    // the names are kept here rather than written into the pattern: Ruby lets two groups carry
    // the same name and names one what `regex-syntax` would refuse, and nothing downstream needs
    // the automaton to know a group's name
    let mut names: Vec<NamedCapture> = Vec::new();
    let mut group = 0u16;
    let mut i = 0usize;
    // whether the token just written can carry a quantifier, which is what tells `a++` (a
    // possessive repeat) from `(a)+` followed by a literal `+`
    let mut after_quantifier = false;
    while i < src.len() {
        let c = src[i];
        // free spacing: outside a class, whitespace separates tokens and `#` opens a comment
        if tr.extended {
            if c.is_ascii_whitespace() { i += 1; continue; }
            if c == b'#' {
                while i < src.len() && src[i] != b'\n' { i += 1; }
                continue;
            }
        }
        let mut quantifier = false;
        match c {
            b'\\' => {
                let mut buf = String::new();
                let mut raw = false;
                i = translate_escape(src, i, false, &tr, &mut buf, &mut raw)?;
                push_wrapped(&mut out, &buf, raw && tr.unicode);
            }
            b'[' => { i = translate_class(src, i, &tr, &mut out)?; }
            b'(' => {
                i = translate_group(src, i, tr.named, &mut group, &mut names, &mut out)?;
            }
            b'*' | b'+' | b'?' => {
                if after_quantifier && c == b'+' {
                    return unsupported("possessive quantifier");
                }
                out.push(c as char);
                i += 1;
                // `*?` and friends are the non-greedy marker, not a second quantifier
                if i < src.len() && src[i] == b'?' { out.push('?'); i += 1; }
                quantifier = true;
            }
            b'{' => {
                match read_interval(src, i)? {
                    Some((text, next)) => {
                        if after_quantifier { return unsupported("possessive quantifier"); }
                        out.push_str(&text);
                        i = next;
                        if i < src.len() && src[i] == b'?' { out.push('?'); i += 1; }
                        quantifier = true;
                    }
                    // `{` that opens no repeat is a literal brace in Ruby, where the automaton
                    // would refuse it
                    None => { out.push_str("\\{"); i += 1; }
                }
            }
            _ if c >= 0x80 => {
                let mut buf = String::new();
                let mut raw = false;
                i = push_high(src, i, tr.unicode, &mut buf, &mut raw);
                push_wrapped(&mut out, &buf, raw && tr.unicode);
            }
            _ => {
                // every other byte stands for itself
                out.push(c as char);
                i += 1;
            }
        }
        after_quantifier = quantifier;
    }
    Ok((out, names))
}

/// A pattern byte above ASCII, written so the automaton reads it the way the build reads a
/// string. Where a string is characters a whole UTF-8 sequence passes through as itself; a byte
/// that starts none stands for the byte, which is what the reference's engine compares and what
/// `raw` reports, since a span holding one has to leave Unicode behind.
fn push_high(src: &[u8], i: usize, unicode: bool, out: &mut String, raw: &mut bool) -> usize {
    if unicode {
        let n = crate::builtins::string::utf8len(src, i, true);
        if n > 1 {
            if let Ok(s) = core::str::from_utf8(&src[i..i + n]) {
                out.push_str(s);
                return i + n;
            }
        }
    }
    *raw = true;
    out.push_str(&format!("\\x{:02X}", src[i]));
    i + 1
}

/// Writes a span of the pattern, turning Unicode off over it where it holds a byte that spells
/// no character: such a byte is the byte itself, which only a span outside Unicode can compare.
fn push_wrapped(out: &mut String, text: &str, wrap: bool) {
    if wrap {
        out.push_str("(?-u:");
        out.push_str(text);
        out.push(')');
    } else {
        out.push_str(text);
    }
}

/// The value of a run of hex digits.
fn hex_value(b: &[u8]) -> u32 {
    let mut v = 0u32;
    for c in b {
        v = v * 16 + (*c as char).to_digit(16).unwrap_or(0);
    }
    v
}

/// Whether the pattern declares a named group anywhere (`re_scan_named_groups`). The scan reads
/// the same constructs the parser does, so a `(?<` inside a class, behind a backslash, in a
/// comment or in free-spacing text opens no group here either.
fn has_named_group(src: &[u8], extended: bool) -> bool {
    let mut i = 0usize;
    while i < src.len() {
        match src[i] {
            b'\\' => { i += 2; }
            b'#' if extended => { while i < src.len() && src[i] != b'\n' { i += 1; } }
            b'[' => { i = skip_class(src, i); }
            b'(' if src.get(i + 1) == Some(&b'?') => match src.get(i + 2) {
                // a comment group is text the pattern does not hold
                Some(b'#') => {
                    let mut j = i + 3;
                    while j < src.len() && src[j] != b')' {
                        if src[j] == b'\\' { j += 1; }
                        j += 1;
                    }
                    i = j + 1;
                }
                Some(b'\'') => return true,
                Some(b'<') if !matches!(src.get(i + 3), Some(b'=') | Some(b'!')) => return true,
                _ => i += 1,
            },
            _ => i += 1,
        }
    }
    false
}

/// How many groups the pattern opens (`re_count_groups`), which is what a digit escape is read
/// against: the demotion a named pattern applies to plain groups comes after the parse, so they
/// count here too.
fn count_groups(src: &[u8], extended: bool) -> u32 {
    let mut n = 0u32;
    let mut i = 0usize;
    while i < src.len() {
        match src[i] {
            b'\\' => { i += 2; }
            b'#' if extended => { while i < src.len() && src[i] != b'\n' { i += 1; } }
            b'[' => { i = skip_class(src, i); }
            b'(' if src.get(i + 1) != Some(&b'?') => { n += 1; i += 1; }
            b'(' => match src.get(i + 2) {
                Some(b'#') => {
                    let mut j = i + 3;
                    while j < src.len() && src[j] != b')' {
                        if src[j] == b'\\' { j += 1; }
                        j += 1;
                    }
                    i = j + 1;
                }
                Some(b'\'') => { n += 1; i += 3; }
                Some(b'<') if !matches!(src.get(i + 3), Some(b'=') | Some(b'!')) => { n += 1; i += 3; }
                _ => i += 1,
            },
            _ => i += 1,
        }
    }
    n
}

/// Past one `[...]`, from the opening bracket (`skip_char_class`): a `]` written first is a
/// member, a POSIX bracket and a nested class are stepped over as units.
fn skip_class(src: &[u8], i: usize) -> usize {
    let mut j = i + 1;
    if src.get(j) == Some(&b'^') { j += 1; }
    if src.get(j) == Some(&b']') { j += 1; }
    while j < src.len() {
        match src[j] {
            b']' => return j + 1,
            b'\\' => j += 2,
            b'[' if src.get(j + 1) == Some(&b':') => {
                let mut k = j + 2;
                while k < src.len() && src[k] != b']' { k += 1; }
                j = k + 1;
            }
            b'[' => j = skip_class(src, j),
            _ => j += 1,
        }
    }
    j
}

/// One `(`, with the prefix that says which construct it opens.
fn translate_group(
    src: &[u8],
    i: usize,
    named: bool,
    group: &mut u16,
    names: &mut Vec<NamedCapture>,
    out: &mut String,
) -> Result<usize, String> {
    if i + 1 >= src.len() || src[i + 1] != b'?' {
        // a plain group, which captures only where the pattern names none
        if named {
            out.push_str("(?:");
        } else {
            *group += 1;
            out.push('(');
        }
        return Ok(i + 1);
    }
    let n2 = src.get(i + 2).copied();
    match n2 {
        Some(b':') => { out.push_str("(?:"); Ok(i + 3) }
        Some(b'#') => {
            // a comment group is text the pattern does not hold
            let mut j = i + 3;
            while j < src.len() && src[j] != b')' {
                if src[j] == b'\\' && j + 1 < src.len() { j += 1; }
                j += 1;
            }
            if j >= src.len() { return Err(String::from("end pattern in group")); }
            Ok(j + 1)
        }
        Some(b'=') | Some(b'!') => {
            // `(?!)` is how a pattern that never matches is written, and a position can no more
            // be a word boundary and not one, which is what stands in for it
            if n2 == Some(b'!') && src.get(i + 3) == Some(&b')') {
                out.push_str("\\b\\B");
                return Ok(i + 4);
            }
            ensure_closed(src, i)?;
            unsupported("lookahead")
        }
        Some(b'<') if matches!(src.get(i + 3), Some(b'=') | Some(b'!')) => {
            ensure_closed(src, i)?;
            unsupported("lookbehind")
        }
        Some(b'>') => { ensure_closed(src, i)?; unsupported("atomic group") }
        Some(b'~') => { ensure_closed(src, i)?; unsupported("absent operator") }
        // the reference refuses a conditional too, and this is the wording it uses
        Some(b'(') => Err(String::from("invalid conditional pattern")),
        Some(b'\'') | Some(b'<') => {
            // `(?'name'…)` is `(?<name>…)` written the other way; both open a group the
            // automaton captures without being told the name
            let close = if n2 == Some(b'\'') { b'\'' } else { b'>' };
            // `(?<` with nothing after it is a prefix the pattern ended inside, which is the
            // unmatched parenthesis rather than a name
            if n2 == Some(b'<') && i + 3 >= src.len() {
                return Err(String::from("end pattern with unmatched parenthesis"));
            }
            let j = read_group_name(src, i + 3, close)?;
            *group += 1;
            names.push(NamedCapture { name: src[i + 3..j].to_vec(), group: *group });
            out.push('(');
            Ok(j + 1)
        }
        // the inline options; Ruby's `m` is the automaton's `s`, the two spelling `.` matching a
        // newline differently (the automaton's `m` is `^`/`$` as line anchors, which Ruby has on
        // whatever the flags)
        Some(b'i') | Some(b'm') | Some(b'x') | Some(b'-') => {
            let letter = |c: u8| if c == b'm' { 's' } else { c as char };
            let mut j = i + 2;
            let (mut on, mut off) = (String::new(), String::new());
            while j < src.len() && matches!(src[j], b'i' | b'm' | b'x') { on.push(letter(src[j])); j += 1; }
            if src.get(j) == Some(&b'-') {
                j += 1;
                while j < src.len() && matches!(src[j], b'i' | b'm' | b'x') { off.push(letter(src[j])); j += 1; }
            }
            let scoped = match src.get(j) {
                Some(b')') => false,
                Some(b':') => true,
                None => return Err(String::from("end pattern in group")),
                _ => return Err(String::from("undefined group option")),
            };
            // a group that names no letter turns nothing on or off, where the automaton would
            // refuse the empty toggle
            if scoped || !on.is_empty() || !off.is_empty() {
                out.push_str("(?");
                out.push_str(&on);
                if !off.is_empty() { out.push('-'); out.push_str(&off); }
                out.push(if scoped { ':' } else { ')' });
            }
            Ok(j + 1)
        }
        None => Err(String::from("end pattern in group")),
        Some(_) => Err(String::from("undefined group option")),
    }
}

/// A group the pattern ends inside lacks its parenthesis, which is what it is refused for
/// before the construct it opens is named.
fn ensure_closed(src: &[u8], i: usize) -> Result<(), String> {
    let mut depth = 0usize;
    let mut j = i;
    while j < src.len() {
        match src[j] {
            b'\\' => j += 2,
            b'[' => j = skip_class(src, j),
            b'(' => { depth += 1; j += 1; }
            b')' => {
                depth -= 1;
                if depth == 0 { return Ok(()); }
                j += 1;
            }
            _ => j += 1,
        }
    }
    Err(String::from("end pattern with unmatched parenthesis"))
}

/// One codepoint a `\u` escape names. Where the build reads a string as characters that is the
/// codepoint; where it reads bytes, a pattern holds the UTF-8 spelling of it, which is what the
/// same character written out holds there. `in_class` says the spelling stands among members.
fn push_codepoint(v: u32, in_class: bool, tr: &Tr, out: &mut String, raw: &mut bool) {
    if !tr.unicode {
        if let Some(c) = char::from_u32(v) {
            let mut buf = [0u8; 4];
            let bytes = c.encode_utf8(&mut buf).as_bytes();
            *raw = true;
            // the codepoint is one atom, so a quantifier after it repeats the whole spelling;
            // inside a class each byte is a member of its own
            let group = bytes.len() > 1 && !in_class;
            if group { out.push_str("(?:"); }
            for b in bytes {
                out.push_str(&format!("\\x{b:02X}"));
            }
            if group { out.push(')'); }
            return;
        }
    }
    out.push_str(&format!("\\x{{{v:X}}}"));
}

/// Whether the digit escape at `at` (the first digit) reaches a group rather than naming a byte.
fn digit_escape_is_backref(src: &[u8], at: usize, tr: &Tr) -> bool {
    if src.get(at) == Some(&b'0') { return false; }
    let mut v: u32 = 0;
    let mut j = at;
    while j < src.len() && src[j].is_ascii_digit() && v < 1_000_000 {
        v = v * 10 + (src[j] - b'0') as u32;
        j += 1;
    }
    v <= 9 || v <= tr.ngroups
}

/// A run of byte escapes, decoded the way the pattern's own bytes are: bytes that spell a
/// character are that character, so `\303\244` and `\xC4\x80` are each one atom.
fn push_escape_bytes(src: &[u8], i: usize, octal: bool, tr: &Tr, out: &mut String, raw: &mut bool) -> Result<usize, String> {
    let mut raws = Vec::new();
    let mut j = i;
    while src.get(j) == Some(&b'\\') {
        let Some(&n) = src.get(j + 1) else { break };
        if octal {
            if !(b'0'..=b'7').contains(&n) { break; }
            if !raws.is_empty() && digit_escape_is_backref(src, j + 1, tr) { break; }
            let (v, k) = read_octal(src, j + 1)?;
            raws.push(v as u8);
            j = k;
        } else {
            if n != b'x' || !src.get(j + 2).map(|c| c.is_ascii_hexdigit()).unwrap_or(false) { break; }
            let start = j + 2;
            let mut k = start;
            while k < src.len() && k < start + 2 && src[k].is_ascii_hexdigit() { k += 1; }
            raws.push(hex_value(&src[start..k]) as u8);
            j = k;
        }
    }
    if raws.is_empty() {
        return Err(String::from(if octal { "invalid escape code" } else { "invalid hex escape" }));
    }
    let mut k = 0usize;
    while k < raws.len() {
        if raws[k] < 0x80 {
            out.push_str(&format!("\\x{:02X}", raws[k]));
            k += 1;
        } else {
            k = push_high(&raws, k, tr.unicode, out, raw);
        }
    }
    Ok(j)
}

/// Up to three octal digits from `at`, as the byte they spell: three of them can write more
/// than a byte, which the reference refuses rather than folding (`read_octal_escape`).
fn read_octal(src: &[u8], at: usize) -> Result<(u32, usize), String> {
    let mut v = 0u32;
    let mut j = at;
    while j < src.len() && j < at + 3 && (b'0'..=b'7').contains(&src[j]) {
        v = v * 8 + (src[j] - b'0') as u32;
        j += 1;
    }
    if v > 0xFF { return Err(String::from("invalid escape code")); }
    Ok((v, j))
}

/// The end of a group name starting at `at` and closed by `close`, or what the reference says of
/// one the pattern ends inside (`read_group_name`).
fn read_group_name(src: &[u8], at: usize, close: u8) -> Result<usize, String> {
    let mut j = at;
    while j < src.len() && src[j] != close { j += 1; }
    if j == at { return Err(String::from("group name is empty")); }
    if j >= src.len() {
        let name = String::from_utf8_lossy(&src[at..]);
        return Err(format!("invalid group name <{name}>"));
    }
    Ok(j)
}

/// `{n}`, `{n,}`, `{n,m}` and Ruby's `{,m}`, or `None` where the brace opens no repeat.
fn read_interval(src: &[u8], i: usize) -> Result<Option<(String, usize)>, String> {
    let mut j = i + 1;
    let start = j;
    while j < src.len() && src[j].is_ascii_digit() { j += 1; }
    let lo = &src[start..j];
    let mut hi: Option<&[u8]> = None;
    let ranged = j < src.len() && src[j] == b',';
    if ranged {
        j += 1;
        let h = j;
        while j < src.len() && src[j].is_ascii_digit() { j += 1; }
        hi = Some(&src[h..j]);
    }
    if j >= src.len() || src[j] != b'}' { return Ok(None); }
    if lo.is_empty() && hi.map(|h| h.is_empty()).unwrap_or(true) { return Ok(None); }
    // the ceiling the reference's compiled repeat carries (`RE_MAX_REPEAT`)
    let count = |d: &[u8]| -> Result<u32, String> {
        let mut v: u64 = 0;
        for c in d {
            v = v * 10 + (c - b'0') as u64;
            if v > MAX_REPEAT as u64 { return Err(String::from("too big number for repeat range")); }
        }
        Ok(v as u32)
    };
    let lo_s = if lo.is_empty() { 0 } else { count(lo)? };
    let text = match hi {
        // `{,m}` has no lower bound in Ruby, where the automaton wants one written
        Some(h) if h.is_empty() => format!("{{{lo_s},}}"),
        Some(h) => format!("{{{lo_s},{}}}", count(h)?),
        None => format!("{{{lo_s}}}"),
    };
    Ok(Some((text, j + 1)))
}

/// The ASCII sets Ruby's shorthands name, where the automaton's are Unicode's. Each is written
/// as a class so that a negated one can stand inside another class (`[\W]`).
fn shorthand(c: u8) -> Option<&'static str> {
    Some(match c {
        b'd' => "[0-9]",
        b'D' => "[^0-9]",
        b'w' => "[0-9A-Za-z_]",
        b'W' => "[^0-9A-Za-z_]",
        b's' => "[ \\t\\r\\n\\x{0C}\\x{0B}]",
        b'S' => "[^ \\t\\r\\n\\x{0C}\\x{0B}]",
        b'h' => "[0-9a-fA-F]",
        b'H' => "[^0-9a-fA-F]",
        _ => return None,
    })
}

/// One `\…` escape, from the backslash; `in_class` says it stands inside `[...]`, where Ruby
/// reads a digit as an octal escape and `\b` as a backspace, and `unicode` how the build reads a
/// byte above ASCII.
fn translate_escape(
    src: &[u8],
    i: usize,
    in_class: bool,
    tr: &Tr,
    out: &mut String,
    raw: &mut bool,
) -> Result<usize, String> {
    let Some(&c) = src.get(i + 1) else { return Err(String::from("too short escape sequence")) };
    if let Some(set) = shorthand(c) {
        // Ruby's shorthands are ASCII sets that hold both cases already, so `/i` folds none of
        // them across the boundary; inside a class there is no span to say that over
        if in_class { out.push_str(set); } else { out.push_str(&format!("(?-i:{set})")); }
        return Ok(i + 2);
    }
    match c {
        // a digit escape is a backreference where a group answers to it and an octal escape
        // where none does: the digits are one decimal number, a reference when it is at most 9
        // or at most the number of groups the pattern opens (`read_digit_escape`)
        b'1'..=b'9' if !in_class => {
            let mut j = i + 1;
            let mut v: u32 = 0;
            while j < src.len() && src[j].is_ascii_digit() && v < 1_000_000 {
                v = v * 10 + (src[j] - b'0') as u32;
                j += 1;
            }
            if v <= 9 || v <= tr.ngroups {
                // a pattern that names a group numbers none, so a numbered reference reaches
                // nothing there whatever this engine could do with it
                if tr.named {
                    return Err(String::from("numbered backref/call is not allowed. (use name)"));
                }
                return unsupported("backreference");
            }
            // 8 and 9 are no octal digits, so a number starting with one is the digit itself
            if matches!(c, b'8' | b'9') {
                out.push(c as char);
                return Ok(i + 2);
            }
            push_escape_bytes(src, i, true, tr, out, raw)
        }
        b'k' | b'g' if !in_class && matches!(src.get(i + 2), Some(b'<') | Some(b'\'')) => {
            // the name is read first, so a reference the pattern ends inside says so rather than
            // naming the construct
            let close = if src[i + 2] == b'<' { b'>' } else { b'\'' };
            read_group_name(src, i + 3, close)?;
            if c == b'k' { unsupported("named backreference") } else { unsupported("subexpression call") }
        }
        // octal, which the automaton spells in hex, and which reads as a run the same way
        b'0'..=b'7' => push_escape_bytes(src, i, true, tr, out, raw),
        b'8' | b'9' if in_class => { out.push(c as char); Ok(i + 2) }
        // the anchors the two share
        b'A' | b'z' | b'B' => { out.push('\\'); out.push(c as char); Ok(i + 2) }
        // `\b` is a word boundary outside a class and a backspace inside one
        b'b' => { out.push_str(if in_class { "\\x{08}" } else { "\\b" }); Ok(i + 2) }
        // `\Z` is the end of the string or just before a final newline, which takes a lookahead
        // to write here; `docs/gems.md` records the difference
        b'Z' => { out.push_str("\\z"); Ok(i + 2) }
        b'G' | b'K' | b'R' | b'X' if !in_class => unsupported(&format!("\\{}", c as char)),
        b'M' if !in_class => unsupported("meta escape"),
        // `\cX` and `\C-X` name a control character
        b'c' | b'C' => {
            let mut j = i + 2;
            if c == b'C' {
                if src.get(j) != Some(&b'-') { return Err(String::from("too short control escape")); }
                j += 1;
            }
            let Some(&x) = src.get(j) else { return Err(String::from("too short control escape")) };
            let v = if x == b'?' { 0x7f } else { x & 0x1f };
            out.push_str(&format!("\\x{{{v:X}}}"));
            Ok(j + 1)
        }
        // `\uXXXX` and `\u{…}` name codepoints, which the automaton spells `\x{…}`. What the
        // reference refuses here it refuses by name, since a shorter codepoint or literal text
        // is never what the pattern meant (`read_unicode_escape`).
        b'u' => {
            let mut j = i + 2;
            if j >= src.len() { return Err(String::from("too short escape sequence")); }
            if src[j] == b'{' {
                j += 1;
                // the list form: every codepoint is an atom of its own
                let mut any = false;
                loop {
                    while j < src.len() && src[j] == b' ' { j += 1; }
                    if src.get(j) == Some(&b'}') {
                        if !any { return Err(String::from("invalid Unicode list")); }
                        return Ok(j + 1);
                    }
                    let start = j;
                    while j < src.len() && src[j].is_ascii_hexdigit() { j += 1; }
                    if j == start || j - start > 6 { return Err(String::from("invalid Unicode list")); }
                    let v = hex_value(&src[start..j]);
                    if v > 0x10FFFF || (0xD800..0xE000).contains(&v) {
                        return Err(String::from("invalid Unicode range"));
                    }
                    push_codepoint(v, in_class, tr, out, raw);
                    any = true;
                }
            }
            if j + 4 > src.len() || !src[j..j + 4].iter().all(|b| b.is_ascii_hexdigit()) {
                return Err(String::from("invalid Unicode escape"));
            }
            let v = hex_value(&src[j..j + 4]);
            if (0xD800..0xE000).contains(&v) { return Err(String::from("invalid Unicode range")); }
            push_codepoint(v, in_class, tr, out, raw);
            Ok(j + 4)
        }
        b'e' => { out.push_str("\\x{1B}"); Ok(i + 2) }
        // a backslash before a character above ASCII is that character
        _ if c >= 0x80 => Ok(push_high(src, i + 1, tr.unicode, out, raw)),
        // `\x{…}` names a codepoint, which the automaton spells the same way
        b'x' if src.get(i + 2) == Some(&b'{') => {
            let mut j = i + 3;
            while j < src.len() && src[j] != b'}' { j += 1; }
            if j >= src.len() { return Err(String::from("invalid hex escape")); }
            out.push_str("\\x{");
            out.push_str(&String::from_utf8_lossy(&src[i + 3..j]));
            out.push('}');
            Ok(j + 1)
        }
        // `\xNN` names a byte, and bytes that spell a character are that character: a run of
        // them is read the way the pattern's own bytes are, so `\xC4\x80+` repeats the whole of
        // what the two spell rather than its last byte
        b'x' => push_escape_bytes(src, i, false, tr, out, raw),
        // `\p{…}` names a Unicode property, which the reference's engine is built without
        // ("character property is not supported"). A bare `\p` names no property and is the
        // letter, as it is in CRuby, and falls through to the arm below.
        b'p' | b'P' if src.get(i + 2) == Some(&b'{') => unsupported("character property"),
        _ => {
            if c.is_ascii_alphabetic() && !matches!(c, b'a' | b'f' | b'n' | b'r' | b't' | b'v') {
                // an escape neither engine reads is the letter itself, which is what CRuby reads
                out.push(c as char);
            } else if c.is_ascii_whitespace() || c == b'#' {
                // a space or a comment marker written as an escape is the character, which free
                // spacing must not drop
                out.push_str(&format!("\\x{:02X}", c));
            } else {
                out.push('\\');
                out.push(c as char);
            }
            Ok(i + 2)
        }
    }
}

/// What a POSIX bracket names where characters are classified by Unicode, as members usable
/// inside a class. `None` leaves the bracket to `regex-syntax`, whose own brackets are ASCII's,
/// which is what a build classifying by ASCII wants and what the two sets ASCII defines are on
/// either build.
fn posix_class(name: &[u8], unicode: bool) -> Result<Option<&'static str>, String> {
    // the two sets ASCII defines hold the same members on either build, and a build that
    // classifies by ASCII reads every bracket the way `regex-syntax` does
    if matches!(name, b"xdigit" | b"ascii") || !unicode {
        return if POSIX_NAMES.contains(&name) { Ok(None) } else { Err(String::from("invalid POSIX bracket type")) };
    }
    Ok(Some(match name {
        b"alpha" => "\\p{Alphabetic}",
        b"digit" => "\\p{Nd}",
        b"alnum" => "\\p{Alphabetic}\\p{Nd}",
        b"upper" => "\\p{Uppercase}",
        b"lower" => "\\p{Lowercase}",
        b"space" => "\\p{White_Space}",
        b"blank" => "\\p{Zs}\\t",
        // a word character is a letter, a mark it carries, a digit, a connector, or the two
        // format characters that join one to the next
        b"word" => "\\p{Alphabetic}\\p{M}\\p{Nd}\\p{Pc}\\p{Join_Control}",
        b"cntrl" => "\\p{Cc}",
        // Unicode punctuation, plus the ASCII characters Unicode files as symbols and POSIX as
        // punctuation
        b"punct" => "\\p{P}$+<=>^`|~",
        // what shows on the page: anything that is not a space, a control or unassigned;
        // `print` admits a space that is not a line or paragraph break
        b"graph" => "[^\\p{Cc}\\p{Cn}\\p{Z}]",
        b"print" => "[^\\p{Cc}\\p{Cn}\\p{Zl}\\p{Zp}]",
        _ => return Err(String::from("invalid POSIX bracket type")),
    }))
}

/// The bracket names the reference reads (`re_posix_bracket_names`).
const POSIX_NAMES: [&[u8]; 14] = [
    b"alpha", b"digit", b"alnum", b"upper", b"lower", b"space", b"blank", b"word", b"cntrl",
    b"punct", b"graph", b"print", b"xdigit", b"ascii",
];

/// One `[...]`, from the opening bracket: the members pass through, the shorthands inside them
/// are rewritten, and a POSIX bracket or a nested class is stepped over as a unit.
fn translate_class(src: &[u8], i: usize, tr: &Tr, out: &mut String) -> Result<usize, String> {
    let mut buf = String::new();
    let mut raw = false;
    let end = class_body(src, i, tr, &mut buf, &mut raw)?;
    push_wrapped(out, &buf, raw && tr.unicode);
    Ok(end)
}

/// The members of one `[...]`, into `out`; `raw` reports that one of them is a byte that spells
/// no character, which is what makes the whole class a class of bytes.
fn class_body(src: &[u8], i: usize, tr: &Tr, out: &mut String, raw: &mut bool) -> Result<usize, String> {
    let mut j = i + 1;
    out.push('[');
    if src.get(j) == Some(&b'^') { out.push('^'); j += 1; }
    // a `]` written first is a member rather than the end
    if src.get(j) == Some(&b']') { out.push_str("\\]"); j += 1; }
    while j < src.len() {
        match src[j] {
            b']' => { out.push(']'); return Ok(j + 1); }
            b'\\' => { j = translate_escape(src, j, true, tr, out, raw)?; }
            b'[' => {
                // `[:name:]` is a POSIX bracket, anything else a class nested in this one
                if src.get(j + 1) == Some(&b':') {
                    let mut k = j + 2;
                    let negated = src.get(k) == Some(&b'^');
                    let name_at = if negated { k + 1 } else { k };
                    k = name_at;
                    while k < src.len() && src[k] != b':' && src[k] != b']' { k += 1; }
                    if src.get(k) == Some(&b':') && src.get(k + 1) == Some(&b']') {
                        let name = &src[name_at..k];
                        match posix_class(name, tr.unicode)? {
                            // where characters are classified by Unicode the bracket is the
                            // property behind it, `regex-syntax`'s own bracket being ASCII's
                            Some(set) => {
                                if negated {
                                    out.push_str("[^");
                                    out.push_str(set);
                                    out.push(']');
                                } else {
                                    out.push_str(set);
                                }
                            }
                            None => out.push_str(&String::from_utf8_lossy(&src[j..k + 2])),
                        }
                        j = k + 2;
                        continue;
                    }
                    return Err(String::from("premature end of char-class"));
                }
                j = class_body(src, j, tr, out, raw)?;
            }
            b if b >= 0x80 => { j = push_high(src, j, tr.unicode, out, raw); }
            b => { out.push(b as char); j += 1; }
        }
    }
    Err(String::from("premature end of char-class"))
}


