//! Tests for fork change H9, thread-safe lazy xref repair, and
//! `Pdf::is_encrypted` (spec §3.2 H9 row, §4.8b, D-34, D-35).
//!
//! hayro repairs a broken xref table lazily: the first lookup that finds no
//! object where the table says one is rebuilds the table from a scan of the
//! whole file. Before H9 that path used non-blocking lock attempts that
//! panicked on contention and asserted that no other thread had repaired
//! first, so concurrent lookups during a repair panicked. These tests drive
//! the repair from many threads at once and pin that every lookup succeeds,
//! that the map is replaced exactly once, and that lookups of intact objects
//! are not held up by the scan.
//!
//! The broken input is `testkit::contract::misoffset_object`, which shifts
//! the xref offset of one page's content stream. The spec's oracle names
//! `XRef::get::<Dict>`; for a stream object that returns the stream's
//! dictionary (pinned below against the intact file), and the tests also read
//! the stream itself and compare its decoded bytes, which is what a text
//! extractor will actually need.
//!
//! The counters and the pause point come from H9's per-thread test hooks
//! (`hayro_syntax::xref::repair_hooks`), so tests running in parallel in this
//! binary do not see each other's repairs.
//!
//! The proof that the same workload fails on hayro without H9 needs a second
//! build of hayro and lives in its own test binary, `h9_pre_fix.rs`.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc;
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::{Duration, Instant};

use hayro_syntax::Pdf;
use hayro_syntax::object::{Dict, Object, ObjectIdentifier, Stream};
use hayro_syntax::xref::repair_hooks;
use pdf_extract_testkit::contract;
use pdf_extract_testkit::hostile::{self, TreeShape};

/// Threads and lookups per thread for the concurrent-lookup bullet.
const THREADS: usize = 16;
const LOOKUPS: usize = 100;
/// Fresh documents the concurrent-lookup test repeats the workload on.
const ROUNDS: usize = 200;

/// Bound on any wait in these tests. A thread stuck past it means a
/// deadlock, which should fail the test rather than hang the suite.
const WATCHDOG: Duration = Duration::from_secs(120);

/// The object id of page `page`'s content stream, read from the page
/// dictionary without resolving the stream (so without repairing).
fn content_id(pdf: &Pdf, page: usize) -> ObjectIdentifier {
    pdf.pages()[page]
        .raw()
        .get_ref(b"Contents")
        .expect("builder pages have one /Contents reference")
        .into()
}

/// What a lookup of the broken object returns, reduced to owned bytes so
/// results from different threads and documents can be compared: the raw
/// dictionary from `get::<Dict>` and the decoded stream from `get::<Stream>`.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Lookup {
    dict: Vec<u8>,
    decoded: Vec<u8>,
}

fn lookup(pdf: &Pdf, id: ObjectIdentifier) -> Option<Lookup> {
    let dict = pdf.xref().get::<Dict<'_>>(id)?;
    let stream = pdf.xref().get::<Stream<'_>>(id)?;
    Some(Lookup {
        dict: dict.data().to_vec(),
        decoded: stream.decoded().ok()?.into_owned(),
    })
}

/// Every object a document's xref resolves, by id up to `max`, as debug
/// text: two documents with equal results have equivalent xref maps as far
/// as any lookup can tell.
fn resolve_all(pdf: &Pdf, max: i32) -> Vec<(i32, String)> {
    (0..=max)
        .filter_map(|n| {
            let obj = pdf.xref().get::<Object<'_>>(ObjectIdentifier::new(n, 0))?;
            Some((n, format!("{obj:?}")))
        })
        .collect()
}

/// Joins `handles` in order, failing the test if any thread panicked or the
/// whole set does not finish within [`WATCHDOG`].
fn join_all<T>(handles: Vec<thread::JoinHandle<T>>) -> Vec<T> {
    let deadline = Instant::now() + WATCHDOG;
    while !handles.iter().all(thread::JoinHandle::is_finished) {
        assert!(
            Instant::now() < deadline,
            "threads did not finish: deadlock?"
        );
        thread::sleep(Duration::from_millis(5));
    }
    handles
        .into_iter()
        .map(|h| h.join().expect("a lookup thread panicked"))
        .collect()
}

/// The single-threaded reference: the intact file's dictionary and stream
/// for the same object, which a repaired lookup must reproduce.
fn intact_lookup(pages: usize, page: usize) -> (Lookup, ObjectIdentifier) {
    let pdf = Pdf::new(contract::misoffset_object(pages, page, 0)).expect("opens");
    let id = content_id(&pdf, page);
    let found = lookup(&pdf, id).expect("intact object resolves");
    assert!(!pdf.xref_was_repaired());
    (found, id)
}

