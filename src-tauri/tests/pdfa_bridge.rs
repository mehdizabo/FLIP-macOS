//! The two PDF/A bridge commands (`pdf-printable`, `pdfa-inspect`) run for real against the fake
//! fixtures made by `scripts/make-pdfa-fixture.swift`, plus one ignored end-to-end run of
//! `pdfa::convert` with the real Ghostscript.
//!
//! Build the bridge first (`./native-bridge/build.sh`): these tests use the binary of this checkout.

use flip_lib::engines::bridge::{self, BridgeError};
use flip_lib::engines::pdfa::{self, PdfaTools};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

/// The bridge of this checkout; fails loudly when it is missing or comes from somewhere else.
fn bridge_path() -> PathBuf {
    let path = bridge::resolve_bridge_binary()
        .expect("flip-macos-bridge not found: run ./native-bridge/build.sh");
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("src-tauri has a parent");
    assert!(
        path.starts_with(repo_root),
        "the bridge must be this checkout's own build, got {} (is FLIP_BRIDGE_PATH set?)",
        path.display()
    );
    path
}

/// Runs one bridge command and parses its last stdout line.
fn run_bridge(args: &[&str]) -> Result<Value, BridgeError> {
    let args: Vec<String> = args.iter().map(|a| (*a).to_string()).collect();
    let (stdout, _stderr) = bridge::run_external_command(&bridge_path(), &args, None, 120, None)?;
    let line = stdout
        .lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .expect("the bridge printed a line");
    Ok(serde_json::from_str(line).expect("the last bridge line is JSON"))
}

fn run_ok(args: &[&str]) -> Value {
    run_bridge(args).unwrap_or_else(|err| panic!("bridge {args:?} failed: {err}"))
}

fn path_str(path: &Path) -> &str {
    path.to_str().expect("test paths are valid UTF-8")
}

#[test]
fn pdf_printable_marks_the_links_and_has_nothing_left_to_do_the_second_time() {
    let dir = tempfile::tempdir().unwrap();
    let source = fixture("pdfa-source-links.pdf");
    let first = dir.path().join("first.pdf");
    let second = dir.path().join("second.pdf");

    // Both links of the fixture lack the Print flag: one has no `/F` entry at all (PDFKit's `shouldPrint`
    // calls that printable, Ghostscript drops it), the other has `/F 0`. Both must be counted as changed.
    let answer = run_ok(&["pdf-printable", path_str(&source), path_str(&first)]);
    assert_eq!(answer["ok"], true, "{answer}");
    assert_eq!(answer["annotations"], 2, "{answer}");
    assert_eq!(answer["changed"], 2, "{answer}");
    assert_eq!(answer["written"], true, "{answer}");
    assert!(
        first.is_file(),
        "the output must be written when something changed"
    );

    // Same pages and same text, so nothing but the flags changed.
    let before = run_ok(&["pdfa-inspect", path_str(&source)]);
    let after = run_ok(&["pdfa-inspect", path_str(&first)]);
    assert_eq!(after["pages"], before["pages"]);
    assert_eq!(after["characters"], before["characters"]);

    let again = run_ok(&["pdf-printable", path_str(&first), path_str(&second)]);
    assert_eq!(again["ok"], true, "{again}");
    assert_eq!(again["annotations"], 2, "{again}");
    assert_eq!(again["changed"], 0, "{again}");
    assert_eq!(again["written"], false, "{again}");
    assert!(
        !second.exists(),
        "nothing may be written when nothing changed"
    );
}

#[test]
fn pdf_printable_refuses_a_password_protected_pdf() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("out.pdf");
    let err = run_bridge(&[
        "pdf-printable",
        path_str(&fixture("pdfa-locked.pdf")),
        path_str(&out),
    ])
    .expect_err("a locked PDF is an error");
    match err {
        BridgeError::CommandFailed(_, detail) => {
            assert!(detail.contains("protected by a password"), "{detail}");
        }
        other => panic!("expected a failed command, got {other:?}"),
    }
    assert!(!out.exists());
}

#[test]
fn pdf_printable_and_pdfa_inspect_fail_cleanly_on_a_missing_file() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("missing.pdf");
    let out = dir.path().join("out.pdf");
    for args in [
        vec!["pdf-printable", path_str(&missing), path_str(&out)],
        vec!["pdfa-inspect", path_str(&missing)],
    ] {
        let err = run_bridge(&args).expect_err("a missing file is an error");
        assert!(matches!(err, BridgeError::CommandFailed(_, _)), "{err:?}");
    }
    assert!(!out.exists());
}

