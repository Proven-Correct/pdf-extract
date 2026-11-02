//! The open contract of spec §4.8 (internal survey §2.2, §13.5): one test per row of the
//! contract table, through the public API only. Unit 11 repeats these rows in
//! Python.
//!
//! Rows that need crate internals live in the crate's own unit tests:
//! the counting-xref bound of the `max_pages` row (`index_tests.rs`), and the
//! close protocol and "text extracted" half of the encryption row
//! (`document_tests.rs`).

mod common;

use std::path::PathBuf;

use common::on_small_stack;
use pdf_extract_core::{
    Document, LimitKind, Limits, OpenError, OpenOptions, PageError, WarningKind,
};
use pdf_extract_testkit::hostile::{self, TreeShape};
use pdf_extract_testkit::{PdfBuilder, contract};

fn open(bytes: Vec<u8>) -> Result<Document, OpenError> {
    Document::from_bytes(bytes, &OpenOptions::default())
}

fn kinds(doc: &Document) -> Vec<WarningKind> {
    doc.warnings().iter().map(|w| w.kind).collect()
}

fn has(doc: &Document, kind: WarningKind) -> bool {
    kinds(doc).contains(&kind)
}

/// A per-test scratch path under cargo's integration-test temp dir.
fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("contract");
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir.join(name)
}

fn valid_pdf() -> Vec<u8> {
    let mut b = PdfBuilder::new();
    b.add_page_a4()
        .raw_content("BT /Helv 11 Tf 72 720 Td (One) Tj ET");
    b.add_page_a4()
        .raw_content("BT /Helv 11 Tf 72 720 Td (Two) Tj ET");
    b.to_bytes()
}

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/corpus-c")
        .join(name)
}

#[test]
fn missing_path_is_not_found() {
    let path = scratch("does-not-exist.pdf");
    let _ = std::fs::remove_file(&path);
    match Document::open(&path, &OpenOptions::default()) {
        Err(OpenError::NotFound(p)) => assert_eq!(p, path),
        other => panic!("expected NotFound, got {other:?}"),
    }
}

#[test]
fn zero_byte_file_is_empty() {
    let path = scratch("empty.pdf");
    std::fs::write(&path, b"").expect("write");
    assert!(matches!(
        Document::open(&path, &OpenOptions::default()),
        Err(OpenError::Empty)
    ));
    assert!(matches!(open(Vec::new()), Err(OpenError::Empty)));
}

#[test]
fn header_then_garbage_is_corrupt() {
    assert!(matches!(
        open(contract::garbage_after_header()),
        Err(OpenError::Corrupt(_))
    ));
}

#[test]
fn plain_text_named_pdf_is_not_pdf() {
    let path = scratch("plain_text.pdf");
    std::fs::write(&path, contract::plain_text()).expect("write");
    assert!(matches!(
        Document::open(&path, &OpenOptions::default()),
        Err(OpenError::NotPdf)
    ));
}

#[test]
fn valid_pdf_with_any_name_opens() {
    for name in ["valid.txt", "valid_noext", "valid.pdf"] {
        let path = scratch(name);
        std::fs::write(&path, valid_pdf()).expect("write");
        let doc = Document::open(&path, &OpenOptions::default())
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(doc.page_count(), 2, "{name}");
        assert!(!doc.is_encrypted(), "{name}");
        assert!(doc.warnings().is_empty(), "{name}: {:?}", doc.warnings());
    }
}

#[test]
fn header_offset_500_opens_and_1500_is_not_pdf() {
    let doc = open(contract::header_at_offset(500)).expect("offset 500 opens");
    assert_eq!(doc.page_count(), 1);
    // Q6: the oracle opens this one; F2 keeps the window at 1024 bytes.
    assert!(matches!(
        open(contract::header_at_offset(1500)),
        Err(OpenError::NotPdf)
    ));
}

