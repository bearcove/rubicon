# rubicon

rubicon enables a dangerous form of dynamic linking in Rust through cdylib crates
and carefully-enforced invariants.

## Name

Webster's Dictionary defines 'rubicon' as:

  > a bounding or limiting line. especially: one that
  > when crossed, commits a person irrevocably.

In this case, I see it as the limiting line between several shared objects,
within the same address space, each including their own copy of the same Rust
code.

## Nomenclature

Dynamic linking concepts have different names on different platforms:

| Concept             | Linux               | macOS                    | Windows                                                              |
|-------------------- | ------------------- | ------------------------ | -------------------------------------------------------------------- |
| Shared library      | shared object       | dynamic library          | DLL (Dynamic Link Library)                                           |
| Library file name   | `libfoo.so`         | `libfoo.dylib`           | `foo.dll`                                                            |
| Library search path | `LD_LIBRARY_PATH`   | `DYLD_LIBRARY_PATH`      | `PATH`                                                               |
| Preload mechanism   | `LD_PRELOAD`        | `DYLD_INSERT_LIBRARIES`  | [It's complicated](https://stackoverflow.com/a/5273439)              |

Throughout this document, macOS naming conventions are preferred.

## Motivation

### Rust's dynamic linking model (`1graph`)

(This section is up-to-date as of Rust 1.79 / 2024-07-18)

cargo and rustc support some form of dynamic linking, through the
[-C prefer-dynamic][prefer-dynamic] compiler flag.

[prefer-dynamic]: https://doc.rust-lang.org/rustc/codegen-options/index.html#prefer-dynamic

This flag will:

  * Link against the pre-built `libstd-HASH.dylib`, shipped via rustup
    (assuming you're not using `-Z build-std`)
  * Try to link against `libfoobar.dylib`, for any crate `foobar` that
    includes `dylib` in its `crate-type`

rustc has an [internal algorithm][] to decide which linkage to use for which
dependency. That algorithm is best-effort, and it can fail.

[internal algorithm]: https://github.com/rust-lang/rust/blob/master/compiler/rustc_metadata/src/dependency_format.rs

Regardless, it assumes that rustc has knowledge of the entire dependency graph
at link time.

### rubicon's dynamic linking model (`xgraph`)

However, one might want to split the dependency graph on purpose:

| Strategy                     | 1graph (one dependency graph)                | xgraph (multiple dependency graphs)                                 |
| ---------------------------- | -------------------------------------------- | ------------------------------------------------------------------- |
| Module crate-type            | dylib                                        | cdylib                                                               |
| Duplicates in address space  | No (rlib/dylib resolution at link time)      | Yes (by design)                                                      |
| Who loads modules?           | the runtime linker                           | the app                                                              |
| When loads modules?          | before main, unconditionally                 | any time (but don't unload)                                          |
| How loads modules?           | `DT_NEEDED` / `LC_LOAD_DYLIB` etc.           | libdl, likely via [libloading](https://docs.rs/libloading/latest/libloading/) |

Let's call Rust's "supported" dynamic linking model "1graph".

rubicon enables (at your own risk), a different model, which we'll call "xgraph".

In the "xgraph" model, every "module" of your application — anything that might make
sense to build separately, like "a bunch of tree-sitter grammars", or "a whole JavaScript runtime",
is its _own_ dependency graph, rooted at a crate with a `crate-type` of `cdylib`.

In the "xgraph" model, your application's "shared object" (Linux executables, macOS executables,
etc. are just shared objects — not too different from libraries, except they have an entry point)
does not have any references to its modules — by the time `main()` is executed, none of the
modules are loaded yet.

Instead, modules are loaded explicitly through a crate like [libloading](https://lib.rs/crates/libloading),
which under the hood, uses whatever facilities the platform's dynamic linker-loader exposes. This
lets you choose which modules to load and when.

### Linkage and discipline

The "xgraph" model is dangerous — we must use discipline to get it to work at all.

In particular, we'll maintain the following invariants:

  * A. Modules are NEVER UNLOADED, only loaded.
  * B. The EXACT SAME RUSTC VERSION is used to build the app and all modules
  * C. The EXACT SAME CARGO FEATURES are enabled for crates that both the app
       and some modules depend on.

Unloading modules ("A") would break a significant assumption in all Rust programs: that `'static`
lasts for the entirety of the program's execution. When unloading a module, we can make something
`'static` disappear.

Although nobody can stop you from unloading modules, what you're writing at this point is no longer
safe Rust.

Mixing rustc versions ("B") might result in differences in struct layouts, for example. For a struct like:

```rust
struct Blah {
    a: u64,
    b: u32,
}
```

...there's no guarantee which field will be first, if there will be padding, what order the fields will
be in. We pray that struct layouts match across the same compiler version, but even that might not be
guaranteed? (citation needed)

Mixing cargo feature sets ("C") might, again, result in differences in struct layouts:

```rust
struct Blah {
    #[cfg(feature = "foo")]
    a: u64,
    b: u32
}

// if the app has `foo` enabled, and we pass a &Blah` to
// a module that doesn't have `foo` enabled, then the
// layout won't match.
```

Or function signatures. Or the (duplicate) code being run at any time.

### Duplicates are unavoidable in `xgraph`

In the `1graph` model, rustc is able to see the entire dependency graph — as a
result, it's able to avoid duplicates of a dependency altogether: if the app
and some of its modules depend on `tokio`, then there'll be a single
`libtokio.dylib` that they all depend on — no duplication whatsoever.

In the `xgraph` model, we're unable to achieve that. By design, the app and all
of its modules are built and linked in complete isolation. As long as they agree
on a thin FFI (Foreign Function Interface) boundary, which might be provided by
a "common" crate everyone depends on, they can be built.

It is possible for the app and its modules to link dynamically against `tokio`:
there will be, for each target (the app is a target, each module is a target),
a `libtokio.dylib` file.

However, that file will not have the same contents for each target, because `tokio`
exposes generic functions.

This code:

```rust
tokio::spawn(async move {
    println!("Hello, world!");
});
```

Will cause the `spawn` function to be monomorphized, turning from this:

```rust
pub fn spawn<F>(future: F) -> JoinHandle<F::Output> ⓘ
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
```

Into something like this (the mangling here is not realistic):

```rust
pub fn spawn__OpaqueType__FOO(future: OpaqueType__FOO) -> JoinHandle<()> ⓘ
```

If in another module, we have that code:

```rust
let jh = tokio::spawn(async move {
    // make yourself wanted
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    println!("Oh hey, you're early!")"
    42
});
let answer = jh.await.unwrap();
```

Then it will cause _another_ monomorphization of `tokio`'s `spawn` function,
which might look something like this:

```rust
pub fn spawn__OpaqueType__BAR(future: OpaqueType__BAR) -> JoinHandle<i32> ⓘ
```

And now, you'll have:

```
bin/
  app/
    executable
    libtokio.dylib
      (exports spawn__OpaqueType__FOO)
  mod_a/
    libmod_a.dylib
    libtokio.dylib
      (export spawn__OpaqueType__BAR)
```



---

rubicon lets those crates expose cargo features to either export or import:

  * thread-locals
  * process-locals (commonly called "statics" in Rust)

This takes care of problem #1, which is: make sure our thread-copy "singleton" actually only
exists once at runtime.

Problem #2 is still on you, specifically: YOU MUST MAKE SURE THE ABI MATCHES. That means using
the exact same version of the compiler. It also means making sure that every copy of `tokio`,
or `tracing-subscriber`, or whatever, has the EXACT SAME SET OF FEATURES ENABLED.

This is trickier than it sounds — you can't just have a `tokio-wrapper` crate of your own with
the features you need — any of your transitive dependencies (from the main app / any of its modules)
can sneakily enable an extra tokio feature. Cargo features are additive and there's no way to
denylist them.

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
