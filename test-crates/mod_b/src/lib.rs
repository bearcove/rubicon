use std::sync::atomic::Ordering;

#[no_mangle]
pub fn init() {
    mokio::MOKIO_TL1.with(|s| {
        rubicon::soprintln!("MOKIO_TL: {}", s.load(Ordering::Relaxed));
        rubicon::soprintln!("Adding 1 to it");
        s.fetch_add(1, Ordering::Relaxed);
    });

    rubicon::soprintln!("MOKIO_PL: {}", mokio::MOKIO_PL1.load(Ordering::Relaxed));
    rubicon::soprintln!("Adding 1 to MOKIO_PL");
    mokio::MOKIO_PL1.fetch_add(1, Ordering::Relaxed);
}