#[test]
fn encrypted_fixtures_owner_open_user_need_a_password() {
    for alg in ["rc4_40", "rc4_128", "aes_128", "aes_256"] {
        let owner = fixture(&format!("enc_{alg}_owner.pdf"));
        let doc = Document::open(&owner, &OpenOptions::default())
            .unwrap_or_else(|e| panic!("{alg} owner: {e}"));
        assert!(doc.is_encrypted(), "{alg} owner");
        assert_eq!(doc.page_count(), 1, "{alg} owner");

        let user = fixture(&format!("enc_{alg}_user.pdf"));
        assert!(
            matches!(
                Document::open(&user, &OpenOptions::default()),
                Err(OpenError::PasswordRequired)
            ),
            "{alg} user"
        );
    }
}

#[test]
fn broken_startxref_and_truncated_tail_open_repaired() {
    let doc = open(contract::broken_startxref()).expect("broken startxref opens");
    assert_eq!(doc.page_count(), 1);
    assert!(has(&doc, WarningKind::XrefRepaired), "{:?}", doc.warnings());

    // Cuts into the startxref offset, the trailer, the xref table and the
    // last object. (Cutting only `%%EOF\n` is not damage hayro notices: the
    // offset before it is still intact, so nothing is repaired.)
    for cut in [8, 20, 60, 120, 200] {
        let doc = open(contract::truncated_tail(cut)).unwrap_or_else(|e| panic!("cut {cut}: {e}"));
        assert_eq!(doc.page_count(), 1, "cut {cut}");
        assert!(
            has(&doc, WarningKind::XrefRepaired),
            "cut {cut}: {:?}",
            doc.warnings()
        );
    }
}

/// Points the classic-xref entry of object `obj` at byte `offset`, in place
/// (every entry is 20 bytes; the offset is its first ten digits).
fn set_xref_offset(mut bytes: Vec<u8>, obj: usize, offset: usize) -> Vec<u8> {
    let tag = b"\nxref\n";
    let section = bytes
        .windows(tag.len())
        .rposition(|w| w == tag)
        .expect("xref table")
        + tag.len();
    let first = section
        + bytes[section..]
            .iter()
            .position(|&b| b == b'\n')
            .expect("subsection header")
        + 1;
    let at = first + 20 * obj;
    bytes[at..at + 10].copy_from_slice(format!("{offset:010}").as_bytes());
    bytes
}

/// Revision 4: the table is intact except for a page-tree node's offset, so
/// nothing is repaired at open; the walk's lookup of that node triggers the
/// repair, and the warning must still be attached (sampled after the walk).
#[test]
fn wrong_offset_of_a_page_tree_node_is_repaired_during_the_walk() {
    // page_tree(Flat(2)): 1 font, 2 catalog, 3 root, then content/leaf pairs:
    // leaf 1 is object 5. Point it at the catalog's offset.
    let intact = hostile::page_tree(TreeShape::Flat(2));
    let catalog_at = intact
        .windows(b"\n2 0 obj".len())
        .position(|w| w == b"\n2 0 obj")
        .expect("catalog")
        + 1;
    let bytes = set_xref_offset(intact, 5, catalog_at);
    let doc = open(bytes).expect("opens");
    assert_eq!(doc.page_count(), 2);
    assert!(has(&doc, WarningKind::XrefRepaired), "{:?}", doc.warnings());
}

#[test]
fn catalog_without_pages_is_corrupt() {
    // The oracle opens it with 0 pages (documented divergence, T5).
    assert!(matches!(
        open(contract::catalog_without_pages()),
        Err(OpenError::Corrupt(_))
    ));
}

#[test]
fn empty_kids_has_no_pages() {
    let doc = open(PdfBuilder::new().to_bytes()).expect("opens");
    assert_eq!(doc.page_count(), 0);
    assert_eq!(doc.pages().len(), 0);
    let doc = open(hostile::deep_page_tree(0)).expect("opens");
    assert_eq!(doc.page_count(), 0);
}

