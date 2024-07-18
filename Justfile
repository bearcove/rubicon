# Just manual: https://github.com/casey/just

check:
    cargo hack --each-feature --exclude-all-features clippy --manifest-path rubicon/Cargo.toml

test:
    SOPRINTLN=1 cargo run --manifest-path test-crates/bin/Cargo.toml
