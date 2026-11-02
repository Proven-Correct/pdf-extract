//! Engine construction against the real Tesseract (spec 0002 O7, O8, O9,
//! O13; §4.8 "Resolution", "Construction", "Info").

mod common;

use std::io::Read as _;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use pdf_extract_core::UnavailableReason;
use pdf_extract_ocr::{Candidates, EngineMode, Isolation, Linkage, OcrConfig, OcrEngine};
use sha2::{Digest, Sha256};

fn build(cfg: OcrConfig, c: &Candidates) -> Result<OcrEngine, pdf_extract_core::OcrUnavailable> {
    OcrEngine::tesseract_with_candidates(cfg, c)
}

#[test]
fn a_good_configuration_builds_quickly_with_one_pooled_instance() {
    let start = Instant::now();
    let engine = build(common::config(), &common::nowhere()).expect("engine");
    let took = start.elapsed();
    assert!(took < Duration::from_secs(2), "construction took {took:?}");
    assert_eq!(engine.pool_len(), Some(1));
    assert_eq!(engine.tessdata(), Some(common::tessdata().as_path()));
    assert_eq!(engine.config().languages, vec!["eng".to_owned()]);
    assert!(format!("{engine:?}").contains("OcrEngine"));
}

#[test]
fn info_reports_version_linkage_and_model_digests() {
    let engine = build(common::config(), &common::nowhere()).expect("engine");
    let info = engine.info();
    assert_eq!(info.name, "tesseract");
    assert_eq!(info.languages, vec!["eng".to_owned()]);
    assert_eq!(info.tessdata, Some(common::tessdata()));
    assert_eq!(info.isolation, Isolation::InProcess);
    if cfg!(feature = "vendored") {
        assert_eq!(info.linkage, Linkage::Vendored);
        assert!(info.version.starts_with("5.5.3"), "{}", info.version);
    } else {
        assert_eq!(info.linkage, Linkage::System);
    }
    let mut bytes = Vec::new();
    std::fs::File::open(common::tessdata().join("eng.traineddata"))
        .and_then(|mut f| f.read_to_end(&mut bytes))
        .expect("read model");
    let expected: [u8; 32] = Sha256::digest(&bytes).into();
    assert_eq!(info.model_sha256, vec![("eng".to_owned(), expected)]);
}

#[test]
fn no_tessdata_anywhere_lists_every_candidate() {
    let missing = common::scratch("no-tessdata").join("absent");
    let c = Candidates {
        prefix: Some(missing.clone()),
        known: vec![
            PathBuf::from("/nonexistent/a"),
            PathBuf::from("/nonexistent/b"),
        ],
    };
    let err = build(OcrConfig::default(), &c).expect_err("nothing to find");
    assert_eq!(err.reason, UnavailableReason::NoTessdata);
    assert_eq!(
        err.searched,
        vec![
            missing.clone(),
            missing.join("tessdata"),
            PathBuf::from("/nonexistent/a"),
            PathBuf::from("/nonexistent/b")
        ]
    );
    let err = build(OcrConfig::default(), &common::nowhere()).expect_err("empty list");
    assert_eq!(err.reason, UnavailableReason::NoTessdata);
    assert!(err.searched.is_empty());
}

#[cfg(unix)]
#[test]
fn the_seventh_known_directory_is_found_after_eight_misses() {
    let root = common::scratch("seventh");
    let known: Vec<PathBuf> = (1..=7).map(|i| root.join(format!("k{i}"))).collect();
    common::link_model(&known[6]);
    let env = root.join("env");
    let c = Candidates {
        prefix: Some(env.clone()),
        known: known.clone(),
    };
    let engine = build(OcrConfig::default(), &c).expect("engine");
    assert_eq!(engine.tessdata(), Some(known[6].as_path()));
    // `searched` is reported on failure; a missing second language shows
    // the same list.
    let cfg = OcrConfig {
        languages: vec!["eng".into(), "deu".into()],
        ..OcrConfig::default()
    };
    let err = build(cfg, &c).expect_err("deu missing");
    assert_eq!(err.reason, UnavailableReason::LanguageDataMissing);
    assert_eq!(err.tessdata.as_deref(), Some(known[6].as_path()));
    assert_eq!(err.searched.len(), 8);
    assert_eq!(&err.searched[..2], &[env.clone(), env.join("tessdata")]);
    assert_eq!(&err.searched[2..], &known[..6]);
}

#[cfg(unix)]
#[test]
fn the_prefix_accepts_a_parent_of_tessdata() {
    let root = common::scratch("prefix-parent");
    common::link_model(&root.join("tessdata"));
    let c = Candidates {
        prefix: Some(root.clone()),
        known: vec![],
    };
    let engine = build(OcrConfig::default(), &c).expect("engine");
    assert_eq!(engine.tessdata(), Some(root.join("tessdata").as_path()));
}

