//! Typed registration: Rust functions as Ruby methods, without writing out the
//! `fn(&mut Vm, self, args, block) -> VmResult<Value>` shape by hand.
//!
//! [`Vm::define_method`](crate::Vm::define_method) and
//! [`Vm::define_closure`](crate::Vm::define_closure) hand the raw call over: the host reads
//! `args` itself, checks their number and their types, and builds a [`Value`] to answer with.
//! [`Vm::define_fn`] does that part, from the Rust signature:
//!
//! ```
//! # fn main() -> Result<(), sabiruby::VmError> {
//! let mut vm = sabiruby::Vm::with_mrblib()?;
//! let object = vm.core.object;
//! vm.define_fn(object, "add", |a: i64, b: i64| a + b);
//! # Ok(()) }
//! ```
//!
//! The arguments are converted with [`FromRuby`] and the answer with [`IntoRuby`], their number
//! is checked (`wrong number of arguments (given 1, expected 2)`, as anywhere else in the VM),
//! and `Method#arity` answers with it — which a closure registered by hand cannot do, having no
//! declared argument list.
//!
//! # The shapes a function may have
//!
//! The first parameters are read as the call's context rather than as arguments, in this order:
//!
//! * `&mut Vm` — the VM, to allocate, call back into Ruby, or raise.
//! * [`This<T>`] — the receiver (`self`), converted with `T: FromRuby`.
//!
//! and a trailing [`Block`] parameter (only in the forms that take a `&mut Vm`, since calling a
//! block needs one) takes the block the caller passed. Everything between them is an ordinary
//! argument, up to six of them.
//!
//! ```
//! # fn main() -> Result<(), sabiruby::VmError> {
//! use sabiruby::convert::{Block, This};
//! use sabiruby::{Value, Vm};
//! let mut vm = Vm::with_mrblib()?;
//! let c = vm.define_class("Greeter", vm.core.object);
//! vm.define_fn(c, "greet", |vm: &mut Vm, name: String| format!("hi {name}"));
//! vm.define_fn(c, "id", |this: This<Value>| this.0);
//! vm.define_fn(c, "twice", |vm: &mut Vm, blk: Block| -> sabiruby::error::VmResult<Value> {
//!     match blk.0 { Some(b) => { vm.call_block(b, &[Value::Int(1)])?; vm.call_block(b, &[Value::Int(2)]) } None => Ok(Value::Nil) }
//! });
//! # Ok(()) }
//! ```
//!
//! # What the function may answer with
//!
//! Any `T: IntoRuby`, or a `Result` of one: `Result<T, VmError>` re-raises (this is how a typed
//! function raises a chosen exception class, with [`Vm::raise`](crate::Vm::raise) and friends),
//! while `Result<T, String>` and `Result<T, &'static str>` raise a `RuntimeError` carrying the
//! message, for a function that has no `&mut Vm` to build an exception with.
//!
//! # What it does not do
//!
//! Optional and rest arguments, keyword arguments and typed blocks are not covered: a method
//! that wants them takes the raw call with `define_closure`. A captured [`Value`] is not a GC
//! root here any more than it is there (`docs/gc.md`).

use alloc::{boxed::Box, string::String, vec::Vec};

use crate::error::{VmError, VmResult};
use crate::symbol::Sym;
use crate::value::{ObjId, Value};
use crate::vm::Vm;

// ---------------------------------------------------------------- FromRuby

/// A Rust type a Ruby [`Value`] can be read as: the argument side of [`Vm::define_fn`].
///
/// A conversion that cannot be made raises the same exception the VM's own natives raise for
/// the same mistake — `TypeError: no implicit conversion of nil into Integer`, and so on.
pub trait FromRuby: Sized {
    fn from_ruby(vm: &mut Vm, v: Value) -> VmResult<Self>;
}

impl FromRuby for Value {
    fn from_ruby(_vm: &mut Vm, v: Value) -> VmResult<Value> { Ok(v) }
}

