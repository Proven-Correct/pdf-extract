// Concurrent instances (spec 0002 §4.6): the probe for whether Tesseract's
// shared, ref-counted `Dict::GlobalDawgCache` survives concurrent Init and
// teardown. Instances are otherwise independent (Appendix B.2). In 5.5.3 the
// cache is an ObjectCache guarded by a std::mutex, and this test is the
// executable evidence that no Init serialisation is needed (§3.4's fallback
// would be a Mutex around pxt_init only).

mod common;

use std::sync::Barrier;
use std::time::Duration;

use common::{hello, tessdata};
use pdf_extract_tesseract_sys::{EngineMode, Instance};

const THREADS: usize = 8;
const ROUNDS: usize = 20;

#[test]
fn eight_threads_create_recognise_and_drop() {
    let dir = tessdata();
    let img = hello();
    // Start every thread's first Init at the same moment, so the dawg cache
    // sees concurrent first loads, not a staggered start.
    let barrier = Barrier::new(THREADS);
    let texts: Vec<Vec<String>> = std::thread::scope(|s| {
        let handles: Vec<_> = (0..THREADS)
            .map(|_| {
                s.spawn(|| {
                    barrier.wait();
                    (0..ROUNDS)
                        .map(|_| {
                            let mut inst =
                                Instance::new(&dir, "eng", EngineMode::LstmOnly).expect("Init");
                            inst.set_image(&img.data, img.width, img.height, img.width)
                                .expect("set_image");
                            inst.set_source_resolution(300).expect("resolution");
                            inst.recognize(Duration::from_secs(120)).expect("recognize");
                            let blocks = inst.blocks().expect("blocks");
                            blocks
                                .iter()
                                .flat_map(|b| &b.lines)
                                .flat_map(|l| &l.words)
                                .map(|w| w.text.as_str())
                                .collect::<Vec<_>>()
                                .join(" ")
                        })
                        .collect()
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().expect("worker"))
            .collect()
    });
    for t in texts.iter().flatten() {
        assert_eq!(t, "Hello");
    }
    assert_eq!(texts.iter().flatten().count(), THREADS * ROUNDS);
}