#[test]
fn lying_count_is_ignored() {
    let doc = open(hostile::page_tree(TreeShape::LyingCount { claimed: 5 })).expect("opens");
    assert_eq!(doc.page_count(), hostile::LYING_COUNT_LEAVES);
    assert_eq!(doc.page_count(), 100);
}

#[test]
fn duplicate_leaf_and_cycle_yield_one_page_with_a_warning() {
    for shape in [TreeShape::DuplicateLeaf, TreeShape::Cycle] {
        let doc = open(hostile::page_tree(shape)).expect("opens");
        assert_eq!(doc.page_count(), 1, "{shape:?}");
        assert!(
            has(&doc, WarningKind::PageTreeRevisited),
            "{shape:?}: {:?}",
            doc.warnings()
        );
        let text = doc.warnings()[0].to_string();
        assert!(text.starts_with("page tree: node revisited"), "{text}");
    }
}

#[test]
fn typeless_leaf_and_direct_dict_kid_are_pages() {
    for shape in [TreeShape::TypelessLeaf, TreeShape::DirectDictKid] {
        let doc = open(hostile::page_tree(shape)).expect("opens");
        assert_eq!(doc.page_count(), 2, "{shape:?}");
        assert!(doc.warnings().is_empty(), "{shape:?}: {:?}", doc.warnings());
    }
}

#[test]
fn deep_page_tree_200_has_its_page() {
    let doc = on_small_stack(|| open(hostile::deep_page_tree(200)).expect("opens"));
    assert_eq!(doc.page_count(), 1);
    assert!(doc.warnings().is_empty(), "{:?}", doc.warnings());
}

#[test]
fn deep_page_tree_3000_is_cut_at_the_depth_limit_on_a_small_stack() {
    let doc = on_small_stack(|| open(hostile::deep_page_tree(3000)).expect("opens"));
    // KP23: the oracle finds the page; we refuse to go past depth 256, and
    // the brute-force fallback does not undo that.
    assert_eq!(doc.page_count(), 0);
    assert!(
        has(&doc, WarningKind::PageTreeDepth),
        "{:?}",
        doc.warnings()
    );
    assert!(!has(&doc, WarningKind::PageTreeBruteForce));
}

#[test]
fn depth_limit_is_configurable_and_inclusive() {
    let opts = |depth| OpenOptions {
        max_pages: None,
        limits: Limits {
            max_page_tree_depth: depth,
            ..Limits::default()
        },
    };
    // The leaf of deep_page_tree(d) is at depth d.
    let at = |d: u32, limit: u32| {
        Document::from_bytes(hostile::deep_page_tree(d as usize), &opts(limit))
            .expect("opens")
            .page_count()
    };
    assert_eq!(at(10, 10), 1);
    assert_eq!(at(11, 10), 0);
}

#[test]
fn max_pages_stops_the_walk() {
    let bytes = hostile::page_tree(TreeShape::Fanout {
        leaves_per_node: 100,
        levels: 2,
    });
    let opts = OpenOptions {
        max_pages: Some(10),
        ..OpenOptions::default()
    };
    match Document::from_bytes(bytes.clone(), &opts) {
        Err(OpenError::TooManyPages { limit, at_least }) => {
            assert_eq!((limit, at_least), (10, 11));
        }
        other => panic!("expected TooManyPages, got {other:?}"),
    }
    // At the limit exactly, it opens.
    let flat = hostile::page_tree(TreeShape::Flat(10));
    assert_eq!(
        Document::from_bytes(flat, &opts)
            .expect("10 pages fit")
            .page_count(),
        10
    );
    assert_eq!(open(bytes).expect("no limit").page_count(), 10_000);
}

#[test]
fn max_objects_limit() {
    let opts = OpenOptions {
        max_pages: None,
        limits: Limits {
            max_objects: 3,
            ..Limits::default()
        },
    };
    for bytes in [valid_pdf(), hostile::page_tree(TreeShape::Flat(3))] {
        match Document::from_bytes(bytes, &opts) {
            Err(OpenError::Limit { kind, value }) => {
                assert_eq!(kind, LimitKind::Objects);
                assert!(value > 3);
            }
            other => panic!("expected Limit, got {other:?}"),
        }
    }
}