/// §4.8b bullet 1 (post-fix half): 16 threads × 100 lookups of the
/// mis-offset object, all at once, repeated on [`ROUNDS`] fresh documents.
///
/// The rounds are there because one of the races is narrow: a thread that
/// read the stale entry just before another thread swapped the repaired map
/// in must still retry rather than conclude the repaired table is wrong
/// (H9 samples `repaired` together with the entry for that reason). No hook
/// can pause a thread inside that window, so the test gives it many chances;
/// even so it catches that mistake in only about 7 runs of 10.
/// [`a_stale_entry_read_during_the_swap_retries`] lines threads up on the
/// window and catches it every time.
#[test]
fn concurrent_lookups_of_a_misoffset_object() {
    let (expected, id) = intact_lookup(40, 6);
    assert_eq!(
        expected.decoded,
        b"BT /Helv 11 Tf 72 720 Td (Page 7) Tj ET\n"
    );
    let bytes = contract::misoffset_object(40, 6, 10);

    let mut scans_per_round = Vec::with_capacity(ROUNDS);
    for round in 0..ROUNDS {
        let pdf = Arc::new(Pdf::new(bytes.clone()).expect("opens"));
        assert_eq!(content_id(&pdf, 6), id);
        assert!(
            !pdf.xref_was_repaired(),
            "the broken entry is not read at open"
        );

        let start = Arc::new(Barrier::new(THREADS));
        let handles = (0..THREADS)
            .map(|_| {
                let pdf = pdf.clone();
                let start = start.clone();
                thread::spawn(move || {
                    start.wait();
                    let found: Vec<Option<Lookup>> =
                        (0..LOOKUPS).map(|_| lookup(&pdf, id)).collect();
                    (found, repair_hooks::scans(), repair_hooks::swaps())
                })
            })
            .collect();
        let results = join_all(handles);

        let mut scans = 0;
        let mut swaps = 0;
        for (found, s, w) in results {
            scans += s;
            swaps += w;
            for f in found {
                assert_eq!(f.as_ref(), Some(&expected), "round {round}");
            }
        }
        assert!(pdf.xref_was_repaired());
        assert_eq!(swaps, 1, "round {round}: the map is replaced exactly once");
        assert!(
            (1..=THREADS as u64).contains(&scans),
            "round {round}: {scans} scans, at most one per thread"
        );
        scans_per_round.push(scans);
    }
    eprintln!("{THREADS} x {LOOKUPS} lookups, repair scans per round: {scans_per_round:?}");
}

/// Rounds of [`a_stale_entry_read_during_the_swap_retries`].
const STALE_ROUNDS: usize = 200;

/// The narrow race of bullet 1, made likely instead of left to chance. One
/// thread repairs and is held by the after-scan hook at a barrier, just
/// before it takes the write lock to swap. Fifteen fresh threads wait at the
/// same barrier and then start their first lookup, so most of them read the
/// stale entry from the old map within microseconds of the swap. A lookup
/// whose failed read is followed by the swap must retry, not treat the now
/// repaired table as wrong and return `None`. H9 guarantees this by
/// sampling `repaired` together with the entry; with that taken out (a fresh
/// `is_repaired()` check after the failed read) this test fails in almost
/// every run, where the plain 16-thread workload catches it only sometimes.
#[test]
fn a_stale_entry_read_during_the_swap_retries() {
    let bytes = contract::misoffset_object(40, 6, 10);
    let (expected, id) = intact_lookup(40, 6);
    for round in 0..STALE_ROUNDS {
        let pdf = Arc::new(Pdf::new(bytes.clone()).expect("opens"));
        let at_swap = Arc::new(Barrier::new(THREADS));
        let repairer = {
            let (pdf, at_swap) = (pdf.clone(), at_swap.clone());
            thread::spawn(move || {
                repair_hooks::set_after_scan(Some(Box::new(move || {
                    at_swap.wait();
                })));
                let found = lookup(&pdf, id);
                repair_hooks::set_after_scan(None);
                (found, repair_hooks::swaps())
            })
        };
        let lookers: Vec<_> = (1..THREADS)
            .map(|_| {
                let (pdf, at_swap) = (pdf.clone(), at_swap.clone());
                thread::spawn(move || {
                    at_swap.wait();
                    (lookup(&pdf, id), repair_hooks::swaps())
                })
            })
            .collect();
        let mut results = join_all(vec![repairer]);
        results.extend(join_all(lookers));
        let swaps: u64 = results.iter().map(|r| r.1).sum();
        assert_eq!(swaps, 1, "round {round}: the map is replaced exactly once");
        for (found, _) in &results {
            assert_eq!(found.as_ref(), Some(&expected), "round {round}");
        }
    }
}

