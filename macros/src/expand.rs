//! What the two macros generate.
//!
//! Kept apart from `lib.rs` so that it can be built as ordinary code: a proc-macro crate
//! exports nothing but its macros, so `tests/expand.rs` includes this module directly
//! (`#[path = "../src/expand.rs"]`) and reads the generated tokens as text. Nothing here may
//! name the `proc_macro` crate for that reason — `proc_macro2` throughout.

use proc_macro2::{Ident, Span, TokenStream};
use quote::{format_ident, quote, quote_spanned};
use syn::spanned::Spanned;
use syn::{DeriveInput, Error, FnArg, ImplItem, ItemImpl, LitStr, Type};

/// `#[derive(RubyClass)]`.
pub fn derive_ruby_class(input: TokenStream) -> TokenStream {
    match derive_ruby_class_inner(input) {
        Ok(ts) => ts,
        Err(e) => e.to_compile_error(),
    }
}

/// `#[ruby_methods]`. The impl block is given back whatever happens, so that an error in one
/// signature does not also make every call of every method in it undefined.
pub fn ruby_methods(attr: TokenStream, item: TokenStream) -> TokenStream {
    match ruby_methods_inner(attr, item.clone()) {
        Ok(ts) => ts,
        Err(e) => {
            let err = e.to_compile_error();
            quote! { #item #err }
        }
    }
}

// ---------------------------------------------------------------- #[ruby(…)]

/// The options of a `#[ruby(…)]` attribute: `name` on both, `skip` on a method.
#[derive(Default)]
struct RubyAttr {
    name: Option<String>,
    skip: bool,
}

fn ruby_attr(attrs: &[syn::Attribute], on_method: bool) -> syn::Result<RubyAttr> {
    let mut out = RubyAttr::default();
    for a in attrs.iter().filter(|a| a.path().is_ident("ruby")) {
        a.parse_nested_meta(|meta| {
            if meta.path.is_ident("name") {
                let s: LitStr = meta.value()?.parse()?;
                out.name = Some(s.value());
                Ok(())
            } else if on_method && meta.path.is_ident("skip") {
                out.skip = true;
                Ok(())
            } else if on_method {
                Err(meta.error("unknown option: #[ruby(name = \"…\")] and #[ruby(skip)] are the ones there are"))
            } else {
                Err(meta.error("unknown option: #[ruby(name = \"…\")] is the one there is"))
            }
        })?;
    }
    Ok(out)
}

// ---------------------------------------------------------------- derive

fn derive_ruby_class_inner(input: TokenStream) -> syn::Result<TokenStream> {
    let ast: DeriveInput = syn::parse2(input)?;
    if !ast.generics.params.is_empty() {
        return Err(Error::new_spanned(
            &ast.generics,
            "#[derive(RubyClass)] does not take a generic type: one Ruby class stands for one Rust type, and the Data tag is per type",
        ));
    }
    let ident = &ast.ident;
    let name = ruby_attr(&ast.attrs, false)?.name.unwrap_or_else(|| ident.to_string());
    Ok(quote! {
        impl ::sabiruby::host_store::RubyClass for #ident {
            const NAME: &'static str = #name;
        }
        impl ::sabiruby::convert::IntoRuby for #ident {
            fn into_ruby(self, vm: &mut ::sabiruby::Vm) -> ::sabiruby::Value {
                <Self as ::sabiruby::host_store::RubyClass>::into_handle(self, vm)
            }
        }
    })
}

// ---------------------------------------------------------------- #[ruby_methods]

/// What the function does with `self`.
#[derive(PartialEq)]
enum Recv {
    /// No `self` at all: a class method (`Player.new`).
    None,
    /// `&self`: the value is read.
    Ref,
    /// `&mut self`: the value is changed.
    Mut,
}

struct MethodSpec {
    ruby_name: String,
    rust_name: Ident,
    recv: Recv,
    takes_vm: bool,
    takes_block: bool,
    args: Vec<(Ident, Type)>,
}

/// The most arguments `Vm::define_fn` has a shape for.
const MAX_ARGS: usize = 6;

