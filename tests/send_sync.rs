//! A `Vm` must be `Send + Sync`: an engine that keeps one in its own world (rubevy puts one in a
//! Bevy resource) requires it of everything it stores, and the only thing the VM holds that is
//! not plain data is the host (`src/host.rs`, whose trait says so).

fn assert_send_sync<T: Send + Sync>() {}

#[test]
fn vm_is_send_and_sync() {
    assert_send_sync::<sabiruby::Vm>();
}
