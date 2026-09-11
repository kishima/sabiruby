
use crate::symbol::Sym;

/// Handle to a heap object (index into [`crate::object::Heap`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ObjId(pub u32);

/// A Ruby value. Immediates are stored inline; everything else is a heap
/// handle. This is the "No Boxing" representation of the book's VM chapter:
/// the type tag is a Rust enum discriminant instead of tag bits in a word.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Value {
    Nil,
    False,
    True,
    Int(i64),
    Float(f64),
    Sym(Sym),
    Obj(ObjId),
}

impl Value {
    #[inline]
    pub fn truthy(self) -> bool {
        !matches!(self, Value::Nil | Value::False)
    }
    #[inline]
    pub fn is_nil(self) -> bool {
        matches!(self, Value::Nil)
    }
    #[inline]
    pub fn bool(b: bool) -> Value {
        if b { Value::True } else { Value::False }
    }
    #[inline]
    pub fn obj(self) -> Option<ObjId> {
        match self {
            Value::Obj(o) => Some(o),
            _ => None,
        }
    }
    /// Identity comparison (`equal?`). Floats compare by value like mruby's
    /// immediate floats.
    pub fn same(self, other: Value) -> bool {
        self == other
    }
}
