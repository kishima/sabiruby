#!/bin/bash
# Enforces the no_std rule: the library must build for a target that has no `std`.
#   rustup target add thumbv7em-none-eabi   (once)
set -eu
cd "$(dirname "$0")/.."
cargo build --lib --no-default-features --target thumbv7em-none-eabi "$@"
if grep -rn "std::" src --include=*.rs | grep -vE "^[^:]+:[0-9]+:\s*//"; then
  echo "error: std:: path in the library (use core::/alloc::)" >&2; exit 1
fi
echo "no_std OK"
