# Just manual: https://github.com/casey/just

check:
    cargo hack --each-feature --exclude-all-features clippy

build:
    #!/usr/bin/env bash -eux
    cd rubicon

    export RUSTFLAGS="-Clink-arg=-undefined -Clink-arg=dynamic_lookup"

    echo "======== Regular build ========"
    cargo build
    nm target/debug/librubicon.dylib | grep -E 'RUBICON_(TL|PL)_SAMPLE'

    echo "======== Export globals ========"
    cargo build --features export-globals
    nm target/debug/librubicon.dylib | grep -E 'RUBICON_(TL|PL)_SAMPLE'

    echo "======== Import globals ========"
    cargo build --features import-globals
    nm target/debug/librubicon.dylib | grep -E 'RUBICON_(TL|PL)_SAMPLE'
