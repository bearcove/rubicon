use std::sync::atomic::AtomicU64;

rubicon::process_local! {
    pub static MOKIO_PL1: AtomicU64 = AtomicU64::new(0);
    pub static MOKIO_PL2: AtomicU64 = AtomicU64::new(0);
}

rubicon::thread_local! {
    pub static MOKIO_TL1: AtomicU64 = AtomicU64::new(0);
    pub static MOKIO_TL2: AtomicU64 = AtomicU64::new(0);
}
