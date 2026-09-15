use alloc::{boxed::Box, string::String, vec::Vec};

use hashbrown::HashMap;

/// Symbol id. Unlike mruby's `mrb_sym` there is no inline/presym encoding:
/// every symbol is an index into the interner (an implementation choice).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Sym(pub u32);

#[derive(Default, Debug)]
pub struct Interner {
    names: Vec<Box<[u8]>>,
    index: HashMap<Box<[u8]>, Sym>,
}

impl Interner {
    pub fn intern(&mut self, name: &[u8]) -> Sym {
        if let Some(&s) = self.index.get(name) {
            return s;
        }
        let s = Sym(self.names.len() as u32);
        let boxed: Box<[u8]> = name.into();
        self.names.push(boxed.clone());
        self.index.insert(boxed, s);
        s
    }
    pub fn intern_str(&mut self, name: &str) -> Sym {
        self.intern(name.as_bytes())
    }
    /// The symbol a name already has, or `None` where nothing interned it. What a reader that
    /// does not want to make one asks (`Vm::ivar_get`, `Vm::global_get`): a name no symbol
    /// stands for cannot be the name of anything the VM holds.
    pub fn lookup_str(&self, name: &str) -> Option<Sym> {
        self.index.get(name.as_bytes()).copied()
    }
    pub fn name(&self, s: Sym) -> &[u8] {
        &self.names[s.0 as usize]
    }
    pub fn name_str(&self, s: Sym) -> String {
        String::from_utf8_lossy(self.name(s)).into_owned()
    }
    pub fn len(&self) -> usize {
        self.names.len()
    }
    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }
}
