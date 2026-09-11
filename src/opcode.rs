//! Opcode table generated from mruby 4.1.0-rc `include/mruby/ops.h`.
//! Do not edit by hand: regenerate with `tools/gen_opcode.py` when ops.h changes.

/// Operand layout of an instruction (see mruby `opcode.h`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operands { Z, B, BB, BBB, BS, BSS, S, W }

#[allow(clippy::upper_case_acronyms)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Op {
    /// `no operation`
    Nop,
    /// `R[a] = R[b]`
    Move,
    /// `R[a] = Pool[b]`
    Loadl,
    /// `R[a] = mrb_int(b)`
    Loadi8,
    /// `R[a] = mrb_int(-b)`
    Loadineg,
    /// `R[a] = mrb_int(-1)`
    LoadiM1,
    /// `R[a] = mrb_int(0)`
    Loadi0,
    /// `R[a] = mrb_int(1)`
    Loadi1,
    /// `R[a] = mrb_int(2)`
    Loadi2,
    /// `R[a] = mrb_int(3)`
    Loadi3,
    /// `R[a] = mrb_int(4)`
    Loadi4,
    /// `R[a] = mrb_int(5)`
    Loadi5,
    /// `R[a] = mrb_int(6)`
    Loadi6,
    /// `R[a] = mrb_int(7)`
    Loadi7,
    /// `R[a] = mrb_int(b)`
    Loadi16,
    /// `R[a] = mrb_int((b<<16)+c)`
    Loadi32,
    /// `R[a] = Syms[b]`
    Loadsym,
    /// `R[a] = nil`
    Loadnil,
    /// `R[a] = self`
    Loadself,
    /// `R[a] = true`
    Loadtrue,
    /// `R[a] = false`
    Loadfalse,
    /// `R[a] = getglobal(Syms[b])`
    Getgv,
    /// `setglobal(Syms[b], R[a])`
    Setgv,
    /// `R[a] = Special[Syms[b]]`
    Getsv,
    /// `Special[Syms[b]] = R[a]`
    Setsv,
    /// `R[a] = ivget(Syms[b])`
    Getiv,
    /// `ivset(Syms[b],R[a])`
    Setiv,
    /// `R[a] = cvget(Syms[b])`
    Getcv,
    /// `cvset(Syms[b],R[a])`
    Setcv,
    /// `R[a] = constget(Syms[b])`
    Getconst,
    /// `constset(Syms[b],R[a])`
    Setconst,
    /// `R[a] = R[a]::Syms[b]`
    Getmcnst,
    /// `R[a+1]::Syms[b] = R[a]`
    Setmcnst,
    /// `R[a] = uvget(b,c)`
    Getupvar,
    /// `uvset(b,c,R[a])`
    Setupvar,
    /// `R[a] = R[a][R[a+1]]`
    Getidx,
    /// `R[a] = R[b][0]; a+1 for method call`
    Getidx0,
    /// `R[a][R[a+1]] = R[a+2]`
    Setidx,
    /// `pc+=a`
    Jmp,
    /// `if R[a] pc+=b`
    Jmpif,
    /// `if !R[a] pc+=b`
    Jmpnot,
    /// `if R[a]==nil pc+=b`
    Jmpnil,
    /// `unwind_and_jump_to(a)`
    Jmpuw,
    /// `R[a] = exc`
    Except,
    /// `R[b] = R[a].isa?(R[b])`
    Rescue,
    /// `raise(R[a]) if R[a]`
    Raiseif,
    /// `raise NoMatchingPatternError unless R[a]`
    Matcherr,
    /// `R[a] = self.send(Syms[b],R[a+1]..,R[a+n+1]:R[a+n+2]..) (c=n|k<<4)`
    Ssend,
    /// `R[a] = self.send(Syms[b]) (no args)`
    Ssend0,
    /// `R[a] = self.send(Syms[b],R[a+1]..,R[a+n+1]:R[a+n+2]..,&R[a+n+2k+1])`
    Ssendb,
    /// `R[a] = R[a].send(Syms[b],R[a+1]..,R[a+n+1]:R[a+n+2]..) (c=n|k<<4)`
    Send,
    /// `R[a] = R[a].send(Syms[b]) (no args)`
    Send0,
    /// `R[a] = R[a].send(Syms[b],R[a+1]..,R[a+n+1]:R[a+n+2]..,&R[a+n+2k+1])`
    Sendb,
    /// `self.call(*, **, &) (But overlay the current call frame; tailcall)`
    Call,
    /// `R[a] = R[a].call(R[a+1],... ,R[a+b]); direct block call`
    Blkcall,
    /// `R[a] = super(R[a+1],... ,R[a+b+1])`
    Super,
    /// `R[a] = argument array (16=m5:r1:m5:d1:lv4)`
    Argary,
    /// `arg setup according to flags (24=n1:m5:o5:r1:m5:k5:d1:b1)`
    Enter,
    /// `R[a] = kdict.key?(Syms[b])`
    KeyP,
    /// `raise unless kdict.empty?`
    Keyend,
    /// `R[a] = kdict[Syms[b]]; kdict.delete(Syms[b])`
    Karg,
    /// `return R[a] (normal)`
    Return,
    /// `return R[a] (in-block return)`
    ReturnBlk,
    /// `return self`
    Retself,
    /// `return nil`
    Retnil,
    /// `return true`
    Rettrue,
    /// `return false`
    Retfalse,
    /// `break R[a]`
    Break,
    /// `R[a] = block (16=m5:r1:m5:d1:lv4)`
    Blkpush,
    /// `R[a] = R[a]+R[a+1]`
    Add,
    /// `R[a] = R[a]+mrb_int(b)`
    Addi,
    /// `R[a] = R[a]-R[a+1]`
    Sub,
    /// `R[a] = R[a]-mrb_int(b)`
    Subi,
    /// `R[a] = R[a]+mrb_int(c); R[b],R[b+1] for method call`
    Addilv,
    /// `R[a] = R[a]-mrb_int(c); R[b],R[b+1] for method call`
    Subilv,
    /// `R[a] = R[a]*R[a+1]`
    Mul,
    /// `R[a] = R[a]/R[a+1]`
    Div,
    /// `R[a] = R[a]==R[a+1]`
    Eq,
    /// `R[a] = R[a]<R[a+1]`
    Lt,
    /// `R[a] = R[a]<=R[a+1]`
    Le,
    /// `R[a] = R[a]>R[a+1]`
    Gt,
    /// `R[a] = R[a]>=R[a+1]`
    Ge,
    /// `R[a] = ary_new(R[a],R[a+1]..R[a+b])`
    Array,
    /// `R[a] = ary_new(R[b],R[b+1]..R[b+c])`
    Array2,
    /// `ary_cat(R[a],R[a+1])`
    Arycat,
    /// `ary_push(R[a],R[a+1]..R[a+b])`
    Arypush,
    /// `R[a] = ary_splat(R[a])`
    Arysplat,
    /// `R[a] = R[b][c]`
    Aref,
    /// `R[b][c] = R[a]`
    Aset,
    /// `*R[a],R[a+1]..R[a+c] = R[a][b..]`
    Apost,
    /// `R[a] = intern(R[a])`
    Intern,
    /// `R[a] = intern(Pool[b])`
    Symbol,
    /// `R[a] = str_dup(Pool[b])`
    String,
    /// `str_cat(R[a],R[a+1])`
    Strcat,
    /// `R[a] = hash_new(R[a],R[a+1]..R[a+b*2-1])`
    Hash,
    /// `hash_push(R[a],R[a+1]..R[a+b*2])`
    Hashadd,
    /// `R[a] = hash_cat(R[a],R[a+1])`
    Hashcat,
    /// `R[a] = lambda(Irep[b],L_LAMBDA)`
    Lambda,
    /// `R[a] = lambda(Irep[b],L_BLOCK)`
    Block,
    /// `R[a] = lambda(Irep[b],L_METHOD)`
    Method,
    /// `R[a] = range_new(R[a],R[a+1],FALSE)`
    RangeInc,
    /// `R[a] = range_new(R[a],R[a+1],TRUE)`
    RangeExc,
    /// `R[a] = ::Object`
    Oclass,
    /// `R[a] = newclass(R[a],Syms[b],R[a+1])`
    Class,
    /// `R[a] = newmodule(R[a],Syms[b])`
    Module,
    /// `R[a] = blockexec(R[a],Irep[b])`
    Exec,
    /// `R[a].newmethod(Syms[b],R[a+1]); R[a] = Syms[b]`
    Def,
    /// `target_class.newmethod(Syms[b],Irep[c]); R[a] = Syms[b]`
    Tdef,
    /// `R[a].singleton_class.newmethod(Syms[b],Irep[c]); R[a] = Syms[b]`
    Sdef,
    /// `alias_method(target_class,Syms[a],Syms[b])`
    Alias,
    /// `undef_method(target_class,Syms[a])`
    Undef,
    /// `R[a] = R[a].singleton_class`
    Sclass,
    /// `R[a] = target_class`
    Tclass,
    /// `print a,b,c`
    Debug,
    /// `raise(LocalJumpError, Pool[a])`
    Err,
    /// `make 1st operand (a) 16bit`
    Ext1,
    /// `make 2nd operand (b) 16bit`
    Ext2,
    /// `make 1st and 2nd operands 16bit`
    Ext3,
    /// `stop VM`
    Stop,
}

