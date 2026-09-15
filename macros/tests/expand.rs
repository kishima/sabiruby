//! What the macros generate, fixed as text.
//!
//! A proc-macro crate exports nothing but its macros, so `cargo expand` (which would need one)
//! is out; the expansion itself is ordinary code in `src/expand.rs`, included here directly and
//! called on source given as a string. Both sides of every comparison go through
//! `proc_macro2`'s own printer, so the expected code can be written out readably and only the
//! tokens are compared. Doc comments are dropped from both sides (a `///` in the expectation
//! and a `///` in a `quote!` print their string literal differently).
//!
//! These are here to say what the generated code *is*: `player.rs` is what it *does*.

#[path = "../src/expand.rs"]
mod expand;

use proc_macro2::{Delimiter, Group, TokenStream, TokenTree};

/// The tokens as bare text: every space is dropped before comparing, because `quote!` marks
/// its punctuation joint or alone a little differently from the lexer (`>::` against `> ::`)
/// and neither spelling is the point. Both sides are real token streams, so two tokens can
/// never run together into one this way.
fn norm(ts: TokenStream) -> String {
    strip_docs(ts).to_string().chars().filter(|c| !c.is_whitespace()).collect()
}

/// The same code with the line breaks a reader wants, for when a comparison fails.
fn show(ts: TokenStream) -> String {
    strip_docs(ts).to_string().replace(" ; ", " ;\n").replace(" { ", " {\n")
}

fn same(got: TokenStream, want: &str) {
    let w: TokenStream = want.parse().expect("the expected code parses");
    if norm(got.clone()) != norm(w.clone()) {
        panic!("the generated code is not the expected one.\n--- generated ---\n{}\n--- expected ---\n{}", show(got), show(w));
    }
}

/// Drops every `#[doc = …]` attribute, at any depth.
fn strip_docs(ts: TokenStream) -> TokenStream {
    let mut out: Vec<TokenTree> = Vec::new();
    let mut it = ts.into_iter().peekable();
    while let Some(t) = it.next() {
        match t {
            TokenTree::Punct(ref p) if p.as_char() == '#' => {
                let is_doc = matches!(it.peek(), Some(TokenTree::Group(g))
                    if g.delimiter() == Delimiter::Bracket && g.stream().to_string().starts_with("doc ="));
                if is_doc {
                    it.next();
                } else {
                    out.push(t);
                }
            }
            TokenTree::Group(g) => {
                out.push(TokenTree::Group(Group::new(g.delimiter(), strip_docs(g.stream()))));
            }
            other => out.push(other),
        }
    }
    out.into_iter().collect()
}

fn derive(src: &str) -> TokenStream {
    expand::derive_ruby_class(src.parse().expect("source parses"))
}

fn methods(src: &str) -> TokenStream {
    expand::ruby_methods(TokenStream::new(), src.parse().expect("source parses"))
}

fn derive_text(src: &str) -> String { derive(src).to_string() }
fn methods_text(src: &str) -> String { methods(src).to_string() }

// ---------------------------------------------------------------- #[derive(RubyClass)]

#[test]
fn the_derive_names_the_class_and_says_how_a_value_becomes_an_object() {
    same(
        derive("struct Player { hp: i64 }"),
        r#"
            impl ::sabiruby::host_store::RubyClass for Player {
                const NAME: &'static str = "Player";
            }
            impl ::sabiruby::convert::IntoRuby for Player {
                fn into_ruby(self, vm: &mut ::sabiruby::Vm) -> ::sabiruby::Value {
                    <Self as ::sabiruby::host_store::RubyClass>::into_handle(self, vm)
                }
            }
        "#,
    );
}

