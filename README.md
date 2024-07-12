# rubicon

Webster's Dictionary defines 'rubicon' as: a bounding or limiting line.
(especially : one that when crossed commits a person irrevocably).

In this case, our rubicon is the boundary between shared objects
(executables, dynamic libraries, etc.) that all include a copy of the same crate.

This can happen if you build an application as "one main binary" and "several modules",
loaded at runtime — and the modules are of `crate-type = ["cdylib"]`, which means
they are completely separate crate graphs than the application.

Libraries like `tokio` and `tracing-subscriber` _really freak out_ if it
turns out there's several copies of their thread-local variables — and who could blame them?

rubicon lets those crates expose cargo features to export thread-locals (from the main binary)
or import thread-locals (from modules).

This takes care of problem #1, which is: make sure our thread-copy "singleton" actually only
exists once at runtime.

Problem #2 is still on you, specifically: YOU MUST MAKE SURE THE ABI MATCHES. That means using
the exact same version of the compiler. It also means making sure that every copy of `tokio`,
or `tracing-subscriber`, or whatever, has the EXACT SAME SET OF FEATURES ENABLED.

This is trickier than it sounds — you can't just have a `tokio-wrapper` crate of your own with
the features you need — any of your transitive dependencies (from the main app / any of its modules)
can sneakily enable an extra tokio feature. Cargo features are additive and there's no way to
denylist them.

## How does it work?

Well, it's only 100 lines of Rust, but there's high levels of
fuckery in there, so I don't blame you for asking.

Essentially _all we're trying to do_ is to export/import thread-locals,
however:

  * `std::thread_local!` is a macro, not some piece of syntax (unlike the
    `#[thread_local]` attribute, which is unstable and has restrictions)
  * You can't shove it in an `extern "C"` because using extern statics
    requires unsafe blocks. That requires patching every use site of the
    thread-local.
  * You can't make it pub/`no_mangle` either (or choose a better, less cluttery
    export name).
  * Thread-local internals (`LocalKey` fields, the constructor) are
    unstable/hidden by design
  * `LocalKey` isn't `Clone` or `Copy` (even though all it carries in Rust 1.79
    is the address of a function — not a closure, an `fn`)
  * You can't initialize a `static` with the address of another one (since
    its address is not known at compile time — the compiler is correct, this is
    link-time fuckery).

If you run `just build` you will see I got _something_ to work.

```shell
❯ just build
======== Regular build ========
cargo build
   Compiling rubicon v0.1.0 (/Users/amos/bearcove/rubicon)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.34s
nm target/debug/librubicon.dylib | grep RUBICON_SAMPLE
00000000000033a8 t __ZN7rubicon14RUBICON_SAMPLE6__init17h7ca17d5c91e84836E
00000000000033d4 t __ZN7rubicon14RUBICON_SAMPLE7__getit17ha51d95268314f0e3E
00000000000032d0 t __ZN7rubicon14RUBICON_SAMPLE7__getit28_$u7b$$u7b$closure$u7d$$u7d$17h4fbd075560ca6b68E
000000000000ddc8 s __ZN7rubicon14RUBICON_SAMPLE7__getit5__KEY17h58899a7072395bddE
000000000000dde0 s __ZN7rubicon14RUBICON_SAMPLE7__getit5__KEY17h58899a7072395bddE$tlv$init

======== Export globals ========
cargo build --features export-globals
   Compiling rubicon v0.1.0 (/Users/amos/bearcove/rubicon)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.10s
nm target/debug/librubicon.dylib | grep RUBICON_SAMPLE
00000000000080a8 S _RUBICON_SAMPLE
0000000000005ba0 T __ZN7rubicon14RUBICON_SAMPLE14RUBICON_SAMPLE28_$u7b$$u7b$closure$u7d$$u7d$17h8210afea3b744e51E
0000000000002d9c t __ZN7rubicon14RUBICON_SAMPLE6__init17h40fdfd56886f5a1fE
0000000000002dc8 t __ZN7rubicon14RUBICON_SAMPLE7__getit17had70f80de9afa53bE
0000000000004b28 t __ZN7rubicon14RUBICON_SAMPLE7__getit28_$u7b$$u7b$closure$u7d$$u7d$17h3527ed6c8c4bd29fE
000000000000f200 s __ZN7rubicon14RUBICON_SAMPLE7__getit5__KEY17h5e9ac340596c033bE
000000000000f218 s __ZN7rubicon14RUBICON_SAMPLE7__getit5__KEY17h5e9ac340596c033bE$tlv$init

======== Import globals ========
cargo build --features import-globals
   Compiling rubicon v0.1.0 (/Users/amos/bearcove/rubicon)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.07s
nm target/debug/librubicon.dylib | grep RUBICON_SAMPLE
                 U _RUBICON_SAMPLE
0000000000004090 s __ZN7rubicon14RUBICON_SAMPLE17h73a76804a859adb2E
```