#[test]
fn max_file_size_is_checked_before_reading() {
    let opts = OpenOptions {
        max_pages: None,
        limits: Limits {
            max_file_size: 100,
            ..Limits::default()
        },
    };
    let bytes = valid_pdf();
    let size = bytes.len() as u64;
    let path = scratch("too_large.pdf");
    std::fs::write(&path, &bytes).expect("write");
    for result in [
        Document::open(&path, &opts),
        Document::from_bytes(bytes, &opts),
    ] {
        match result {
            Err(OpenError::TooLarge { size: s, limit }) => assert_eq!((s, limit), (size, 100)),
            other => panic!("expected TooLarge, got {other:?}"),
        }
    }
}

#[test]
fn deep_nesting_in_the_page_dictionary_opens_without_pages() {
    let doc = on_small_stack(|| open(hostile::deep_nesting_in_page_dict(200_000)).expect("opens"));
    assert_eq!(doc.page_count(), 0);
    assert!(
        has(&doc, WarningKind::PageTreeNodeUnreadable),
        "{:?}",
        doc.warnings()
    );
    assert!(has(&doc, WarningKind::ObjectDepth), "{:?}", doc.warnings());
    // Spec §4.8 (revision 10): reading the leaf fails at its offset, so
    // hayro runs its lazy repair, and the document reports it.
    assert!(has(&doc, WarningKind::XrefRepaired), "{:?}", doc.warnings());
}

#[test]
fn dangling_pages_reference_opens_without_pages() {
    // Object 9 does not exist and nothing in the file is a /Type /Page.
    let bytes = common::assemble_pdf(&["<< /Type /Catalog /Pages 9 0 R >>", "<< /Producer (x) >>"]);
    let doc = open(bytes).expect("opens");
    assert_eq!(doc.page_count(), 0);
    assert!(has(&doc, WarningKind::PageTreeNodeUnreadable));
    assert!(has(&doc, WarningKind::PageTreeBruteForce));
}

#[test]
fn page_out_of_range() {
    let doc = open(valid_pdf()).expect("opens");
    assert_eq!(doc.page(1).expect("in range").index(), 1);
    assert_eq!(
        doc.page(2).expect_err("out of range"),
        PageError::IndexOutOfRange { index: 2, count: 2 }
    );
    assert!(doc.page(usize::MAX).is_err());
}

/// T7 after a lazy repair: the table lists a few objects, the body holds
/// many more, and the walk's lookup of a mis-offset leaf rebuilds the xref
/// from a scan. The limit is re-checked after the walk.
#[test]
fn max_objects_is_rechecked_after_a_repair_during_the_walk() {
    let mut bodies = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
        "<< /Type /Page /Parent 2 0 R >>".to_owned(),
    ];
    for i in 0..200 {
        bodies.push(format!("<< /Filler {i} >>"));
    }
    let full = common::assemble_pdf(&bodies);
    // Keep only the first four xref entries (free head + objects 1-3) and
    // point object 3 at object 1's offset, so the walk triggers a repair.
    let xref_at = full
        .windows(6)
        .rposition(|w| w == b"\nxref\n")
        .expect("xref")
        + 1;
    let entry = |bytes: &[u8], n: usize| {
        let first = xref_at
            + bytes[xref_at..]
                .iter()
                .position(|&b| b == b'\n')
                .expect("line")
            + 1;
        let first = first
            + bytes[first..]
                .iter()
                .position(|&b| b == b'\n')
                .expect("line")
            + 1;
        String::from_utf8(bytes[first + 20 * n..first + 20 * n + 20].to_vec()).expect("ascii")
    };
    let mut table = String::from("xref\n0 4\n");
    for n in 0..3 {
        table.push_str(&entry(&full, n));
    }
    table.push_str(&entry(&full, 1)); // object 3 -> object 1's offset
    let mut bytes = full[..xref_at].to_vec();
    bytes.extend_from_slice(table.as_bytes());
    bytes.extend_from_slice(
        format!("trailer\n<< /Size 4 /Root 1 0 R >>\nstartxref\n{xref_at}\n%%EOF\n").as_bytes(),
    );

    let doc = open(bytes.clone()).expect("opens without a limit");
    assert!(has(&doc, WarningKind::XrefRepaired), "{:?}", doc.warnings());
    let opts = OpenOptions {
        max_pages: None,
        limits: Limits {
            max_objects: 10,
            ..Limits::default()
        },
    };
    match Document::from_bytes(bytes, &opts) {
        Err(OpenError::Limit { kind, value }) => {
            assert_eq!(kind, LimitKind::Objects);
            assert!(value > 200, "{value}");
        }
        other => panic!("expected Limit, got {other:?}"),
    }
}

