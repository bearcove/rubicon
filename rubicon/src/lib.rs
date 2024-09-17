//! ![The rubicon logo: a shallow river in northeastern Italy famously crossed by Julius Caesar in 49 BC](https://github.com/user-attachments/assets/7e10888d-9f44-4395-a2ad-3e3fc0801996)
//!
//! _Logo by [MisiasArt](https://misiasart.com)_
//!
//! rubicon enables a dangerous form of dynamic linking in Rust through cdylib crates
//! and carefully-enforced invariants.
//!
//! This crate provides macros to handle global state (thread-locals and process-locals/statics)
//! in a way that's compatible with the "xgraph" dynamic linking model, where multiple
//! copies of the same crate can coexist in the same address space.
//!
//! The main macros provided are:
//!
//! - [`thread_local!`]: A drop-in replacement for [`std::thread_local!`]
//! - [`process_local!`]: Used to declare statics (including `static mut`)
//!
//! These macros behave differently depending on which feature is enabled:
//!
//! - `export-globals`: symbols are exported for use by other shared objects
//! - `import-globals`: symbols are imported from "the dynamic loader namespace"
//! - neither: the macros act as pass-through to standard Rust constructs
//!
//! Additionally, the [`compatibility_check!`] macro is provided to help ensure that
//! common dependencies used by various shared objects are ABI-compatible.
//!
//! ## Explain like I'm five
//!
//! Let's assume you're a very precocious five-year old: say you're making a
//! static site generator. Part of its job is to compile LaTeX markup into HTML:
//! this uses KaTeX, which requires a JavaScript runtime, that takes a long time
//! to compile.
//!
//! You decide you want to put this functionality in a shared object, so that you
//! can iterate on the _rest_ of the static site generator without the whole JS
//! runtime being recompiled every time, or even taken into account by cargo when
//! doing check/clippy/build/test/etc.
//!
//! However, both your app and your "latex module" use the [tracing](https://crates.io/crates/tracing)
//! crate for structured logging. tracing uses "globals" (thread-locals and process-locals) to
//! keep track of the current span, and where to log events (ie. the "subscriber").
//!
//! If you do `tracing_subscriber::fmt::init()` from the app, any use of `tracing` in the app
//! will work fine, but if you do the same from the module, the log events will go nowhere:
//! as far as it's concerned (because it has a copy of the entire code of `tracing`), there
//! _is_ no subscriber.
//!
//! This is where `rubicon` comes in: by patching `tracing` to use rubicon's macros, like
//! [`thread_local!`] and [`process_local!`], we can have the app _export_ the globals, and
//! the module _import_ them, so that there's only one "global subscriber" for all shared
//! objects.
//!
//! ## That's it?
//!
//! Not quite — it's actually annoyingly hard to export symbols from an executable. So really
//! what you have instead is a `rubicon-exports` shared object that both the app and the module
//! link against, and import all globals from.
//!
//! ## Why isn't this built into rustc/cargo?
//!
//! Because of the "Safety" section below. However, I believe if we work together,
//! we can make this crate redundant. A global `-C globals-linkage=[import,export]`
//! rustc flag would singlehandedly solve the problem.
//!
//! Someone just has to do it. In the meantime, this crate (and source-patching crates like
//! `tokio`, `tracing`, `parking_lot`, `eyre`, see the [compatibility tracker](https://github.com/bearcove/rubicon/issues/3).
//!
//! ## Safety
//!
//! By using this crate, you agree to:
//!
//! 1. Use the exact same rustc version for all shared objects
//! 2. Not use [`-Z randomize-layout`](https://github.com/rust-lang/rust/issues/77316) (duh)
//! 3. Enable the exact same cargo features for all common dependencies (e.g. `tokio`)
//!
//! In short: don't do anything that would cause crates to have a different ABI from one shared
//! object to the next. 1 and 2 are trivial, as for 3, the [`compatibility_check!`] macro is here
//! to help.
//!
//! For more details on the motivation and implementation of the "xgraph" model,
//! refer to the [crate's README and documentation](https://github.com/bearcove/rubicon?tab=readme-ov-file#rubicon).