The "regular" (neither import nor export) build doesn'thave any dynamic symbols,
the "export" build has `S _RUBICON_SAMPLE`, and the "import" build has `U _RUBICON_SAMPLE`.

To get the "import" build to build at all, I used `-undefined dynamic_lookup`,
see `.cargo/config.toml` in this repository. This is a much bigger gun than
needed, I'd much rather use weak linking or target those symbols specifically
but for that we need to get cargo, the compiler, and the linker to cooperate.

## Adding rubicon to your crate

Okay, so you have a crate like `tokio` or `tracing-subscriber` that everyone uses, and you'd like
to cater to the needs of rubicon-crossers such as myself. What should you do?

### 1. Add an unconditional dependency on rubicon

As simple as `cargo add rubicon` — it has zero dependencies and is < 100 lines of code.

It just exports one declarative macro.

### 2. Add cargo features

Add features "import-globals" and "export-globals", and forward them to "rubicon".

```toml
# in Cargo.toml

[features]
# (cut: existing features)
import-globals = ["rubicon/import-globals"]
export-globals = ["rubicon/export-globals"]
```

Important notes:

  * rubicon will trigger a compile error if both of these are enabled at the same time.
  * DO NOT MAKE EITHER OF THESE DEFAULT
  * "not enabling either" simply forwards to `std::thread_local!` which is the only
    safe thing to do anyway and most users should want.

### 3. Use `rubicon::thread_local!`

...instead of `std::thread_local!`.

### 4. Consider making `{import,export}-globals` trigger your `full` feature

To _help ensure_ that the ABIs match. You'll trade users complaining about random
failed asserts / segmentation faults / bus errors / various other kinds of memory corruption,
for users complaining that the build is too big. Well, buddy, none of us should be
here, and yet here we are.

So, here's my recommended scheme:

```toml
# in Cargo.toml

[features]
default = ["foo"]
full = ["foo", "bar", "baz"]
# (cut: existing features)
import-globals = ["full", "rubicon/import-globals"]
export-globals = ["full", "rubicon/export-globals"]
```

## Using a rubicon-powered crate

First off: don't. You'll be sorry.

If you must:

  * Make a crate named `rubicon-exports` with `crate-type = ["dylib"]` (NOT CDYLIB)
  * Have it depend on `tokio`, `tracing-subscriber`, etc.: whatever your crates depend on, and enable the `export-globals` features
  * In its `src/lib.rs`, don't forget `use tokio as  _`, etc., otherwise
  shit will get tree-shook
  * Have your main crate depend on `rubicon-exports`
  * In its `src/main.rs`, don't forget to `use rubicon_exports as _`, otherwise
  shit will get dead-code-eliminated
  * In all your "cdylib" crates (your modules/plugins/etc.), enable the `import-globals` feature on `tokio`, `tracing-subscriber`, etc.
  * For all those crates, you'll need to pass the `-undefined dynamic_lookup`
    linker option, or something equivalent (look at this repo's `.cargo/config.toml` for inspiration).

## FAQ

### Is this safe?

God no. Prepare for fun stack traces (if you get stack traces at all).

### Doesn't `crate-type = ["dylib"]` fix all this?

Yes but no, because then you have a single crate graph and everything takes forever% CPU/RAM, which is what we're trying to avoid. The whole point of splitting your app into binary + several cdylibs (load-bearing "c") is that they are separate crate graphs, you can rust-analyze them fully independently, build them with full concurrency in CI (compute is cheap, waiting for the linker is not), and as long as you rebuild everything when you bump your rustc version you _should_ be okay.

### Can't we just have all shared objects share a single "libtokio.so"?

No, because monomorphization: if you have an app and modules A through C, there's 4 versions of `libtokio.so`, none of which have all the "generic instantiations" you need. The only way to get _all the instantiations_ you need would be to depend onthe app AND all its modules, creating a single ginormous crate graph again, which defeats
the whole point.

### What about LTO / the runtime cost of dynamic linking?

Yeah sorry, no LTO, obviously, and yes, dynamic linking has a cost. No
cross-object inlining. That's the deal.

### Should I use this in production?

You shouldn't, but I'm gonna.
