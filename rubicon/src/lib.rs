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

        #[ctor]
        fn check_compatibility() {
            let ref_compatibility_info: &[(&str, &str)] = &[
                ("rustc-version", $crate::RUBICON_RUSTC_VERSION),
                ("target-triple", $crate::RUBICON_TARGET_TRIPLE),
                $($feature)*
            ];

            let exported = unsafe { COMPATIBILITY_INFO };

            let missing: Vec<_> = ref_compatibility_info.iter().filter(|&item| !exported.contains(item)).collect();
            let extra: Vec<_> = exported.iter().filter(|&item| !ref_compatibility_info.contains(item)).collect();

            if !missing.is_empty() || !extra.is_empty() {
                eprintln!("Compatibility mismatch detected!");
                if !missing.is_empty() {
                    eprintln!("Missing features: {:?}", missing);
                }
                if !extra.is_empty() {
                    eprintln!("Extra features: {:?}", extra);
                }
                std::process::exit(1);
            }
        }
    };
}

#[cfg(not(any(feature = "export-globals", feature = "import-globals")))]
#[macro_export]
macro_rules! compatibility_check {
    ($($feature:tt)*) => {};
}