#[test]
fn pdfa_inspect_reads_the_source_the_locked_file_and_the_pdfa_sample() {
    let source = run_ok(&["pdfa-inspect", path_str(&fixture("pdfa-source-links.pdf"))]);
    assert_eq!(source["ok"], true, "{source}");
    assert_eq!(source["pages"], 2, "{source}");
    assert!(source["characters"].as_u64().unwrap() > 200, "{source}");
    assert_eq!(source["locked"], false, "{source}");
    assert_eq!(source["outputIntent"], false, "{source}");
    assert!(source["part"].is_null(), "{source}");
    assert!(source["conformance"].is_null(), "{source}");

    let locked = run_ok(&["pdfa-inspect", path_str(&fixture("pdfa-locked.pdf"))]);
    assert_eq!(locked["ok"], true, "{locked}");
    assert_eq!(locked["locked"], true, "{locked}");
    assert_eq!(locked["pages"], 0, "{locked}");
    assert_eq!(locked["characters"], 0, "{locked}");
    assert_eq!(locked["outputIntent"], false, "{locked}");
    assert!(
        locked["part"].is_null() && locked["conformance"].is_null(),
        "{locked}"
    );

    // The sample is the committed, veraPDF-checked Ghostscript result of the source fixture.
    let sample = run_ok(&["pdfa-inspect", path_str(&fixture("pdfa-sample-2b.pdf"))]);
    assert_eq!(sample["ok"], true, "{sample}");
    assert_eq!(sample["pages"], 2, "{sample}");
    assert_eq!(sample["part"], 2, "{sample}");
    assert_eq!(sample["conformance"], "B", "{sample}");
    assert_eq!(sample["outputIntent"], true, "{sample}");
    assert_eq!(sample["locked"], false, "{sample}");
    assert!(sample["characters"].as_u64().unwrap() > 200, "{sample}");
}

#[test]
fn the_pdfa_sample_kept_both_links_and_they_print() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("unused.pdf");
    let answer = run_ok(&[
        "pdf-printable",
        path_str(&fixture("pdfa-sample-2b.pdf")),
        path_str(&out),
    ]);
    assert_eq!(
        answer["annotations"], 2,
        "the links must survive Ghostscript: {answer}"
    );
    assert_eq!(
        answer["changed"], 0,
        "PDF/A annotations all print: {answer}"
    );
    assert_eq!(answer["written"], false, "{answer}");
}

/// Candidate places of a real Ghostscript on the lead's Mac.
fn real_ghostscript() -> Option<PathBuf> {
    ["/opt/homebrew/bin/gs", "/usr/local/bin/gs"]
        .iter()
        .map(PathBuf::from)
        .find(|candidate| candidate.is_file())
}

/// The full recipe with the REAL Ghostscript and the REAL bridge on the committed fixture, in paths full of
/// `%`, parentheses, spaces and accents. Ignored by default (it needs Ghostscript, which tests must not require):
/// `cargo test --test pdfa_bridge -- --ignored --nocapture real_ghostscript_run`.
/// Set `FLIP_PDFA_KEEP_SAMPLE=<path>` to also copy the result there (that is how `pdfa-sample-2b.pdf` was made).
#[test]
#[ignore = "needs the real Ghostscript; run by hand"]
fn real_ghostscript_run() {
    let Some(gs) = real_ghostscript() else {
        eprintln!("skipped: no Ghostscript at /opt/homebrew/bin/gs or /usr/local/bin/gs");
        return;
    };
    let profile =
        pdfa::srgb_profile_for(&gs).expect("the sRGB profile of this Ghostscript install");
    eprintln!(
        "ghostscript: {}  profile: {}",
        gs.display(),
        profile.display()
    );

    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("Manuale 100% (è) sorgente.pdf");
    fs::copy(fixture("pdfa-source-links.pdf"), &source).unwrap();
    let work = dir.path().join("lavoro 100% (è)");
    let out = dir.path().join("Manuale 100% (è) sorgente 2.pdf");

    let bridge = bridge_path();
    let tools = PdfaTools {
        ghostscript: &gs,
        bridge: &bridge,
    };
    let cancel = Arc::new(AtomicBool::new(false));
    let signals = pdfa::convert(&source, &out, &tools, &work, &cancel)
        .unwrap_or_else(|err| panic!("the real conversion failed: {err}"));
    eprintln!("signals: {signals:?}");

    assert_eq!(signals.part, Some(2));
    assert_eq!(signals.conformance, Some('B'));
    assert!(signals.has_output_intent);
    assert!(out.is_file());
    assert_eq!(
        fs::read_dir(&work).unwrap().count(),
        0,
        "the work dir is empty afterwards"
    );

    let result = run_ok(&["pdfa-inspect", path_str(&out)]);
    eprintln!("inspection of the result: {result}");
    assert_eq!(result["pages"], 2);
    assert!(result["characters"].as_u64().unwrap() > 200, "{result}");

    // The pre-pass worked: both links are still there (before it, Ghostscript drops them).
    let probe = dir.path().join("probe.pdf");
    let links = run_ok(&["pdf-printable", path_str(&out), path_str(&probe)]);
    assert_eq!(links["annotations"], 2, "links lost: {links}");

    if let Ok(keep) = std::env::var("FLIP_PDFA_KEEP_SAMPLE") {
        fs::copy(&out, &keep)
            .unwrap_or_else(|err| panic!("could not keep the sample at {keep}: {err}"));
        eprintln!("kept the result at {keep}");
    }
}