impl FromRuby for i64 {
    fn from_ruby(vm: &mut Vm, v: Value) -> VmResult<i64> { vm.expect_int(v, "Integer") }
}

/// Out of range raises `RangeError: integer out of range`, as a too-wide Integer does
/// anywhere else in the VM (`mrb_bint_as_int`).
impl FromRuby for i32 {
    fn from_ruby(vm: &mut Vm, v: Value) -> VmResult<i32> {
        let i = vm.expect_int(v, "Integer")?;
        match i32::try_from(i) {
            Ok(x) => Ok(x),
            Err(_) => Err(vm.raise(vm.core.range_error, "integer out of range")),
        }
    }
}

/// `mrb_ensure_float_type`: an Integer widens, anything else is a `TypeError`.
impl FromRuby for f64 {
    fn from_ruby(vm: &mut Vm, v: Value) -> VmResult<f64> {
        match v {
            Value::Float(f) => Ok(f),
            Value::Int(i) => Ok(i as f64),
            Value::Obj(o) if vm.heap.bigint(o).is_some() => Ok(vm.heap.bigint(o).unwrap().to_f64()),
            _ => Err(crate::builtins::numeric::coerce_fail(vm, v, "")),
        }
    }
}

/// Every value is a `bool`: only `nil` and `false` are false, as in Ruby. Never raises.
impl FromRuby for bool {
    fn from_ruby(_vm: &mut Vm, v: Value) -> VmResult<bool> { Ok(v.truthy()) }
}

/// A String, lossily as UTF-8 (the VM keeps strings as bytes; [`Bytes`] gets them unchanged).
/// Anything else is a `TypeError`, as `mrb_get_args`'s `S` is — `to_s` is *not* called, so a
/// function that wants that coercion takes a [`Value`] and calls
/// [`Vm::as_string`](crate::Vm::as_string) itself.
impl FromRuby for String {
    fn from_ruby(vm: &mut Vm, v: Value) -> VmResult<String> {
        let b = str_bytes(vm, v)?;
        Ok(alloc::string::String::from_utf8_lossy(&b).into_owned())
    }
}

/// [`Vm::expect_str`](crate::Vm::expect_str) with the value's own type in the message, which is
/// what a conversion asked for by type (rather than by the name of a parameter) can say.
fn str_bytes(vm: &mut Vm, v: Value) -> VmResult<Vec<u8>> {
    match vm.str_bytes(v) {
        Some(b) => Ok(b.to_vec()),
        None => { let d = vm.describe_for_type_error(v); Err(vm.raise_type(&alloc::format!("{d} cannot be converted to String"))) }
    }
}

impl FromRuby for Sym {
    fn from_ruby(vm: &mut Vm, v: Value) -> VmResult<Sym> {
        crate::builtins::object::sym_arg(vm, v)
    }
}

/// `nil` is `None`; anything else is converted.
impl<T: FromRuby> FromRuby for Option<T> {
    fn from_ruby(vm: &mut Vm, v: Value) -> VmResult<Option<T>> {
        if v.is_nil() { Ok(None) } else { Ok(Some(T::from_ruby(vm, v)?)) }
    }
}

/// An Array, element by element. Not an Array is a `TypeError`.
impl<T: FromRuby> FromRuby for Vec<T> {
    fn from_ruby(vm: &mut Vm, v: Value) -> VmResult<Vec<T>> {
        let items = match vm.ary_vals(v) {
            Some(items) => items,
            None => return Err(vm.raise_type("no implicit conversion into Array")),
        };
        let mut out = Vec::with_capacity(items.len());
        for it in items { out.push(T::from_ruby(vm, it)?); }
        Ok(out)
    }
}

/// The bytes of a String, as the VM holds them.
///
/// `Vec<u8>` itself reads an Array of Integers (that is what `Vec<T>` means above), so the byte
/// string has a name of its own; it is the answer side too, where it builds a String rather
/// than an Array.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bytes(pub Vec<u8>);

impl FromRuby for Bytes {
    fn from_ruby(vm: &mut Vm, v: Value) -> VmResult<Bytes> {
        Ok(Bytes(str_bytes(vm, v)?))
    }
}

