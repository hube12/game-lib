#!/usr/bin/env sh
# To be run in ./afl-fuzz
# Optional: set AFL_SANITIZE=1 to build with AddressSanitizer (requires the nightly toolchain).
set -eu

# `afl` is only the library, the CLI is shipped by `cargo-afl`. After a toolchain change, rebuild the
# AFL runtime; it is always rebuilt below so it matches the active toolchain.
cargo install --locked cargo-afl

if [ "${AFL_SANITIZE:-0}" = "1" ]; then
  TOOLCHAIN="+nightly"
  export RUSTFLAGS="-Zsanitizer=address"
else
  TOOLCHAIN=""
fi

# shellcheck disable=SC2086 # TOOLCHAIN is intentionally unquoted so it vanishes when empty
cargo $TOOLCHAIN afl config --build --force
# shellcheck disable=SC2086
cargo $TOOLCHAIN afl build

# On first use AFL may ask for: cargo afl system-config
# shellcheck disable=SC2086
cargo $TOOLCHAIN afl fuzz -i in-dummy -o out-dummy target/debug/afl-fuzz