fn ruby_methods_inner(attr: TokenStream, item: TokenStream) -> syn::Result<TokenStream> {
    if !attr.is_empty() {
        return Err(Error::new(Span::call_site(), "#[ruby_methods] takes no arguments"));
    }
    let mut block: ItemImpl = syn::parse2(item)?;
    if let Some((_, path, _)) = &block.trait_ {
        return Err(Error::new_spanned(path, "#[ruby_methods] reads an inherent impl block, not a trait implementation"));
    }
    if !block.generics.params.is_empty() {
        return Err(Error::new_spanned(&block.generics, "#[ruby_methods] does not take a generic impl block"));
    }
    let self_ty = (*block.self_ty).clone();
    if !matches!(self_ty, Type::Path(_)) {
        return Err(Error::new_spanned(&self_ty, "#[ruby_methods] wants a named type, the one #[derive(RubyClass)] is on"));
    }

    // Read every `fn`, and take the `#[ruby(…)]` attributes off the block that is given back:
    // they are ours, and an inherent method may not carry an unknown attribute.
    let mut specs = Vec::new();
    for it in block.items.iter_mut() {
        let f = match it {
            ImplItem::Fn(f) => f,
            _ => continue,
        };
        let opts = ruby_attr(&f.attrs, true)?;
        f.attrs.retain(|a| !a.path().is_ident("ruby"));
        if opts.skip {
            continue;
        }
        specs.push(method_spec(f, opts)?);
    }

    let class_methods: Vec<TokenStream> =
        specs.iter().filter(|s| s.recv == Recv::None).map(|s| define(s, &self_ty, true)).collect();
    let instance_methods: Vec<TokenStream> =
        specs.iter().filter(|s| s.recv != Recv::None).map(|s| define(s, &self_ty, false)).collect();
    // Without this, a parameter of a type `FromRuby` does not cover fails as
    // "the trait bound `{closure@…}: RubyFn<_>` is not satisfied", pointing at the attribute.
    // Asked for here, at the span of the type the host wrote, it names the type and the line.
    let arg_types: Vec<TokenStream> = specs
        .iter()
        .flat_map(|s| s.args.iter())
        .map(|(_, ty)| quote_spanned! {ty.span()=> __arg::<#ty>(); })
        .collect();
    let assert_args = if arg_types.is_empty() {
        quote! {}
    } else {
        quote! {
            /// Every argument type is one `FromRuby` covers. Never called.
            #[allow(dead_code)]
            fn __ruby_argument_types(){
                fn __arg<T: ::sabiruby::convert::FromRuby>() {}
                #(#arg_types)*
            }
        }
    };

    let meta = if class_methods.is_empty() {
        quote! {}
    } else {
        quote! { let __meta = vm.singleton_class(::sabiruby::Value::Obj(__class))?; }
    };

    Ok(quote! {
        #block

        impl #self_ty {
            /// Defines the Ruby class, its `Data` tag and store, and every method of this
            /// `impl` block, and answers the class. Generated by `#[ruby_methods]`; calling it
            /// twice is harmless. It is a `VmResult` because asking a class for its singleton
            /// class is one — the answer for a class object is always `Ok`, but the generated
            /// code does not get to say so with an `expect`.
            pub fn register(vm: &mut ::sabiruby::Vm) -> ::sabiruby::error::VmResult<::sabiruby::value::ObjId> {
                let __class = <#self_ty as ::sabiruby::host_store::RubyClass>::register_class(vm);
                let _ = <#self_ty as ::sabiruby::host_store::RubyClass>::tag(vm);
                #meta
                #(#class_methods)*
                #(#instance_methods)*
                Ok(__class)
            }

            #assert_args
        }
    })
}

/// Reads one `fn`: what it does with `self`, whether it wants the VM, and what is left over as
/// the Ruby arguments.
fn method_spec(f: &syn::ImplItemFn, opts: RubyAttr) -> syn::Result<MethodSpec> {
    let sig = &f.sig;
    if sig.asyncness.is_some() {
        return Err(Error::new_spanned(sig.asyncness, "an async fn cannot be a Ruby method: the VM calls it and waits for the answer"));
    }
    if sig.unsafety.is_some() {
        return Err(Error::new_spanned(sig.unsafety, "an unsafe fn cannot be a Ruby method"));
    }
    if !sig.generics.params.is_empty() {
        return Err(Error::new_spanned(&sig.generics, "a generic fn cannot be a Ruby method: the call has one set of types, decided here"));
    }
    if let Some(v) = &sig.variadic {
        return Err(Error::new_spanned(v, "a variadic fn cannot be a Ruby method"));
    }

    let mut inputs = sig.inputs.iter();
    let mut recv = Recv::None;
    let mut rest: Vec<&syn::PatType> = Vec::new();
    match inputs.next() {
        Some(FnArg::Receiver(r)) => {
            if r.reference.is_none() {
                return Err(Error::new_spanned(r, "a method that takes `self` by value is not supported: the value lives in the host's store and the Ruby object would be left naming nothing (take `&self` or `&mut self`)"));
            }
            recv = if r.mutability.is_some() { Recv::Mut } else { Recv::Ref };
        }
        Some(FnArg::Typed(t)) => rest.push(t),
        None => {}
    }
    for a in inputs {
        match a {
            FnArg::Typed(t) => rest.push(t),
            FnArg::Receiver(r) => return Err(Error::new_spanned(r, "`self` comes first")),
        }
    }

    // A leading `&mut Vm` is context rather than an argument, as it is in `Vm::define_fn`.
    let takes_vm = rest.first().is_some_and(|t| is_vm(&t.ty));
    if takes_vm {
        rest.remove(0);
    }
    if rest.iter().any(|t| is_vm(&t.ty)) {
        return Err(Error::new_spanned(sig, "`&mut Vm` comes first (after `self`), where `Vm::define_fn` reads it as the call's context"));
    }
    // A trailing `Block` is the block the caller passed, not an argument — the same place
    // `Vm::define_fn` reads it. `Vm::define_fn` only has that shape together with the `&mut Vm`,
    // which is also the only way the method could call the block.
    let takes_block = rest.last().is_some_and(|t| is_block(&t.ty));
    if takes_block {
        rest.pop();
        if !takes_vm {
            return Err(Error::new_spanned(sig, "a method that takes the `Block` takes the `&mut Vm` too (first, after `self`): calling the block needs it, and `Vm::define_fn` has no shape without it"));
        }
    }
    if let Some(t) = rest.iter().find(|t| is_block(&t.ty)) {
        return Err(Error::new_spanned(t, "`Block` comes last, where `Vm::define_fn` reads it as the block the caller passed"));
    }
    if rest.len() > MAX_ARGS {
        return Err(Error::new_spanned(sig, "a Ruby method takes at most six arguments here; one that wants more takes the raw call with `Vm::define_closure`"));
    }

    let args = rest
        .iter()
        .enumerate()
        .map(|(i, t)| (format_ident!("__a{}", i), (*t.ty).clone()))
        .collect();
    Ok(MethodSpec {
        ruby_name: opts.name.unwrap_or_else(|| sig.ident.to_string()),
        rust_name: sig.ident.clone(),
        recv,
        takes_vm,
        takes_block,
        args,
    })
}

/// Whether a parameter is spelled `Block` (however the path to it is written).
fn is_block(ty: &Type) -> bool {
    match ty {
        Type::Path(p) => p.path.segments.last().is_some_and(|s| s.ident == "Block"),
        _ => false,
    }
}

/// Whether a parameter is spelled `&mut Vm` (however the path to `Vm` is written).
fn is_vm(ty: &Type) -> bool {
    match ty {
        Type::Reference(r) if r.mutability.is_some() => match &*r.elem {
            Type::Path(p) => p.path.segments.last().is_some_and(|s| s.ident == "Vm"),
            _ => false,
        },
        _ => false,
    }
}

/// One `Vm::define_fn` call: the closure takes the arguments the Rust signature declares, so
/// `FromRuby`, `IntoRuby` and the arity all come from `define_fn` rather than from here.
fn define(m: &MethodSpec, self_ty: &Type, class_method: bool) -> TokenStream {
    let ruby_name = &m.ruby_name;
    let rust_name = &m.rust_name;
    let names: Vec<&Ident> = m.args.iter().map(|(n, _)| n).collect();
    let types: Vec<&Type> = m.args.iter().map(|(_, t)| t).collect();
    let params = quote! { #(, #names: #types)* };
    // the block comes last in `Vm::define_fn`'s parameter list, after the arguments
    let blk_param = if m.takes_block { quote! { , __blk: ::sabiruby::convert::Block } } else { quote! {} };
    let blk_arg = if m.takes_block { quote! { , __blk } } else { quote! {} };
    let ret = quote! { -> ::sabiruby::error::VmResult<::sabiruby::Value> };

    if class_method {
        let call = if m.takes_vm {
            quote! { <#self_ty>::#rust_name(vm #(, #names)* #blk_arg) }
        } else {
            quote! { <#self_ty>::#rust_name(#(#names),*) }
        };
        return quote! {
            vm.define_fn(__meta, #ruby_name, |vm: &mut ::sabiruby::Vm #params #blk_param| #ret {
                let __out = #call;
                ::sabiruby::convert::IntoRubyRet::into_ruby_ret(__out, vm)
            });
        };
    }

    let this = quote! { __this: ::sabiruby::convert::This<::sabiruby::Value> };
    let body = if m.takes_vm {
        // The value cannot stay borrowed from the store while the VM it is stored in is also
        // the method's: it moves out for the call and back afterwards, its handle reserved.
        let take = if m.recv == Recv::Mut {
            quote! { let (__handle, mut __recv) = <#self_ty as ::sabiruby::host_store::RubyClass>::take_out(vm, __this.0)?; }
        } else {
            quote! { let (__handle, __recv) = <#self_ty as ::sabiruby::host_store::RubyClass>::take_out(vm, __this.0)?; }
        };
        let borrow = if m.recv == Recv::Mut { quote! { &mut __recv } } else { quote! { &__recv } };
        quote! {
            #take
            let __out = <#self_ty>::#rust_name(#borrow, vm #(, #names)* #blk_arg);
            <#self_ty as ::sabiruby::host_store::RubyClass>::give_back(vm, __handle, __recv);
        }
    } else {
        let borrow = if m.recv == Recv::Mut { quote! { borrow_mut } } else { quote! { borrow } };
        quote! {
            let __out = {
                let __recv = <#self_ty as ::sabiruby::host_store::RubyClass>::#borrow(vm, __this.0)?;
                <#self_ty>::#rust_name(__recv #(, #names)*)
            };
        }
    };
    quote! {
        vm.define_fn(__class, #ruby_name, |vm: &mut ::sabiruby::Vm, #this #params #blk_param| #ret {
            #body
            ::sabiruby::convert::IntoRubyRet::into_ruby_ret(__out, vm)
        });
    }
}