/// The receiver of the call (`self`), in the parameter list of a [`Vm::define_fn`] function.
///
/// `This<Value>` takes it untouched; `This<T>` converts it like an argument, so a function on an
/// Integer writes `This<i64>` and gets the number.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct This<T>(pub T);

impl<T> core::ops::Deref for This<T> {
    type Target = T;
    fn deref(&self) -> &T { &self.0 }
}

/// The block the caller passed, `None` when there was none. Last in the parameter list.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Block(pub Option<Value>);

// ---------------------------------------------------------------- IntoRuby

/// A Rust type that becomes a Ruby [`Value`]: the answer side of [`Vm::define_fn`].
pub trait IntoRuby {
    fn into_ruby(self, vm: &mut Vm) -> Value;
}

impl IntoRuby for Value {
    fn into_ruby(self, _vm: &mut Vm) -> Value { self }
}
/// `nil`: what a function that answers nothing gives Ruby.
impl IntoRuby for () {
    fn into_ruby(self, _vm: &mut Vm) -> Value { Value::Nil }
}
impl IntoRuby for bool {
    fn into_ruby(self, _vm: &mut Vm) -> Value { Value::bool(self) }
}
impl IntoRuby for Sym {
    fn into_ruby(self, _vm: &mut Vm) -> Value { Value::Sym(self) }
}
impl IntoRuby for f64 {
    fn into_ruby(self, _vm: &mut Vm) -> Value { Value::Float(self) }
}
impl IntoRuby for f32 {
    fn into_ruby(self, _vm: &mut Vm) -> Value { Value::Float(self as f64) }
}
impl IntoRuby for i64 {
    fn into_ruby(self, _vm: &mut Vm) -> Value { Value::Int(self) }
}
macro_rules! into_ruby_int {
    ($($t:ty),*) => { $(impl IntoRuby for $t {
        fn into_ruby(self, _vm: &mut Vm) -> Value { Value::Int(self as i64) }
    })* };
}
into_ruby_int!(i8, i16, i32, u8, u16, u32);
/// A `u64` too wide for an `i64` becomes a wide Integer (`BigInt`), as Ruby has no bound.
impl IntoRuby for u64 {
    fn into_ruby(self, vm: &mut Vm) -> Value {
        match i64::try_from(self) {
            Ok(i) => Value::Int(i),
            Err(_) => { let b = crate::bigint::BigInt::from_u64(self); vm.bint_value(b) }
        }
    }
}
impl IntoRuby for usize {
    fn into_ruby(self, vm: &mut Vm) -> Value { (self as u64).into_ruby(vm) }
}
impl IntoRuby for &str {
    fn into_ruby(self, vm: &mut Vm) -> Value { vm.str_new(self.as_bytes()) }
}
impl IntoRuby for String {
    fn into_ruby(self, vm: &mut Vm) -> Value { vm.str_from(self) }
}
impl IntoRuby for Bytes {
    fn into_ruby(self, vm: &mut Vm) -> Value { vm.str_new(&self.0) }
}
/// `None` is `nil`.
impl<T: IntoRuby> IntoRuby for Option<T> {
    fn into_ruby(self, vm: &mut Vm) -> Value {
        match self { Some(t) => t.into_ruby(vm), None => Value::Nil }
    }
}
/// An Array.
impl<T: IntoRuby> IntoRuby for Vec<T> {
    fn into_ruby(self, vm: &mut Vm) -> Value {
        let mut out = Vec::with_capacity(self.len());
        for t in self { out.push(t.into_ruby(vm)); }
        vm.ary_new(out)
    }
}

