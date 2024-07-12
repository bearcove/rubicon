#[cfg(all(feature = "export-globals", feature = "import-globals"))]
compile_error!("The features `export-globals` and `import-globals` cannot be used together");

use std::sync::Arc;

#[cfg(not(any(feature = "import-globals", feature = "export-globals")))]
#[macro_export]
macro_rules! thread_local {
    ($($tts:tt)+) => {
        ::std::thread_local!{ $($tts)+ }
    }
}

#[cfg(feature = "export-globals")]
#[macro_export]
macro_rules! thread_local {
    ($(#[$attrs:meta])* $vis:vis static $name:ident: $ty:ty = const { $expr:expr } $(;)?) => {
        thread_local! {
            $(#[$attrs])*
            $vis static $name: $ty = $expr;
        }
    };

    ($(#[$attrs:meta])* $vis:vis static $name:ident: $ty:ty = $expr:expr $(;)?) => {
        ::std::thread_local! {
            $(#[$attrs])*
            $vis static $name: $ty = $expr;
        }

        #[allow(non_snake_case)]
        mod $name {
            struct Rust1_79LocalKeyLayout<T: 'static> {
                inner: unsafe fn(Option<&mut Option<T>>) -> Option<&'static T>,
            }

            #[no_mangle]
            static $name: Rust1_79LocalKeyLayout<()> = Rust1_79LocalKeyLayout {
                inner: |v| {
                    unsafe {
                        // pretty weak guarantee but oh well
                        assert_eq!(
                            ::std::mem::size_of::<std::thread::LocalKey<()>>(),
                            ::std::mem::size_of::<Rust1_79LocalKeyLayout<()>>()
                        );

                        // we don't have `$ty` in this scope, so we can't put the proper annotations
                        #[allow(clippy::missing_transmute_annotations)]
                        let lk = ::std::mem::transmute::<_, Rust1_79LocalKeyLayout<()>>(super::$name);
                        (lk.inner)(v)
                    }
                }
            };
        }
    };
}

#[cfg(feature = "import-globals")]
#[macro_export]
macro_rules! thread_local {
    ($(#[$attrs:meta])* $vis:vis static $name:ident: $ty:ty = const { $expr:expr } $(;)?) => {
        ::std::thread_local! {
            $(#[$attrs])*
            $vis static $name: $ty = $expr;
        }
    };

    ($(#[$attrs:meta])* $vis:vis static $name:ident: $ty:ty = $expr:expr $(;)?) => {
        #[allow(non_snake_case)]
        mod $name {
            extern "C" {
                #[link_name = stringify!($name)]
                #[allow(improper_ctypes)]
                pub(super) static LK: ::std::thread::LocalKey<()>;
            }
        }

        static $name: &'static ::std::thread::LocalKey<$ty> = unsafe { std::mem::transmute(&$name::LK) };
    };
}

struct RubiconSample {
    contents: Arc<u64>,
}

crate::thread_local! {
    static RUBICON_SAMPLE: RubiconSample = RubiconSample {
        contents: Arc::new(123),
    };
}

pub fn world_goes_round() {
    RUBICON_SAMPLE.with(|s| {
        let contents = s.contents.clone();
        println!("contents: {}", contents);
    });
}
