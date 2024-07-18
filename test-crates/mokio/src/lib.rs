use std::sync::atomic::AtomicU64;

rubicon::process_local! {
    pub static MOKIO_PL1: AtomicU64 = AtomicU64::new(0);
    pub static MOKIO_PL2: AtomicU64 = AtomicU64::new(0);

    pub static mut DANGEROUS: u64 = 0;
    static DANGEROUS_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());
}

rubicon::thread_local! {
    pub static MOKIO_TL1: AtomicU64 = AtomicU64::new(0);
    pub static MOKIO_TL2: AtomicU64 = AtomicU64::new(0);
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
