//! A prototype: RBS generated from an `#[ruby_methods]` block.
//!
//! This is the throwaway that grounds `docs/design/rbs.md` (stage 2 of
//! `docs/plans/from-mrubyedge-plan.md`), not a feature. It is on purpose a test and not part of
//! `src/`: the shipped macros are unchanged by this stage, and a proc-macro crate exports
//! nothing but its macros, so there would be nowhere to put a public generator anyway.
//!
//! It reads an `impl` block as text with `syn` — the same view `#[ruby_methods]` has at
//! expansion time, a *syntactic* one — and writes the RBS the methods would have. What it
//! cannot do from there is written up in the design document: the Ruby class name lives on the
//! `#[derive(RubyClass)]` of the struct (passed in by hand here), and a type it does not
//! recognise by spelling becomes `untyped` rather than a guess.

use syn::{FnArg, ImplItem, ItemImpl, LitStr, ReturnType, Type};

// ---------------------------------------------------------------- the generator

/// The RBS of one `impl` block, as `#[ruby_methods]` sees it. `class` is the Ruby class name,
/// which the derive on the struct knows and this block does not.
fn rbs_of_impl(src: &str, class: &str) -> String {
    let block: ItemImpl = syn::parse_str(src).expect("an impl block");
    let mut out = format!("class {class}\n");
    for it in block.items.iter() {
        let f = match it {
            ImplItem::Fn(f) => f,
            _ => continue,
        };
        let (name, skip) = ruby_attr(&f.attrs);
        if skip {
            continue;
        }
        let name = name.unwrap_or_else(|| f.sig.ident.to_string());

        let mut inputs = f.sig.inputs.iter().peekable();
        let class_method = !matches!(inputs.peek(), Some(FnArg::Receiver(_)));
        if !class_method {
            inputs.next();
        }
        // A leading `&mut Vm` is the call's context, not an argument (`expand.rs`, `is_vm`).
        if let Some(FnArg::Typed(t)) = inputs.peek() {
            if is_vm(&t.ty) {
                inputs.next();
            }
        }
        let args: Vec<String> = inputs
            .filter_map(|a| match a {
                FnArg::Typed(t) => Some(format!("{} {}", rbs_type(&t.ty, class), pat_name(&t.pat))),
                FnArg::Receiver(_) => None,
            })
            .collect();

        let ret = match &f.sig.output {
            ReturnType::Default => String::from("void"),
            ReturnType::Type(_, ty) => rbs_return(ty, class),
        };
        let recv = if class_method { "self." } else { "" };
        out.push_str(&format!("  def {recv}{name}: ({}) -> {ret}\n", args.join(", ")));
    }
    out.push_str("end\n");
    out
}

/// `#[ruby(name = "…")]` and `#[ruby(skip)]`, as `expand.rs` reads them.
fn ruby_attr(attrs: &[syn::Attribute]) -> (Option<String>, bool) {
    let (mut name, mut skip) = (None, false);
    for a in attrs.iter().filter(|a| a.path().is_ident("ruby")) {
        let _ = a.parse_nested_meta(|meta| {
            if meta.path.is_ident("name") {
                let s: LitStr = meta.value()?.parse()?;
                name = Some(s.value());
            } else if meta.path.is_ident("skip") {
                skip = true;
            }
            Ok(())
        });
    }
    (name, skip)
}

fn is_vm(ty: &Type) -> bool {
    match ty {
        Type::Reference(r) if r.mutability.is_some() => match &*r.elem {
            Type::Path(p) => p.path.segments.last().is_some_and(|s| s.ident == "Vm"),
            _ => false,
        },
        _ => false,
    }
}

fn pat_name(pat: &syn::Pat) -> String {
    match pat {
        syn::Pat::Ident(i) => i.ident.to_string(),
        _ => String::from("_"),
    }
}

/// The answer side: `Result<T, E>` is `T` (RBS says nothing about what a method raises), and
/// nothing at all is `void` rather than `nil`.
fn rbs_return(ty: &Type, class: &str) -> String {
    if let Some((head, args)) = path_head(ty) {
        if head == "Result" {
            return match args.first() {
                Some(t) => rbs_return(t, class),
                None => String::from("untyped"),
            };
        }
        if head == "VmResult" {
            return match args.first() {
                Some(t) => rbs_return(t, class),
                None => String::from("untyped"),
            };
        }
    }
    if matches!(ty, Type::Tuple(t) if t.elems.is_empty()) {
        return String::from("void");
    }
    rbs_type(ty, class)
}

/// One Rust type as RBS. Every spelling `FromRuby`/`IntoRuby` covers has an entry; anything
/// else is `untyped`, because at expansion time this is a *name*, not a resolved type.
fn rbs_type(ty: &Type, class: &str) -> String {
    match ty {
        // `&str` and `&mut T` alike: what matters is what is inside.
        Type::Reference(r) => rbs_type(&r.elem, class),
        Type::Tuple(t) if t.elems.is_empty() => String::from("nil"),
        Type::Tuple(t) => {
            let parts: Vec<String> = t.elems.iter().map(|e| rbs_type(e, class)).collect();
            format!("[{}]", parts.join(", "))
        }
        Type::Path(_) => {
            let (head, args) = match path_head(ty) {
                Some(x) => x,
                None => return String::from("untyped"),
            };
            let arg0 = || args.first().map(|t| rbs_type(t, class)).unwrap_or_else(|| String::from("untyped"));
            match head.as_str() {
                "i8" | "i16" | "i32" | "i64" | "u8" | "u16" | "u32" | "u64" | "usize" | "isize" => String::from("Integer"),
                "f32" | "f64" => String::from("Float"),
                "bool" => String::from("bool"),
                "str" | "String" => String::from("String"),
                // The bytes of a String, as the VM holds them: still a String in Ruby.
                "Bytes" => String::from("String"),
                "Sym" => String::from("Symbol"),
                // A Ruby value untouched, and a `Data` handle of some host type.
                "Value" | "DataRef" => String::from("untyped"),
                // `Serde<T>`: the shape is serde's, not the signature's.
                "Serde" => String::from("untyped"),
                "Option" => format!("{}?", arg0()),
                "Vec" => format!("Array[{}]", arg0()),
                "Self" => String::from(class),
                _ => String::from("untyped"),
            }
        }
        _ => String::from("untyped"),
    }
}

