//! Line numbers from the DBG section (`mrbc -g`): the `(pc, line)` pairs of `vm::dump` must be
//! the ones the reference `mrbc -g --verbose` prints for the same file (`tests/custom/*.dump`,
//! recorded by `tools/custom.sh`).

use std::path::Path;

/// `(irep index, pc, line)` of every instruction line of a listing, in order.
/// Reference lines look like `    9 000 TDEF\t\tR2\t:f`, SabiRuby's like `    9 000 TDEF\t2`;
/// both start a new irep with a line beginning `irep `.
fn positions(listing: &str) -> Vec<(usize, u32, u32)> {
    let mut out = Vec::new();
    let mut irep = 0usize;
    let mut first = true;
    for line in listing.lines() {
        if line.starts_with("irep ") {
            if !first { irep += 1; }
            first = false;
            continue;
        }
        // `%5d %03d ` then the opcode; without debug info the line column is blank
        let (head, rest) = line.split_at(line.len().min(10));
        if rest.is_empty() || !head.ends_with(' ') { continue; }
        let mut it = head.split_whitespace();
        let (Some(l), Some(pc)) = (it.next(), it.next()) else { continue };
        if it.next().is_some() { continue; }
        let (Ok(l), Ok(pc)) = (l.parse::<u32>(), pc.parse::<u32>()) else { continue };
        out.push((irep, pc, l));
    }
    out
}

#[test]
fn dump_line_numbers_match_the_reference_listing() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/custom");
    let mut cases = 0;
    for entry in std::fs::read_dir(&dir).expect("tests/custom") {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|e| e != "dump") { continue; }
        let name = path.file_stem().unwrap().to_string_lossy().into_owned();
        // a listing quotes the source's own bytes, which need not be UTF-8
        let text = std::fs::read(&path).unwrap();
        let expected = positions(&String::from_utf8_lossy(&text));
        let bin = std::fs::read(dir.join(format!("{name}.mrb"))).expect(".mrb");
        let rite = sabiruby::rite::parse(&bin).expect("parse");
        let ours = positions(&sabiruby::vm::dump(&rite));
        assert!(!expected.is_empty(), "{name}: no line numbers in the reference listing");
        assert_eq!(ours, expected, "{name}: line numbers differ from mrbc --verbose");
        cases += 1;
    }
    assert!(cases >= 4, "only {cases} recorded listings");
}

#[test]
fn line_of_follows_the_debug_entries() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/custom");
    let bin = std::fs::read(dir.join("eval_locals.mrb")).expect(".mrb");
    let rite = sabiruby::rite::parse(&bin).expect("parse");
    let irep = &rite.ireps[rite.root];
    assert!(!irep.lines.is_empty(), "no DBG section");
    assert_eq!(irep.filename.as_deref(), Some(&b"/w/eval_locals.rb"[..]));
    // the line of a pc between two entries is the earlier entry's (mrb_debug_get_line)
    for (pc, line) in irep.lines.clone() {
        assert_eq!(irep.line_of(pc as usize), Some(line));
        assert_eq!(irep.line_of(pc as usize + 1).is_some(), true);
    }
    // a program compiled without -g has no lines at all
    let plain = std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/hello.mrb")).unwrap();
    let plain = sabiruby::rite::parse(&plain).unwrap();
    assert!(plain.ireps[plain.root].lines.is_empty());
    assert_eq!(plain.ireps[plain.root].line_of(0), None);
}
