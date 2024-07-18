#[cfg(all(feature = "export-globals", feature = "import-globals"))]
compile_error!("The features `export-globals` and `import-globals` cannot be used together");

pub use paste::paste;

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

//===== crimes

/// This gets rid of Rust compiler errors when trying to refer to an `extern`
/// static. That error is there for a reason, but we're doing crimes.
pub struct TrustedExtern<T: 'static>(pub &'static T);

use std::ops::Deref;

impl<T> Deref for TrustedExtern<T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        self.0
    }
}

pub struct TrustedExternDouble<T: 'static>(pub &'static &'static T);

impl<T> Deref for TrustedExternDouble<T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        // autoderef plays a role here
        self.0
    }
}

//===== thread-locals

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

//===== process-locals (statics)

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

    // handle multiple declarations
    ($(#[$attrs:meta])* $vis:vis static $name:ident: $ty:ty = $expr:expr; $($rest:tt)*) => {
        $crate::process_local_inner!($(#[$attrs])* $vis $name, $ty, $expr);
        $crate::process_local!($($rest)*);
    };
}

#[cfg(feature = "export-globals")]
#[macro_export]
macro_rules! process_local_inner {
    ($(#[$attrs:meta])* $vis:vis $name:ident, $ty:ty, $expr:expr) => {
        $crate::paste! {
            // we _could_ export with a mangled name, but we couldn't
            // import with a mangled name (extern disables mangling)
            #[export_name = stringify!([<$name __rubicon_export>])]
            $(#[$attrs])*
            $vis static $name: $ty = $expr;
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

//===== soprintln!

#[no_mangle]
static SHARED_OBJECT_ID_REF: u64 = 0;

/// Returns a unique identifier for the current shared object
/// (based on the address of the `shared_object_id_ref` function).
pub fn shared_object_id() -> u64 {
    &SHARED_OBJECT_ID_REF as *const _ as u64
}

#[cfg(feature = "import-globals")]
pub static RUBICON_MODE: &str = "I"; // "import"

#[cfg(feature = "export-globals")]
pub static RUBICON_MODE: &str = "E"; // "export"

#[cfg(not(any(feature = "import-globals", feature = "export-globals")))]
pub static RUBICON_MODE: &str = "N"; // "normal"

#[cfg(all(feature = "import-globals", feature = "export-globals"))]
compile_error!("The features \"import-globals\" and \"export-globals\" are mutually exclusive");

/// A u64 value, with an automatically-generated foreground and background color,
/// with a `Display` implementation that prints the value with 24-bit color ANSI escape codes.
pub struct Beacon<'a> {
    fg: (u8, u8, u8),
    bg: (u8, u8, u8),
    name: &'a str,
    val: u64,
}

impl<'a> Beacon<'a> {
    /// Creates a new `Beacon` from a pointer.
    pub fn from_ptr<T>(name: &'a str, ptr: *const T) -> Self {
        Self::new(name, ptr as u64)
    }

    /// Creates a new `Beacon` from a reference.
    pub fn from_ref<T>(name: &'a str, r: &T) -> Self {
        Self::new(name, r as *const T as u64)
    }

    /// Creates a new `Beacon` with the given extra string and value.
    pub fn new(name: &'a str, u: u64) -> Self {
        fn hash(mut x: u64) -> u64 {
            const K: u64 = 0x517cc1b727220a95;
            x = x.wrapping_mul(K);
            x ^= x >> 32;
            x = x.wrapping_mul(K);
            x ^= x >> 32;
            x = x.wrapping_mul(K);
            x
        }

        let hashed_float = (hash(u) as f64) / (u64::MAX as f64);
        let h = hashed_float * 360.0;
        let s = 50.0;
        let l = 70.0;

        fn hsl_to_rgb(h: f64, s: f64, l: f64) -> (u8, u8, u8) {
            let h = h / 360.0;
            let s = s / 100.0;
            let l = l / 100.0;

            let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
            let x = c * (1.0 - ((h * 6.0) % 2.0 - 1.0).abs());
            let m = l - c / 2.0;

            let (r, g, b) = match (h * 6.0) as u8 {
                0 | 6 => (c, x, 0.0),
                1 => (x, c, 0.0),
                2 => (0.0, c, x),
                3 => (0.0, x, c),
                4 => (x, 0.0, c),
                _ => (c, 0.0, x),
            };

            (
                ((r + m) * 255.0) as u8,
                ((g + m) * 255.0) as u8,
                ((b + m) * 255.0) as u8,
            )
        }

        let fg = hsl_to_rgb(h, s, l);
        let bg = hsl_to_rgb(h, s * 0.8, l * 0.5);

        Self {
            fg,
            bg,
            name,
            val: u,
        }
    }
}

impl<'a> std::fmt::Display for Beacon<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "\x1b[48;2;{};{};{}m\x1b[38;2;{};{};{}m{}#{:0x}\x1b[0m",
            self.bg.0, self.bg.1, self.bg.2, self.fg.0, self.fg.1, self.fg.2, self.name, self.val
        )
    }
}

/// Prints a message, prefixed with a cycling millisecond timestamp (wraps at 99999),
/// a colorized shared object id, a colorized thread name+id, and the given message.
#[macro_export]
#[cfg(feature = "soprintln")]
macro_rules! soprintln {
    ($($arg:tt)*) => {
        {
            use std::sync::atomic::{AtomicBool, Ordering};
            static ENV_CHECKED: std::sync::Once = std::sync::Once::new();
            static SHOULD_PRINT: AtomicBool = AtomicBool::new(false);
            ENV_CHECKED.call_once(|| {
                let should_print = std::env::var("SO_PRINTLN").map(|v| v == "1").unwrap_or(false);
                SHOULD_PRINT.store(should_print, Ordering::Relaxed);
            });

            if SHOULD_PRINT.load(Ordering::Relaxed) {
                // this formatting is terribly wasteful — PRs welcome

                let so_id = $crate::shared_object_id();
                let so_mode_and_id = $crate::Beacon::new($crate::RUBICON_MODE, so_id);
                let curr_thread = std::thread::current();
                let tid = format!("{:?}", curr_thread.id());
                // strip `ThreadId(` prefix
                let tid = tid.strip_prefix("ThreadId(").unwrap_or(&tid);
                // strip `)` suffix
                let tid = tid.strip_suffix(")").unwrap_or(&tid);
                // parse tid as u64
                let tid = tid.parse::<u64>().unwrap_or(0);

                let thread_name = curr_thread.name().unwrap_or("<unnamed>");
                let thread = $crate::Beacon::new(thread_name, tid);

                let timestamp = ::std::time::SystemTime::now().duration_since(::std::time::UNIX_EPOCH).unwrap().as_millis() % 99999;
                // FIXME: this is probably not necessary, but without it, rustc complains about
                // capturing variables in format_args?
                let msg = format!($($arg)*);
                eprintln!("{timestamp:05} {so_mode_and_id} {thread} {msg}");
            }
        }
    };
}

#[macro_export]
#[cfg(not(feature = "soprintln"))]
macro_rules! soprintln {
    ($($arg:tt)*) => {};
}

struct RubiconSample {
    contents: Arc<u64>,
}

crate::thread_local! {
    static RUBICON_TL_SAMPLE1: RubiconSample = RubiconSample {
        contents: Arc::new(12),
    };
}

crate::thread_local! {
    static RUBICON_TL_SAMPLE2: RubiconSample = RubiconSample {
        contents: Arc::new(23),
    };
}

crate::thread_local! {
    static RUBICON_TL_SAMPLE3: RubiconSample = RubiconSample {
        contents: Arc::new(34),
    };
    static RUBICON_TL_SAMPLE4: RubiconSample = RubiconSample {
        contents: Arc::new(45),
    }
}

crate::process_local! {
    static RUBICON_PL_SAMPLE1: AtomicU64 = AtomicU64::new(12);
}

crate::process_local! {
    static RUBICON_PL_SAMPLE2: AtomicU64 = AtomicU64::new(23);
}

crate::process_local! {
    static RUBICON_PL_SAMPLE3: AtomicU64 = AtomicU64::new(34);
    static RUBICON_PL_SAMPLE4: AtomicU64 = AtomicU64::new(45);
}

pub fn world_goes_round() {
    crate::soprintln!("hi");
    RUBICON_TL_SAMPLE1.with(|s| {
        let contents = s.contents.clone();
        println!("TL_SAMPLE1: {}", contents);
    });
    RUBICON_TL_SAMPLE2.with(|s| {
        let contents = s.contents.clone();
        println!("TL_SAMPLE2: {}", contents);
    });
    RUBICON_TL_SAMPLE3.with(|s| {
        let contents = s.contents.clone();
        println!("TL_SAMPLE3: {}", contents);
    });
    RUBICON_TL_SAMPLE4.with(|s| {
        let contents = s.contents.clone();
        println!("TL_SAMPLE4: {}", contents);
    });

    RUBICON_PL_SAMPLE1.fetch_add(1, Ordering::Relaxed);
    RUBICON_PL_SAMPLE2.fetch_add(1, Ordering::Relaxed);
    RUBICON_PL_SAMPLE3.fetch_add(1, Ordering::Relaxed);
    RUBICON_PL_SAMPLE4.fetch_add(1, Ordering::Relaxed);
}