/// §4.8b bullet 2: two threads that both observe the table unrepaired and
/// both run the repair scan. The after-scan hook holds each thread at a
/// barrier until the other has scanned too, so neither can swap before both
/// have committed to repairing; that is the window in which hayro without H9
/// failed its `assert!(!repaired)` or its `try_write().unwrap()`.
#[test]
fn two_threads_racing_into_repair_replace_the_map_once() {
    let bytes = contract::misoffset_object(10, 3, 10);

    // Single-threaded reference repair.
    let reference = Pdf::new(bytes.clone()).expect("opens");
    let id = content_id(&reference, 3);
    let expected = lookup(&reference, id).expect("repaired lookup");
    assert!(reference.xref_was_repaired());
    let max = reference.len() as i32 + 8;
    let reference_objects = resolve_all(&reference, max);

    let pdf = Arc::new(Pdf::new(bytes).expect("opens"));
    assert!(!pdf.xref_was_repaired());
    let both_scanned = Arc::new(Barrier::new(2));
    let handles = (0..2)
        .map(|_| {
            let pdf = pdf.clone();
            let both_scanned = both_scanned.clone();
            thread::spawn(move || {
                repair_hooks::set_after_scan(Some(Box::new(move || {
                    both_scanned.wait();
                })));
                let found = lookup(&pdf, id);
                repair_hooks::set_after_scan(None);
                (found, repair_hooks::scans(), repair_hooks::swaps())
            })
        })
        .collect();
    let results = join_all(handles);

    let scans: u64 = results.iter().map(|r| r.1).sum();
    let swaps: u64 = results.iter().map(|r| r.2).sum();
    assert_eq!(scans, 2, "both threads ran the scan");
    assert_eq!(swaps, 1, "only the first to take the lock replaced the map");
    for (found, ..) in &results {
        assert_eq!(found.as_ref(), Some(&expected));
    }
    assert!(pdf.xref_was_repaired());
    assert_eq!(pdf.len(), reference.len());
    assert_eq!(resolve_all(&pdf, max), reference_objects);
}

/// A repair that finds the map already repaired is a no-op, and a lookup
/// after the repair never scans again, on any thread.
#[test]
fn repair_is_idempotent_across_threads() {
    let pdf = Arc::new(Pdf::new(contract::misoffset_object(5, 2, 10)).expect("opens"));
    let id = content_id(&pdf, 2);
    assert!(lookup(&pdf, id).is_some());
    assert_eq!((repair_hooks::scans(), repair_hooks::swaps()), (1, 1));
    let other = {
        let pdf = pdf.clone();
        thread::spawn(move || {
            let found = lookup(&pdf, id);
            (found.is_some(), repair_hooks::scans())
        })
    };
    assert_eq!(join_all(vec![other]), vec![(true, 0)]);
    assert!(lookup(&pdf, id).is_some());
    assert_eq!((repair_hooks::scans(), repair_hooks::swaps()), (1, 1));
}

