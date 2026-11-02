//! Unit tests of the pool's bookkeeping (spec 0002 O10, O11) with an
//! in-crate fake factory. The behavioural tests through an engine, with the
//! test kit's `ScriptedBackend`, are in `tests/pool.rs`; these reach the
//! states those cannot: a fork reset simulated by rewriting the recorded
//! pid, and an instance returned across a reset.

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;

use pdf_extract_core::{OcrUnavailable, UnavailableReason};
use pdf_extract_render::GrayImage;

use super::*;
use crate::backend::{BackendInfo, Linkage, OcrOutput, OcrRequest};
use crate::config::Isolation;

#[derive(Default)]
struct Fake {
    created: AtomicUsize,
    dropped: Arc<AtomicUsize>,
    fail: AtomicBool,
    /// Makes the next `create` (or `clear`) panic.
    panic_create: AtomicBool,
    panic_clear: Arc<AtomicBool>,
    /// When set, `create` makes this pool look forked first: a reset that
    /// lands while a creation is outside the lock.
    reset_during_create: Mutex<Option<std::sync::Weak<Pool<Fake>>>>,
}

struct FakeInstance {
    dropped: Arc<AtomicUsize>,
    panic_clear: Arc<AtomicBool>,
    cleared: usize,
}

impl Drop for FakeInstance {
    fn drop(&mut self) {
        self.dropped.fetch_add(1, Ordering::SeqCst);
    }
}

impl Recognizer for FakeInstance {
    fn recognize(&mut self, _: &GrayImage, _: &OcrRequest) -> Result<OcrOutput, OcrPageError> {
        Ok(OcrOutput::default())
    }
    fn clear(&mut self) {
        if self.panic_clear.swap(false, Ordering::SeqCst) {
            panic!("scripted panic in clear");
        }
        self.cleared += 1;
    }
}

impl RecognizerFactory for Fake {
    type Instance = FakeInstance;
    fn create(&self) -> Result<FakeInstance, OcrUnavailable> {
        let target = self.reset_during_create.lock().expect("lock").take();
        if let Some(pool) = target.and_then(|w| w.upgrade()) {
            pretend_forked(&pool);
        }
        if self.panic_create.swap(false, Ordering::SeqCst) {
            panic!("scripted panic in create");
        }
        if self.fail.load(Ordering::SeqCst) {
            return Err(OcrUnavailable {
                reason: UnavailableReason::InitFailed,
                tessdata: None,
                languages: vec![],
                searched: vec![],
                detail: "no".into(),
            });
        }
        self.created.fetch_add(1, Ordering::SeqCst);
        Ok(FakeInstance {
            dropped: self.dropped.clone(),
            panic_clear: self.panic_clear.clone(),
            cleared: 0,
        })
    }
    fn info(&self) -> BackendInfo {
        BackendInfo {
            name: "fake",
            version: String::new(),
            languages: vec![],
            tessdata: None::<PathBuf>,
            model_sha256: vec![],
            linkage: Linkage::System,
            isolation: Isolation::InProcess,
        }
    }
}

fn pool(max: usize) -> Pool<Fake> {
    Pool::new(Arc::new(Fake::default()), max, None)
}

/// Pretends the pool was created by another process.
fn pretend_forked(p: &Pool<Fake>) {
    let mut state = p.lock();
    state.pid = state.pid.wrapping_add(1);
}

#[test]
fn success_clears_and_returns_failure_drops() {
    let p = pool(2);
    assert_eq!(p.len(), 0);
    let cleared = p
        .run(|i| {
            assert_eq!(i.cleared, 0);
            Ok(())
        })
        .map(|()| p.lock().idle[0].cleared);
    assert_eq!(cleared.expect("ok"), 1);
    assert_eq!(p.len(), 1);
    let err = p
        .run(|_| Err::<(), _>(OcrPageError::Recognition("x".into())))
        .expect_err("fails");
    assert!(matches!(err, OcrError::Page(OcrPageError::Recognition(_))));
    assert_eq!(p.len(), 0);
    assert_eq!(p.factory().dropped.load(Ordering::SeqCst), 1);
}

#[test]
fn a_failed_creation_frees_its_reservation() {
    let p = pool(1);
    p.factory().fail.store(true, Ordering::SeqCst);
    let err = p.run(|_| Ok(())).expect_err("cannot create");
    assert!(matches!(
        err,
        OcrError::Unavailable(OcrUnavailable {
            reason: UnavailableReason::InitFailed,
            ..
        })
    ));
    assert_eq!(p.len(), 0);
    p.factory().fail.store(false, Ordering::SeqCst);
    p.run(|_| Ok(())).expect("recovers");
    assert_eq!(p.len(), 1);
}

#[test]
fn a_fork_reset_leaks_the_old_instances() {
    let p = Pool::new(Arc::new(Fake::default()), 2, None);
    p.run(|_| Ok(())).expect("ok");
    assert_eq!(p.len(), 1);
    let resets = fork_resets();
    pretend_forked(&p);
    assert_eq!(p.len(), 0, "the parent's instance is abandoned");
    assert!(fork_resets() > resets);
    // Leaked, never dropped (a parent thread may have been inside it).
    assert_eq!(p.factory().dropped.load(Ordering::SeqCst), 0);
    p.run(|_| Ok(())).expect("the child builds its own");
    assert_eq!(p.factory().created.load(Ordering::SeqCst), 2);
    assert_eq!(p.len(), 1);
}

