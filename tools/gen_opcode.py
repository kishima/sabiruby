#!/usr/bin/env python3
"""Regenerate src/opcode.rs from mruby's include/mruby/ops.h.
usage: tools/gen_opcode.py ../ref/mruby/include/mruby/ops.h > src/opcode.rs

ops.h is a list of `OPCODE(NAME, LAYOUT) /* semantics */` lines, in the order the
bytes are numbered. Each line becomes one `Op` variant (with the semantics as its
doc comment) and one entry in each of the three tables: the mruby name, the operand
layout, and `OP_TABLE`, which `Op::from_u8` indexes instead of transmuting a byte.

The Rust name of an opcode is its mruby name in CamelCase: the parts between the
underscores, each capitalised (`GETIDX0` -> `Getidx0`, `RANGE_INC` -> `RangeInc`).
A double underscore is mruby's way of writing a minus sign, so it becomes an `M`
(`LOADI__1` -> `LoadiM1`).
"""
import re
import sys

LINE = re.compile(r"^OPCODE\(\s*(\w+)\s*,\s*(\w+)\s*\)\s*/\*(.*?)\*/")


def rust_name(name: str) -> str:
    return "".join("M" if p == "" else p.capitalize() for p in name.lower().split("_"))


def main(path: str) -> None:
    ops = []
    with open(path) as f:
        for line in f:
            m = LINE.match(line.strip())
            if m:
                ops.append((m.group(1), m.group(2), m.group(3).strip()))
    if not ops:
        sys.exit(f"no OPCODE() lines in {path}")

    out = sys.stdout.write
    out("//! Opcode table generated from mruby 4.1.0-rc `include/mruby/ops.h`.\n")
    out("//! Do not edit by hand: regenerate with `tools/gen_opcode.py` when ops.h changes.\n")
    out("\n\n")
    out("/// Operand layout of an instruction (see mruby `opcode.h`).\n")
    out("#[derive(Clone, Copy, Debug, PartialEq, Eq)]\n")
    out("pub enum Operands { Z, B, BB, BBB, BS, BSS, S, W }\n")
    out("\n")
    out("#[allow(clippy::upper_case_acronyms)]\n")
    out("#[derive(Clone, Copy, Debug, PartialEq, Eq)]\n")
    out("#[repr(u8)]\n")
    out("pub enum Op {\n")
    for name, _layout, doc in ops:
        out(f"    /// `{doc}`\n")
        out(f"    {rust_name(name)},\n")
    out("}\n")
    out("\n")
    out(f"pub const OP_COUNT: usize = {len(ops)};\n")
    out("\n")
    out("impl Op {\n")
    out("    /// Decode a byte into an opcode. Returns `None` for an undefined byte.\n")
    out("    pub fn from_u8(b: u8) -> Option<Op> { OP_TABLE.get(b as usize).copied() }\n")
    out("    /// mruby name of the opcode (as printed by `mrbc --verbose`).\n")
    out("    pub fn name(self) -> &'static str { OP_NAMES[self as usize] }\n")
    out("    /// Operand layout.\n")
    out("    pub fn operands(self) -> Operands { OP_OPERANDS[self as usize] }\n")
    out("}\n")
    out("\n")
    out("/// The byte-to-opcode table `from_u8` reads, in opcode order.\n")
    out("pub const OP_TABLE: [Op; OP_COUNT] = [\n")
    for name, _layout, _doc in ops:
        out(f"    Op::{rust_name(name)},\n")
    out("];\n")
    out("\n")
    out("pub const OP_NAMES: [&str; OP_COUNT] = [\n")
    for name, _layout, _doc in ops:
        out(f'    "{name}",\n')
    out("];\n")
    out("\n")
    out("pub const OP_OPERANDS: [Operands; OP_COUNT] = [\n")
    for _name, layout, _doc in ops:
        out(f"    Operands::{layout},\n")
    out("];\n")


if __name__ == "__main__":
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    main(sys.argv[1])