#[test]
fn an_empty_model_fails_init_with_tesseracts_message() {
    let dir = common::scratch("empty-model");
    std::fs::write(dir.join("eng.traineddata"), b"").expect("write");
    let cfg = OcrConfig {
        tessdata: Some(dir.clone()),
        ..OcrConfig::default()
    };
    let err = build(cfg, &common::nowhere()).expect_err("0-byte model");
    assert_eq!(err.reason, UnavailableReason::InitFailed);
    assert_eq!(err.tessdata.as_deref(), Some(dir.as_path()));
    assert!(!err.detail.is_empty());
}

/// A directory holding a model that loads under the name `good` (a
/// symlink to the test model) and a 0-byte `bad` one.
#[cfg(unix)]
fn good_and_empty(name: &str, good: &str, bad: &str) -> PathBuf {
    let dir = common::scratch(name);
    std::os::unix::fs::symlink(
        common::tessdata().join("eng.traineddata"),
        dir.join(format!("{good}.traineddata")),
    )
    .expect("symlink model");
    std::fs::write(dir.join(format!("{bad}.traineddata")), b"").expect("write");
    dir
}

/// Spec 0003 F20: Tesseract's `Init` succeeds when any language loaded; a
/// requested language that did not load must fail construction instead of
/// being dropped (a second language) or replaced (a first one, which
/// promoted the next to primary). Both isolations go through the one shim.
#[cfg(unix)]
#[test]
fn every_requested_language_must_load() {
    for isolation in [Isolation::InProcess, Isolation::Process] {
        for (name, langs, good, bad) in [
            ("unloaded-second", ["eng", "deu"], "eng", "deu"),
            ("unloaded-first", ["eng", "deu"], "deu", "eng"),
        ] {
            let dir = good_and_empty(&format!("{name}-{isolation:?}"), good, bad);
            let cfg = OcrConfig {
                tessdata: Some(dir.clone()),
                languages: langs.iter().map(|l| (*l).to_owned()).collect(),
                isolation,
                helper_dir: Some(common::helper_dir()),
                ..OcrConfig::default()
            };
            let err = build(cfg, &common::nowhere()).expect_err(name);
            assert_eq!(err.reason, UnavailableReason::InitFailed, "{name}: {err:?}");
            assert!(
                err.detail
                    .contains(&format!("language \"{bad}\" did not load")),
                "{name} {isolation:?}: {}",
                err.detail
            );
            assert_eq!(err.tessdata.as_deref(), Some(dir.as_path()));
        }
    }
}

#[test]
fn a_path_like_language_is_refused_before_any_lookup() {
    let cfg = OcrConfig {
        tessdata: Some(common::tessdata()),
        languages: vec!["../eng".into()],
        ..OcrConfig::default()
    };
    let err = build(cfg, &common::nowhere()).expect_err("bad name");
    assert_eq!(err.reason, UnavailableReason::InitFailed);
    assert!(err.detail.contains("../eng"), "{}", err.detail);
    // Refused by validation, before resolution: nothing was searched.
    assert!(err.searched.is_empty());
    assert_eq!(err.tessdata, None);
}

#[test]
fn out_of_range_fields_fail_naming_the_field() {
    let cases: [(OcrConfig, &str); 4] = [
        (
            OcrConfig {
                dpi: 0,
                ..common::config()
            },
            "dpi",
        ),
        (
            OcrConfig {
                dpi: 5000,
                ..common::config()
            },
            "dpi",
        ),
        (
            OcrConfig {
                max_instances: 0,
                ..common::config()
            },
            "max_instances",
        ),
        (
            OcrConfig {
                page_timeout: Duration::ZERO,
                ..common::config()
            },
            "page_timeout",
        ),
    ];
    for (cfg, field) in cases {
        let err = build(cfg, &common::nowhere()).expect_err(field);
        assert_eq!(err.reason, UnavailableReason::InitFailed);
        assert!(err.detail.contains(field), "{} lacks {field}", err.detail);
    }
}

#[test]
fn legacy_engine_modes_fail_fast_on_the_vendored_build() {
    let cfg = OcrConfig {
        oem: EngineMode::TesseractOnly,
        ..common::config()
    };
    let r = build(cfg, &common::nowhere());
    if cfg!(feature = "vendored") {
        let err = r.expect_err("no legacy engine");
        assert_eq!(err.reason, UnavailableReason::InitFailed);
        assert!(err.detail.contains("legacy engine not built"));
    }
    let cfg = OcrConfig {
        oem: EngineMode::Default,
        ..common::config()
    };
    build(cfg, &common::nowhere()).expect("Default is LSTM here");
}

#[test]
fn process_isolation_is_not_built_yet() {
    let cfg = OcrConfig {
        isolation: Isolation::Process,
        ..common::config()
    };
    let err = build(cfg, &common::nowhere()).expect_err("unit 09");
    assert_eq!(err.reason, UnavailableReason::EngineNotBuilt);
}

#[test]
fn the_public_constructor_uses_the_environment() {
    // With an explicit directory the environment is not needed.
    let engine = OcrEngine::tesseract(common::config()).expect("engine");
    assert_eq!(engine.tessdata(), Some(common::tessdata().as_path()));
    let backend = pdf_extract_ocr::TesseractBackend::new(&common::config()).expect("backend");
    assert!(format!("{backend:?}").contains("TesseractBackend"));
}