/// A lookup of a missing object in a repaired table still returns `None`
/// (one repair, no loop).
#[test]
fn a_missing_object_after_repair_is_none() {
    let pdf = Pdf::new(contract::misoffset_object(3, 1, 10)).expect("opens");
    let id = content_id(&pdf, 1);
    assert!(lookup(&pdf, id).is_some());
    let missing = ObjectIdentifier::new(9_999, 0);
    assert!(pdf.xref().get::<Object<'_>>(missing).is_none());
    assert_eq!(repair_hooks::scans(), 1);
}

/// Bytes appended after `%%EOF` to make the repair scan slow: ~50 MB of
/// content-like tokens the scanner has to step through. hayro finds the last
/// `startxref`, which this padding does not contain, so the file still opens
/// from its own table and only the lazy repair reads the padding.
fn pad_to_50_mb(mut bytes: Vec<u8>) -> Vec<u8> {
    const TARGET: usize = 50 * 1024 * 1024;
    let line = b"0 0 m 100 100 l S\n";
    bytes.reserve(TARGET);
    while bytes.len() < TARGET {
        bytes.extend_from_slice(line);
    }
    bytes
}

/// §4.8b bullet 3 (report-only timing): while one thread repairs a 50 MB
/// file, another thread looping on lookups of an intact object is not held
/// up for the scan. The gate is correctness and no panic; the numbers are
/// printed (run with `--nocapture`) for the review, not asserted, because
/// shared CI runners are too noisy for a timing gate.
#[test]
fn lookups_proceed_during_a_repair_scan() {
    let pdf =
        Arc::new(Pdf::new(pad_to_50_mb(contract::misoffset_object(10, 6, 10))).expect("opens"));
    assert!(!pdf.xref_was_repaired());
    let broken = content_id(&pdf, 6);
    let intact = content_id(&pdf, 0);
    let intact_expected = lookup(&pdf, intact).expect("intact object resolves");
    assert!(!pdf.xref_was_repaired(), "intact lookups do not repair");

    let ready = Arc::new(Barrier::new(2));
    let done = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let looper = {
        let (pdf, ready, done) = (pdf.clone(), ready.clone(), done.clone());
        thread::spawn(move || {
            let mut max = Duration::ZERO;
            let mut n = 0_u64;
            ready.wait();
            while !done.load(std::sync::atomic::Ordering::Acquire) {
                let t = Instant::now();
                let found = lookup(&pdf, intact);
                max = max.max(t.elapsed());
                assert_eq!(found.as_ref(), Some(&intact_expected));
                n += 1;
            }
            (max, n)
        })
    };
    let repairer = {
        let (pdf, ready, done) = (pdf.clone(), ready, done);
        thread::spawn(move || {
            ready.wait();
            let t = Instant::now();
            let found = lookup(&pdf, broken);
            let took = t.elapsed();
            done.store(true, std::sync::atomic::Ordering::Release);
            (found.is_some(), took)
        })
    };
    let (max_get, gets) = join_all(vec![looper]).remove(0);
    let (repaired_ok, repair_took) = join_all(vec![repairer]).remove(0);
    assert!(repaired_ok, "the repaired lookup finds the object");
    assert!(pdf.xref_was_repaired());
    assert!(gets > 0, "the looping thread ran during the repair");
    eprintln!(
        "lock-hold report (not gated): repair lookup on {} MB took {:?}; \
         {gets} concurrent lookups of an intact object, slowest {:?} \
         (spec target: < 50 ms while the scan takes > 200 ms)",
        pdf.data().as_ref().len() / (1024 * 1024),
        repair_took,
        max_get
    );
}

/// How long the repairing thread waits, parked in the after-scan hook, for
/// the concurrent lookup to finish. Generous because a healthy run returns in
/// microseconds; it only bounds the failure case, where the lookup is blocked
/// on a lock the repair holds and the test must fail rather than hang.
const HOOK_WAIT: Duration = Duration::from_secs(10);

/// T38 / §4.8b, the deterministic form of the lock-hold bullet: while a
/// repairing thread is parked after its scan and before its swap, a lookup of
/// an intact object on another thread completes.
///
/// The after-scan hook runs inside `repair()` after the scan and before the
/// map is replaced. If `repair()` took the map's write lock before scanning
/// (the pre-H9 design), the lock would still be held at that point and the
/// concurrent `get()` would block until the hook gives up, so the assertion
/// fails; the timing test above cannot catch that because it only checks that
/// some lookups ran. The wait is bounded so the failure case is an
/// assertion, not a deadlock. A small file suffices: no timing is involved.
#[test]
fn a_lookup_completes_while_repair_is_parked_after_its_scan() {
    let pdf = Arc::new(Pdf::new(contract::misoffset_object(4, 2, 10)).expect("opens"));
    let broken = content_id(&pdf, 2);
    let intact = content_id(&pdf, 0);
    let intact_expected = lookup(&pdf, intact).expect("intact object resolves");
    assert!(!pdf.xref_was_repaired(), "intact lookups do not repair");

    let (go_tx, go_rx) = mpsc::channel::<()>();
    let (done_tx, done_rx) = mpsc::channel::<Option<Lookup>>();
    let reader = {
        let pdf = pdf.clone();
        thread::spawn(move || {
            // Start only once the repairer is parked in the hook, so the
            // lookup is certain to overlap the repair.
            let parked = go_rx.recv_timeout(HOOK_WAIT).is_ok();
            let found = parked.then(|| lookup(&pdf, intact));
            // The repairer may have given up and gone; nothing to report to.
            let _ = done_tx.send(found.flatten());
        })
    };
    let repairer = {
        let pdf = pdf.clone();
        thread::spawn(move || {
            // The hook records what it saw instead of asserting: a panic here
            // would unwind through `repair()` with a lock possibly held.
            let seen = Rc::new(RefCell::new(None));
            let hook_seen = seen.clone();
            repair_hooks::set_after_scan(Some(Box::new(move || {
                let _ = go_tx.send(());
                *hook_seen.borrow_mut() = Some(done_rx.recv_timeout(HOOK_WAIT));
            })));
            let found = lookup(&pdf, broken);
            repair_hooks::set_after_scan(None);
            let seen = seen.borrow_mut().take();
            (found.is_some(), seen, repair_hooks::swaps())
        })
    };
    let (repaired_ok, seen, swaps) = join_all(vec![repairer]).remove(0);
    join_all(vec![reader]);

    assert!(repaired_ok, "the repaired lookup finds the object");
    assert_eq!(swaps, 1, "the parked repair then swapped the map in");
    assert_eq!(
        seen,
        Some(Ok(Some(intact_expected))),
        "a lookup of an intact object must complete while the repair is \
         between its scan and its swap (the write lock must not be held \
         across the scan)"
    );
}

/// The eight encrypted fixtures (spec Appendix A.6), and whether a user
/// password is needed to open each.
const ENCRYPTED: [(&str, bool); 8] = [
    ("enc_rc4_40_owner.pdf", false),
    ("enc_rc4_40_user.pdf", true),
    ("enc_rc4_128_owner.pdf", false),
    ("enc_rc4_128_user.pdf", true),
    ("enc_aes_128_owner.pdf", false),
    ("enc_aes_128_user.pdf", true),
    ("enc_aes_256_owner.pdf", false),
    ("enc_aes_256_user.pdf", true),
];

/// The user password `tools/oracle/mkfixtures.py` sets on the `*_user`
/// fixtures.
const USER_PASSWORD: &str = "user-secret";

/// §4.8b bullet 4: `is_encrypted()` after open. Owner-only files open with
/// the empty password; user-password files cannot be opened without their
/// password (so there is no `Pdf` to ask) and are checked opened with it.
#[test]
fn is_encrypted_for_the_encrypted_fixtures() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures/corpus-c");
    for (name, needs_user) in ENCRYPTED {
        let bytes = std::fs::read(format!("{dir}/{name}")).expect("fixture");
        let pdf = if needs_user {
            assert!(
                Pdf::new(bytes.clone()).is_err(),
                "{name} needs its user password"
            );
            Pdf::new_with_password(bytes, USER_PASSWORD)
        } else {
            Pdf::new(bytes)
        }
        .unwrap_or_else(|e| panic!("{name}: {e:?}"));
        assert!(pdf.is_encrypted(), "{name}");
    }
}

