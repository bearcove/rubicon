# Just manual: https://github.com/casey/just

check:
    cargo hack --each-feature --exclude-all-features clippy

build:
    @echo "======== Regular build ========"
    cargo build
    nm target/debug/librubicon.dylib | grep -E 'RUBICON_(TL|PL)_SAMPLE'

    @echo
    @echo "======== Export globals ========"
    cargo build --features export-globals
    nm target/debug/librubicon.dylib | grep -E 'RUBICON_(TL|PL)_SAMPLE'

    @echo
    @echo "======== Import globals ========"
    cargo build --features import-globals
    nm target/debug/librubicon.dylib | grep -E 'RUBICON_(TL|PL)_SAMPLE'
