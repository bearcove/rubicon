use std::sync::atomic::Ordering;

#[no_mangle]
pub fn init() {
    mokio::MOKIO_TL1.with(|s| s.fetch_add(1, Ordering::Relaxed));
    mokio::MOKIO_PL1.fetch_add(1, Ordering::Relaxed);
}
