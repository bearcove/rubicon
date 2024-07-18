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

At this point, `executable` refers to its own `libtokio.dylib` (by absolute path),
and `libmod_a.dylib`, to its own, separate, `libtokio.dylib`.

Even if you were to edit the `DT_NEEDED` / `LC_LOAD_DYLIB` information to have the
modules point to `executable`'s version of the dynamic libraries, you would find
yourself with a "missing symbol" error at runtime!

| libtokio.dylib from | Has __FOO | Has __BAR |
|---------------------|-----------|-----------|
|     executable      |     ✅    |     ❌    |
|        mod_a        |     ❌    |     ✅    |

None of the `libtokio.dylib` files you have contain all the symbols required.

To make a `libtokio.dylib` file that contains ALL THE SYMBOLS required, you
would need rustc to be aware of the whole dependency graph: hence, you'd be back
to the `1graph` model.

Hence, when using the `xgraph`, we accept the reality that code from dependencies
_will_ be duplicated.

| target | non-generic code | app generics | mod_a generics | mod_b generics |
|--------|------------------|--------------|----------------|----------------|
| app    |        ✅        |      ✅      |       ❌       |       ❌       |
| mod_a  |        ✅        |      ❌      |       ✅       |       ❌       |
| mod_b  |        ✅        |      ❌      |       ❌       |       ✅       |

That first column corresponds to all functions, types, etc. that are not generic,
or that are instantiated the exact same way in each independent depgraph.

There will be a copy of each of these in the application executable AND in each
`libmod_etc.dylib` file. That's unavoidable for now.

## Duplicating globals is never okay

Now that we've made our peace with the fact there _will_ be code duplication, and
that, as long as that code EXACTLY MATCHES across different copies, it's okay,
we need to address the fact that duplicating globals isn't _never okay_.

In particular, by globals, we mean:

  * thread-locals (declared via the [std::thread_local!][] macro)
  * process-locals (more commonly called "statics", declared via the [static keyword][])

```rust
static sample_process_local: AtomicU64 = AtomicU64::new(0);

std::thread_local! {
    static sample_thread_local: u64 = 42;
}

fn blah() {
    let sample_local = 42;
}
```

| kind                 | process-local | thread-local | local  |
|----------------------|---------------|--------------|--------|
| unique per scope     |      ❌       |      ❌      |   ✅   |
| unique per thread    |      ❌       |      ✅      |   ✅   |
| unique per process   |      ✅       |      ✅      |   ✅   |

[std::thread_local!]: https://doc.rust-lang.org/std/macro.thread_local.html
[static keyword]: https://doc.rust-lang.org/reference/items/static-items.html

Take `tracing`, for example: it lets you emit "events" that a "subscriber" can process.
It's used for structured logging: the event could be of level INFO and include information
about some HTTP request, for example.

`tracing` allows registering a "global" dispatcher, through [tracing::dispatcher::set_global_default][].
This sets a process-global:

[tracing::dispatcher::set_global_default]: https://docs.rs/tracing/latest/tracing/dispatcher/fn.set_global_default.html

```rust
static mut GLOBAL_DISPATCH: Dispatch = Dispatch {
    subscriber: Kind::Global(&NO_SUBSCRIBER),
};
```

The problem is that, since all targets (the app, all its modules) have their own
copy of `tracing`, they also have their own `GLOBAL_DISPATCH` process-local.

It doesn't matter to `mod_a` if we've registered a global dispatcher from the app:
according to `mod_a`'s copy of `GLOBAL_DISPATH` — there's no subscriber!

There's only one fix for this: everyone must share the same `GLOBAL_DISPATCH`:
it must be exported from `app`, and imported from all its modules.

## Global symbol commerce

In a perfect world, there'd be a rustc flag like `-C globals-linkage=[import,export]`:
we'd set it to `export` for our app, so that it would declare those as exported symbols,
the kind you can look up with [dlsym][], and that dynamic libraries you load later can
use, because they're part of the set of symbols the dynamic linker-loader searches.

[dlsym]: https://man7.org/linux/man-pages/man3/dlsym.3.html

There are, however, two roadblocks we must hop.

The first is that dynamic symbols are not exported for executables. Luckily, there's
a linker flag for that: `-rdynamic` (also known as `--export-dynamic`).

The second is that _there is no such rustc flag at all_.

Export a static is easy enough. Instead of:

```rust
static MERCHANDISE: u64 = 42;
```

We can do:

```rust
#[used]
static MERCHANDISE: u64 = 42;
```

And we'll get a mangled symbol:

