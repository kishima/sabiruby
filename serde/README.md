# sabiruby-serde

serde for [SabiRuby](https://crates.io/crates/sabiruby): a Rust value as a Ruby value and back,
and a Ruby `JSON` built on top of it.

```rust
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, PartialEq, Debug)]
struct Config { name: String, retries: u32, verbose: bool }

let cfg = Config { name: "a".into(), retries: 3, verbose: true };
let v = sabiruby_serde::to_value(&mut vm, &cfg)?;   // {"name" => "a", "retries" => 3, …}
let back: Config = sabiruby_serde::from_value(&mut vm, v)?;
```

`Serde<T>` puts a serde type into a typed method's signature:

```rust
vm.define_fn(object, "configure", |cfg: Serde<Config>| cfg.0.name);
```

and `install_json` gives the VM a `JSON` class — `JSON.parse`, `JSON.generate`,
`JSON.pretty_generate`, `Object#to_json` — written in Rust on `serde_json`:

```rust
sabiruby_serde::install_json(&mut vm);
```

The VM itself never depends on serde: it is `no_std` and small on purpose, and everything here
is a layer above it. This crate is `no_std` + `alloc` too; the default feature `json` is what
brings in `serde_json` (and, through its `preserve_order`, `std`), so
`default-features = false` leaves the conversion layer alone.

The data model, the error mapping and what differs from CRuby's JSON are
[`docs/design/serde.md`](https://github.com/sabiruby/sabiruby/blob/main/docs/design/serde.md)
of the repository.

MIT licensed, like the rest of SabiRuby.
