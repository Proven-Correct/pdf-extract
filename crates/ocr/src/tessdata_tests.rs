//! Unit tests of model-directory resolution (spec 0002 O9, §4.8
//! "Resolution"), on a fake filesystem under the temp directory.

use super::*;

/// A fresh, empty directory unique to this test.
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pxo-tessdata-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir");
    dir
}

fn touch(dir: &Path, lang: &str) {
    std::fs::create_dir_all(dir).expect("mkdir");
    std::fs::write(model_path(dir, lang), b"model").expect("write");
}

fn langs(names: &[&str]) -> Vec<String> {
    names.iter().map(|s| (*s).to_owned()).collect()
}

#[test]
fn explicit_directory_wins() {
    let root = scratch("explicit");
    let explicit = root.join("explicit");
    let env = root.join("env");
    touch(&explicit, "eng");
    touch(&env, "eng");
    let c = Candidates {
        prefix: Some(env),
        known: vec![],
    };
    let r = resolve(Some(&explicit), &c, &langs(&["eng"])).expect("resolves");
    assert_eq!(r.dir, explicit);
    assert!(r.searched.is_empty());
}

#[test]
fn prefix_accepts_both_forms() {
    let root = scratch("prefix");
    let flat = root.join("flat");
    touch(&flat, "eng");
    let c = Candidates {
        prefix: Some(flat.clone()),
        known: vec![],
    };
    assert_eq!(resolve(None, &c, &langs(&["eng"])).expect("flat").dir, flat);

    let nested = root.join("nested");
    touch(&nested.join("tessdata"), "eng");
    let c = Candidates {
        prefix: Some(nested.clone()),
        known: vec![],
    };
    let r = resolve(None, &c, &langs(&["eng"])).expect("nested");
    assert_eq!(r.dir, nested.join("tessdata"));
    assert_eq!(r.searched, vec![nested]);
}

#[test]
fn known_directories_are_tried_in_order() {
    let root = scratch("known");
    let known: Vec<PathBuf> = (1..=7).map(|i| root.join(format!("k{i}"))).collect();
    touch(&known[6], "eng");
    // A later directory holding the model too must not be reached.
    let later = root.join("k8");
    touch(&later, "eng");
    let mut list = known.clone();
    list.push(later);
    let env = root.join("env");
    let c = Candidates {
        prefix: Some(env.clone()),
        known: list,
    };
    let r = resolve(None, &c, &langs(&["eng"])).expect("resolves");
    assert_eq!(r.dir, known[6]);
    let mut expected = vec![env.clone(), env.join("tessdata")];
    expected.extend(known[..6].iter().cloned());
    assert_eq!(r.searched, expected);
    assert_eq!(r.searched.len(), 8);
}

#[test]
fn a_missing_second_language_names_the_chosen_directory() {
    let root = scratch("second");
    let first = root.join("first");
    let second = root.join("second");
    touch(&first, "eng");
    touch(&second, "eng");
    touch(&second, "deu");
    let c = Candidates {
        prefix: None,
        known: vec![first.clone(), second],
    };
    let err = resolve(None, &c, &langs(&["eng", "deu"])).expect_err("no fall-through");
    assert_eq!(err.reason, UnavailableReason::LanguageDataMissing);
    assert_eq!(err.tessdata.as_deref(), Some(first.as_path()));
    assert!(err.detail.contains("deu.traineddata"), "{}", err.detail);
    assert!(err.detail.contains(&first.display().to_string()));
    assert_eq!(err.languages, langs(&["eng", "deu"]));
}

#[test]
fn nothing_found_lists_every_candidate() {
    let root = scratch("none");
    let env = root.join("env");
    let known = vec![root.join("a"), root.join("b")];
    let explicit = root.join("explicit");
    let c = Candidates {
        prefix: Some(env.clone()),
        known: known.clone(),
    };
    let err = resolve(Some(&explicit), &c, &langs(&["eng"])).expect_err("none");
    assert_eq!(err.reason, UnavailableReason::NoTessdata);
    assert_eq!(err.tessdata, None);
    assert_eq!(
        err.searched,
        vec![
            explicit,
            env.clone(),
            env.join("tessdata"),
            known[0].clone(),
            known[1].clone()
        ]
    );
    assert!(err.detail.contains("eng.traineddata"));
}