/// The last segment of a path and its generic arguments (`Vec<Option<i64>>` → `("Vec", [Option<i64>])`).
fn path_head(ty: &Type) -> Option<(String, Vec<Type>)> {
    let p = match ty {
        Type::Path(p) => p,
        _ => return None,
    };
    let seg = p.path.segments.last()?;
    let mut args = Vec::new();
    if let syn::PathArguments::AngleBracketed(a) = &seg.arguments {
        for a in a.args.iter() {
            if let syn::GenericArgument::Type(t) = a {
                args.push(t.clone());
            }
        }
    }
    Some((seg.ident.to_string(), args))
}

// ---------------------------------------------------------------- what it makes

/// `macros/tests/player.rs`'s own `impl` block, word for word: the example the design document
/// quotes the output of.
const PLAYER: &str = r#"
impl Player {
    fn new(hp: i64) -> Self { todo!() }
    fn strongest(vm: &mut Vm, a: i64, b: i64) -> Self { todo!() }
    fn hp(&self) -> i64 { todo!() }
    fn damage(&mut self, n: i64) { todo!() }
    fn rename(&mut self, to: String) -> String { todo!() }
    fn name(&self) -> String { todo!() }
    #[ruby(name = "alive?")]
    fn alive(&self) -> bool { todo!() }
    fn greet(&self, vm: &mut Vm, other: Value) -> sabiruby::error::VmResult<String> { todo!() }
    #[ruby(skip)]
    fn secret(&self) -> i64 { todo!() }
}
"#;

#[test]
fn the_player_of_the_macro_tests() {
    assert_eq!(
        rbs_of_impl(PLAYER, "Player"),
        concat!(
            "class Player\n",
            "  def self.new: (Integer hp) -> Player\n",
            "  def self.strongest: (Integer a, Integer b) -> Player\n",
            "  def hp: () -> Integer\n",
            "  def damage: (Integer n) -> void\n",
            "  def rename: (String to) -> String\n",
            "  def name: () -> String\n",
            "  def alive?: () -> bool\n",
            "  def greet: (untyped other) -> String\n",
            "end\n",
        )
    );
}

/// The second class of `player.rs`, whose Ruby name (`Monster`) is on the struct rather than on
/// the `impl` block: the generator has to be told it.
#[test]
fn a_class_whose_ruby_name_is_not_its_rust_name() {
    let src = r#"
impl Beast {
    fn new(kind: String) -> Self { todo!() }
    fn kind(&self) -> String { todo!() }
    fn hp(&self) -> i64 { todo!() }
}
"#;
    assert_eq!(
        rbs_of_impl(src, "Monster"),
        concat!(
            "class Monster\n",
            "  def self.new: (String kind) -> Monster\n",
            "  def kind: () -> String\n",
            "  def hp: () -> Integer\n",
            "end\n",
        )
    );
}

/// Every other shape `FromRuby`/`IntoRuby` covers, in one block: the mapping table of
/// `docs/design/rbs.md` as a test.
#[test]
fn the_rest_of_the_mapping() {
    let src = r#"
impl Shapes {
    fn floats(&self, a: f64, b: f32) -> f64 { todo!() }
    fn maybe(&self, n: Option<i64>) -> Option<String> { todo!() }
    fn list(&self, xs: Vec<i64>) -> Vec<Vec<f64>> { todo!() }
    fn nested(&self, xs: Vec<Option<i64>>) -> Option<Vec<String>> { todo!() }
    fn bytes(&self, b: Bytes) -> Bytes { todo!() }
    fn sym(&self, s: Sym) -> Sym { todo!() }
    fn raw(&self, v: Value) -> Value { todo!() }
    fn handle(&self, d: DataRef) -> DataRef { todo!() }
    fn pair(&self) -> (i64, String) { todo!() }
    fn borrowed(&self) -> &str { todo!() }
    fn fallible(&self) -> Result<i64, String> { todo!() }
    fn nothing(&self) -> () { todo!() }
    fn config(&self, c: Serde<Config>) -> Serde<Config> { todo!() }
    fn unknown(&self, w: Widget) -> Widget { todo!() }
}
"#;
    assert_eq!(
        rbs_of_impl(src, "Shapes"),
        concat!(
            "class Shapes\n",
            "  def floats: (Float a, Float b) -> Float\n",
            "  def maybe: (Integer? n) -> String?\n",
            "  def list: (Array[Integer] xs) -> Array[Array[Float]]\n",
            "  def nested: (Array[Integer?] xs) -> Array[String]?\n",
            "  def bytes: (String b) -> String\n",
            "  def sym: (Symbol s) -> Symbol\n",
            "  def raw: (untyped v) -> untyped\n",
            "  def handle: (untyped d) -> untyped\n",
            "  def pair: () -> [Integer, String]\n",
            "  def borrowed: () -> String\n",
            "  def fallible: () -> Integer\n",
            "  def nothing: () -> void\n",
            "  def config: (untyped c) -> untyped\n",
            "  def unknown: (untyped w) -> untyped\n",
            "end\n",
        )
    );
}