/// An encrypted file whose trailer spells `/Encrypt` with a `#xx` escape is
/// still reported as encrypted.
#[test]
fn escaped_encrypt_key_is_still_encrypted() {
    let mut bytes = std::fs::read(fixture("enc_rc4_128_owner.pdf")).expect("fixture");
    while let Some(at) = bytes.windows(8).position(|w| w == b"/Encrypt") {
        bytes.splice(at..at + 8, b"/Encr#79pt".iter().copied());
    }
    let doc = open(bytes).expect("opens");
    assert!(doc.is_encrypted());
}

// --- Rows added by spec revision 5 -----------------------------------------

/// Asserts `result` is the non-regular-file refusal.
fn assert_not_regular(result: Result<Document, OpenError>, what: &str) {
    match result {
        Err(OpenError::Io(e)) => {
            assert_eq!(e.kind(), std::io::ErrorKind::InvalidInput, "{what}");
            assert_eq!(e.to_string(), "not a regular file", "{what}");
        }
        other => panic!("{what}: expected Io(InvalidInput), got {other:?}"),
    }
}

#[test]
fn directory_path_is_not_a_regular_file() {
    let dir = scratch("a_directory.pdf");
    std::fs::create_dir_all(&dir).expect("mkdir");
    assert_not_regular(Document::open(&dir, &OpenOptions::default()), "dir");
}

#[cfg(unix)]
#[test]
fn device_and_socket_paths_are_not_regular_files() {
    assert_not_regular(
        Document::open("/dev/null", &OpenOptions::default()),
        "/dev/null",
    );
    // Socket paths are limited to ~104 bytes, too short for the target dir.
    let sock = std::env::temp_dir().join(format!("pe-{}.sock", std::process::id()));
    let _ = std::fs::remove_file(&sock);
    let listener = std::os::unix::net::UnixListener::bind(&sock).expect("bind socket");
    let result = Document::open(&sock, &OpenOptions::default());
    drop(listener);
    let _ = std::fs::remove_file(&sock);
    assert_not_regular(result, "socket");
}

/// A FIFO must be refused without opening it (opening a FIFO for reading
/// blocks until a writer appears). The open runs on a helper thread with a
/// watchdog: if it blocks, the test opens the write end to release it and
/// fails instead of hanging.
#[cfg(unix)]
#[test]
fn fifo_path_is_refused_without_blocking() {
    use std::sync::mpsc;
    use std::time::Duration;

    let fifo = scratch("a_fifo.pdf");
    let _ = std::fs::remove_file(&fifo);
    let status = std::process::Command::new("mkfifo")
        .arg(&fifo)
        .status()
        .expect("run mkfifo");
    assert!(status.success(), "mkfifo failed");

    let (tx, rx) = mpsc::channel();
    let path = fifo.clone();
    let worker = std::thread::spawn(move || {
        let _ = tx.send(Document::open(&path, &OpenOptions::default()));
    });
    match rx.recv_timeout(Duration::from_secs(10)) {
        Ok(result) => {
            worker.join().expect("no panic");
            assert_not_regular(result, "fifo");
        }
        Err(_) => {
            // Unblock the reader so the process can exit, then fail.
            let _ = std::fs::OpenOptions::new().write(true).open(&fifo);
            panic!("Document::open blocked on a FIFO");
        }
    }
}