```shell
❯ cargo build --quiet
❯ nm -gp ./target/debug/librubicon.dylib | grep MERCHANDISE
00000000000099f0 S __ZN7rubicon11MERCHANDISE17h03e39e78778de1fdE
```

The `#[no_mangle]` attribute implies `#[used]`, and also
disables name mangling:

```rust
#[no_mangle]
static MERCHANDISE: u64 = 42;
```

```shell
❯ cargo build --quiet
❯ nm -gp ./target/debug/librubicon.dylib | grep MERCHANDISE
00000000000099f0 S _MERCHANDISE
```

(Just ignore the `_` prefix — linkers are cute like that.)

In fact, we can even specify our own export name if we want:

```rust
#[export_name = "STILL_MERCHANDISE"]
static PINK_UNICORN: u64 = 42;
```

```shell
❯ cargo build --quiet
❯ nm -gp ./target/debug/librubicon.dylib | grep MERCHANDISE
00000000000099f0 S _STILL_MERCHANDISE
```

However, when importing, there is no way to opt into mangling.

We can either import it as-is, without mangling:

```rust
extern "C" {
    static MERCHANDISE: u64;
}

// (only here to force the linker to import MERCHANDISE)
#[used]
static MERCHANDISE_ADDR: &u64 = unsafe { &MERCHANDISE };
```

```shell
# needed to avoid link errors: `MERCHANDISE` is not present at link time, it's
# only expected to be present at load time.
❯ export RUSTFLAGS="-Clink-arg=-undefined -Clink-arg=dynamic_lookup"

❯ cargo build --quiet
❯ nm -gp ./target/debug/librubicon.dylib | grep MERCHANDISE
00000000000e0210 S __ZN7rubicon16MERCHANDISE_ADDR17h2755f244419dcf79E
                 U _MERCHANDISE
```

Or we can specify a `link_name` explicitly:

```rust
extern "C" {
    #[link_name = "STILL_MERCHANDISE"]
    static MERCHANDISE: u64;
}

// (only here to force the linker to import MERCHANDISE)
#[used]
static MERCHANDISE_ADDR: &u64 = unsafe { &MERCHANDISE };
```

```shell
00000000000e0210 S __ZN7rubicon16MERCHANDISE_ADDR17h2755f244419dcf79E
                 U _STILL_MERCHANDISE
```

All these alternatives, quite frankly, suck.

If we opt into mangling, we're safe from name collisions, but we _cannot_ import
that symbol again (I'm not counting "manually copying and pasting the mangled name
into Rust source code").

If we opt out of mangling, two crates that export `CURRENT_STATE` will clash.

In practice, we have no choice but to opt out of mangling, and make sure there's no
collision between the unmangled globals of various crates in the dependency graph —
which means, that's right, we're back to manually prefixing things, like in C.

We've just covered process-locals. The situation for thread-locals is much the
same, except worse, since the internals of `::std::thread::LocalKey` are private.

This:

```rust
std::thread_local! {
    static NUM_SUBSCRIBERS: AtomicUsize = AtomicUsize::new(0);
}
```

Expands (as of Rust 1.79) to something like this:

```rust
use std::thread::{LocalKey, local_impl::Key}

const NUM_SUBSCRIBERS: LocalKey<AtomicUsize> = {
    #[inline]
    fn __init() -> AtomicUsize {
        AtomicUsize::new(0)
    }
    #[inline]
    unsafe fn __getit(init: Option<&mut Option<AtomicUsize>>) -> Option<&'static AtomicUsize> {
        #[thread_local]
        static __KEY: Key<AtomicUsize> = Key::<AtomicUsize>::new();
        unsafe {
            __KEY.get(move || {
                if let Option::Some(init) = init {
                    if let Option::Some(value) = init.take() {
                        return value;
                    }
                    unreachable!("init invariants violated");
                }
                __init()
            })
        }
    }
    unsafe { LocalKey::new(__getit) }
};
```

However, we can't build a `LocalKey` ourselves it has a hidden constructor that's
"rustc-const-unstable":

```rust
impl<T: 'static> LocalKey<T> {
    #[doc(hidden)]
    #[unstable(
        feature = "thread_local_internals",
        reason = "recently added to create a key",
        issue = "none"
    )]
    #[rustc_const_unstable(feature = "thread_local_internals", issue = "none")]
    pub const unsafe fn new(
        inner: unsafe fn(Option<&mut Option<T>>) -> Option<&'static T>,
    ) -> LocalKey<T> {
        LocalKey { inner }
    }
}
```

If you're targetting nightly, you can get access to this yourself:

```rust
#![allow(internal_features)]
#![feature(thread_local_internals)]
```

But rubicon targest Rust stable at the time of this writing.