#[cfg(all(feature = "export-globals", feature = "import-globals"))]
compile_error!("The features `export-globals` and `import-globals` are mutually exclusive, see https://github.com/bearcove/rubicon");

#[cfg(any(feature = "export-globals", feature = "import-globals"))]
pub use paste::paste;

#[cfg(feature = "import-globals")]
pub use libc;

#[cfg(any(feature = "export-globals", feature = "import-globals"))]
pub const RUBICON_RUSTC_VERSION: &str = env!("RUBICON_RUSTC_VERSION");

#[cfg(any(feature = "export-globals", feature = "import-globals"))]
pub const RUBICON_TARGET_TRIPLE: &str = env!("RUBICON_TARGET_TRIPLE");

//==============================================================================
// Wrappers
//==============================================================================

/// Wrapper around an `extern` `static` ref to avoid requiring `unsafe` for imported globals.
#[doc(hidden)]
pub struct TrustedExtern<T: 'static>(pub &'static T, pub fn());

use std::ops::Deref;

impl<T> Deref for TrustedExtern<T> {
    type Target = T;

    #[inline(always)]
    fn deref(&self) -> &Self::Target {
        // this is a good time to run compatibility checks
        #[cfg(not(feature = "no-compatibility-checks-yolo"))]
        (self.1)();

        self.0
    }
}

/// Wrapper around an `extern` `static` double-ref to avoid requiring `unsafe` for imported globals.
///
/// The reason we have a double-ref is that when exporting thread-locals, the dynamic symbol is
/// already a ref. Then, in our own static, we can only access the address of that ref, not its
/// value (since its value is only known as load time, not compile time).
///
/// As a result, imported thread-locals have an additional layer of indirection.
#[doc(hidden)]
pub struct TrustedExternDouble<T: 'static>(pub &'static &'static T, pub fn());

impl<T> Deref for TrustedExternDouble<T> {
    type Target = T;

    #[inline(always)]
    fn deref(&self) -> &Self::Target {
        // this is a good time to run compatibility checks
        #[cfg(not(feature = "no-compatibility-checks-yolo"))]
        (self.1)();

        self.0
    }
}

//==============================================================================
// Thread-locals
//==============================================================================

/// A drop-in replacement for [`std::thread_local`] that imports/exports the
/// thread-local, depending on the enabled cargo features.
///
/// Before:
///
/// ```rust
/// # use std::sync::atomic::AtomicU32;
/// std::thread_local! {
///     static FOO: AtomicU32 = AtomicU32::new(42);
/// }
/// ```
///
/// After:
///
/// ```rust
/// # use std::sync::atomic::AtomicU32;
/// rubicon::thread_local! {
///     static FOO: AtomicU32 = AtomicU32::new(42);
/// }
/// ```
///
/// This will import `FOO` if the `import-globals` feature is enabled, and export it if the
/// `export-globals` feature is enabled.
///
/// rubicon tries to be non-obtrusive: when neither feature is enabled, the macro
/// forwards to [`std::thread_local`], resulting in no performance penalty,
/// no difference in binary size, etc.
///
/// ## Name mangling, collisions
///
/// When the `import-globals` or `export-globals` feature is enabled, name mangling
/// will be disabled for thread-locals declared through this macro (due to unfortunate
/// limitations of the Rust attributes used to implement this).
///
/// We recommend prefixing your thread-locals with your crate/module name to
/// avoid collisions:
///
/// ```rust
/// # use std::sync::atomic::AtomicU32;
/// rubicon::thread_local! {
///     static MY_CRATE_FOO: AtomicU32 = AtomicU32::new(42);
/// }
/// ```
///
/// ## Multiple declarations
///
/// This macro supports multiple declarations in the same invocation, just like
/// [`std::thread_local`] would:
///
/// ```rust
/// # use std::sync::atomic::AtomicU32;
/// rubicon::thread_local! {
///     static FOO: AtomicU32 = AtomicU32::new(42);
///     static BAR: AtomicU32 = AtomicU32::new(43);
/// }
/// ```
#[cfg(not(any(feature = "import-globals", feature = "export-globals")))]
#[macro_export]
macro_rules! thread_local {
    ($($tts:tt)+) => {
        ::std::thread_local!{ $($tts)+ }
    }
}