pub const OP_COUNT: usize = 119;

impl Op {
    /// Decode a byte into an opcode. Returns `None` for an undefined byte.
    pub fn from_u8(b: u8) -> Option<Op> {
        if (b as usize) < OP_COUNT { Some(unsafe { std::mem::transmute::<u8, Op>(b) }) } else { None }
    }
    /// mruby name of the opcode (as printed by `mrbc --verbose`).
    pub fn name(self) -> &'static str { OP_NAMES[self as usize] }
    /// Operand layout.
    pub fn operands(self) -> Operands { OP_OPERANDS[self as usize] }
}

pub const OP_NAMES: [&str; OP_COUNT] = [
    "NOP",
    "MOVE",
    "LOADL",
    "LOADI8",
    "LOADINEG",
    "LOADI__1",
    "LOADI_0",
    "LOADI_1",
    "LOADI_2",
    "LOADI_3",
    "LOADI_4",
    "LOADI_5",
    "LOADI_6",
    "LOADI_7",
    "LOADI16",
    "LOADI32",
    "LOADSYM",
    "LOADNIL",
    "LOADSELF",
    "LOADTRUE",
    "LOADFALSE",
    "GETGV",
    "SETGV",
    "GETSV",
    "SETSV",
    "GETIV",
    "SETIV",
    "GETCV",
    "SETCV",
    "GETCONST",
    "SETCONST",
    "GETMCNST",
    "SETMCNST",
    "GETUPVAR",
    "SETUPVAR",
    "GETIDX",
    "GETIDX0",
    "SETIDX",
    "JMP",
    "JMPIF",
    "JMPNOT",
    "JMPNIL",
    "JMPUW",
    "EXCEPT",
    "RESCUE",
    "RAISEIF",
    "MATCHERR",
    "SSEND",
    "SSEND0",
    "SSENDB",
    "SEND",
    "SEND0",
    "SENDB",
    "CALL",
    "BLKCALL",
    "SUPER",
    "ARGARY",
    "ENTER",
    "KEY_P",
    "KEYEND",
    "KARG",
    "RETURN",
    "RETURN_BLK",
    "RETSELF",
    "RETNIL",
    "RETTRUE",
    "RETFALSE",
    "BREAK",
    "BLKPUSH",
    "ADD",
    "ADDI",
    "SUB",
    "SUBI",
    "ADDILV",
    "SUBILV",
    "MUL",
    "DIV",
    "EQ",
    "LT",
    "LE",
    "GT",
    "GE",
    "ARRAY",
    "ARRAY2",
    "ARYCAT",
    "ARYPUSH",
    "ARYSPLAT",
    "AREF",
    "ASET",
    "APOST",
    "INTERN",
    "SYMBOL",
    "STRING",
    "STRCAT",
    "HASH",
    "HASHADD",
    "HASHCAT",
    "LAMBDA",
    "BLOCK",
    "METHOD",
    "RANGE_INC",
    "RANGE_EXC",
    "OCLASS",
    "CLASS",
    "MODULE",
    "EXEC",
    "DEF",
    "TDEF",
    "SDEF",
    "ALIAS",
    "UNDEF",
    "SCLASS",
    "TCLASS",
    "DEBUG",
    "ERR",
    "EXT1",
    "EXT2",
    "EXT3",
    "STOP",
];

