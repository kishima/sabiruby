//! Ruby → Rust: a [`Value`] read as serde's data model.
//!
//! The reverse of [`ser`](crate::ser), and deliberately a little more forgiving on the way in:
//! a struct's fields are found under String *or* Symbol keys, and an Integer widens to a float
//! where a float is asked for. What it will not do is narrow — a Float where an Integer is
//! asked for is an error, not a rounding.

use alloc::string::{String, ToString};

use sabiruby::value::Value;
use sabiruby::Vm;
use serde::de::{self, DeserializeSeed, IntoDeserializer, Visitor};

use crate::error::{Error, Result};

/// One Ruby value, being read as some Rust type.
pub struct Deserializer<'v> {
    vm: &'v mut Vm,
    value: Value,
}

impl<'v> Deserializer<'v> {
    pub fn new(vm: &'v mut Vm, value: Value) -> Deserializer<'v> {
        Deserializer { vm, value }
    }
    /// What the value is, for a message: its class name.
    fn what(&mut self) -> String {
        match self.value {
            Value::Nil => "nil".to_string(),
            Value::True => "true".to_string(),
            Value::False => "false".to_string(),
            _ => { let c = self.vm.real_class_of(self.value); self.vm.class_name(c) }
        }
    }
    fn wrong(&mut self, wanted: &str) -> Error {
        let w = self.what();
        Error::Message(alloc::format!("cannot deserialize {w} as {wanted}"))
    }
    /// An Integer, immediate or wide, as an `i128` (which holds every `i64` and `u64`).
    fn as_i128(&mut self) -> Option<i128> {
        match self.value {
            Value::Int(i) => Some(i as i128),
            // a wide Integer: `as_bigint` answers only for one of those (`bint_value` keeps
            // the two representations apart), and i128 may still be too narrow, in which case
            // the number is out of range for every Rust integer this deserializer offers
            Value::Obj(_) => {
                let b = self.vm.as_bigint(self.value)?;
                match b.to_i64() {
                    Some(i) => Some(i as i128),
                    // wider than an i64: the decimal text is the only way out of `BigInt`
                    // that keeps every digit (`to_f64` rounds, `to_u64` gives up at 2**64)
                    None => b.to_string_radix(10).parse::<i128>().ok(),
                }
            }
            _ => None,
        }
    }
    fn as_f64(&mut self) -> Option<f64> {
        match self.value {
            Value::Float(f) => Some(f),
            Value::Int(i) => Some(i as f64),
            Value::Obj(_) if self.vm.is_bigint(self.value) => self.vm.as_bigint(self.value).map(|b| b.to_f64()),
            _ => None,
        }
    }
    /// The text of a String or a Symbol.
    fn as_str(&mut self) -> Option<String> {
        match self.value {
            Value::Sym(s) => Some(self.vm.sym_name(s)),
            Value::Obj(_) => {
                let b = self.vm.str_bytes(self.value)?;
                core::str::from_utf8(b).ok().map(|s| s.to_string())
            }
            _ => None,
        }
    }
}

/// `deserialize_i8` … `deserialize_u64`: an Integer that fits, and nothing else.
macro_rules! int_method {
    ($name:ident, $t:ty, $visit:ident) => {
        fn $name<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
            let mut me = self;
            match me.as_i128() {
                Some(i) => match <$t>::try_from(i) {
                    Ok(x) => visitor.$visit(x),
                    Err(_) => Err(Error::Message(alloc::format!(
                        "{i} is out of range for {}", core::any::type_name::<$t>()))),
                },
                None => Err(me.wrong(core::any::type_name::<$t>())),
            }
        }
    };
}

impl<'de, 'v> de::Deserializer<'de> for Deserializer<'v> {
    type Error = Error;