#[test]
fn an_instance_returned_across_a_reset_is_leaked() {
    let p = pool(1);
    for healthy in [true, false] {
        let before = p.factory().dropped.load(Ordering::SeqCst);
        let slot = p.checkout().expect("checkout");
        pretend_forked(&p);
        assert_eq!(p.len(), 0);
        let mut slot = slot;
        slot.healthy = healthy;
        drop(slot);
        assert_eq!(p.len(), 0, "not returned to the new pool");
        assert_eq!(p.factory().dropped.load(Ordering::SeqCst), before);
    }
}

#[test]
fn a_creation_across_a_reset_does_not_corrupt_the_count() {
    for fail in [true, false] {
        let p = Arc::new(pool(2));
        p.factory().fail.store(fail, Ordering::SeqCst);
        *p.factory().reset_during_create.lock().expect("lock") = Some(Arc::downgrade(&p));
        let r = p.run(|_| Ok(()));
        assert_eq!(r.is_err(), fail);
        // The reset zeroed the count; neither the failed reservation nor
        // the instance made for the old generation may touch the new one.
        let state = p.lock();
        assert_eq!((state.live, state.idle.len()), (0, 0), "fail = {fail}");
    }
}

#[test]
fn dropping_a_pool_in_a_forked_child_leaks() {
    let p = pool(1);
    p.run(|_| Ok(())).expect("ok");
    let dropped = p.factory().dropped.clone();
    pretend_forked(&p);
    // `len` would reset; drop directly instead.
    drop(p);
    assert_eq!(dropped.load(Ordering::SeqCst), 0);

    let p = pool(1);
    p.run(|_| Ok(())).expect("ok");
    let dropped = p.factory().dropped.clone();
    drop(p);
    assert_eq!(dropped.load(Ordering::SeqCst), 1, "normal drop frees");
}

#[test]
fn a_poisoned_lock_is_recovered() {
    let p = Arc::new(pool(1));
    let q = p.clone();
    let _ = std::thread::spawn(move || {
        let _g = q.state.lock().expect("lock");
        panic!("poison the pool lock");
    })
    .join();
    assert!(p.state.is_poisoned());
    p.run(|_| Ok(())).expect("still usable");
    assert_eq!(p.len(), 1);
}

#[test]
fn a_panicking_page_drops_its_instance_and_frees_the_slot() {
    let p = pool(1);
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        p.run(|_| -> Result<(), OcrPageError> { panic!("inside recognition") })
    }));
    assert!(r.is_err());
    assert_eq!(p.len(), 0);
    assert_eq!(p.factory().dropped.load(Ordering::SeqCst), 1);
    p.run(|_| Ok(())).expect("slot was freed");
}

#[test]
fn max_zero_is_treated_as_one() {
    let p = pool(0);
    p.run(|_| Ok(())).expect("one slot");
    assert_eq!(p.len(), 1);
}

/// Runs `f` on a helper thread and fails the test instead of hanging if it
/// does not finish: the bug these tests pin is a wait that never ends. A
/// panic in `f` drops the sender, which must not be reported as a hang.
fn within_seconds<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    use std::sync::mpsc::RecvTimeoutError;
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(f());
    });
    match rx.recv_timeout(std::time::Duration::from_secs(30)) {
        Ok(v) => v,
        Err(RecvTimeoutError::Timeout) => panic!("the pool hung: a slot was leaked"),
        Err(RecvTimeoutError::Disconnected) => {
            panic!("the closure panicked (its message is above), not a hang")
        }
    }
}

#[test]
#[should_panic(expected = "the closure panicked")]
fn a_panic_under_within_seconds_is_not_reported_as_a_hang() {
    within_seconds(|| panic!("inside the closure"));
}

/// A caller asleep on a full pool is woken when a returning pre-fork slot
/// is what performs the fork reset: the reset frees every slot, while the
/// slot's own generation-mismatch return changes no count. Found by
/// tests/loom_pool.rs (`a_return_across_a_fork_reset`); before the fix the
/// waiter slept forever. The short sleep only makes it likely that the
/// waiter is already waiting; if it is not, the test passes vacuously.
#[test]
fn a_reset_by_a_returning_slot_wakes_a_waiter() {
    let p = Arc::new(pool(1));
    let slot = p.checkout().expect("checkout");
    let (tx, rx) = std::sync::mpsc::channel();
    let waiter = Arc::clone(&p);
    std::thread::spawn(move || {
        let _ = tx.send(waiter.run(|_| Ok(())).is_ok());
    });
    std::thread::sleep(std::time::Duration::from_millis(100));
    pretend_forked(&p);
    drop(slot);
    let ok = rx
        .recv_timeout(std::time::Duration::from_secs(30))
        .expect("the waiter was never woken");
    assert!(ok);
    assert_eq!(p.len(), 1, "the waiter's new instance, idle");
}

#[test]
fn a_panicking_create_frees_its_reservation() {
    within_seconds(|| {
        let p = pool(1);
        p.factory().panic_create.store(true, Ordering::SeqCst);
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| p.run(|_| Ok(()))));
        assert!(r.is_err(), "the panic propagates");
        assert_eq!(p.len(), 0);
        p.run(|_| Ok(())).expect("the next page gets the slot");
        assert_eq!(p.len(), 1);
    });
}

#[test]
fn a_panicking_clear_frees_its_slot() {
    within_seconds(|| {
        let p = pool(1);
        p.factory().panic_clear.store(true, Ordering::SeqCst);
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| p.run(|_| Ok(()))));
        assert!(r.is_err(), "the panic propagates");
        assert_eq!(p.len(), 0);
        assert_eq!(p.factory().dropped.load(Ordering::SeqCst), 1);
        p.run(|_| Ok(())).expect("the next page gets the slot");
        assert_eq!(p.len(), 1);
    });
}
