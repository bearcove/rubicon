//! rubicon enables a dangerous form of dynamic linking in Rust through cdylib crates
//! and carefully-enforced invariants.
//!
//! This crate provides macros to handle global state (thread-locals and process-locals/statics)
//! in a way that's compatible with the "xgraph" dynamic linking model, where multiple
//! copies of the same crate can coexist in the same address space.
//!
//! The main macros provided are:
//!
//! - `thread_local!`: A drop-in replacement for `std::thread_local!`
//! - `process_local!`: Used to declare statics (including `static mut`)
//!
//! These macros behave differently depending on whether the `export-globals` or
//! `import-globals` feature is enabled:
//!
//! - With `export-globals`: Symbols are exported for use by dynamically loaded modules
//! - With `import-globals`: Symbols are imported from the main executable
//! - With neither: The macros act as pass-through to standard Rust constructs
//!
//! # Safety
//!
//! Using this crate requires careful adherence to several invariants:
//!
//! 1. Modules must never be unloaded, only loaded.
//! 2. The exact same Rust compiler version must be used for the app and all modules.
//! 3. The exact same cargo features must be enabled for shared dependencies.
//!
//! Failure to maintain these invariants can lead to undefined behavior.
//!
//! For more details on the motivation and implementation of the "xgraph" model,
//! refer to the [crate's README and documentation](https://github.com/bearcove/rubicon?tab=readme-ov-file#rubicon).

#[cfg(all(feature = "export-globals", feature = "import-globals"))]
compile_error!("The features `export-globals` and `import-globals` are mutually exclusive, see https://github.com/bearcove/rubicon");

#[cfg(any(feature = "export-globals", feature = "import-globals"))]
pub use paste::paste;

#[cfg(feature = "import-globals")]
pub use ctor;

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
pub struct TrustedExtern<T: 'static>(pub &'static T);

use std::ops::Deref;

impl<T> Deref for TrustedExtern<T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
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
pub struct TrustedExternDouble<T: 'static>(pub &'static &'static T);

impl<T> Deref for TrustedExternDouble<T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        // autoderef goes brrr
        self.0
    }
}

//==============================================================================
// Thread-locals
//==============================================================================

/// Imports or exports a thread-local, depending on the enabled cargo features.
///
/// Usage:
///
///   ```ignore
///   rubicon::thread_local! {
///       static FOO: AtomicU32 = AtomicU32::new(42);
///   }
///   ```
///
/// This will import `FOO` if the `import-globals` feature is enabled, and export it if the
/// `export-globals` feature is enabled.
///
/// If neither feature is enabled, this will be equivalent to `std::thread_local!`.
///
/// This macro supports multiple declarations:
///
///   ```ignore
///   rubicon::thread_local! {
///       static FOO: AtomicU32 = AtomicU32::new(42);
///       static BAR: AtomicU32 = AtomicU32::new(43);
///   }
///   ```
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
            static [<$name __rubicon_export>]: &::std::thread::LocalKey<$ty> = &$name;
        }
    };
}

#[cfg(feature = "import-globals")]
#[macro_export]
macro_rules! thread_local_inner {
    ($(#[$attrs:meta])* $vis:vis $name:ident, $ty:ty, $expr:expr) => {
        $crate::paste! {
            extern "Rust" {
                #[link_name = stringify!([<$name __rubicon_export>])]
                #[allow(improper_ctypes)]
                static [<$name __rubicon_import>]: &'static ::std::thread::LocalKey<$ty>;
            }

            // even though this ends up being not a LocalKey, but a type that Derefs to LocalKey,
            // in practice, most codebases work just fine with this, since they call methods
            // that takes `self: &LocalKey`: they don't see the difference.
            $vis static $name: $crate::TrustedExternDouble<::std::thread::LocalKey<$ty>> = $crate::TrustedExternDouble(unsafe { &[<$name __rubicon_import>] });
        }
    };
}

//==============================================================================
// Process-locals (statics)
//==============================================================================

/// Imports or exports a `static`, depending on the enabled cargo features.
///
/// Usage:
///
///   ```ignore
///   rubicon::process_local! {
///       static FOO: u32 = 42;
///   }
///   ```
///
/// This will import `FOO` if the `import-globals` feature is enabled, and export it if the
/// `export-globals` feature is enabled.
///
/// If neither feature is enabled, this will expand to the static declaration itself.
///
/// This macro supports multiple declarations, along with `static mut` declarations
/// (which have a slightly different expansion).
///
///   ```ignore
///   rubicon::thread_local! {
///       static FOO: AtomicU32 = AtomicU32::new(42);
///       static mut BAR: Dispatcher = Dispatcher::new();
///   }
///   ```
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
macro_rules! process_local_inner {
    ($(#[$attrs:meta])* $vis:vis $name:ident, $ty:ty, $expr:expr) => {
        $crate::paste! {
            extern "Rust" {
                #[link_name = stringify!([<$name __rubicon_export>])]
                #[allow(improper_ctypes)]
                static [<$name __rubicon_import>]: $ty;
            }

            $vis static $name: $crate::TrustedExtern<$ty> = $crate::TrustedExtern(unsafe { &[<$name __rubicon_import>] });
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

#[cfg(feature = "import-globals")]
#[macro_export]
macro_rules! compatibility_check {
    ($($feature:tt)*) => {
        use std::env;
        use $crate::ctor::ctor;

        extern "C" {
            #[link_name = concat!(env!("CARGO_PKG_NAME"), "_compatibility_info")]
            static COMPATIBILITY_INFO: &'static [(&'static str, &'static str)];
        }

        use $crate::libc::{c_void, Dl_info};
        use std::ffi::CStr;
        use std::ptr;

        extern "C" {
            fn dladdr(addr: *const c_void, info: *mut Dl_info) -> i32;
        }

        fn get_shared_object_name() -> Option<String> {
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

        #[ctor]
        fn check_compatibility() {
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

            error_message.push_str(&format!("Loading {} would mix different configurations of the {} crate.\n\n", blue(so_name), red(env!("CARGO_PKG_NAME"))));

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
                let binary_column = format_column(imported_value.as_deref().copied(), exported_value.as_deref().copied(), AnsiColor::RED);
                let module_column = format_column(exported_value.as_deref().copied(), imported_value.as_deref().copied(), AnsiColor::GREEN);

                fn format_column(primary: Option<&str>, secondary: Option<&str>, highlight_color: AnsiColor) -> String {
                    match primary {
                        Some(value) => {
                            if secondary.map_or(false, |v| v == value) {
                                colored(AnsiColor::GREY, value).to_string()
                            } else {
                                colored(highlight_color, value).to_string()
                            }
                        },
                        None => colored(AnsiColor::GREY, "∅").to_string(),
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

#[cfg(not(any(feature = "export-globals", feature = "import-globals")))]
#[macro_export]
macro_rules! compatibility_check {
    ($($feature:tt)*) => {};
}
