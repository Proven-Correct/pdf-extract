//! Model-checks the OCR instance pool's checkout and return (`Pool`,
//! spec 0002 O10/O11) with loom: every interleaving, within loom's
//! preemption bound, of callers, failed creations and a simulated fork
//! reset (concurrency spec, "The OCR pool's checkout and return are
//! model-checked").
//!
//! Compiled only under `--cfg loom`, where the crate's `sync` shim is loom's
//! and `Pool` is re-exported. A scripted factory stands in for Tesseract,
//! and a fork is staged by changing the pool's pid source, a loom atomic.
//! Loom fails a case on a deadlock, which is how a lost wake-up shows: a
//! caller left asleep on the condvar with nobody to wake it. Run with:
//!
//! ```text
//! RUSTFLAGS="--cfg loom" LOOM_MAX_PREEMPTIONS=3 \
//!     cargo test -p pdf-extract-ocr --test loom_pool --release
//! ```

#![cfg(loom)]

use std::path::PathBuf;

use loom::sync::Arc;
use loom::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use loom::thread;
use pdf_extract_core::{OcrError, OcrPageError, OcrUnavailable, UnavailableReason};
use pdf_extract_ocr::{
    BackendInfo, GrayImage, Isolation, Linkage, OcrOutput, OcrRequest, Pool, Recognizer,
    RecognizerFactory,
};

loom::lazy_static! {
    /// The simulated process id; loom resets it for every execution.
    static ref PID: AtomicU32 = AtomicU32::new(1);
}

fn fake_pid() -> u32 {
    PID.load(Ordering::SeqCst)
}

/// A scripted factory. Counts are loom atomics so that every access is a
/// point loom can interleave at.
#[derive(Default)]
struct Fake {
    /// `create` calls so far, failed ones included.
    attempts: AtomicUsize,
    /// Fail the first `fail_first` creations.
    fail_first: usize,
    /// Instances in existence, from creation to drop or abandon.
    live: Arc<AtomicUsize>,
    /// Instances abandoned after a fork reset.
    abandoned: Arc<AtomicUsize>,
    /// Creations plus pages in progress; each holds a counted slot.
    busy: AtomicUsize,
    /// When set, `busy` may never exceed it. Left unset in the fork cases,
    /// where a reset rightly stops counting pre-fork pages still running.
    busy_cap: Option<usize>,
}

impl Fake {
    fn capped(max: usize) -> Self {
        Self {
            busy_cap: Some(max),
            ..Self::default()
        }
    }

    /// Marks one more slot in use, checking the cap.
    fn enter(&self) {
        let busy = self.busy.fetch_add(1, Ordering::SeqCst) + 1;
        if let Some(cap) = self.busy_cap {
            assert!(busy <= cap, "{busy} slots in use, max is {cap}");
        }
    }

    fn leave(&self) {
        self.busy.fetch_sub(1, Ordering::SeqCst);
    }
}

struct Instance {
    /// The (simulated) process the instance was created in.
    born_pid: u32,
    live: Arc<AtomicUsize>,
    abandoned: Arc<AtomicUsize>,
    /// Set by `abandon` so the drop it ends in is not counted twice.
    gone: bool,
}

impl Drop for Instance {
    fn drop(&mut self) {
        if !self.gone {
            self.live.fetch_sub(1, Ordering::SeqCst);
        }
    }
}

impl Recognizer for Instance {
    fn recognize(&mut self, _: &GrayImage, _: &OcrRequest) -> Result<OcrOutput, OcrPageError> {
        Ok(OcrOutput::default())
    }
    fn clear(&mut self) {}
    /// Counted and then dropped rather than leaked as the default does: a
    /// leaked loom `Arc` fails the model, and the property checked is that
    /// the pool abandons, not what abandoning does.
    fn abandon(mut self) {
        self.live.fetch_sub(1, Ordering::SeqCst);
        self.abandoned.fetch_add(1, Ordering::SeqCst);
        self.gone = true;
    }
}