macro_rules! into_ruby_tuple {
    ($(($t:ident, $i:tt)),+) => {
        /// An Array of the parts.
        impl<$($t: IntoRuby),+> IntoRuby for ($($t,)+) {
            fn into_ruby(self, vm: &mut Vm) -> Value {
                let v = alloc::vec![$(self.$i.into_ruby(vm)),+];
                vm.ary_new(v)
            }
        }
    };
}
into_ruby_tuple!((A, 0), (B, 1));
into_ruby_tuple!((A, 0), (B, 1), (C, 2));
into_ruby_tuple!((A, 0), (B, 1), (C, 2), (D, 3));
into_ruby_tuple!((A, 0), (B, 1), (C, 2), (D, 3), (E, 4));
into_ruby_tuple!((A, 0), (B, 1), (C, 2), (D, 3), (E, 4), (F, 5));

// ------------------------------------------------- what a function may answer with

/// How a function's Rust return type reaches Ruby: a plain value, or a `Result` that may raise.
///
/// The marker parameter is what keeps the four cases apart for the compiler (a blanket impl for
/// every `T: IntoRuby` and one for `Result<T, _>` would otherwise overlap); it is inferred and
/// never written out.
pub trait IntoRubyRet<M> {
    #[doc(hidden)]
    fn into_ruby_ret(self, vm: &mut Vm) -> VmResult<Value>;
}

/// Marker of [`IntoRubyRet`]: a plain `T: IntoRuby`.
#[doc(hidden)]
pub struct RetValue;
/// Marker of [`IntoRubyRet`]: `Result<T, VmError>`, whose `Err` is re-raised as it is.
#[doc(hidden)]
pub struct RetVmResult;
/// Marker of [`IntoRubyRet`]: `Result<T, String>`, whose `Err` becomes a `RuntimeError`.
#[doc(hidden)]
pub struct RetString;
/// Marker of [`IntoRubyRet`]: `Result<T, &'static str>`, whose `Err` becomes a `RuntimeError`.
#[doc(hidden)]
pub struct RetStr;

impl<T: IntoRuby> IntoRubyRet<RetValue> for T {
    fn into_ruby_ret(self, vm: &mut Vm) -> VmResult<Value> { Ok(self.into_ruby(vm)) }
}
impl<T: IntoRuby> IntoRubyRet<RetVmResult> for Result<T, VmError> {
    fn into_ruby_ret(self, vm: &mut Vm) -> VmResult<Value> { Ok(self?.into_ruby(vm)) }
}
impl<T: IntoRuby> IntoRubyRet<RetString> for Result<T, String> {
    fn into_ruby_ret(self, vm: &mut Vm) -> VmResult<Value> {
        match self { Ok(t) => Ok(t.into_ruby(vm)), Err(e) => Err(vm.raise(vm.core.runtime_error, &e)) }
    }
}
impl<T: IntoRuby> IntoRubyRet<RetStr> for Result<T, &'static str> {
    fn into_ruby_ret(self, vm: &mut Vm) -> VmResult<Value> {
        match self { Ok(t) => Ok(t.into_ruby(vm)), Err(e) => Err(vm.raise(vm.core.runtime_error, e)) }
    }
}

// ---------------------------------------------------------------- RubyFn

/// A Rust function [`Vm::define_fn`] can register, whatever shape it has.
///
/// `M` is a marker the compiler infers from the function's own signature (which context
/// parameters it takes, how many arguments, what it answers with); it is never written out.
/// There is no reason to implement this trait: it is implemented for every `Fn` of a shape the
/// module describes.
pub trait RubyFn<M>: Send + Sync + 'static {
    /// The number of arguments, which becomes the method's `arity`.
    #[doc(hidden)]
    fn ruby_arity(&self) -> i64;
    #[doc(hidden)]
    fn call_ruby(&self, vm: &mut Vm, recv: Value, args: &[Value], blk: Value) -> VmResult<Value>;
}

/// Marker of [`RubyFn`]: `Fn(A…) -> R`.
#[doc(hidden)]
pub struct MArgs;
/// Marker of [`RubyFn`]: `Fn(This<S>, A…) -> R`.
#[doc(hidden)]
pub struct MThis;
/// Marker of [`RubyFn`]: `Fn(&mut Vm, A…) -> R`.
#[doc(hidden)]
pub struct MVm;
/// Marker of [`RubyFn`]: `Fn(&mut Vm, This<S>, A…) -> R`.
#[doc(hidden)]
pub struct MVmThis;
/// Marker of [`RubyFn`]: `Fn(&mut Vm, A…, Block) -> R`.
#[doc(hidden)]
pub struct MVmBlock;
/// Marker of [`RubyFn`]: `Fn(&mut Vm, This<S>, A…, Block) -> R`.
#[doc(hidden)]
pub struct MVmThisBlock;