#[test]
fn ruby_name_says_what_the_class_is_called_there() {
    same(
        derive(r#"#[ruby(name = "Monster")] struct Beast { kind: String }"#),
        r#"
            impl ::sabiruby::host_store::RubyClass for Beast {
                const NAME: &'static str = "Monster";
            }
            impl ::sabiruby::convert::IntoRuby for Beast {
                fn into_ruby(self, vm: &mut ::sabiruby::Vm) -> ::sabiruby::Value {
                    <Self as ::sabiruby::host_store::RubyClass>::into_handle(self, vm)
                }
            }
        "#,
    );
}

#[test]
fn the_derive_refuses_what_it_cannot_stand_for() {
    assert!(derive_text("struct Pair<T> { a: T }").contains("does not take a generic type"));
    assert!(derive_text(r#"#[ruby(named = "x")] struct P;"#).contains("unknown option"));
}

// ---------------------------------------------------------------- #[ruby_methods]

#[test]
fn a_fn_with_no_self_is_a_class_method_and_one_with_self_an_instance_method() {
    let src = r#"
        impl Player {
            fn new(hp: i64) -> Self { Player { hp } }
            fn hp(&self) -> i64 { self.hp }
            fn damage(&mut self, n: i64) { self.hp -= n; }
        }
    "#;
    same(
        methods(src),
        r#"
            impl Player {
                fn new(hp: i64) -> Self { Player { hp } }
                fn hp(&self) -> i64 { self.hp }
                fn damage(&mut self, n: i64) { self.hp -= n; }
            }

            impl Player {
                pub fn register(vm: &mut ::sabiruby::Vm)
                    -> ::sabiruby::error::VmResult<::sabiruby::value::ObjId>
                {
                    let __class = <Player as ::sabiruby::host_store::RubyClass>::register_class(vm);
                    let _ = <Player as ::sabiruby::host_store::RubyClass>::tag(vm);
                    let __meta = vm.singleton_class(::sabiruby::Value::Obj(__class))?;

                    vm.define_fn(__meta, "new", |vm: &mut ::sabiruby::Vm, __a0: i64|
                        -> ::sabiruby::error::VmResult<::sabiruby::Value>
                    {
                        let __out = <Player>::new(__a0);
                        ::sabiruby::convert::IntoRubyRet::into_ruby_ret(__out, vm)
                    });

                    vm.define_fn(__class, "hp", |vm: &mut ::sabiruby::Vm,
                        __this: ::sabiruby::convert::This<::sabiruby::Value>|
                        -> ::sabiruby::error::VmResult<::sabiruby::Value>
                    {
                        let __out = {
                            let __recv = <Player as ::sabiruby::host_store::RubyClass>::borrow(vm, __this.0)?;
                            <Player>::hp(__recv)
                        };
                        ::sabiruby::convert::IntoRubyRet::into_ruby_ret(__out, vm)
                    });

                    vm.define_fn(__class, "damage", |vm: &mut ::sabiruby::Vm,
                        __this: ::sabiruby::convert::This<::sabiruby::Value>, __a0: i64|
                        -> ::sabiruby::error::VmResult<::sabiruby::Value>
                    {
                        let __out = {
                            let __recv = <Player as ::sabiruby::host_store::RubyClass>::borrow_mut(vm, __this.0)?;
                            <Player>::damage(__recv, __a0)
                        };
                        ::sabiruby::convert::IntoRubyRet::into_ruby_ret(__out, vm)
                    });

                    Ok(__class)
                }

                // asked for at the span of each parameter, so that a type `FromRuby` does not
                // cover is an error pointing at the parameter rather than at the macro
                #[allow(dead_code)]
                fn __ruby_argument_types() {
                    fn __arg<T: ::sabiruby::convert::FromRuby>() {}
                    __arg::<i64>();
                    __arg::<i64>();
                }
            }
        "#,
    );
}

#[test]
fn a_method_given_the_vm_borrows_its_receiver_out_of_the_store_and_back() {
    let src = r#"
        impl Player {
            fn greet(&mut self, vm: &mut Vm, other: Value) -> String { String::new() }
        }
    "#;
    same(
        methods(src),
        r#"
            impl Player {
                fn greet(&mut self, vm: &mut Vm, other: Value) -> String { String::new() }
            }

            impl Player {
                pub fn register(vm: &mut ::sabiruby::Vm)
                    -> ::sabiruby::error::VmResult<::sabiruby::value::ObjId>
                {
                    let __class = <Player as ::sabiruby::host_store::RubyClass>::register_class(vm);
                    let _ = <Player as ::sabiruby::host_store::RubyClass>::tag(vm);

                    vm.define_fn(__class, "greet", |vm: &mut ::sabiruby::Vm,
                        __this: ::sabiruby::convert::This<::sabiruby::Value>, __a0: Value|
                        -> ::sabiruby::error::VmResult<::sabiruby::Value>
                    {
                        let (__handle, mut __recv) =
                            <Player as ::sabiruby::host_store::RubyClass>::take_out(vm, __this.0)?;
                        let __out = <Player>::greet(&mut __recv, vm, __a0);
                        <Player as ::sabiruby::host_store::RubyClass>::give_back(vm, __handle, __recv);
                        ::sabiruby::convert::IntoRubyRet::into_ruby_ret(__out, vm)
                    });

                    Ok(__class)
                }

                #[allow(dead_code)]
                fn __ruby_argument_types() {
                    fn __arg<T: ::sabiruby::convert::FromRuby>() {}
                    __arg::<Value>();
                }
            }
        "#,
    );
}

#[test]
fn skip_and_name_change_what_ruby_sees_and_leave_the_rust_side_alone() {
    let src = r#"
        impl Player {
            #[ruby(name = "alive?")]
            fn alive(&self) -> bool { self.hp > 0 }
            #[ruby(skip)]
            fn secret(&self) -> i64 { 1 }
        }
    "#;
    let out = norm(methods(src));
    // both functions are still there, with the `#[ruby(…)]` attributes taken off
    let kept: String = norm(r#"
        impl Player {
            fn alive(&self) -> bool { self.hp > 0 }
            fn secret(&self) -> i64 { 1 }
        }
    "#.parse().unwrap());
    assert!(out.starts_with(&kept), "the impl block comes back with our attributes removed:\n{out}");
    assert!(out.contains("\"alive?\""), "registered under the Ruby name: {out}");
    assert!(!out.contains("\"secret\""), "and the skipped one is not registered: {out}");
}

#[test]
fn a_block_parameter_is_the_callers_block_and_not_an_argument() {
    let src = r#"
        impl Player {
            fn each_hit(&mut self, vm: &mut Vm, n: i64, blk: Block) {}
        }
    "#;
    same(
        methods(src),
        r#"
            impl Player {
                fn each_hit(&mut self, vm: &mut Vm, n: i64, blk: Block) {}
            }

            impl Player {
                pub fn register(vm: &mut ::sabiruby::Vm)
                    -> ::sabiruby::error::VmResult<::sabiruby::value::ObjId>
                {
                    let __class = <Player as ::sabiruby::host_store::RubyClass>::register_class(vm);
                    let _ = <Player as ::sabiruby::host_store::RubyClass>::tag(vm);

                    vm.define_fn(__class, "each_hit", |vm: &mut ::sabiruby::Vm,
                        __this: ::sabiruby::convert::This<::sabiruby::Value>, __a0: i64,
                        __blk: ::sabiruby::convert::Block|
                        -> ::sabiruby::error::VmResult<::sabiruby::Value>
                    {
                        let (__handle, mut __recv) =
                            <Player as ::sabiruby::host_store::RubyClass>::take_out(vm, __this.0)?;
                        let __out = <Player>::each_hit(&mut __recv, vm, __a0, __blk);
                        <Player as ::sabiruby::host_store::RubyClass>::give_back(vm, __handle, __recv);
                        ::sabiruby::convert::IntoRubyRet::into_ruby_ret(__out, vm)
                    });

                    Ok(__class)
                }

                // the block is not one of the argument types: `Block` is not `FromRuby`
                #[allow(dead_code)]
                fn __ruby_argument_types() {
                    fn __arg<T: ::sabiruby::convert::FromRuby>() {}
                    __arg::<i64>();
                }
            }
        "#,
    );
}

#[test]
fn a_class_with_no_class_method_does_not_ask_for_the_metaclass() {
    let out = methods_text("impl Player { fn hp(&self) -> i64 { self.hp } }");
    assert!(!out.contains("singleton_class"), "{out}");
    assert!(out.contains("register_class"));
}

#[test]
fn what_cannot_be_a_ruby_method_is_a_compile_error_saying_why() {
    let cases: &[(&str, &str)] = &[
        ("impl Player { fn into_hp(self) -> i64 { self.hp } }", "takes `self` by value"),
        ("impl<T> Player<T> { fn hp(&self) -> i64 { 0 } }", "generic impl block"),
        ("impl Widen for Player { fn hp(&self) -> i64 { 0 } }", "not a trait implementation"),
        ("impl Player { fn wide(&self, a: i64, b: i64, c: i64, d: i64, e: i64, f: i64, g: i64) {} }", "at most six arguments"),
        ("impl Player { fn late(&self, n: i64, vm: &mut Vm) {} }", "`&mut Vm` comes first"),
        ("impl Player { fn early(&self, b: Block, n: i64) {} }", "`Block` comes last"),
        ("impl Player { fn lonely(&self, b: Block) {} }", "takes the `&mut Vm` too"),
        ("impl Player { async fn slow(&self) {} }", "async fn"),
        ("impl Player { #[ruby(hidden)] fn x(&self) {} }", "unknown option"),
    ];
    for (src, want) in cases {
        let out = methods_text(src);
        assert!(out.contains(want), "expected {want:?} in the error for {src:?}, got:\n{out}");
        assert!(out.contains("compile_error"), "and it is a compile error: {out}");
    }
}

#[test]
fn the_impl_block_is_given_back_even_when_a_signature_is_refused() {
    // so that one bad signature does not also make every call of every other method undefined
    let out = norm(methods("impl Player { fn hp(&self) -> i64 { self.hp } fn eat(self) {} }"));
    assert!(out.contains(&norm("fn hp(&self) -> i64 { self.hp }".parse().unwrap())), "{out}");
}