    /// What the value *is*, for a type that has not said what it wants (`serde_json::Value`,
    /// an untagged enum, `#[serde(flatten)]`). A String that is valid UTF-8 is text; one that
    /// is not is bytes, which is the only honest answer — Ruby's String is a byte string.
    fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        let mut me = self;
        match me.value {
            Value::Nil => visitor.visit_unit(),
            Value::True => visitor.visit_bool(true),
            Value::False => visitor.visit_bool(false),
            Value::Int(i) => visitor.visit_i64(i),
            Value::Float(f) => visitor.visit_f64(f),
            Value::Sym(s) => { let n = me.vm.sym_name(s); visitor.visit_string(n) }
            Value::Obj(_) => {
                if me.vm.is_bigint(me.value) {
                    return match me.as_i128() {
                        Some(i) if i64::try_from(i).is_ok() => visitor.visit_i64(i as i64),
                        Some(i) if u64::try_from(i).is_ok() => visitor.visit_u64(i as u64),
                        Some(i) => visitor.visit_i128(i),
                        None => Err(Error::Message("Integer is too wide for any Rust integer".to_string())),
                    };
                }
                if let Some(b) = me.vm.str_bytes(me.value) {
                    return match core::str::from_utf8(b) {
                        Ok(s) => { let s = s.to_string(); visitor.visit_string(s) }
                        Err(_) => { let b = b.to_vec(); visitor.visit_byte_buf(b) }
                    };
                }
                if let Some(items) = me.vm.ary_vals(me.value) {
                    return visitor.visit_seq(SeqDe { vm: me.vm, items: items.into_iter() });
                }
                if let Some(entries) = me.vm.hash_entries(me.value) {
                    return visitor.visit_map(MapDe { vm: me.vm, entries: entries.into_iter(), value: None });
                }
                Err(me.wrong("a value serde understands"))
            }
        }
    }

    fn deserialize_bool<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        let mut me = self;
        match me.value {
            Value::True => visitor.visit_bool(true),
            Value::False => visitor.visit_bool(false),
            _ => Err(me.wrong("bool")),
        }
    }

    int_method!(deserialize_i8, i8, visit_i8);
    int_method!(deserialize_i16, i16, visit_i16);
    int_method!(deserialize_i32, i32, visit_i32);
    int_method!(deserialize_i64, i64, visit_i64);
    int_method!(deserialize_i128, i128, visit_i128);
    int_method!(deserialize_u8, u8, visit_u8);
    int_method!(deserialize_u16, u16, visit_u16);
    int_method!(deserialize_u32, u32, visit_u32);
    int_method!(deserialize_u64, u64, visit_u64);
    int_method!(deserialize_u128, u128, visit_u128);

    fn deserialize_f32<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        let mut me = self;
        match me.as_f64() { Some(f) => visitor.visit_f32(f as f32), None => Err(me.wrong("f32")) }
    }
    fn deserialize_f64<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        let mut me = self;
        match me.as_f64() { Some(f) => visitor.visit_f64(f), None => Err(me.wrong("f64")) }
    }

    fn deserialize_char<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        let mut me = self;
        match me.as_str().and_then(|s| { let mut it = s.chars(); match (it.next(), it.next()) { (Some(c), None) => Some(c), _ => None } }) {
            Some(c) => visitor.visit_char(c),
            None => Err(me.wrong("char")),
        }
    }

    fn deserialize_str<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        self.deserialize_string(visitor)
    }
    /// A String or a Symbol. A String that is not UTF-8 is an error rather than a lossy
    /// replacement: read it as bytes (`serde_bytes`, or a field of type `Vec<u8>` behind it).
    fn deserialize_string<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        let mut me = self;
        if let Some(s) = me.as_str() { return visitor.visit_string(s); }
        if me.vm.str_bytes(me.value).is_some() {
            return Err(Error::Message("String is not valid UTF-8; read it as bytes".to_string()));
        }
        Err(me.wrong("String"))
    }

    fn deserialize_bytes<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        self.deserialize_byte_buf(visitor)
    }
    fn deserialize_byte_buf<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        let mut me = self;
        if let Some(b) = me.vm.str_bytes(me.value) { let b = b.to_vec(); return visitor.visit_byte_buf(b); }
        if let Some(items) = me.vm.ary_vals(me.value) {
            return visitor.visit_seq(SeqDe { vm: me.vm, items: items.into_iter() });
        }
        Err(me.wrong("bytes"))
    }

    fn deserialize_option<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        if self.value.is_nil() { visitor.visit_none() } else { visitor.visit_some(self) }
    }

    fn deserialize_unit<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        let mut me = self;
        if me.value.is_nil() { visitor.visit_unit() } else { Err(me.wrong("nil")) }
    }
    fn deserialize_unit_struct<V: Visitor<'de>>(self, _name: &'static str, visitor: V) -> Result<V::Value> {
        self.deserialize_unit(visitor)
    }
    fn deserialize_newtype_struct<V: Visitor<'de>>(self, _name: &'static str, visitor: V) -> Result<V::Value> {
        visitor.visit_newtype_struct(self)
    }

    fn deserialize_seq<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        let mut me = self;
        match me.vm.ary_vals(me.value) {
            Some(items) => visitor.visit_seq(SeqDe { vm: me.vm, items: items.into_iter() }),
            None => Err(me.wrong("Array")),
        }
    }
    fn deserialize_tuple<V: Visitor<'de>>(self, _len: usize, visitor: V) -> Result<V::Value> {
        self.deserialize_seq(visitor)
    }
    fn deserialize_tuple_struct<V: Visitor<'de>>(self, _name: &'static str, _len: usize, visitor: V) -> Result<V::Value> {
        self.deserialize_seq(visitor)
    }

    fn deserialize_map<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        let mut me = self;
        match me.vm.hash_entries(me.value) {
            Some(entries) => visitor.visit_map(MapDe { vm: me.vm, entries: entries.into_iter(), value: None }),
            None => Err(me.wrong("Hash")),
        }
    }
    fn deserialize_struct<V: Visitor<'de>>(self, _name: &'static str, _fields: &'static [&'static str], visitor: V) -> Result<V::Value> {
        self.deserialize_map(visitor)
    }

    /// A Symbol (or a String) is a unit variant; a Hash of exactly one entry is a variant that
    /// carries something — the shape [`ser`](crate::ser) writes.
    fn deserialize_enum<V: Visitor<'de>>(
        self, _name: &'static str, _variants: &'static [&'static str], visitor: V,
    ) -> Result<V::Value> {
        let mut me = self;
        if let Some(entries) = me.vm.hash_entries(me.value) {
            if entries.len() != 1 {
                return Err(Error::Message(alloc::format!(
                    "a variant is a Hash of one entry, got {}", entries.len())));
            }
            let (k, v) = entries[0];
            return visitor.visit_enum(EnumDe { vm: me.vm, tag: k, value: Some(v) });
        }
        if me.as_str().is_some() {
            let tag = me.value;
            return visitor.visit_enum(EnumDe { vm: me.vm, tag, value: None });
        }
        Err(me.wrong("a variant (Symbol, String or a Hash of one entry)"))
    }

    /// A field name: a Symbol or a String, so that a Hash written either way fits the same
    /// struct.
    fn deserialize_identifier<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        self.deserialize_string(visitor)
    }

    /// A field the target type does not have. Nothing is read out of it: the value stays in
    /// the VM, and a type that cannot be described in serde's model (a Proc, a Data object)
    /// passes by instead of failing the whole conversion.
    fn deserialize_ignored_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        visitor.visit_unit()
    }
}

