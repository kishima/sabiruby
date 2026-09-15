//! Ruby's `JSON`, written in Rust on top of `serde_json`.
//!
//! It is installed by [`install_json`] and by nothing else: the VM has no JSON of its own and
//! never links `serde_json`, so a host that does not want it pays nothing for it. The class is
//! not a port of the unofficial mruby-json gem and does not try to be compatible with it — it
//! is CRuby's `JSON` as far as the methods below go, which is what a script is likely to
//! expect (`docs/design/serde.md`).

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use sabiruby::convert::{Bytes, This};
use sabiruby::error::{VmError, VmResult};
use sabiruby::value::{ObjId, Value};
use sabiruby::Vm;

use crate::to_value;

/// CRuby's `JSON.generate` stops at 100 levels; so does this, for the same reason — a Hash or
/// an Array that contains itself would otherwise take the host's stack down with it.
const MAX_NESTING: usize = 100;

/// Defines `JSON.parse`, `JSON.generate`, `JSON.pretty_generate` and `Object#to_json` in `vm`,
/// with `JSON::ParserError` and `JSON::GeneratorError` under `StandardError`.
///
/// ```no_run
/// # fn main() -> Result<(), sabiruby::VmError> {
/// let mut vm = sabiruby::Vm::with_mrblib()?;
/// sabiruby_serde::install_json(&mut vm);
/// # Ok(()) }
/// ```
///
/// Calling it twice is harmless: the module and the classes are looked up before they are
/// made, and the methods are redefined with the same bodies.
pub fn install_json(vm: &mut Vm) {
    let json = vm.define_module("JSON");
    let parser_error = vm.define_class_under(json, "ParserError", vm.core.standard_error);
    let generator_error = vm.define_class_under(json, "GeneratorError", vm.core.standard_error);
    let meta = vm.singleton_class(Value::Obj(json)).expect("a module has a metaclass");

    vm.define_fn(meta, "parse", move |vm: &mut Vm, src: Bytes| -> VmResult<Value> {
        parse(vm, &src.0, parser_error)
    });
    vm.define_fn(meta, "generate", move |vm: &mut Vm, v: Value| -> VmResult<Value> {
        let j = ruby_to_json(vm, v, generator_error, 0)?;
        let s = serde_json::to_string(&j).map_err(|e| vm.raise(generator_error, &e.to_string()))?;
        Ok(vm.str_from(s))
    });
    vm.define_fn(meta, "pretty_generate", move |vm: &mut Vm, v: Value| -> VmResult<Value> {
        let j = ruby_to_json(vm, v, generator_error, 0)?;
        let s = serde_json::to_string_pretty(&j).map_err(|e| vm.raise(generator_error, &e.to_string()))?;
        Ok(vm.str_from(s))
    });
    vm.define_fn(vm.core.object, "to_json", move |vm: &mut Vm, this: This<Value>| -> VmResult<Value> {
        let j = ruby_to_json(vm, this.0, generator_error, 0)?;
        let s = serde_json::to_string(&j).map_err(|e| vm.raise(generator_error, &e.to_string()))?;
        Ok(vm.str_from(s))
    });
}

/// `JSON.parse`. The text becomes a `serde_json::Value` and that becomes a Ruby value through
/// [`to_value`], so the mapping is the crate's own and needs no second table: object → Hash
/// with String keys, array → Array, number → Integer or Float, `null` → nil.
fn parse(vm: &mut Vm, src: &[u8], parser_error: ObjId) -> VmResult<Value> {
    let text = match core::str::from_utf8(src) {
        Ok(s) => s,
        Err(_) => return Err(vm.raise(parser_error, "source is not valid UTF-8")),
    };
    let j: serde_json::Value = match serde_json::from_str(text) {
        Ok(j) => j,
        Err(e) => return Err(vm.raise(parser_error, &e.to_string())),
    };
    to_value(vm, &j)
}

