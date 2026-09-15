//! The error the two serde halves speak, and how it becomes a Ruby exception.

use alloc::string::{String, ToString};

use sabiruby::error::VmError;
use sabiruby::Vm;

/// What went wrong while converting between a Rust value and a Ruby one.
///
/// serde needs an error type of its own: [`serde::ser::Error`] and [`serde::de::Error`] build
/// one from a message alone (`Error::custom`), with no VM in reach, which
/// [`sabiruby::error::VmError`] cannot be built from — a `VmError::Raise` carries an
/// exception *object*, and allocating one needs a `&mut Vm`. So the conversion collects its
/// failures here and turns them into a raise at the boundary, where the VM is in hand:
/// [`to_value`](crate::to_value) and [`from_value`](crate::from_value) answer with
/// [`VmResult`](sabiruby::error::VmResult) and never show this type unless a caller drives the
/// [`Serializer`](crate::ser::Serializer) itself.
#[derive(Debug)]
pub enum Error {
    /// A message from serde or from this crate: a type that does not fit, a missing field, an
    /// unknown variant. Becomes a `TypeError`, which is what the VM's own
    /// [`FromRuby`](sabiruby::FromRuby) raises for the same kind of mistake.
    Message(String),
    /// The VM itself failed or raised while the conversion ran (a Ruby `hash`/`eql?` raised,
    /// say). Re-raised unchanged rather than described: the exception the host or the script
    /// will see is the one that was actually raised.
    Vm(VmError),
}

impl Error {
    /// The Ruby exception this error raises. `Message` becomes a `TypeError`; a `Vm` error is
    /// handed back as it is.
    pub fn into_vm_error(self, vm: &mut Vm) -> VmError {
        match self {
            Error::Message(m) => vm.raise_type(&m),
            Error::Vm(e) => e,
        }
    }
}

impl From<VmError> for Error {
    fn from(e: VmError) -> Error { Error::Vm(e) }
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Error::Message(m) => f.write_str(m),
            Error::Vm(e) => write!(f, "{e}"),
        }
    }
}

impl core::error::Error for Error {}

impl serde::ser::Error for Error {
    fn custom<T: core::fmt::Display>(msg: T) -> Error { Error::Message(msg.to_string()) }
}

impl serde::de::Error for Error {
    fn custom<T: core::fmt::Display>(msg: T) -> Error { Error::Message(msg.to_string()) }
}

/// This crate's own `Result`.
pub type Result<T> = core::result::Result<T, Error>;