/// Spec 0003 F22: the detail keeps its prefix and names what was tried.
#[test]
fn the_no_tessdata_hint_names_what_was_tried() {
    let root = scratch("hint");
    let explicit = root.join("explicit");
    let env = root.join("env");
    let nothing = Candidates {
        prefix: None,
        known: vec![root.join("known")],
    };
    let with_prefix = Candidates {
        prefix: Some(env.clone()),
        known: vec![],
    };
    let detail = |explicit: Option<&Path>, c: &Candidates| {
        let err = resolve(explicit, c, &langs(&["eng"])).expect_err("no model");
        assert_eq!(err.reason, UnavailableReason::NoTessdata);
        assert!(
            err.detail.starts_with("no directory holds eng.traineddata"),
            "{}",
            err.detail
        );
        err.detail
    };

    let d = detail(Some(&explicit), &nothing);
    assert!(d.contains("the tessdata setting"), "{d}");
    assert!(d.contains(&explicit.display().to_string()), "{d}");
    assert!(!d.contains("TESSDATA_PREFIX"), "{d}");

    let d = detail(None, &with_prefix);
    let p = env.display().to_string();
    assert!(d.contains(&format!("TESSDATA_PREFIX={p}")), "{d}");
    assert!(d.contains(&format!("{p}/tessdata lack it")), "{d}");
    assert!(!d.contains("set the tessdata option"), "{d}");
    assert!(!d.contains("tessdata setting"), "{d}");

    let d = detail(Some(&explicit), &with_prefix);
    assert!(d.contains("the tessdata setting"), "{d}");
    assert!(d.contains(&format!("TESSDATA_PREFIX={p}")), "{d}");
    assert!(!d.contains("set the tessdata option"), "{d}");

    let d = detail(None, &nothing);
    assert_eq!(
        d,
        "no directory holds eng.traineddata; set the tessdata option or TESSDATA_PREFIX"
    );
}

#[test]
fn a_directory_named_like_the_model_is_not_a_model() {
    let root = scratch("dir-model");
    std::fs::create_dir_all(model_path(&root, "eng")).expect("mkdir");
    let c = Candidates {
        prefix: Some(root),
        known: vec![],
    };
    let err = resolve(None, &c, &langs(&["eng"])).expect_err("not a file");
    assert_eq!(err.reason, UnavailableReason::NoTessdata);
    // A file where a directory is expected is skipped too.
    let file = scratch("file-candidate").join("plain");
    std::fs::write(&file, b"x").expect("write");
    let c = Candidates {
        prefix: None,
        known: vec![file],
    };
    assert!(resolve(None, &c, &langs(&["eng"])).is_err());
}

#[cfg(unix)]
#[test]
fn an_unreadable_second_model_is_missing() {
    use std::os::unix::fs::PermissionsExt;
    let root = scratch("unreadable");
    touch(&root, "eng");
    touch(&root, "deu");
    let deu = model_path(&root, "deu");
    std::fs::set_permissions(&deu, std::fs::Permissions::from_mode(0o000)).expect("chmod");
    // Root can read anything; the check is meaningful only otherwise.
    let readable_anyway = File::open(&deu).is_ok();
    let c = Candidates {
        prefix: Some(root),
        known: vec![],
    };
    let r = resolve(None, &c, &langs(&["eng", "deu"]));
    if readable_anyway {
        assert!(r.is_ok());
    } else {
        assert_eq!(
            r.expect_err("unreadable").reason,
            UnavailableReason::LanguageDataMissing
        );
    }
}

#[test]
fn the_environment_and_override_feed_current() {
    assert_eq!(prefix_from(None), None);
    assert_eq!(prefix_from(Some(OsString::new())), None);
    assert_eq!(
        prefix_from(Some(OsString::from("/x"))),
        Some(PathBuf::from("/x"))
    );
    let env = Candidates::from_env();
    assert_eq!(env.known.len(), 7);
    assert_eq!(env.known[6], PathBuf::from("/opt/local/share/tessdata"));
    // Override then restore; no other unit test reads `current`.
    let fake = Candidates {
        prefix: Some(PathBuf::from("/fake")),
        known: vec![],
    };
    set_candidates_override(Some(fake.clone()));
    assert_eq!(Candidates::current(), fake);
    set_candidates_override(None);
    assert_eq!(Candidates::current(), Candidates::from_env());
    let before = resolutions();
    let _ = resolve(None, &fake, &langs(&["eng"]));
    assert!(resolutions() > before);
}