#[cfg(any(feature = "export-globals", feature = "import-globals"))]
#[macro_export]
macro_rules! thread_local {
    // empty (base case for the recursion)
    () => {};

    ($(#[$attrs:meta])* $vis:vis static $name:ident: $ty:ty = const { $expr:expr } $(;)?) => {
        $crate::thread_local! {
            $(#[$attrs])*
            $vis static $name: $ty = $expr;
        }
    };

    ($(#[$attrs:meta])* $vis:vis static $name:ident: $ty:ty = $expr:expr $(;)?) => {
        $crate::thread_local_inner!($(#[$attrs])* $vis $name, $ty, $expr);
    };

    // handle multiple declarations
    ($(#[$attr:meta])* $vis:vis static $name:ident: $t:ty = $init:expr; $($rest:tt)*) => (
        $crate::thread_local_inner!($(#[$attr])* $vis $name, $t, $init);
        $crate::thread_local!($($rest)*);
    );
}

#[cfg(feature = "export-globals")]
#[macro_export]
macro_rules! thread_local_inner {
    ($(#[$attrs:meta])* $vis:vis $name:ident, $ty:ty, $expr:expr) => {
        $crate::paste! {
            // regular thread-local macro, not exported.
            ::std::thread_local! {
                $(#[$attrs])*
                $vis static $name: $ty = $expr;
            }

            #[no_mangle]
            #[allow(clippy::non_upper_case_globals)]
            static [<$name __rubicon_export>]: &::std::thread::LocalKey<$ty> = &$name;
        }
    };
}

#[cfg(feature = "import-globals")]
#[macro_export]
#[allow(clippy::crate_in_macro_def)] // we _do_ mean the invocation site's crate, not the macro's
macro_rules! thread_local_inner {
    ($(#[$attrs:meta])* $vis:vis $name:ident, $ty:ty, $expr:expr) => {
        $crate::paste! {
            extern "Rust" {
                #[link_name = stringify!([<$name __rubicon_export>])]
                #[allow(improper_ctypes)]
                #[allow(clippy::non_upper_case_globals)]
                static [<$name __rubicon_import>]: &'static ::std::thread::LocalKey<$ty>;
            }

            // even though this ends up being not a LocalKey, but a type that Derefs to LocalKey,
            // in practice, most codebases work just fine with this, since they call methods
            // that takes `self: &LocalKey`: they don't see the difference.
            $vis static $name: $crate::TrustedExternDouble<::std::thread::LocalKey<$ty>> = $crate::TrustedExternDouble(unsafe { &[<$name __rubicon_import>] }, crate::compatibility_check_once);
        }
    };
}

//==============================================================================
// Process-locals (statics)
//==============================================================================

/// Imports or exports a `static`, depending on the enabled cargo features.
///
/// Before:
///
/// ```rust
/// static FOO: u32 = 42;
/// ```
///
/// After:
///
/// ```rust
/// rubicon::process_local! {
///     static FOO: u32 = 42;
/// }
/// ```
///
/// This will import `FOO` if the `import-globals` feature is enabled, and export it if the
/// `export-globals` feature is enabled.
///
/// rubicon tries to be non-obtrusive: when neither feature is enabled, the macro
/// will expand to the static declaration itself, resulting in no performance penalty,
/// no difference in binary size, etc.
///
/// ## Name mangling, collisions
///
/// When the `import-globals` or `export-globals` feature is enabled, name mangling
/// will be disabled for process-locals declared through this macro (due to unfortunate
/// limitations of the Rust attributes used to implement this).
///
/// We recommend prefixing your process-locals with your crate/module name to
/// avoid collisions:
///
/// ```rust
/// rubicon::process_local! {
///     static MY_CRATE_FOO: u32 = 42;
/// }
/// ```
///
/// ## Multiple declarations, `mut`
///
/// This macro supports multiple declarations, along with `static mut` declarations
/// (which have a slightly different expansion).
///
/// ```rust
/// # use std::sync::atomic::AtomicU32;
/// # struct Dispatcher;
/// # impl Dispatcher {
/// #     const fn new() -> Self { Self }
/// # }
/// rubicon::process_local! {
///     static FOO: AtomicU32 = AtomicU32::new(42);
///     static mut BAR: Dispatcher = Dispatcher::new();
/// }
/// ```
///
/// If you're curious about the exact macro expansion, ask rust-analyzer to
/// expand it for you via its [Expand Macro Recursively](https://rust-analyzer.github.io/manual.html#expand-macro-recursively)
/// functionalityl.
#[cfg(all(not(feature = "import-globals"), not(feature = "export-globals")))]
#[macro_export]
macro_rules! process_local {
    // pass through
    ($($tts:tt)+) => {
        $($tts)+
    }
}

#[cfg(any(feature = "export-globals", feature = "import-globals"))]
#[macro_export]
macro_rules! process_local {
    // empty (base case for the recursion)
    () => {};

    // single declaration
    ($(#[$attrs:meta])* $vis:vis static $name:ident: $ty:ty = $expr:expr $(;)?) => {
        $crate::process_local_inner!($(#[$attrs])* $vis $name, $ty, $expr);
    };

    // single declaration (mut)
    ($(#[$attrs:meta])* $vis:vis static mut $name:ident: $ty:ty = $expr:expr $(;)?) => {
        $crate::process_local_inner_mut!($(#[$attrs])* $vis $name, $ty, $expr);
    };


    // handle multiple declarations
    ($(#[$attrs:meta])* $vis:vis static $name:ident: $ty:ty = $expr:expr; $($rest:tt)*) => {
        $crate::process_local_inner!($(#[$attrs])* $vis $name, $ty, $expr);
        $crate::process_local!($($rest)*);
    };

    // handle multiple declarations
    ($(#[$attrs:meta])* $vis:vis static mut $name:ident: $ty:ty = $expr:expr; $($rest:tt)*) => {
        $crate::process_local_inner_mut!($(#[$attrs])* $vis $name, $ty, $expr);
        $crate::process_local!($($rest)*);
    }
}

#[cfg(feature = "export-globals")]
#[macro_export]
macro_rules! process_local_inner {
    ($(#[$attrs:meta])* $vis:vis $name:ident, $ty:ty, $expr:expr) => {
        $crate::paste! {
            #[export_name = stringify!([<$name __rubicon_export>])]
            $(#[$attrs])*
            $vis static $name: $ty = $expr;
        }
    };
}

#[cfg(feature = "export-globals")]
#[macro_export]
macro_rules! process_local_inner_mut {
    ($(#[$attrs:meta])* $vis:vis $name:ident, $ty:ty, $expr:expr) => {
        $crate::paste! {
            #[export_name = stringify!([<$name __rubicon_export>])]
            $(#[$attrs])*
            $vis static mut $name: $ty = $expr;
        }
    };
}

#[cfg(feature = "import-globals")]
#[macro_export]
#[allow(clippy::crate_in_macro_def)] // we _do_ mean the invocation site's crate, not the macro's
macro_rules! process_local_inner {
    ($(#[$attrs:meta])* $vis:vis $name:ident, $ty:ty, $expr:expr) => {
        $crate::paste! {
            extern "Rust" {
                #[link_name = stringify!([<$name __rubicon_export>])]
                #[allow(improper_ctypes)]
                #[allow(clippy::non_upper_case_globals)]
                static [<$name __rubicon_import>]: $ty;
            }

            $vis static $name: $crate::TrustedExtern<$ty> = $crate::TrustedExtern(unsafe { &[<$name __rubicon_import>] }, crate::compatibility_check_once);
        }
    };
}

#[cfg(feature = "import-globals")]
#[macro_export]
macro_rules! process_local_inner_mut {
    ($(#[$attrs:meta])* $vis:vis $name:ident, $ty:ty, $expr:expr) => {
        $crate::paste! {
            // externs require "unsafe" to access, but so do "static mut", so,
            // no need to wrap in `TrustedExtern`
            extern "Rust" {
                #[link_name = stringify!([<$name __rubicon_export>])]
                #[allow(improper_ctypes)]
                $vis static mut $name: $ty;
            }
        }
    };
}

//==============================================================================
// Compatibility check
//==============================================================================

#[cfg(feature = "export-globals")]
#[macro_export]
macro_rules! compatibility_check {
    ($($feature:tt)*) => {
        use std::env;

        $crate::paste! {
            #[no_mangle]
            #[export_name = concat!(env!("CARGO_PKG_NAME"), "_compatibility_info")]
            static __RUBICON_COMPATIBILITY_INFO_: &'static [(&'static str, &'static str)] = &[
                ("rustc-version", $crate::RUBICON_RUSTC_VERSION),
                ("target-triple", $crate::RUBICON_TARGET_TRIPLE),
                $($feature)*
            ];
        }
    };
}

#[cfg(all(unix, feature = "import-globals"))]
#[macro_export]
macro_rules! compatibility_check {
    ($($feature:tt)*) => {
        use std::env;

        extern "Rust" {
            #[link_name = concat!(env!("CARGO_PKG_NAME"), "_compatibility_info")]
            static COMPATIBILITY_INFO: &'static [(&'static str, &'static str)];
        }


        fn get_shared_object_name() -> Option<String> {
            use $crate::libc::{c_void, Dl_info};
            use std::ffi::CStr;
            use std::ptr;

            extern "C" {
                fn dladdr(addr: *const c_void, info: *mut Dl_info) -> i32;
            }

            unsafe {
                let mut info: Dl_info = std::mem::zeroed();
                if dladdr(get_shared_object_name as *const c_void, &mut info) != 0 {
                    let c_str = CStr::from_ptr(info.dli_fname);
                    return Some(c_str.to_string_lossy().into_owned());
                }
            }
            None
        }

        struct AnsiEscape<D: std::fmt::Display>(u64, D);

        impl<D: std::fmt::Display> std::fmt::Display for AnsiEscape<D> {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                let inner = format!("\x1b[{}m{}\x1b[0m", self.0, self.1);
                f.pad(&inner)
            }
        }

        #[derive(Clone, Copy)]
        struct AnsiColor(u64);

        impl AnsiColor {
            const BLUE: AnsiColor = AnsiColor(34);
            const GREEN: AnsiColor = AnsiColor(32);
            const RED: AnsiColor = AnsiColor(31);
            const GREY: AnsiColor = AnsiColor(37);
        }

        fn colored<D: std::fmt::Display>(color: AnsiColor, d: D) -> AnsiEscape<D> {
            AnsiEscape(color.0, d)
        }
        fn blue<D: std::fmt::Display>(d: D) -> AnsiEscape<D> {
            AnsiEscape(34, d)
        }
        fn green<D: std::fmt::Display>(d: D) -> AnsiEscape<D> {
            AnsiEscape(32, d)
        }
        fn red<D: std::fmt::Display>(d: D) -> AnsiEscape<D> {
            AnsiEscape(31, d)
        }
        fn grey<D: std::fmt::Display>(d: D) -> AnsiEscape<D> {
            AnsiEscape(35, d)
        }

        // Helper function to count visible characters (ignoring ANSI escapes)
        fn visible_len(s: &str) -> usize {
            let mut len = 0;
            let mut in_escape = false;
            for c in s.chars() {
                if c == '\x1b' {
                    in_escape = true;
                } else if in_escape {
                    if c.is_alphabetic() {
                        in_escape = false;
                    }
                } else {
                    len += 1;
                }
            }
            len
        }

        // this one is _actually_ meant to exist once per shared object
        static COMPATIBILITY_CHECK_ONCE: std::sync::Once = std::sync::Once::new();

        pub fn compatibility_check_once() {
            COMPATIBILITY_CHECK_ONCE.call_once(|| {
                check_compatibility();
            });
        }

        pub fn check_compatibility() {
            let imported: &[(&str, &str)] = &[
                ("rustc-version", $crate::RUBICON_RUSTC_VERSION),
                ("target-triple", $crate::RUBICON_TARGET_TRIPLE),
                $($feature)*
            ];
            let exported = unsafe { COMPATIBILITY_INFO };

            let missing: Vec<_> = imported.iter().filter(|&item| !exported.contains(item)).collect();
            let extra: Vec<_> = exported.iter().filter(|&item| !imported.contains(item)).collect();

            if missing.is_empty() && extra.is_empty() {
                // all good
                return;
            }

            let so_name = get_shared_object_name().unwrap_or("unknown_so".to_string());
            // get only the last bit of the path
            let so_name = so_name.rsplit('/').next().unwrap_or("unknown_so");

            let exe_name = std::env::current_exe().map(|p| p.file_name().unwrap().to_string_lossy().to_string()).unwrap_or_else(|_| "unknown_exe".to_string());

            let mut error_message = String::new();
            error_message.push_str("\n\x1b[31m━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\x1b[0m\n");
            error_message.push_str(&format!(" 💀 Feature mismatch for crate \x1b[31m{}\x1b[0m\n\n", env!("CARGO_PKG_NAME")));

            error_message.push_str(&format!("{} has an incompatible configuration for {}.\n\n", blue(so_name), red(env!("CARGO_PKG_NAME"))));

            // Compute max lengths for alignment
            let max_exported_len = exported.iter().map(|(k, v)| format!("{}={}", k, v).len()).max().unwrap_or(0);
            let max_ref_len = imported.iter().map(|(k, v)| format!("{}={}", k, v).len()).max().unwrap_or(0);
            let column_width = max_exported_len.max(max_ref_len);

            // Gather all unique keys
            let mut all_keys: Vec<&str> = Vec::new();
            for (key, _) in exported.iter() {
                if !all_keys.contains(key) {
                    all_keys.push(key);
                }
            }
            for (key, _) in imported.iter() {
                if !all_keys.contains(key) {
                    all_keys.push(key);
                }
            }

            struct Grid {
                rows: Vec<Vec<String>>,
                column_widths: Vec<usize>,
            }

            impl Grid {
                fn new() -> Self {
                    Grid {
                        rows: Vec::new(),
                        column_widths: Vec::new(),
                    }
                }

                fn add_row(&mut self, row: Vec<String>) {
                    if self.column_widths.len() < row.len() {
                        self.column_widths.resize(row.len(), 0);
                    }
                    for (i, cell) in row.iter().enumerate() {
                        self.column_widths[i] = self.column_widths[i].max(visible_len(cell));
                    }
                    self.rows.push(row);
                }

                fn write_to(&self, out: &mut String) {
                    let total_width: usize = self.column_widths.iter().sum::<usize>() + self.column_widths.len() * 3 - 1;

                    // Top border
                    out.push_str(&format!("┌{}┐\n", "─".repeat(total_width)));

                    for (i, row) in self.rows.iter().enumerate() {
                        if i == 1 {
                            // Separator after header
                            out.push_str(&format!("╞{}╡\n", "═".repeat(total_width)));
                        }

                        for (j, cell) in row.iter().enumerate() {
                            out.push_str("│ ");
                            out.push_str(cell);
                            out.push_str(&" ".repeat(self.column_widths[j] - visible_len(cell)));
                            out.push_str(" ");
                        }
                        out.push_str("│\n");
                    }

                    // Bottom border
                    out.push_str(&format!("└{}┘\n", "─".repeat(total_width)));
                }
            }

            let mut grid = Grid::new();

            // Add header
            grid.add_row(vec!["Key".to_string(), format!("Binary {}", blue(&exe_name)), format!("Module {}", blue(so_name))]);

            for key in all_keys.iter() {
                let exported_value = exported.iter().find(|&(k, _)| k == key).map(|(_, v)| v);
                let imported_value = imported.iter().find(|&(k, _)| k == key).map(|(_, v)| v);

                let key_column = colored(AnsiColor::GREY, key).to_string();
                let binary_column = format_column(exported_value.as_deref().copied(), imported_value.as_deref().copied(), AnsiColor::GREEN);
                let module_column = format_column(imported_value.as_deref().copied(), exported_value.as_deref().copied(), AnsiColor::RED);

                fn format_column(primary: Option<&str>, secondary: Option<&str>, highlight_color: AnsiColor) -> String {
                    match primary {
                        Some(value) => {
                            if secondary.map_or(false, |v| v == value) {
                                colored(AnsiColor::GREY, value).to_string()
                            } else {
                                colored(highlight_color, value).to_string()
                            }
                        },
                        None => colored(AnsiColor::RED, "∅").to_string(),
                    }
                }

                grid.add_row(vec![key_column, binary_column, module_column]);
            }

            grid.write_to(&mut error_message);

            struct MessageBox {
                lines: Vec<String>,
                max_width: usize,
            }

            impl MessageBox {
                fn new() -> Self {
                    MessageBox {
                        lines: Vec::new(),
                        max_width: 0,
                    }
                }

                fn add_line(&mut self, line: String) {
                    self.max_width = self.max_width.max(visible_len(&line));
                    self.lines.push(line);
                }

                fn add_empty_line(&mut self) {
                    self.lines.push(String::new());
                }

                fn write_to(&self, out: &mut String) {
                    let box_width = self.max_width + 4;

                    out.push_str("\n");
                    out.push_str(&format!("┌{}┐\n", "─".repeat(box_width - 2)));

                    for line in &self.lines {
                        if line.is_empty() {
                            out.push_str(&format!("│{}│\n", " ".repeat(box_width - 2)));
                        } else {
                            let visible_line_len = visible_len(line);
                            let padding = " ".repeat(box_width - 4 - visible_line_len);
                            out.push_str(&format!("│ {}{} │\n", line, padding));
                        }
                    }

                    out.push_str(&format!("└{}┘", "─".repeat(box_width - 2)));
                }
            }

            error_message.push_str("\nDifferent feature sets may result in different struct layouts, which\n");
            error_message.push_str("would lead to memory corruption. Instead, we're going to panic now.\n\n");

            error_message.push_str("More info: \x1b[4m\x1b[34mhttps://crates.io/crates/rubicon\x1b[0m\n");

            let mut message_box = MessageBox::new();
            message_box.add_line(format!("To fix this issue, {} needs to enable", blue(so_name)));
            message_box.add_line(format!("the same cargo features as {} for crate {}.", blue(&exe_name), red(env!("CARGO_PKG_NAME"))));
            message_box.add_empty_line();
            message_box.add_line("\x1b[34mHINT:\x1b[0m".to_string());
            message_box.add_line(format!("Run `cargo tree -i {} -e features` from both.", red(env!("CARGO_PKG_NAME"))));

            message_box.write_to(&mut error_message);
            error_message.push_str("\n\x1b[31m━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\x1b[0m\n");

            panic!("{}", error_message);
        }
    };
}

#[cfg(all(not(unix), feature = "import-globals"))]
#[macro_export]
macro_rules! compatibility_check {
    ($($feature:tt)*) => {
        // compatibility checks are only supported on unix-like system
    };
}

/// Performs a compatibility check for the crate when using Rubicon's dynamic linking features.
///
/// This macro is mandatory when the `import-globals` feature is enabled (as of Rubicon 3.3.3).
/// It exports information about the crate's version and enabled features, which is then used
/// by the import macros to ensure compatibility between different shared objects.
///
/// # Usage
///
/// At a minimum, you should include the crate's version:
///
/// ```
/// rubicon::compatibility_check! {
///     ("version", env!("CARGO_PKG_VERSION")),
/// }
/// ```
///
/// For crates with feature flags that affect struct layouts, you should include those as well:
///
/// ```
/// rubicon::compatibility_check! {
///     ("version", env!("CARGO_PKG_VERSION")),
///     #[cfg(feature = "my_feature")]
///     ("my_feature", "enabled"),
///     #[cfg(feature = "another_feature")]
///     ("another_feature", "enabled"),
/// }
/// ```
///
/// # Why is this necessary?
///
/// When using Rubicon for dynamic linking, different shared objects may handle the same structs.
/// If these shared objects have different features enabled, it can lead to incompatible struct
/// layouts, causing memory corruption and safety issues.
///
/// For example, in the Tokio runtime, enabling different features like timers or file system
/// support can change the internal structure of various components. If one shared object expects
/// a struct with certain fields (due to its feature set) and another shared object operates on
/// that struct with a different expectation, it can lead to undefined behavior.
///
/// This macro ensures that all shared objects agree on the crate's configuration, preventing
/// such mismatches.
///
/// # Real-world example (from tokio)
///
/// See [this pull request](https://github.com/bearcove/tokio/pull/2)
///
/// ```
/// rubicon::compatibility_check! {
///     ("version", env!("CARGO_PKG_VERSION")),
///     #[cfg(feature = "fs")]
///     ("fs", "enabled"),
///     #[cfg(feature = "io-util")]
///     ("io-util", "enabled"),
///     #[cfg(feature = "io-std")]
///     ("io-std", "enabled"),
///     #[cfg(feature = "net")]
///     ("net", "enabled"),
///     #[cfg(feature = "process")]
///     ("process", "enabled"),
///     #[cfg(feature = "rt")]
///     ("rt", "enabled"),
///     #[cfg(feature = "rt-multi-thread")]
///     ("rt-multi-thread", "enabled"),
///     #[cfg(feature = "signal")]
///     ("signal", "enabled"),
///     #[cfg(feature = "sync")]
///     ("sync", "enabled"),
///     #[cfg(feature = "time")]
///     ("time", "enabled"),
/// }
/// ```
///
/// # When does the check happen and what happens if it fails?
///
/// The check happens at runtime, lazily, when a global imported from a rubicon-aware
/// crate is accessed (behind a [`std::sync::Once`]).
///
/// If the check fails, the process will panic with a message like:
///
/// ```text
/// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
///  💀 Feature mismatch for crate mokio
///
/// libmod_b.dylib has an incompatible configuration for mokio.
///
/// ┌──────────────────────────────────────────────────────────────────┐
/// │ Key               │ Binary samplebin     │ Module libmod_b.dylib │
/// ╞══════════════════════════════════════════════════════════════════╡
/// │ rustc-version     │ 1.81.0               │ 1.81.0                │
/// │ target-triple     │ aarch64-apple-darwin │ aarch64-apple-darwin  │
/// │ mokio_pkg_version │ 0.1.0                │ 0.1.0                 │
/// │ timer             │ disabled             │ enabled               │
/// │ timer_is_disabled │ 1                    │ ∅                     │
/// └──────────────────────────────────────────────────────────────────┘
///
/// Different feature sets may result in different struct layouts, which
/// would lead to memory corruption. Instead, we're going to panic now.
///
/// More info: https://crates.io/crates/rubicon
///
/// ┌───────────────────────────────────────────────────────┐
/// │ To fix this issue, libmod_b.dylib needs to enable     │
/// │ the same cargo features as samplebin for crate mokio. │
/// │                                                       │
/// │ HINT:                                                 │
/// │ Run `cargo tree -i mokio -e features` from both.      │
/// └───────────────────────────────────────────────────────┘
/// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
/// ```

#[cfg(not(any(feature = "export-globals", feature = "import-globals")))]
#[macro_export]
macro_rules! compatibility_check {
    ($($feature:tt)*) => {};
}
