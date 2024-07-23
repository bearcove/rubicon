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
                write!(f, "\x1b[{}m{}\x1b[0m", self.0, self.1)
            }
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
            AnsiEscape(90, d)
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

            error_message.push_str(&format!("Loading this module would mix different configurations of the {} crate.\n\n", red(env!("CARGO_PKG_NAME"))));

            // Compute max lengths for alignment
            let max_exported_len = exported.iter().map(|(k, v)| format!("{}={}", k, v).len()).max().unwrap_or(0);
            let max_ref_len = imported.iter().map(|(k, v)| format!("{}={}", k, v).len()).max().unwrap_or(0);
            let column_width = max_exported_len.max(max_ref_len);

            let binary_label = format!("Binary {}", blue(&exe_name));
            let module_label = format!("Module {}", blue(so_name));
            println!("visible_len(binary_label) = {}", visible_len(&binary_label));
            println!("visible_len(module_label) = {}", visible_len(&module_label));

            let binary_label_width = visible_len(&binary_label);
            let module_label_width = visible_len(&module_label);
            let binary_padding = " ".repeat(column_width.saturating_sub(binary_label_width));
            let module_padding = " ".repeat(column_width.saturating_sub(module_label_width));

            error_message.push_str(&format!("{}{}    {}{}\n",
                binary_label,
                binary_padding,
                module_label,
                module_padding
            ));
            error_message.push_str(&format!("{:━<width$}    {:━<width$}\n", "", "", width = column_width));

            let mut i = 0;
            let mut j = 0;

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

            for key in all_keys.iter() {
                let exported_value = exported.iter().find(|&(k, _)| k == key).map(|(_, v)| v);
                let imported_value = imported.iter().find(|&(k, _)| k == key).map(|(_, v)| v);

                match (exported_value, imported_value) {
                    (Some(value), Some(expected_value)) => {
                        // Item in both
                        if value == expected_value {
                            let left_item = format!("{}{}{}", grey(key), grey("="), grey(value));
                            let right_item = format!("{}{}{}", grey(key), grey("="), grey(expected_value));
                            let left_item_len = key.len() + value.len() + 1; // +1 for '='
                            let padding = " ".repeat(column_width.saturating_sub(left_item_len));
                            error_message.push_str(&format!("{}{}    {}\n", left_item, padding, right_item));
                        } else {
                            let left_item = format!("{}{}{}", blue(key), grey("="), green(value));
                            let right_item = format!("{}{}{}", blue(key), grey("="), red(expected_value));
                            let left_item_len = key.len() + value.len() + 1; // +1 for '='
                            let padding = " ".repeat(column_width.saturating_sub(left_item_len));
                            error_message.push_str(&format!("{}{}    {}\n", left_item, padding, right_item));
                        }
                    }
                    (Some(value), None) => {
                        // Item only in exported
                        let left_item = format!("{}{}{}", green(key), grey("="), green(value));
                        let right_item = format!("{}", red("MISSING!"));
                        let left_item_len = key.len() + value.len() + 1; // +1 for '='
                        let padding = " ".repeat(column_width.saturating_sub(left_item_len));
                        error_message.push_str(&format!("{}{}    {}\n", left_item, padding, right_item));
                    }
                    (None, Some(value)) => {
                        // Item only in imported
                        let left_item = format!("{}", red("MISSING!"));
                        let right_item = format!("{}{}{}", green(key), grey("="), green(value));
                        let left_item_len = "MISSING!".len();
                        let padding = " ".repeat(column_width.saturating_sub(left_item_len));
                        error_message.push_str(&format!("{}{}    {}\n", left_item, padding, right_item));
                    }
                    (None, None) => {
                        // This should never happen as the key is from all_keys
                        unreachable!()
                    }
                }
            }

            error_message.push_str("\nDifferent feature sets may result in different struct layouts, which\n");
            error_message.push_str("would lead to memory corruption. Instead, we're going to panic now.\n\n");

            error_message.push_str("More info: \x1b[4m\x1b[34mhttps://crates.io/crates/rubicon\x1b[0m\n");

            let rebuild_line = format!("To fix this issue, {} needs to enable", blue(so_name));
            let transitive_line = format!("the same cargo features as {} for crate {}.", blue(&exe_name), red(env!("CARGO_PKG_NAME")));
            let empty_line = "";
            let hint_line = "\x1b[34mHINT:\x1b[0m";
            let cargo_tree_line = format!("Run `cargo tree -i {} -e features` from both.", red(env!("CARGO_PKG_NAME")));

            let lines = vec![
                &rebuild_line,
                &transitive_line,
                empty_line,
                hint_line,
                &cargo_tree_line,
            ];

            let max_width = lines.iter().map(|line| visible_len(line)).max().unwrap_or(0);
            let box_width = max_width + 4; // Add 4 for left and right borders and spaces

            error_message.push_str("\n");
            error_message.push_str(&format!("┌{}┐\n", "─".repeat(box_width - 2)));

            for line in lines {
                if line.is_empty() {
                    error_message.push_str(&format!("│{}│\n", " ".repeat(box_width - 2)));
                } else {
                    let visible_line_len = visible_len(line);
                    let padding = " ".repeat(box_width - 4 - visible_line_len);
                    error_message.push_str(&format!("│ {}{} │\n", line, padding));
                }
            }

            error_message.push_str(&format!("└{}┘\n", "─".repeat(box_width - 2)));
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