pub const OP_OPERANDS: [Operands; OP_COUNT] = [
    Operands::Z,
    Operands::BB,
    Operands::BB,
    Operands::BB,
    Operands::BB,
    Operands::B,
    Operands::B,
    Operands::B,
    Operands::B,
    Operands::B,
    Operands::B,
    Operands::B,
    Operands::B,
    Operands::B,
    Operands::BS,
    Operands::BSS,
    Operands::BB,
    Operands::B,
    Operands::B,
    Operands::B,
    Operands::B,
    Operands::BB,
    Operands::BB,
    Operands::BB,
    Operands::BB,
    Operands::BB,
    Operands::BB,
    Operands::BB,
    Operands::BB,
    Operands::BB,
    Operands::BB,
    Operands::BB,
    Operands::BB,
    Operands::BBB,
    Operands::BBB,
    Operands::B,
    Operands::BB,
    Operands::B,
    Operands::S,
    Operands::BS,
    Operands::BS,
    Operands::BS,
    Operands::S,
    Operands::B,
    Operands::BB,
    Operands::B,
    Operands::B,
    Operands::BBB,
    Operands::BB,
    Operands::BBB,
    Operands::BBB,
    Operands::BB,
    Operands::BBB,
    Operands::Z,
    Operands::BB,
    Operands::BB,
    Operands::BS,
    Operands::W,
    Operands::BB,
    Operands::Z,
    Operands::BB,
    Operands::B,
    Operands::B,
    Operands::Z,
    Operands::Z,
    Operands::Z,
    Operands::Z,
    Operands::B,
    Operands::BS,
    Operands::B,
    Operands::BB,
    Operands::B,
    Operands::BB,
    Operands::BBB,
    Operands::BBB,
    Operands::B,
    Operands::B,
    Operands::B,
    Operands::B,
    Operands::B,
    Operands::B,
    Operands::B,
    Operands::BB,
    Operands::BBB,
    Operands::B,
    Operands::BB,
    Operands::B,
    Operands::BBB,
    Operands::BBB,
    Operands::BBB,
    Operands::B,
    Operands::BB,
    Operands::BB,
    Operands::B,
    Operands::BB,
    Operands::BB,
    Operands::B,
    Operands::BB,
    Operands::BB,
    Operands::BB,
    Operands::B,
    Operands::B,
    Operands::B,
    Operands::BB,
    Operands::BB,
    Operands::BB,
    Operands::BB,
    Operands::BBB,
    Operands::BBB,
    Operands::BB,
    Operands::B,
    Operands::B,
    Operands::B,
    Operands::BBB,
    Operands::B,
    Operands::Z,
    Operands::Z,
    Operands::Z,
    Operands::Z,
];