impl RecognizerFactory for Fake {
    type Instance = Instance;
    fn create(&self) -> Result<Instance, OcrUnavailable> {
        self.enter();
        let fail = self.attempts.fetch_add(1, Ordering::SeqCst) < self.fail_first;
        self.leave();
        if fail {
            return Err(OcrUnavailable {
                reason: UnavailableReason::InitFailed,
                tessdata: None,
                languages: vec![],
                searched: vec![],
                detail: "scripted failure".into(),
            });
        }
        self.live.fetch_add(1, Ordering::SeqCst);
        Ok(self.instance())
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

impl Fake {
    fn instance(&self) -> Instance {
        Instance {
            born_pid: fake_pid(),
            live: self.live.clone(),
            abandoned: self.abandoned.clone(),
            gone: false,
        }
    }
}

type Shared = Arc<Pool<Fake>>;

fn pool(fake: Fake, max: usize, with_first: bool) -> Shared {
    let first = with_first.then(|| {
        fake.live.fetch_add(1, Ordering::SeqCst);
        fake.instance()
    });
    Arc::new(Pool::with_pid_source(
        std::sync::Arc::new(fake),
        max,
        first,
        fake_pid,
    ))
}

/// One page through the pool; `ok` decides whether the instance comes back
/// healthy (returned) or not (dropped). Inside the page the cap must hold.
///
/// The cap is on the pool's count of live or reserved slots, not on
/// instances in existence: an instance dropped after an error is freed
/// after its slot is, outside the lock, so the factory's `live` count can
/// briefly exceed `max` by design. What is checked is the pool's own count
/// and, where `busy_cap` is set, that creations plus pages in progress
/// (each a counted slot) never exceed `max`.
fn page(pool: &Shared, max: usize, ok: bool) -> Result<u32, OcrError> {
    pool.run(|instance| {
        pool.factory().enter();
        assert!(pool.len() <= max, "more than max live or reserved");
        pool.factory().leave();
        if ok {
            Ok(instance.born_pid)
        } else {
            Err(OcrPageError::Recognition("scripted".into()))
        }
    })
}

/// Three callers against `max_instances = 2`: no interleaving has more
/// than two instances live or reserved, and every caller finishes (a lost
/// wake-up would leave one asleep: a loom deadlock). The third caller's
/// page fails, so the drop-on-error return is in the mix.
#[test]
fn three_callers_never_exceed_two_instances() {
    loom::model(|| {
        let p = pool(Fake::capped(2), 2, false);
        let callers: Vec<_> = [true, true, false]
            .into_iter()
            .map(|ok| {
                let p = p.clone();
                thread::spawn(move || page(&p, 2, ok).is_ok())
            })
            .collect();
        let outcomes: Vec<bool> = callers
            .into_iter()
            .map(|c| c.join().expect("caller"))
            .collect();
        assert_eq!(outcomes, [true, true, false]);
        assert!(p.len() <= 2);
        assert_eq!(p.len(), p.live_instances());
    });
}

/// A failed creation frees its slot: with `max_instances = 1` and the
/// first creation scripted to fail, the other caller (which may be asleep
/// on the full pool) is woken and succeeds, and the count goes back to
/// what it was before the failure.
#[test]
fn a_failed_creation_wakes_a_waiter() {
    loom::model(|| {
        let fake = Fake {
            fail_first: 1,
            ..Fake::capped(1)
        };
        let p = pool(fake, 1, false);
        let callers: Vec<_> = (0..2)
            .map(|_| {
                let p = p.clone();
                thread::spawn(move || page(&p, 1, true))
            })
            .collect();
        let results: Vec<_> = callers
            .into_iter()
            .map(|c| c.join().expect("caller"))
            .collect();
        let failed = results
            .iter()
            .filter(|r| matches!(r, Err(OcrError::Unavailable(_))))
            .count();
        assert_eq!(failed, 1, "exactly the scripted creation fails");
        assert!(results.iter().any(Result::is_ok));
        // The failure's reservation was given back: only the successful
        // instance is counted, now idle.
        assert_eq!(p.len(), 1);
        assert_eq!(p.live_instances(), 1);
    });
}

/// A return across a fork reset: one caller's instance (made before the
/// fork) comes back while the process id changes and another caller,
/// possibly asleep on the full pool, wants an instance. The old instance is
/// abandoned, never reused; the count stays within `0..=max`; and nobody is
/// left asleep.
#[test]
fn a_return_across_a_fork_reset() {
    loom::model(|| {
        let p = pool(Fake::default(), 1, true);
        let callers: Vec<_> = (0..2)
            .map(|_| {
                let p = p.clone();
                thread::spawn(move || page(&p, 1, true).expect("page"))
            })
            .collect();
        PID.store(2, Ordering::SeqCst);
        for c in callers {
            c.join().expect("caller");
        }
        // After the fork, only post-fork instances are handed out.
        let born = page(&p, 1, true).expect("page after the fork");
        assert_eq!(born, 2, "a pre-fork instance was reused after the reset");
        assert_eq!(p.abandoned(), 1, "the pre-fork instance is abandoned");
        assert!(p.len() <= 1);
    });
}

/// `Slot::drop`'s generation-mismatch path: a caller holds a pre-fork
/// instance while another caller sees the new process id, resets the pool
/// and creates a fresh instance; a third caller competes for the one slot.
/// The pre-fork instance returns to nowhere (abandoned), the counts of the
/// new pool stay exact, and every caller finishes.
#[test]
fn a_pre_fork_slot_returns_to_nowhere() {
    loom::model(|| {
        let p = pool(Fake::default(), 1, true);
        let holder = {
            let p = p.clone();
            thread::spawn(move || page(&p, 1, true).expect("page"))
        };
        let forked = {
            let p = p.clone();
            thread::spawn(move || {
                PID.store(2, Ordering::SeqCst);
                let born = page(&p, 1, true).expect("page");
                assert_eq!(born, 2, "a caller after the fork got a pre-fork instance");
            })
        };
        let other = {
            let p = p.clone();
            thread::spawn(move || page(&p, 1, true).expect("page"))
        };
        holder.join().expect("holder");
        forked.join().expect("forked caller");
        other.join().expect("other caller");
        assert_eq!(p.abandoned(), 1, "the pre-fork instance is abandoned");
        assert_eq!(p.len(), 1, "exactly the new instance, idle");
        assert_eq!(p.live_instances(), 1);
    });
}

/// Observations through the factory the pool holds.
trait Observe {
    fn live_instances(&self) -> usize;
    fn abandoned(&self) -> usize;
}

impl Observe for Pool<Fake> {
    fn live_instances(&self) -> usize {
        self.factory().live.load(Ordering::SeqCst)
    }
    fn abandoned(&self) -> usize {
        self.factory().abandoned.load(Ordering::SeqCst)
    }
}
