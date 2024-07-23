# Just manual: https://github.com/casey/just

check:
    cargo hack --each-feature --exclude-all-features clippy --manifest-path rubicon/Cargo.toml

test *args:
    #!/usr/bin/env bash -eux
    BIN_CHANNEL="${BIN_CHANNEL:-stable}"
    BIN_FLAGS="${BIN_FLAGS:-}"

    SOPRINTLN=1 cargo "+${BIN_CHANNEL}" build --manifest-path test-crates/bin/Cargo.toml "${BIN_FLAGS}"

    export DYLD_LIBRARY_PATH=$(rustc "+stable" --print sysroot)/lib
    export DYLD_LIBRARY_PATH=$DYLD_LIBRARY_PATH:$(rustc "+nightly" --print sysroot)/lib
    export LD_LIBRARY_PATH=$(rustc "+stable" --print sysroot)/lib
    export LD_LIBRARY_PATH=$LD_LIBRARY_PATH:$(rustc "+nightly" --print sysroot)/lib

    ./test-crates/bin/target/debug/bin {{args}}