/// §4.8b bullet 4, the other half: every unencrypted input the test kit
/// generates opens with `is_encrypted() == false`, including after a lazy
/// repair (which rebuilds the map but not the decryptor).
#[test]
fn is_encrypted_false_for_unencrypted_inputs() {
    let mut b = pdf_extract_testkit::PdfBuilder::new();
    b.add_page_a4().raw_content("% /Encrypt in a comment");
    let inputs: Vec<(&str, Vec<u8>)> = vec![
        ("builder", b.to_bytes()),
        ("header_at_offset(500)", contract::header_at_offset(500)),
        ("broken_startxref", contract::broken_startxref()),
        ("truncated_tail(20)", contract::truncated_tail(20)),
        ("misoffset_object", contract::misoffset_object(3, 1, 10)),
        ("page_tree(Flat(5))", hostile::page_tree(TreeShape::Flat(5))),
        ("page_tree(Cycle)", hostile::page_tree(TreeShape::Cycle)),
        ("form_self_recursion", hostile::form_self_recursion()),
    ];
    for (name, bytes) in inputs {
        let pdf = Pdf::new(bytes).unwrap_or_else(|e| panic!("{name}: {e:?}"));
        assert!(!pdf.is_encrypted(), "{name}");
        // Force any pending lazy repair, then ask again.
        let _ = pdf.pages();
        for id in 0..=pdf.len() as i32 + 2 {
            let _ = pdf.xref().get::<Object<'_>>(ObjectIdentifier::new(id, 0));
        }
        assert!(!pdf.is_encrypted(), "{name} after lookups");
    }
}
