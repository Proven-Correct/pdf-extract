//! Unit tests of `OcrConfig` defaults and validation (spec 0002 O8, §4.8
//! "Construction").

use super::*;

fn reject(cfg: OcrConfig, needle: &str) {
    let err = cfg.validate().expect_err("must be rejected");
    assert_eq!(err.reason, UnavailableReason::InitFailed);
    assert!(
        err.detail.contains(needle),
        "{:?} lacks {needle:?}",
        err.detail
    );
    assert!(err.searched.is_empty() && err.tessdata.is_none());
}

#[test]
fn defaults_match_o8() {
    let c = OcrConfig::default();
    assert_eq!(c.tessdata, None);
    assert_eq!(c.languages, vec!["eng".to_owned()]);
    assert_eq!(c.dpi, 300);
    let expected = std::thread::available_parallelism().map_or(1, |n| n.get().min(4));
    assert_eq!(c.max_instances, expected);
    assert!((1..=4).contains(&c.max_instances));
    assert_eq!(c.page_timeout, Duration::from_secs(60));
    assert_eq!(c.psm, PageSegMode::Auto);
    assert_eq!(c.oem, EngineMode::LstmOnly);
    assert_eq!(c.max_render_pixels, 40_000_000);
    assert_eq!(c.isolation, Isolation::InProcess);
    assert_eq!(c.helper_dir, None);
    assert!(
        c.noise_precheck,
        "run 0003 Q3: the pre-check is on by default"
    );
    c.validate().expect("defaults are valid");
}

#[test]
fn out_of_range_fields_are_named() {
    let base = OcrConfig::default;
    reject(OcrConfig { dpi: 0, ..base() }, "dpi");
    reject(
        OcrConfig {
            dpi: 5000,
            ..base()
        },
        "dpi",
    );
    reject(
        OcrConfig {
            max_instances: 0,
            ..base()
        },
        "max_instances",
    );
    reject(
        OcrConfig {
            page_timeout: Duration::ZERO,
            ..base()
        },
        "page_timeout",
    );
    reject(
        OcrConfig {
            max_render_pixels: 0,
            ..base()
        },
        "max_render_pixels",
    );
    reject(
        OcrConfig {
            languages: vec![],
            ..base()
        },
        "languages",
    );
    // "+", "eng+", "+eng", "eng++fra": the empty-member cases the sys crate
    // and Python refuse too (spec 0003 F19).
    for bad in [
        "../eng", "eng/x", "", "eng+deu", "+", "eng+", "+eng", "eng++fra", "e n", "é",
    ] {
        reject(
            OcrConfig {
                languages: vec!["eng".into(), bad.into()],
                ..base()
            },
            "invalid language name",
        );
    }
    // The bounds themselves are accepted.
    for dpi in [MIN_DPI, MAX_DPI] {
        OcrConfig { dpi, ..base() }.validate().expect("in range");
    }
    OcrConfig {
        languages: vec!["chi_sim".into(), "Eng2".into()],
        ..base()
    }
    .validate()
    .expect("plain names");
}

#[test]
fn process_isolation_caps_the_render_pixels_at_one_frame() {
    // The cap is the frame's pixel room less the rounding slack, so a raster
    // at the cap still fits one Recognize frame (64 MiB − 17 bytes).
    assert_eq!(MAX_PROCESS_RENDER_PIXELS, 66_977_776);
    assert!(
        MAX_PROCESS_RENDER_PIXELS + pdf_extract_render::RASTER_PIXEL_SLACK
            <= wire::MAX_RASTER_BYTES as u64
    );
    let at = |isolation, max_render_pixels| OcrConfig {
        isolation,
        max_render_pixels,
        ..OcrConfig::default()
    };
    at(Isolation::Process, MAX_PROCESS_RENDER_PIXELS)
        .validate()
        .expect("the cap itself is accepted");
    reject(
        at(Isolation::Process, MAX_PROCESS_RENDER_PIXELS + 1),
        "max_render_pixels: 66977777 is over 66977776",
    );
    reject(at(Isolation::Process, u64::MAX), "Isolation::Process");
    // In-process recognition has no frame, so no such cap.
    at(Isolation::InProcess, u64::MAX)
        .validate()
        .expect("no frame in process");
}

#[test]
fn legacy_modes_follow_the_build() {
    for oem in [EngineMode::TesseractOnly, EngineMode::TesseractLstmCombined] {
        let cfg = OcrConfig {
            oem,
            ..OcrConfig::default()
        };
        if sys::LEGACY_ENGINE_BUILT {
            cfg.validate().expect("system build lets Init decide");
        } else {
            reject(cfg, "legacy engine not built");
        }
    }
    for oem in [EngineMode::LstmOnly, EngineMode::Default] {
        OcrConfig {
            oem,
            ..OcrConfig::default()
        }
        .validate()
        .expect("LSTM modes");
    }
}

#[test]
fn enums_map_to_tesseract_numbering() {
    let all = [
        PageSegMode::OsdOnly,
        PageSegMode::AutoOsd,
        PageSegMode::AutoOnly,
        PageSegMode::Auto,
        PageSegMode::SingleColumn,
        PageSegMode::SingleBlockVertText,
        PageSegMode::SingleBlock,
        PageSegMode::SingleLine,
        PageSegMode::SingleWord,
        PageSegMode::CircleWord,
        PageSegMode::SingleChar,
        PageSegMode::SparseText,
        PageSegMode::SparseTextOsd,
        PageSegMode::RawLine,
    ];
    for (i, m) in all.into_iter().enumerate() {
        assert_eq!(m.to_sys() as usize, i, "{m:?}");
    }
    let oems = [
        EngineMode::TesseractOnly,
        EngineMode::LstmOnly,
        EngineMode::TesseractLstmCombined,
        EngineMode::Default,
    ];
    for (i, m) in oems.into_iter().enumerate() {
        assert_eq!(m.to_sys() as usize, i, "{m:?}");
    }
}

#[test]
fn joined_languages_use_plus() {
    let c = OcrConfig {
        languages: vec!["eng".into(), "deu".into()],
        ..OcrConfig::default()
    };
    assert_eq!(c.joined_languages(), "eng+deu");
}