/// The block argument as the host sees it: `nil` is "no block".
#[inline]
fn block_of(blk: Value) -> Block {
    Block(if blk.is_nil() { None } else { Some(blk) })
}

/// Six shapes of function per argument count. `$n` is the arity, and each `($t, $i)` an argument
/// type and its place in `args` (a running counter would need a `mut` binding that the
/// zero-argument expansion would then not use).
macro_rules! ruby_fn_impls {
    ($n:literal; $(($t:ident, $i:literal)),*) => {
        impl<Fun, Ret, RM $(, $t)*> RubyFn<(MArgs, RM, ($($t,)*))> for Fun
        where
            Fun: Fn($($t),*) -> Ret + Send + Sync + 'static,
            $($t: FromRuby,)*
            Ret: IntoRubyRet<RM>,
        {
            fn ruby_arity(&self) -> i64 { $n }
            #[allow(non_snake_case)]
            fn call_ruby(&self, vm: &mut Vm, _recv: Value, args: &[Value], _blk: Value) -> VmResult<Value> {
                vm.check_argc(args, $n, $n)?;
                $(let $t = <$t as FromRuby>::from_ruby(vm, args[$i])?;)*
                (self)($($t),*).into_ruby_ret(vm)
            }
        }

        impl<Fun, Ret, RM, S $(, $t)*> RubyFn<(MThis, RM, (S, $($t,)*))> for Fun
        where
            Fun: Fn(This<S>, $($t),*) -> Ret + Send + Sync + 'static,
            S: FromRuby,
            $($t: FromRuby,)*
            Ret: IntoRubyRet<RM>,
        {
            fn ruby_arity(&self) -> i64 { $n }
            #[allow(non_snake_case)]
            fn call_ruby(&self, vm: &mut Vm, recv: Value, args: &[Value], _blk: Value) -> VmResult<Value> {
                vm.check_argc(args, $n, $n)?;
                let this = This(<S as FromRuby>::from_ruby(vm, recv)?);
                $(let $t = <$t as FromRuby>::from_ruby(vm, args[$i])?;)*
                (self)(this, $($t),*).into_ruby_ret(vm)
            }
        }

        impl<Fun, Ret, RM $(, $t)*> RubyFn<(MVm, RM, ($($t,)*))> for Fun
        where
            Fun: Fn(&mut Vm, $($t),*) -> Ret + Send + Sync + 'static,
            $($t: FromRuby,)*
            Ret: IntoRubyRet<RM>,
        {
            fn ruby_arity(&self) -> i64 { $n }
            #[allow(non_snake_case)]
            fn call_ruby(&self, vm: &mut Vm, _recv: Value, args: &[Value], _blk: Value) -> VmResult<Value> {
                vm.check_argc(args, $n, $n)?;
                $(let $t = <$t as FromRuby>::from_ruby(vm, args[$i])?;)*
                (self)(vm, $($t),*).into_ruby_ret(vm)
            }
        }

        impl<Fun, Ret, RM, S $(, $t)*> RubyFn<(MVmThis, RM, (S, $($t,)*))> for Fun
        where
            Fun: Fn(&mut Vm, This<S>, $($t),*) -> Ret + Send + Sync + 'static,
            S: FromRuby,
            $($t: FromRuby,)*
            Ret: IntoRubyRet<RM>,
        {
            fn ruby_arity(&self) -> i64 { $n }
            #[allow(non_snake_case)]
            fn call_ruby(&self, vm: &mut Vm, recv: Value, args: &[Value], _blk: Value) -> VmResult<Value> {
                vm.check_argc(args, $n, $n)?;
                let this = This(<S as FromRuby>::from_ruby(vm, recv)?);
                $(let $t = <$t as FromRuby>::from_ruby(vm, args[$i])?;)*
                (self)(vm, this, $($t),*).into_ruby_ret(vm)
            }
        }

        impl<Fun, Ret, RM $(, $t)*> RubyFn<(MVmBlock, RM, ($($t,)*))> for Fun
        where
            Fun: Fn(&mut Vm, $($t,)* Block) -> Ret + Send + Sync + 'static,
            $($t: FromRuby,)*
            Ret: IntoRubyRet<RM>,
        {
            fn ruby_arity(&self) -> i64 { $n }
            #[allow(non_snake_case)]
            fn call_ruby(&self, vm: &mut Vm, _recv: Value, args: &[Value], blk: Value) -> VmResult<Value> {
                vm.check_argc(args, $n, $n)?;
                $(let $t = <$t as FromRuby>::from_ruby(vm, args[$i])?;)*
                (self)(vm, $($t,)* block_of(blk)).into_ruby_ret(vm)
            }
        }

        impl<Fun, Ret, RM, S $(, $t)*> RubyFn<(MVmThisBlock, RM, (S, $($t,)*))> for Fun
        where
            Fun: Fn(&mut Vm, This<S>, $($t,)* Block) -> Ret + Send + Sync + 'static,
            S: FromRuby,
            $($t: FromRuby,)*
            Ret: IntoRubyRet<RM>,
        {
            fn ruby_arity(&self) -> i64 { $n }
            #[allow(non_snake_case)]
            fn call_ruby(&self, vm: &mut Vm, recv: Value, args: &[Value], blk: Value) -> VmResult<Value> {
                vm.check_argc(args, $n, $n)?;
                let this = This(<S as FromRuby>::from_ruby(vm, recv)?);
                $(let $t = <$t as FromRuby>::from_ruby(vm, args[$i])?;)*
                (self)(vm, this, $($t,)* block_of(blk)).into_ruby_ret(vm)
            }
        }
    };
}

