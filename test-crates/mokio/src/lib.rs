use std::sync::{atomic::AtomicU64, Arc, Mutex};

rubicon::compatibility_check! {
    ("mokio_pkg_version", env!("CARGO_PKG_VERSION")),
    #[cfg(not(feature = "timer"))]
    ("timer", "disabled"),
    #[cfg(feature = "timer")]
    ("timer", "enabled"),
}

#[derive(Default)]
#[cfg(feature = "timer")]
struct TimerInternals {
    #[allow(dead_code)]
    random_stuff: [u64; 4],
}

#[derive(Default)]
pub struct Runtime {
    #[cfg(feature = "timer")]
    #[allow(dead_code)]
    timer: TimerInternals,

    // this field is second on purpose so that it'll be offset
    // if the feature is enabled/disabled
    pub counter: u64,
}

rubicon::process_local! {
    pub static MOKIO_PL1: AtomicU64 = AtomicU64::new(0);
    pub static MOKIO_PL2: AtomicU64 = AtomicU64::new(0);

    pub static mut DANGEROUS: u64 = 0;
    static DANGEROUS_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());
}

rubicon::thread_local! {
    pub static MOKIO_TL1: AtomicU64 = AtomicU64::new(0);
    pub static MOKIO_TL2: Arc<Mutex<Runtime>> = Arc::new(Mutex::new(Runtime::default()));
}

pub fn inc_dangerous() -> u64 {
    let _guard = DANGEROUS_MUTEX.lock().unwrap();
    unsafe {
        DANGEROUS += 1;
        DANGEROUS
    }
}

pub fn get_dangerous() -> u64 {
    let _guard = DANGEROUS_MUTEX.lock().unwrap();
    unsafe { DANGEROUS }
}