struct SeqDe<'v> {
    vm: &'v mut Vm,
    items: alloc::vec::IntoIter<Value>,
}

impl<'de, 'v> de::SeqAccess<'de> for SeqDe<'v> {
    type Error = Error;
    fn next_element_seed<T: DeserializeSeed<'de>>(&mut self, seed: T) -> Result<Option<T::Value>> {
        match self.items.next() {
            Some(v) => seed.deserialize(Deserializer { vm: &mut *self.vm, value: v }).map(Some),
            None => Ok(None),
        }
    }
    fn size_hint(&self) -> Option<usize> { Some(self.items.len()) }
}

struct MapDe<'v> {
    vm: &'v mut Vm,
    entries: alloc::vec::IntoIter<(Value, Value)>,
    value: Option<Value>,
}

impl<'de, 'v> de::MapAccess<'de> for MapDe<'v> {
    type Error = Error;
    fn next_key_seed<K: DeserializeSeed<'de>>(&mut self, seed: K) -> Result<Option<K::Value>> {
        match self.entries.next() {
            Some((k, v)) => {
                self.value = Some(v);
                seed.deserialize(Deserializer { vm: &mut *self.vm, value: k }).map(Some)
            }
            None => Ok(None),
        }
    }
    fn next_value_seed<V: DeserializeSeed<'de>>(&mut self, seed: V) -> Result<V::Value> {
        let v = self.value.take().unwrap_or(Value::Nil);
        seed.deserialize(Deserializer { vm: &mut *self.vm, value: v })
    }
    fn size_hint(&self) -> Option<usize> { Some(self.entries.len()) }
}

struct EnumDe<'v> {
    vm: &'v mut Vm,
    tag: Value,
    value: Option<Value>,
}

impl<'de, 'v> de::EnumAccess<'de> for EnumDe<'v> {
    type Error = Error;
    type Variant = VariantDe<'v>;
    fn variant_seed<V: DeserializeSeed<'de>>(self, seed: V) -> Result<(V::Value, VariantDe<'v>)> {
        let mut d = Deserializer { vm: self.vm, value: self.tag };
        let name = match d.as_str() {
            Some(s) => s,
            None => return Err(d.wrong("a variant name (Symbol or String)")),
        };
        let nd: de::value::StringDeserializer<Error> = name.into_deserializer();
        let v = seed.deserialize(nd)?;
        Ok((v, VariantDe { vm: d.vm, value: self.value }))
    }
}

struct VariantDe<'v> {
    vm: &'v mut Vm,
    value: Option<Value>,
}

impl<'de, 'v> de::VariantAccess<'de> for VariantDe<'v> {
    type Error = Error;
    fn unit_variant(self) -> Result<()> {
        match self.value {
            None | Some(Value::Nil) => Ok(()),
            Some(_) => Err(Error::Message("a unit variant carries nothing".to_string())),
        }
    }
    fn newtype_variant_seed<T: DeserializeSeed<'de>>(self, seed: T) -> Result<T::Value> {
        let v = self.value.unwrap_or(Value::Nil);
        seed.deserialize(Deserializer { vm: self.vm, value: v })
    }
    fn tuple_variant<V: Visitor<'de>>(self, _len: usize, visitor: V) -> Result<V::Value> {
        let v = self.value.unwrap_or(Value::Nil);
        de::Deserializer::deserialize_seq(Deserializer { vm: self.vm, value: v }, visitor)
    }
    fn struct_variant<V: Visitor<'de>>(self, _fields: &'static [&'static str], visitor: V) -> Result<V::Value> {
        let v = self.value.unwrap_or(Value::Nil);
        de::Deserializer::deserialize_map(Deserializer { vm: self.vm, value: v }, visitor)
    }
}