ruby_fn_impls!(0;);
ruby_fn_impls!(1; (A1, 0));
ruby_fn_impls!(2; (A1, 0), (A2, 1));
ruby_fn_impls!(3; (A1, 0), (A2, 1), (A3, 2));
ruby_fn_impls!(4; (A1, 0), (A2, 1), (A3, 2), (A4, 3));
ruby_fn_impls!(5; (A1, 0), (A2, 1), (A3, 2), (A4, 3), (A5, 4));
ruby_fn_impls!(6; (A1, 0), (A2, 1), (A3, 2), (A4, 3), (A5, 4), (A6, 5));

impl Vm {
    /// Defines a method from a Rust function, converting its arguments and its answer.
    ///
    /// The shapes a function may have, and what it may answer with, are the
    /// [module's documentation](crate::convert). Unlike
    /// [`define_closure`](Vm::define_closure), the number of arguments is known here: it is
    /// checked on every call and `Method#arity` answers with it.
    ///
    /// ```
    /// # fn main() -> Result<(), sabiruby::VmError> {
    /// let mut vm = sabiruby::Vm::with_mrblib()?;
    /// let object = vm.core.object;
    /// vm.define_fn(object, "add", |a: i64, b: i64| a + b);
    /// vm.define_fn(object, "shout", |s: String| s.to_uppercase());
    /// # Ok(()) }
    /// ```
    ///
    /// A closure's parameter types have to be written out (`|a: i64|`, not `|a|`): they are what
    /// the conversion is chosen by.
    pub fn define_fn<M, F>(&mut self, class: ObjId, name: &str, f: F)
    where
        F: RubyFn<M>,
    {
        let arity = f.ruby_arity();
        let body: Box<dyn Fn(&mut Vm, Value, &[Value], Value) -> VmResult<Value> + Send + Sync> =
            Box::new(move |vm, recv, args, blk| f.call_ruby(vm, recv, args, blk));
        self.define_closure_body(class, name, arity, body);
    }
}