/// A Ruby value as a `serde_json::Value`.
///
/// Written out rather than run through [`from_value`](crate::from_value) into a
/// `serde_json::Value`, because generating JSON is not the same question as deserializing: a
/// key here is whatever `to_s` says (`{1 => 2}` is `{"1":2}`, as CRuby's JSON does it), and an
/// object of any other class is its `to_s`, which serde's data model has no way to ask for.
fn ruby_to_json(vm: &mut Vm, v: Value, err: ObjId, depth: usize) -> VmResult<serde_json::Value> {
    if depth > MAX_NESTING {
        return Err(vm.raise(err, &alloc::format!("nesting of {depth} is too deep")));
    }
    match v {
        Value::Nil => Ok(serde_json::Value::Null),
        Value::True => Ok(serde_json::Value::Bool(true)),
        Value::False => Ok(serde_json::Value::Bool(false)),
        Value::Int(i) => Ok(serde_json::Value::Number(i.into())),
        Value::Float(f) => match serde_json::Number::from_f64(f) {
            Some(n) => Ok(serde_json::Value::Number(n)),
            None => Err(vm.raise(err, &alloc::format!("{f} not allowed in JSON"))),
        },
        Value::Sym(s) => { let n = vm.sym_name(s); Ok(serde_json::Value::String(n)) }
        Value::Obj(_) => {
            // a wide Integer: JSON's own grammar has no bound, but serde_json's Number does
            if let Some(b) = vm.as_bigint(v).filter(|_| vm.is_bigint(v)) {
                return match b.to_i64().map(serde_json::Value::from).or_else(|| b.to_u64().map(serde_json::Value::from)) {
                    Some(n) => Ok(n),
                    None => Err(vm.raise(err, "Integer is too wide for JSON")),
                };
            }
            if let Some(bytes) = vm.str_bytes(v) {
                return match core::str::from_utf8(bytes) {
                    Ok(s) => { let s = s.to_string(); Ok(serde_json::Value::String(s)) }
                    Err(_) => Err(vm.raise(err, "source sequence is illegal/malformed utf-8")),
                };
            }
            if let Some(items) = vm.ary_vals(v) {
                let mut out = Vec::with_capacity(items.len());
                for it in items { out.push(ruby_to_json(vm, it, err, depth + 1)?); }
                return Ok(serde_json::Value::Array(out));
            }
            if let Some(entries) = vm.hash_entries(v) {
                let mut map = serde_json::Map::with_capacity(entries.len());
                for (k, val) in entries {
                    let key = json_key(vm, k, err)?;
                    map.insert(key, ruby_to_json(vm, val, err, depth + 1)?);
                }
                return Ok(serde_json::Value::Object(map));
            }
            // anything else is its `to_s`, which is what CRuby's `Object#to_json` does
            let s = to_s(vm, v)?;
            Ok(serde_json::Value::String(s))
        }
    }
}

/// A Hash key as a JSON name: JSON has only String names, so everything goes through `to_s`,
/// as CRuby's JSON does.
fn json_key(vm: &mut Vm, k: Value, err: ObjId) -> VmResult<String> {
    match k {
        Value::Sym(s) => Ok(vm.sym_name(s)),
        Value::Obj(_) if vm.str_bytes(k).is_some() => {
            let bytes = vm.str_bytes(k).expect("checked").to_vec();
            match String::from_utf8(bytes) {
                Ok(s) => Ok(s),
                Err(_) => Err(vm.raise(err, "source sequence is illegal/malformed utf-8")),
            }
        }
        _ => to_s(vm, k),
    }
}

fn to_s(vm: &mut Vm, v: Value) -> VmResult<String> {
    let mid = vm.intern("to_s");
    let s = vm.funcall(v, mid, &[], Value::Nil)?;
    match vm.str_bytes(s) {
        Some(b) => Ok(String::from_utf8_lossy(b).into_owned()),
        None => Err(VmError::Internal("to_s did not answer with a String".into())),
    }
}