#[cfg(unix)]
#[test]
fn symlink_to_a_regular_file_opens() {
    let target = scratch("link_target.pdf");
    std::fs::write(&target, valid_pdf()).expect("write");
    let link = scratch("link.pdf");
    let _ = std::fs::remove_file(&link);
    std::os::unix::fs::symlink(&target, &link).expect("symlink");
    let doc = Document::open(&link, &OpenOptions::default()).expect("link opens");
    assert_eq!(doc.page_count(), 2);
}

/// T3 (revision 5): the header must lie wholly within the first 1024 bytes;
/// one starting at 1020 straddles the boundary.
#[test]
fn header_straddling_byte_1024_is_not_pdf() {
    assert!(matches!(
        open(contract::header_at_offset(1020)),
        Err(OpenError::NotPdf)
    ));
    assert_eq!(
        open(contract::header_at_offset(1019))
            .expect("header ending at byte 1024 opens")
            .page_count(),
        1
    );
}

/// T13 (revision 5): a direct dictionary at `/Kids` position 4097 is not
/// indexed; positions up to 4096 are.
#[test]
fn direct_kid_beyond_position_4096_is_not_indexed() {
    let kids = "<< /MediaBox [0 0 10 10] >> ".repeat(4098); // positions 0..=4097
    let bytes = common::assemble_pdf(&[
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        format!("<< /Type /Pages /Kids [{kids}] >>"),
    ]);
    let doc = open(bytes).expect("opens");
    assert_eq!(doc.page_count(), 4097);
    let w = doc.warnings();
    assert_eq!(w.len(), 1, "{w:?}");
    assert!(
        w[0].to_string()
            .starts_with("page tree: direct kid beyond position limit"),
        "{}",
        w[0]
    );
}

/// Revision 5: 1,000 references to one leaf give one coalesced warning.
#[test]
fn a_thousand_duplicate_leaves_coalesce_into_one_warning() {
    let kids = "3 0 R ".repeat(1000);
    let bytes = common::assemble_pdf(&[
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        format!("<< /Type /Pages /Kids [{kids}] /Count 1000 >>"),
        "<< /Type /Page /Parent 2 0 R >>".to_owned(),
    ]);
    let doc = open(bytes).expect("opens");
    assert_eq!(doc.page_count(), 1);
    let w = doc.warnings();
    assert_eq!(w.len(), 1, "{w:?}");
    assert_eq!(w[0].kind, WarningKind::PageTreeRevisited);
    assert_eq!(w[0].count, 999);
    assert!(w[0].to_string().ends_with(" (×999)"), "{}", w[0]);
}

/// T12 applied to a kid that points back at the catalog: the catalog has no
/// `/Kids`, so it is a leaf like any other typeless dictionary and counts as
/// a page (whose rebuild later has no content). Nothing loops and nothing
/// warns; the oracle's handling of such a file is not pinned.
#[test]
fn kid_pointing_at_the_catalog_is_a_leaf() {
    let bytes = common::assemble_pdf(&[
        "<< /Type /Catalog /Pages 2 0 R >>",
        "<< /Type /Pages /Kids [3 0 R 1 0 R] /Count 2 >>",
        "<< /Type /Page /Parent 2 0 R >>",
    ]);
    let doc = open(bytes).expect("opens");
    assert_eq!(doc.page_count(), 2);
    assert!(doc.warnings().is_empty(), "{:?}", doc.warnings());
}

/// The mis-offset generator's file opens and indexes without a repair: the
/// broken entry is a content stream, which open never reads.
#[test]
fn misoffset_content_stream_does_not_repair_at_open() {
    let doc = open(contract::misoffset_object(40, 6, 10)).expect("opens");
    assert_eq!(doc.page_count(), 40);
    assert!(doc.warnings().is_empty(), "{:?}", doc.warnings());
}
