# sabiruby-macros

Derive and attribute macros for [SabiRuby](https://crates.io/crates/sabiruby): a Rust struct
and its `impl` block as a Ruby class, without writing the registration by hand.

```rust
use sabiruby::Vm;
use sabiruby_macros::{RubyClass, ruby_methods};

#[derive(RubyClass)]
struct Player { hp: i64 }

#[ruby_methods]
impl Player {
    fn new(hp: i64) -> Self { Player { hp } }        // Player.new(100)
    fn damage(&mut self, n: i64) { self.hp -= n; }   // player.damage(10)
    fn hp(&self) -> i64 { self.hp }                  // player.hp
}

Player::register(&mut vm);                           // the class, the store, the methods
```

The value stays in Rust — it lives in a `HostStore` the VM carries and Ruby holds a handle to
it, which the collector gives back when nothing refers to it any more. Arguments and answers
go through `FromRuby` and `IntoRuby`, so a type neither covers is a compile error.

This crate does not depend on `sabiruby`; a host writes both. What is generated, and what it
does not cover, is `docs/design/macros.md` of the
[repository](https://github.com/sabiruby/sabiruby).
