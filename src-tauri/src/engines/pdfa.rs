//! PDF to PDF/A-2b with Ghostscript, checked afterwards instead of trusted.
//!
//! The engine is pure: the two tools arrive as explicit paths and nothing is searched or read from
//! the environment. One conversion runs these steps:
//!
//! 1. the bridge command `pdfa-inspect` reads the source (pages, text size, password lock);
//! 2. the bridge command `pdf-printable` marks every annotation as printable (Ghostscript's PDF/A mode
//!    silently drops the ones that are not), so links survive; if that step fails the original is used;
//! 3. Ghostscript converts with FLIP's own PostScript prefix (`pdfa_prefix.ps`), which embeds an sRGB
//!    output intent;
//! 4. `pdfa-inspect` reads the result back: same page count, PDF/A-2b declared, output intent present;
//! 5. a text guard refuses a result that lost most of the selectable text.
//!
//! Ghostscript's exit code proves nothing (it exits 0 on a password-protected file and writes a
//! page-less PDF, and it leaves a file behind when it fails), so steps 4 and 5 are the contract.
//! Everything is staged in the work directory and moved to the final path only at the very end, so a
//! failed run never leaves a partial output.

use crate::engines::bridge::{self, BridgeError};
use crate::engines::EngineError;
use serde_json::Value;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Declarations read from a PDF: PDF/A part, conformance level and whether an output intent exists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PdfaSignals {
    /// The `pdfaid:part` declaration (2 for PDF/A-2), or `None` when absent or unreadable.
    pub part: Option<u8>,
    /// The `pdfaid:conformance` letter, upper-cased (`B`), or `None` when absent or unreadable.
    pub conformance: Option<char>,
    /// Whether the catalog has a non-empty `/OutputIntents` array.
    pub has_output_intent: bool,
}

/// What the bridge command `pdfa-inspect` reports about one PDF.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PdfInspection {
    /// Number of pages.
    pub pages: u32,
    /// Non-whitespace characters PDFKit extracts, summed over all pages.
    pub characters: u64,
    /// True when the PDF is protected by a password and could not be read.
    pub locked: bool,
    /// The PDF/A declarations found in the file.
    pub signals: PdfaSignals,
}

/// Paths of the two tools one conversion needs (both are required).
#[derive(Debug, Clone, Copy)]
pub struct PdfaTools<'a> {
    /// The Ghostscript binary (`gs`), version 10 or newer.
    pub ghostscript: &'a Path,
    /// The `flip-macos-bridge` binary that answers `pdf-printable` and `pdfa-inspect`.
    pub bridge: &'a Path,
}

/// Minimum share of the source's text that the result must keep, in percent.
pub const MIN_TEXT_KEPT_PERCENT: u64 = 50;
/// Smallest source text size, in non-whitespace characters, from which the text guard applies.
pub const TEXT_CHECK_MIN_CHARACTERS: u64 = 200;

/// FLIP's original PostScript prefix; `@ICC_PROFILE@` is replaced with the sRGB profile path.
const PREFIX_TEMPLATE: &str = include_str!("pdfa_prefix.ps");
const PREFIX_TOKEN: &str = "@ICC_PROFILE@";

/// Output of Ghostscript that means the file cannot be trusted even when the exit code is 0.
const GHOSTSCRIPT_FAILURE_MARKERS: [&str; 7] = [
    "PDF/A processing aborted",
    "reverting to normal output",
    "Failed to open the supplied ICCProfile",
    "requires a password",
    "Couldn't initialise file",
    "No pages will be processed",
    "Unable to open the initial device",
];

/// Safety bound for the pre-pass that marks annotations printable.
const PRE_PASS_TIMEOUT_SECS: u64 = 120;
/// Safety bound for one `pdfa-inspect` call (text extraction of a large manual takes a while).
const INSPECT_TIMEOUT_SECS: u64 = 300;
/// Safety bound for the Ghostscript run.
const GHOSTSCRIPT_TIMEOUT_SECS: u64 = 30 * 60;
/// How much of Ghostscript's error output is quoted in an error message.
const ERROR_TAIL_CHARS: usize = 300;

/// The sRGB profile that belongs to this Ghostscript install, found from the binary's real path (symlinks resolved), or None.
pub fn srgb_profile_for(ghostscript: &Path) -> Option<PathBuf> {
    let real = fs::canonicalize(ghostscript).ok()?;
    let install_prefix = real.parent()?.parent()?;
    let share = install_prefix.join("share").join("ghostscript");

    let flat = share.join("iccprofiles").join("srgb.icc");
    if flat.is_file() {
        return Some(flat);
    }

    // Versioned layout: share/ghostscript/<version>/iccprofiles/srgb.icc, first match in sorted order.
    let mut version_dirs: Vec<PathBuf> = fs::read_dir(&share)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect();
    version_dirs.sort();
    version_dirs
        .into_iter()
        .map(|dir| dir.join("iccprofiles").join("srgb.icc"))
        .find(|candidate| candidate.is_file())
}

/// Convert `source` to PDF/A-2b at `out_pdf` (must not exist yet). Never leaves a partial `out_pdf`; every failure is an `EngineError`.
///
/// `work_dir` holds the staged files of this one run, so two conversions must not share it.
pub fn convert(
    source: &Path,
    out_pdf: &Path,
    tools: &PdfaTools,
    work_dir: &Path,
    cancel: &Arc<AtomicBool>,
) -> Result<PdfaSignals, EngineError> {
    // 1. Refuse early.
    if !source.is_absolute() {
        return Err(EngineError::ConversionFailed(format!(
            "The source path must be absolute: {}",
            source.display()
        )));
    }
    if out_pdf.symlink_metadata().is_ok() {
        return Err(EngineError::ConversionFailed(format!(
            "The output file already exists: {}",
            out_pdf.display()
        )));
    }
    if cancel.load(Ordering::Relaxed) {
        return Err(EngineError::Cancelled);
    }
    let profile = srgb_profile_for(tools.ghostscript).ok_or_else(|| {
        EngineError::Unsupported(
            "Ghostscript's sRGB colour profile was not found next to the Ghostscript installation, \
             so PDF/A cannot be made."
                .to_string(),
        )
    })?;

    // Ghostscript runs inside the work directory: make it absolute so every path handed to it starts with `/`.
    fs::create_dir_all(work_dir)?;
    let work_dir = fs::canonicalize(work_dir)?;
    let files = WorkFiles::new(&work_dir);

    // 2. Read the source.
    let source_info = inspect(tools.bridge, source, cancel).map_err(|failure| match failure {
        InspectFailure::Cancelled => EngineError::Cancelled,
        InspectFailure::Failed(detail) => {
            EngineError::ConversionFailed(format!("FLIP could not read this PDF: {detail}"))
        }
    })?;
    if source_info.locked {
        return Err(EngineError::ConversionFailed(
            "This PDF is protected by a password. Remove the password first.".to_string(),
        ));
    }
    if source_info.pages == 0 {
        return Err(EngineError::ConversionFailed(
            "This PDF has no pages, so there is nothing to convert.".to_string(),
        ));
    }

    // 3. Pre-pass: keep every link. It is an improvement, never a requirement.
    let ghostscript_input =
        if mark_annotations_printable(tools.bridge, source, &files.printable, cancel)? {
            files.printable.clone()
        } else {
            source.to_path_buf()
        };

    // 4. The PostScript prefix with the profile path.
    fs::write(&files.prefix, render_prefix(&profile)?)?;

    // 5. Ghostscript.
    let args = ghostscript_args(&profile, &files.staged, &files.prefix, &ghostscript_input);
    let run = run_tool(
        tools.ghostscript,
        &args,
        &work_dir,
        Duration::from_secs(GHOSTSCRIPT_TIMEOUT_SECS),
        cancel,
    )?;
    if !run.success {
        let detail = if run.stderr.trim().is_empty() {
            &run.stdout
        } else {
            &run.stderr
        };
        let tail = tail_chars(detail, ERROR_TAIL_CHARS);
        let tail = if tail.is_empty() {
            "no error output".to_string()
        } else {
            tail
        };
        return Err(EngineError::ConversionFailed(format!(
            "Ghostscript failed{}: {tail}",
            run.code
                .map(|code| format!(" (exit status {code})"))
                .unwrap_or_default()
        )));
    }
    let transcript = format!("{}\n{}", run.stdout, run.stderr);
    if let Some(marker) = GHOSTSCRIPT_FAILURE_MARKERS
        .iter()
        .find(|marker| transcript.contains(**marker))
    {
        return Err(if *marker == "requires a password" {
            EngineError::ConversionFailed(
                "This PDF is protected by a password. Remove the password first.".to_string(),
            )
        } else {
            EngineError::ConversionFailed(format!(
                "Ghostscript could not convert this PDF to PDF/A (it reported: {marker})."
            ))
        });
    }
    if !files.staged.is_file() {
        return Err(EngineError::ConversionFailed(
            "Ghostscript finished but wrote no file.".to_string(),
        ));
    }

    // 6. Read the result back.
    let output_info =
        inspect(tools.bridge, &files.staged, cancel).map_err(|failure| match failure {
            InspectFailure::Cancelled => EngineError::Cancelled,
            InspectFailure::Failed(detail) => EngineError::ConversionFailed(format!(
                "The converted file is not a valid PDF/A-2b (it could not be read back: {detail})."
            )),
        })?;
    if let Some(problem) = pdfa_problem(&source_info, &output_info) {
        return Err(EngineError::ConversionFailed(format!(
            "The converted file is not a valid PDF/A-2b ({problem})."
        )));
    }

    // 7. Text guard.
    if !text_guard_passes(source_info.characters, output_info.characters) {
        return Err(EngineError::ConversionFailed(
            "FLIP cannot keep the text of this PDF selectable in PDF/A, so nothing was written."
                .to_string(),
        ));
    }

    // 8. Move the staged file into place.
    if cancel.load(Ordering::Relaxed) {
        return Err(EngineError::Cancelled);
    }
    move_into_place(&files.staged, out_pdf)?;
    Ok(output_info.signals)
}

/// Staged files of one run; they are deleted when the run ends, whatever the outcome.
struct WorkFiles {
    printable: PathBuf,
    prefix: PathBuf,
    staged: PathBuf,
}

impl WorkFiles {
    fn new(work_dir: &Path) -> Self {
        let files = Self {
            printable: work_dir.join("printable.pdf"),
            prefix: work_dir.join("flip-pdfa-prefix.ps"),
            staged: work_dir.join("pdfa-out.pdf"),
        };
        // A leftover of an earlier run must never be mistaken for this run's output.
        files.remove_all();
        files
    }

    fn remove_all(&self) {
        for path in [&self.printable, &self.prefix, &self.staged] {
            let _ = fs::remove_file(path);
        }
    }
}

impl Drop for WorkFiles {
    fn drop(&mut self) {
        self.remove_all();
    }
}

/// Why a conversion result is not accepted as PDF/A-2b, or `None` when it is.
fn pdfa_problem(source: &PdfInspection, output: &PdfInspection) -> Option<String> {
    if output.locked {
        return Some("it cannot be opened".to_string());
    }
    if output.pages != source.pages {
        return Some(format!(
            "it has {} pages instead of {}",
            output.pages, source.pages
        ));
    }
    if output.signals.part != Some(2) {
        return Some("it does not declare PDF/A part 2".to_string());
    }
    if output.signals.conformance != Some('B') {
        return Some("it does not declare conformance level B".to_string());
    }
    if !output.signals.has_output_intent {
        return Some("it has no colour output intent".to_string());
    }
    None
}

/// The text guard: a source with enough text must keep at least `MIN_TEXT_KEPT_PERCENT` of it.
fn text_guard_passes(source_characters: u64, output_characters: u64) -> bool {
    source_characters < TEXT_CHECK_MIN_CHARACTERS
        || u128::from(output_characters) * 100
            >= u128::from(source_characters) * u128::from(MIN_TEXT_KEPT_PERCENT)
}

/// The exact Ghostscript command line of the proven recipe; every item is one argument, never a shell string.
fn ghostscript_args(profile: &Path, staged: &Path, prefix: &Path, input: &Path) -> Vec<String> {
    vec![
        "-q".to_string(),
        "-dSAFER".to_string(),
        "-dBATCH".to_string(),
        "-dNOPAUSE".to_string(),
        "-dNOOUTERSAVE".to_string(),
        "-dPDFA=2".to_string(),
        "-dPDFACompatibilityPolicy=1".to_string(),
        "-sColorConversionStrategy=RGB".to_string(),
        "-sDEVICE=pdfwrite".to_string(),
        format!("--permit-file-read={}", profile.to_string_lossy()),
        // `%` starts a printf-style field in the output file name; `%%` is a literal percent sign.
        format!(
            "-sOutputFile={}",
            staged.to_string_lossy().replace('%', "%%")
        ),
        prefix.to_string_lossy().into_owned(),
        input.to_string_lossy().into_owned(),
    ]
}

/// The PostScript prefix with the profile path escaped for a PostScript string.
fn render_prefix(profile: &Path) -> Result<String, EngineError> {
    let profile = profile.to_str().ok_or_else(|| {
        EngineError::Unsupported(
            "The path of Ghostscript's sRGB colour profile is not valid text.".to_string(),
        )
    })?;
    if profile.contains(['\n', '\r', '\0']) {
        return Err(EngineError::Unsupported(
            "The path of Ghostscript's sRGB colour profile contains a line break.".to_string(),
        ));
    }
    Ok(PREFIX_TEMPLATE.replace(PREFIX_TOKEN, &escape_postscript_string(profile)))
}

/// Backslash, `(` and `)` get a backslash so the text can sit inside a PostScript `( … )` string.
fn escape_postscript_string(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for ch in text.chars() {
        if matches!(ch, '\\' | '(' | ')') {
            escaped.push('\\');
        }
        escaped.push(ch);
    }
    escaped
}

/// Runs `pdf-printable`. `Ok(true)`: `printable` was written and is the better Ghostscript input.
/// `Ok(false)`: nothing changed or the step failed, so the original is used (only a cancel is an error).
fn mark_annotations_printable(
    bridge_exe: &Path,
    source: &Path,
    printable: &Path,
    cancel: &Arc<AtomicBool>,
) -> Result<bool, EngineError> {
    let args = vec![
        "pdf-printable".to_string(),
        source.to_string_lossy().into_owned(),
        printable.to_string_lossy().into_owned(),
    ];
    match bridge_json(bridge_exe, &args, PRE_PASS_TIMEOUT_SECS, cancel) {
        Ok(answer) => {
            let written = answer.get("ok").and_then(Value::as_bool) == Some(true)
                && answer.get("written").and_then(Value::as_bool) == Some(true);
            Ok(written && printable.is_file())
        }
        Err(BridgeError::Cancelled) => Err(EngineError::Cancelled),
        Err(_) => Ok(false),
    }
}

enum InspectFailure {
    Cancelled,
    Failed(String),
}

/// Runs `pdfa-inspect` on `pdf`.
fn inspect(
    bridge_exe: &Path,
    pdf: &Path,
    cancel: &Arc<AtomicBool>,
) -> Result<PdfInspection, InspectFailure> {
    let args = vec![
        "pdfa-inspect".to_string(),
        pdf.to_string_lossy().into_owned(),
    ];
    let answer =
        bridge_json(bridge_exe, &args, INSPECT_TIMEOUT_SECS, cancel).map_err(|err| match err {
            BridgeError::Cancelled => InspectFailure::Cancelled,
            BridgeError::TimedOut(_, secs) => {
                InspectFailure::Failed(format!("it took longer than {secs} seconds"))
            }
            BridgeError::CommandFailed(_, detail) => {
                InspectFailure::Failed(bridge_error_text(&detail))
            }
            other => InspectFailure::Failed(other.to_string()),
        })?;
    parse_inspection(&answer).map_err(InspectFailure::Failed)
}

/// Runs one bridge command and parses the last non-empty stdout line as JSON.
fn bridge_json(
    bridge_exe: &Path,
    args: &[String],
    timeout_secs: u64,
    cancel: &Arc<AtomicBool>,
) -> Result<Value, BridgeError> {
    let (stdout, _stderr) =
        bridge::run_external_command(bridge_exe, args, None, timeout_secs, Some(cancel))?;
    let last_line = stdout
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .ok_or_else(|| BridgeError::InvalidOutput("Empty bridge stdout".to_string()))?;
    serde_json::from_str(last_line).map_err(|err| BridgeError::InvalidOutput(err.to_string()))
}

/// The `error` text of a bridge failure line (`{"ok":false,"error":"…"}`), or the raw text.
fn bridge_error_text(detail: &str) -> String {
    let from_json = detail
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .and_then(|line| serde_json::from_str::<Value>(line).ok())
        .and_then(|value| {
            value
                .get("error")
                .and_then(Value::as_str)
                .map(str::to_string)
        });
    from_json.unwrap_or_else(|| tail_chars(detail, ERROR_TAIL_CHARS))
}

/// Reads the answer of `pdfa-inspect`; anything missing or of the wrong type is an error, never a guess.
fn parse_inspection(answer: &Value) -> Result<PdfInspection, String> {
    if answer.get("ok").and_then(Value::as_bool) != Some(true) {
        return Err(answer
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("the bridge did not report success")
            .to_string());
    }
    let locked = answer
        .get("locked")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let pages = answer
        .get("pages")
        .and_then(Value::as_u64)
        .and_then(|n| u32::try_from(n).ok())
        .ok_or_else(|| "the page count is missing".to_string())?;
    let characters = answer
        .get("characters")
        .and_then(Value::as_u64)
        .ok_or_else(|| "the text size is missing".to_string())?;
    let part = answer
        .get("part")
        .and_then(Value::as_u64)
        .and_then(|n| u8::try_from(n).ok());
    let conformance = answer
        .get("conformance")
        .and_then(Value::as_str)
        .and_then(|text| {
            let mut letters = text.chars();
            match (letters.next(), letters.next()) {
                (Some(letter), None) if letter.is_ascii_alphabetic() => {
                    Some(letter.to_ascii_uppercase())
                }
                _ => None,
            }
        });
    let has_output_intent = answer
        .get("outputIntent")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    Ok(PdfInspection {
        pages,
        characters,
        locked,
        signals: PdfaSignals {
            part,
            conformance,
            has_output_intent,
        },
    })
}

/// What a finished external tool left behind.
struct ToolRun {
    success: bool,
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

/// Pause before checking a running child again: 1 ms doubling up to 20 ms (also the cancel reaction time).
fn poll_delay(attempt: u32) -> Duration {
    Duration::from_millis((1u64 << attempt.min(5)).min(20))
}

fn drain<R: Read + Send + 'static>(pipe: Option<R>) -> std::thread::JoinHandle<String> {
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        if let Some(mut reader) = pipe {
            let _ = reader.read_to_end(&mut bytes);
        }
        String::from_utf8_lossy(&bytes).into_owned()
    })
}

/// Runs a tool to the end with a timeout and the cancel flag, keeping its whole output.
/// (`bridge::run_external_command` keeps the start of stderr, but for Ghostscript the end is what matters.)
fn run_tool(
    exe: &Path,
    args: &[String],
    cwd: &Path,
    timeout: Duration,
    cancel: &Arc<AtomicBool>,
) -> Result<ToolRun, EngineError> {
    let mut child = Command::new(exe)
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| {
            EngineError::ConversionFailed(format!("Ghostscript could not be started: {err}"))
        })?;
    let out_handle = drain(child.stdout.take());
    let err_handle = drain(child.stderr.take());

    let start = Instant::now();
    let mut attempt = 0u32;
    let outcome = loop {
        if cancel.load(Ordering::Relaxed) {
            break Err(EngineError::Cancelled);
        }
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) => {}
            Err(err) => break Err(EngineError::Io(err)),
        }
        if start.elapsed() >= timeout {
            break Err(EngineError::ConversionFailed(format!(
                "Ghostscript took longer than {} seconds and was stopped.",
                timeout.as_secs()
            )));
        }
        std::thread::sleep(poll_delay(attempt));
        attempt = attempt.saturating_add(1);
    };

    let status = match outcome {
        Ok(status) => status,
        Err(err) => {
            let _ = child.kill();
            let _ = child.wait();
            let _ = out_handle.join();
            let _ = err_handle.join();
            return Err(err);
        }
    };
    Ok(ToolRun {
        success: status.success(),
        code: status.code(),
        stdout: out_handle.join().unwrap_or_default(),
        stderr: err_handle.join().unwrap_or_default(),
    })
}

/// The last `max` characters of `text`, trimmed.
fn tail_chars(text: &str, max: usize) -> String {
    let trimmed = text.trim();
    let count = trimmed.chars().count();
    if count <= max {
        return trimmed.to_string();
    }
    let tail: String = trimmed.chars().skip(count - max).collect();
    format!("…{}", tail.trim_start())
}

/// Moves the staged file to `dest`, which must not exist (checked again here, just before the move).
fn move_into_place(staged: &Path, dest: &Path) -> Result<(), EngineError> {
    if dest.symlink_metadata().is_ok() {
        return Err(EngineError::ConversionFailed(format!(
            "The output file already exists: {}",
            dest.display()
        )));
    }
    if fs::rename(staged, dest).is_ok() {
        return Ok(());
    }
    copy_then_delete(staged, dest)
}

/// Fallback for a rename that fails (another volume): copy without ever overwriting, then delete the staged file.
fn copy_then_delete(staged: &Path, dest: &Path) -> Result<(), EngineError> {
    let mut source = fs::File::open(staged)?;
    let mut target = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(dest)?;
    let copied = std::io::copy(&mut source, &mut target).and_then(|_| target.sync_all());
    drop(target);
    if let Err(err) = copied {
        // Only the file this function just created is removed.
        let _ = fs::remove_file(dest);
        return Err(EngineError::Io(err));
    }
    let _ = fs::remove_file(staged);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{symlink, PermissionsExt};

    const SOURCE_INSPECTION: &str = r#"{"ok":true,"pages":3,"characters":1000,"locked":false,"outputIntent":false,"part":null,"conformance":null}"#;
    const VALID_OUTPUT_INSPECTION: &str = r#"{"ok":true,"pages":3,"characters":990,"locked":false,"outputIntent":true,"part":2,"conformance":"B"}"#;

    fn make_executable(path: &Path, body: &str) {
        fs::write(path, body).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }

    fn fake_ghostscript(ctl: &Path) -> String {
        let ctl = ctl.display();
        format!(
            r#"#!/bin/sh
CTL='{ctl}'
printf '%s\n' "$@" > "$CTL/gs-args.txt"
pwd -P > "$CTL/gs-cwd.txt"
outfile=
for a in "$@"; do
  case "$a" in
    -sOutputFile=*) outfile=${{a#-sOutputFile=}} ;;
    */flip-pdfa-prefix.ps) cp "$a" "$CTL/prefix-seen.ps" ;;
  esac
done
touch "$CTL/gs-ran"
if [ -f "$CTL/gs.sleep" ]; then
  echo $$ > "$CTL/gs.pid"
  echo started > "$CTL/gs-started"
  exec sleep 60
fi
if [ ! -f "$CTL/gs.nowrite" ]; then
  real=$(printf '%s' "$outfile" | sed 's/%%/%/g')
  cp "$CTL/gs-out.pdf" "$real"
fi
cat "$CTL/gs.stdout"
cat "$CTL/gs.stderr" >&2
exit $(cat "$CTL/gs.exit")
"#
        )
    }

    fn fake_bridge(ctl: &Path) -> String {
        let ctl = ctl.display();
        format!(
            r#"#!/bin/sh
CTL='{ctl}'
printf '%s\n' "$*" >> "$CTL/bridge.log"
case "$1" in
  pdf-printable)
    case "$(cat "$CTL/printable.mode")" in
      written) cp "$CTL/printable.pdf" "$3"; echo '{{"ok":true,"annotations":2,"changed":2,"written":true}}' ;;
      unchanged) echo '{{"ok":true,"annotations":0,"changed":0,"written":false}}' ;;
      missing) echo '{{"ok":true,"annotations":2,"changed":2,"written":true}}' ;;
      garbage) echo 'not json at all' ;;
      fail) echo '{{"ok":false,"error":"boom"}}'; exit 1 ;;
    esac ;;
  pdfa-inspect)
    case "$2" in
      */pdfa-out.pdf) cat "$CTL/inspect-out.json" ;;
      *) cat "$CTL/inspect-src.json" ;;
    esac ;;
  *) echo '{{"ok":false,"error":"unknown command"}}'; exit 1 ;;
esac
"#
        )
    }

    struct Env {
        _root: tempfile::TempDir,
        ctl: PathBuf,
        work: PathBuf,
        source: PathBuf,
        out: PathBuf,
        gs: PathBuf,
        bridge: PathBuf,
        profile: PathBuf,
    }

    impl Env {
        fn new() -> Self {
            Self::with_names("prefix", "work", "Manual 2.pdf")
        }

        /// `install_dir`, `work_name` and `out_name` let a test use awkward characters in the paths.
        fn with_names(install_dir: &str, work_name: &str, out_name: &str) -> Self {
            let root = tempfile::tempdir().unwrap();
            let ctl = root.path().join("ctl");
            let install = root.path().join(install_dir);
            fs::create_dir_all(&ctl).unwrap();
            fs::create_dir_all(install.join("bin")).unwrap();
            let icc_dir = install
                .join("share")
                .join("ghostscript")
                .join("iccprofiles");
            fs::create_dir_all(&icc_dir).unwrap();
            let profile = icc_dir.join("srgb.icc");
            fs::write(&profile, b"fake icc").unwrap();

            let env = Self {
                gs: install.join("bin").join("gs"),
                bridge: root.path().join("flip-macos-bridge"),
                work: root.path().join(work_name),
                source: root.path().join("Manual.pdf"),
                out: root.path().join(out_name),
                profile: fs::canonicalize(&profile).unwrap(),
                ctl,
                _root: root,
            };
            fs::write(&env.source, b"%PDF-1.7 fake source").unwrap();
            make_executable(&env.gs, &fake_ghostscript(&env.ctl));
            make_executable(&env.bridge, &fake_bridge(&env.ctl));
            env.set("gs-out.pdf", "%PDF-1.7 fake pdfa output");
            env.set("printable.pdf", "%PDF-1.7 fake printable");
            env.set("printable.mode", "written");
            env.set("inspect-src.json", SOURCE_INSPECTION);
            env.set("inspect-out.json", VALID_OUTPUT_INSPECTION);
            env.set("gs.exit", "0");
            env.set("gs.stdout", "");
            env.set("gs.stderr", "");
            env
        }

        fn set(&self, name: &str, content: &str) {
            fs::write(self.ctl.join(name), content).unwrap();
        }

        fn flag(&self, name: &str) {
            fs::write(self.ctl.join(name), "").unwrap();
        }

        fn ctl_text(&self, name: &str) -> String {
            fs::read_to_string(self.ctl.join(name)).unwrap_or_default()
        }

        fn tools(&self) -> PdfaTools<'_> {
            PdfaTools {
                ghostscript: &self.gs,
                bridge: &self.bridge,
            }
        }

        fn run(&self) -> Result<PdfaSignals, EngineError> {
            self.run_with(&no_cancel())
        }

        fn run_with(&self, cancel: &Arc<AtomicBool>) -> Result<PdfaSignals, EngineError> {
            convert(&self.source, &self.out, &self.tools(), &self.work, cancel)
        }

        fn gs_ran(&self) -> bool {
            self.ctl.join("gs-ran").exists()
        }

        fn gs_args(&self) -> Vec<String> {
            self.ctl_text("gs-args.txt")
                .lines()
                .map(str::to_string)
                .collect()
        }

        /// No partial output and no staged leftovers.
        fn assert_clean(&self) {
            assert!(
                !self.out.exists(),
                "a failed run left {}",
                self.out.display()
            );
            let leftovers: Vec<_> = fs::read_dir(&self.work)
                .map(|entries| entries.filter_map(Result::ok).map(|e| e.path()).collect())
                .unwrap_or_default();
            assert!(
                leftovers.is_empty(),
                "leftovers in the work dir: {leftovers:?}"
            );
        }

        fn expect_failure(&self) -> String {
            let err = self.run().expect_err("the conversion must fail");
            assert!(matches!(err, EngineError::ConversionFailed(_)), "{err:?}");
            self.assert_clean();
            err.to_string()
        }
    }

    fn no_cancel() -> Arc<AtomicBool> {
        Arc::new(AtomicBool::new(false))
    }

    fn inspection_json(
        pages: u32,
        characters: u64,
        part: &str,
        conformance: &str,
        intent: bool,
    ) -> String {
        format!(
            r#"{{"ok":true,"pages":{pages},"characters":{characters},"locked":false,"outputIntent":{intent},"part":{part},"conformance":{conformance}}}"#
        )
    }

    #[test]
    fn converts_with_the_exact_recipe_and_cleans_up() {
        let e = Env::new();
        let signals = e.run().unwrap();

        assert_eq!(
            signals,
            PdfaSignals {
                part: Some(2),
                conformance: Some('B'),
                has_output_intent: true
            }
        );
        assert_eq!(
            fs::read_to_string(&e.out).unwrap(),
            "%PDF-1.7 fake pdfa output"
        );
        assert_eq!(
            fs::read_dir(&e.work).unwrap().count(),
            0,
            "the work dir must be empty"
        );

        let work = fs::canonicalize(&e.work).unwrap();
        let expected = vec![
            "-q".to_string(),
            "-dSAFER".to_string(),
            "-dBATCH".to_string(),
            "-dNOPAUSE".to_string(),
            "-dNOOUTERSAVE".to_string(),
            "-dPDFA=2".to_string(),
            "-dPDFACompatibilityPolicy=1".to_string(),
            "-sColorConversionStrategy=RGB".to_string(),
            "-sDEVICE=pdfwrite".to_string(),
            format!("--permit-file-read={}", e.profile.display()),
            format!("-sOutputFile={}", work.join("pdfa-out.pdf").display()),
            work.join("flip-pdfa-prefix.ps").display().to_string(),
            work.join("printable.pdf").display().to_string(),
        ];
        assert_eq!(e.gs_args(), expected);
        assert_eq!(e.ctl_text("gs-cwd.txt").trim(), work.to_string_lossy());

        let bridge_calls: Vec<String> = e
            .ctl_text("bridge.log")
            .lines()
            .map(str::to_string)
            .collect();
        assert_eq!(
            bridge_calls,
            vec![
                format!("pdfa-inspect {}", e.source.display()),
                format!(
                    "pdf-printable {} {}",
                    e.source.display(),
                    work.join("printable.pdf").display()
                ),
                format!("pdfa-inspect {}", work.join("pdfa-out.pdf").display()),
            ]
        );

        let prefix = e.ctl_text("prefix-seen.ps");
        assert!(prefix.starts_with("%!PS"));
        assert!(prefix.contains(&format!("({}) (r) file", e.profile.display())));
        assert!(!prefix.contains(PREFIX_TOKEN));
    }

    #[test]
    fn the_pre_pass_file_is_used_when_it_was_written() {
        let e = Env::new();
        e.run().unwrap();
        let work = fs::canonicalize(&e.work).unwrap();
        assert_eq!(
            e.gs_args().last().cloned(),
            Some(work.join("printable.pdf").display().to_string())
        );
    }

    #[test]
    fn the_original_is_used_when_the_pre_pass_writes_nothing_or_fails() {
        for mode in ["unchanged", "fail", "garbage", "missing"] {
            let e = Env::new();
            e.set("printable.mode", mode);
            e.run().unwrap_or_else(|err| panic!("mode {mode}: {err}"));

            assert_eq!(
                e.gs_args().last().cloned(),
                Some(e.source.display().to_string()),
                "mode {mode}: Ghostscript must read the original"
            );
            assert!(e.out.exists(), "mode {mode}");
            assert_eq!(fs::read_dir(&e.work).unwrap().count(), 0, "mode {mode}");
        }
    }

    #[test]
    fn a_locked_source_is_refused_before_ghostscript_runs() {
        let e = Env::new();
        e.set(
            "inspect-src.json",
            r#"{"ok":true,"locked":true,"pages":0,"characters":0,"outputIntent":false,"part":null,"conformance":null}"#,
        );
        let message = e.expect_failure();
        assert_eq!(
            message,
            "This PDF is protected by a password. Remove the password first."
        );
        assert!(!e.gs_ran());
        assert!(!e.ctl_text("bridge.log").contains("pdf-printable"));
    }

    #[test]
    fn a_source_without_pages_or_an_unreadable_one_is_refused() {
        let e = Env::new();
        e.set(
            "inspect-src.json",
            &inspection_json(0, 0, "null", "null", false),
        );
        assert!(e.expect_failure().contains("no pages"));
        assert!(!e.gs_ran());

        let e = Env::new();
        e.set(
            "inspect-src.json",
            r#"{"ok":false,"error":"Failed to open PDF"}"#,
        );
        let message = e.expect_failure();
        assert!(message.contains("could not read this PDF"), "{message}");
        assert!(!e.gs_ran());
    }

    #[test]
    fn every_failure_marker_fails_even_with_exit_zero() {
        for (index, marker) in GHOSTSCRIPT_FAILURE_MARKERS.iter().enumerate() {
            let e = Env::new();
            let text = format!("   **** {marker}\n");
            if index % 2 == 0 {
                e.set("gs.stderr", &text);
            } else {
                e.set("gs.stdout", &text);
            }
            let message = e.expect_failure();
            if *marker == "requires a password" {
                assert!(message.contains("protected by a password"), "{message}");
            } else {
                assert!(message.contains(marker), "{marker}: {message}");
            }
        }
    }

    #[test]
    fn plain_warnings_do_not_fail_the_conversion() {
        let e = Env::new();
        e.set(
            "gs.stderr",
            "**** Warning: overprint is not permitted in PDF/A-2\n**** Error: an image was skipped\n",
        );
        e.run().unwrap();
        assert!(e.out.exists());
    }

    #[test]
    fn a_non_zero_exit_quotes_the_end_of_stderr() {
        let e = Env::new();
        let filler = "x".repeat(1500);
        e.set(
            "gs.stderr",
            &format!("first line\n{filler}\nFinal line: the real reason\n"),
        );
        e.set("gs.exit", "1");
        let message = e.expect_failure();
        assert!(message.contains("Final line: the real reason"), "{message}");
        assert!(message.contains("exit status 1"), "{message}");
        assert!(
            !message.contains("first line"),
            "only the tail is quoted: {message}"
        );
        assert!(message.chars().count() < 450, "{}", message.chars().count());
    }

    #[test]
    fn ghostscript_writing_a_file_while_failing_leaves_nothing_behind() {
        let e = Env::new();
        e.set("gs.exit", "1");
        e.set("gs.stderr", "garbage input\n");
        let message = e.expect_failure();
        assert!(message.contains("garbage input"), "{message}");
    }

    #[test]
    fn a_missing_output_file_is_an_error_and_a_stale_one_is_never_used() {
        let e = Env::new();
        fs::create_dir_all(&e.work).unwrap();
        fs::write(
            e.work.join("pdfa-out.pdf"),
            b"%PDF stale result of an earlier run",
        )
        .unwrap();
        e.flag("gs.nowrite");
        let message = e.expect_failure();
        assert!(message.contains("wrote no file"), "{message}");
    }

    #[test]
    fn an_output_without_pdfa_declarations_is_refused() {
        let cases = [
            (
                inspection_json(3, 990, "null", "null", true),
                "does not declare PDF/A part 2",
            ),
            (
                inspection_json(3, 990, "1", "\"B\"", true),
                "does not declare PDF/A part 2",
            ),
            (
                inspection_json(3, 990, "2", "\"U\"", true),
                "conformance level B",
            ),
            (
                inspection_json(3, 990, "2", "null", true),
                "conformance level B",
            ),
            (
                inspection_json(3, 990, "2", "\"B\"", false),
                "no colour output intent",
            ),
        ];
        for (answer, expected) in cases {
            let e = Env::new();
            e.set("inspect-out.json", &answer);
            let message = e.expect_failure();
            assert!(message.contains("not a valid PDF/A-2b"), "{message}");
            assert!(message.contains(expected), "{answer} -> {message}");
        }
    }

    #[test]
    fn an_output_that_cannot_be_read_back_is_refused() {
        let e = Env::new();
        e.set(
            "inspect-out.json",
            r#"{"ok":false,"error":"Failed to open PDF"}"#,
        );
        let message = e.expect_failure();
        assert!(message.contains("not a valid PDF/A-2b"), "{message}");

        let e = Env::new();
        e.set(
            "inspect-out.json",
            r#"{"ok":true,"locked":true,"pages":0,"characters":0,"outputIntent":false,"part":null,"conformance":null}"#,
        );
        assert!(e.expect_failure().contains("not a valid PDF/A-2b"));
    }

    #[test]
    fn a_page_count_mismatch_is_refused() {
        let e = Env::new();
        e.set(
            "inspect-out.json",
            &inspection_json(0, 0, "2", "\"B\"", true),
        );
        let message = e.expect_failure();
        assert!(message.contains("0 pages instead of 3"), "{message}");
    }

    #[test]
    fn the_text_guard_refuses_a_collapse_and_accepts_a_small_loss() {
        let e = Env::new();
        e.set(
            "inspect-out.json",
            &inspection_json(3, 28, "2", "\"B\"", true),
        );
        let message = e.expect_failure();
        assert_eq!(
            message,
            "FLIP cannot keep the text of this PDF selectable in PDF/A, so nothing was written."
        );

        let e = Env::new();
        e.set(
            "inspect-out.json",
            &inspection_json(3, 990, "2", "\"B\"", true),
        );
        e.run().unwrap();
        assert!(e.out.exists());
    }

    #[test]
    fn the_text_guard_boundaries_follow_the_named_constants() {
        assert_eq!(MIN_TEXT_KEPT_PERCENT, 50);
        assert_eq!(TEXT_CHECK_MIN_CHARACTERS, 200);
        assert!(!text_guard_passes(1000, 28));
        assert!(!text_guard_passes(1000, 499));
        assert!(
            text_guard_passes(1000, 500),
            "exactly the minimum is accepted"
        );
        assert!(text_guard_passes(1000, 990));
        assert!(
            text_guard_passes(1000, 5000),
            "more text than before is fine"
        );
        assert!(!text_guard_passes(200, 99));
        assert!(text_guard_passes(200, 100));
        assert!(
            text_guard_passes(199, 0),
            "under the minimum the check is skipped"
        );
        assert!(text_guard_passes(0, 0));
        assert!(!text_guard_passes(u64::MAX, 1), "no overflow");
    }

    #[test]
    fn the_text_guard_is_skipped_for_a_source_with_little_text() {
        let e = Env::new();
        e.set(
            "inspect-src.json",
            &inspection_json(3, 199, "null", "null", false),
        );
        e.set(
            "inspect-out.json",
            &inspection_json(3, 0, "2", "\"B\"", true),
        );
        e.run().unwrap();
        assert!(e.out.exists());
    }

    #[test]
    fn cancel_in_the_middle_kills_ghostscript_and_leaves_nothing() {
        let e = Env::new();
        e.flag("gs.sleep");
        let cancel = no_cancel();
        let flag = cancel.clone();
        let started = e.ctl.join("gs-started");
        let setter = std::thread::spawn(move || {
            let begin = Instant::now();
            while !started.exists() && begin.elapsed() < Duration::from_secs(10) {
                std::thread::sleep(Duration::from_millis(5));
            }
            flag.store(true, Ordering::Relaxed);
        });

        let begin = Instant::now();
        let err = e.run_with(&cancel).expect_err("a cancelled run fails");
        setter.join().unwrap();

        assert!(matches!(err, EngineError::Cancelled), "{err:?}");
        assert!(err.is_cancelled());
        assert!(
            begin.elapsed() < Duration::from_secs(8),
            "cancel latency {:?}",
            begin.elapsed()
        );
        e.assert_clean();
        let pid = e.ctl_text("gs.pid").trim().to_string();
        let alive = Command::new("/bin/kill")
            .args(["-0", &pid])
            .stderr(Stdio::null())
            .status()
            .unwrap();
        assert!(!alive.success(), "Ghostscript {pid} is still running");
    }

    #[test]
    fn a_cancel_that_is_already_set_stops_before_any_tool_runs() {
        let e = Env::new();
        let cancel = Arc::new(AtomicBool::new(true));
        let err = e.run_with(&cancel).unwrap_err();
        assert!(matches!(err, EngineError::Cancelled), "{err:?}");
        assert!(!e.gs_ran());
        assert!(!e.ctl.join("bridge.log").exists());
        assert!(!e.out.exists());
    }

    #[test]
    fn an_existing_output_is_refused_and_left_untouched() {
        let e = Env::new();
        fs::write(&e.out, b"precious").unwrap();
        let err = e.run().unwrap_err();
        assert!(matches!(err, EngineError::ConversionFailed(_)), "{err:?}");
        assert!(err.to_string().contains("already exists"), "{err}");
        assert_eq!(fs::read(&e.out).unwrap(), b"precious");
        assert!(!e.gs_ran());
    }

    #[test]
    fn a_relative_source_is_refused() {
        let e = Env::new();
        let err = convert(
            Path::new("Manual.pdf"),
            &e.out,
            &e.tools(),
            &e.work,
            &no_cancel(),
        )
        .unwrap_err();
        assert!(matches!(err, EngineError::ConversionFailed(_)), "{err:?}");
        assert!(err.to_string().contains("absolute"), "{err}");
        assert!(!e.gs_ran());
        assert!(!e.out.exists());
    }

    #[test]
    fn a_missing_colour_profile_is_reported_as_unsupported() {
        let e = Env::new();
        fs::remove_file(&e.profile).unwrap();
        let err = e.run().unwrap_err();
        assert!(matches!(err, EngineError::Unsupported(_)), "{err:?}");
        assert!(err.to_string().contains("sRGB colour profile"), "{err}");
        assert!(!e.gs_ran());
        assert!(!e.ctl.join("bridge.log").exists());
    }

    #[test]
    fn percent_signs_parentheses_spaces_and_accents_survive_every_hop() {
        let e = Env::with_names(
            "Ghostscript (10) é",
            "work 100% (è) dir",
            "Manuale 100% (è) 2.pdf",
        );
        e.run().unwrap();

        assert_eq!(
            fs::read_to_string(&e.out).unwrap(),
            "%PDF-1.7 fake pdfa output"
        );
        assert_eq!(fs::read_dir(&e.work).unwrap().count(), 0);

        let work = fs::canonicalize(&e.work).unwrap();
        let output_arg = e
            .gs_args()
            .into_iter()
            .find(|line| line.starts_with("-sOutputFile="))
            .unwrap();
        let staged = work.join("pdfa-out.pdf").to_string_lossy().into_owned();
        assert!(staged.contains('%'));
        assert_eq!(
            output_arg,
            format!("-sOutputFile={}", staged.replace('%', "%%"))
        );
        assert!(output_arg.contains("100%% (è)"), "{output_arg}");
        assert_eq!(e.ctl_text("gs-cwd.txt").trim(), work.to_string_lossy());

        // The profile sits in a folder with parentheses: the PostScript string must escape them.
        let prefix = e.ctl_text("prefix-seen.ps");
        let escaped = e
            .profile
            .to_string_lossy()
            .replace('(', "\\(")
            .replace(')', "\\)");
        assert!(escaped.contains("\\(10\\)"), "{escaped}");
        assert!(
            prefix.contains(&format!("({escaped}) (r) file")),
            "{prefix}"
        );
    }

    #[test]
    fn a_missing_work_dir_is_created() {
        let e = Env::with_names("prefix", "nested/deeper/work", "Out 2.pdf");
        e.run().unwrap();
        assert!(e.out.exists());
        assert_eq!(fs::read_dir(&e.work).unwrap().count(), 0);
    }

    #[test]
    fn the_prefix_template_is_original_and_names_the_profile_once_in_code() {
        assert!(PREFIX_TEMPLATE.starts_with("%!PS"));
        // Once in the header comment, once in the `( … ) (r) file` line that reads the profile.
        assert_eq!(PREFIX_TEMPLATE.matches(PREFIX_TOKEN).count(), 2);
        assert_eq!(
            PREFIX_TEMPLATE.matches("(@ICC_PROFILE@) (r) file").count(),
            1
        );
        assert!(PREFIX_TEMPLATE.contains("/GTS_PDFA1"));
        assert!(PREFIX_TEMPLATE.contains("/OutputIntents"));
        assert!(PREFIX_TEMPLATE.contains("/N 3"));
        assert!(
            !PREFIX_TEMPLATE.contains("/DOCINFO"),
            "the document keeps its own title"
        );
        assert!(!PREFIX_TEMPLATE.contains("Title"));
    }

    #[test]
    fn the_profile_path_is_escaped_for_a_postscript_string() {
        assert_eq!(
            escape_postscript_string("/plain/path.icc"),
            "/plain/path.icc"
        );
        assert_eq!(
            escape_postscript_string(r"/a (b)/c\d.icc"),
            r"/a \(b\)/c\\d.icc"
        );
        let rendered = render_prefix(Path::new("/opt/Ghostscript (x)/srgb.icc")).unwrap();
        assert!(
            rendered.contains(r"(/opt/Ghostscript \(x\)/srgb.icc) (r) file"),
            "{rendered}"
        );
        assert!(!rendered.contains(PREFIX_TOKEN));
    }

    #[test]
    fn a_profile_path_with_a_line_break_is_rejected() {
        for bad in ["/tmp/a\nb/srgb.icc", "/tmp/a\rb/srgb.icc"] {
            let err = render_prefix(Path::new(bad)).unwrap_err();
            assert!(matches!(err, EngineError::Unsupported(_)), "{err:?}");
        }
    }

    #[test]
    fn the_command_line_keeps_each_path_as_its_own_argument() {
        let args = ghostscript_args(
            Path::new("/opt/gs share/srgb.icc"),
            Path::new("/w 100%/pdfa-out.pdf"),
            Path::new("/w 100%/flip-pdfa-prefix.ps"),
            Path::new("/in put/a b.pdf"),
        );
        assert_eq!(args.len(), 13);
        assert_eq!(args[9], "--permit-file-read=/opt/gs share/srgb.icc");
        assert_eq!(args[10], "-sOutputFile=/w 100%%/pdfa-out.pdf");
        assert_eq!(args[11], "/w 100%/flip-pdfa-prefix.ps");
        assert_eq!(args[12], "/in put/a b.pdf");
        assert!(args[12].starts_with('/'));
    }

    fn install_tree(root: &Path, relative_profile: Option<&str>) -> PathBuf {
        let cellar = root.join("Cellar").join("ghostscript").join("10.8.0");
        fs::create_dir_all(cellar.join("bin")).unwrap();
        fs::write(cellar.join("bin").join("gs"), b"#!/bin/sh\n").unwrap();
        if let Some(rel) = relative_profile {
            let profile = cellar.join(rel);
            fs::create_dir_all(profile.parent().unwrap()).unwrap();
            fs::write(profile, b"icc").unwrap();
        }
        fs::create_dir_all(root.join("bin")).unwrap();
        symlink(
            "../Cellar/ghostscript/10.8.0/bin/gs",
            root.join("bin").join("gs"),
        )
        .unwrap();
        root.join("bin").join("gs")
    }

    #[test]
    fn srgb_profile_is_found_through_a_symlinked_gs_in_the_flat_layout() {
        let dir = tempfile::tempdir().unwrap();
        let gs = install_tree(dir.path(), Some("share/ghostscript/iccprofiles/srgb.icc"));
        let found = srgb_profile_for(&gs).expect("flat layout");
        let expected = fs::canonicalize(
            dir.path()
                .join("Cellar/ghostscript/10.8.0/share/ghostscript/iccprofiles/srgb.icc"),
        )
        .unwrap();
        assert_eq!(found, expected);
        assert!(found.is_absolute());
    }

    #[test]
    fn srgb_profile_is_found_in_the_versioned_layout_in_sorted_order() {
        let dir = tempfile::tempdir().unwrap();
        let gs = install_tree(
            dir.path(),
            Some("share/ghostscript/10.0.0/iccprofiles/srgb.icc"),
        );
        let other = dir
            .path()
            .join("Cellar/ghostscript/10.8.0/share/ghostscript/9.5.0/iccprofiles");
        fs::create_dir_all(&other).unwrap();
        fs::write(other.join("srgb.icc"), b"icc").unwrap();
        // A sibling without a profile must be skipped, not stop the search.
        fs::create_dir_all(
            dir.path()
                .join("Cellar/ghostscript/10.8.0/share/ghostscript/0.0.1/lib"),
        )
        .unwrap();

        let found = srgb_profile_for(&gs).expect("versioned layout");
        assert!(
            found.ends_with("share/ghostscript/10.0.0/iccprofiles/srgb.icc"),
            "{found:?}"
        );
        assert_eq!(srgb_profile_for(&gs), Some(found), "deterministic");
    }

    #[test]
    fn srgb_profile_prefers_the_flat_layout_and_is_none_when_missing() {
        let dir = tempfile::tempdir().unwrap();
        let gs = install_tree(dir.path(), Some("share/ghostscript/iccprofiles/srgb.icc"));
        let versioned = dir
            .path()
            .join("Cellar/ghostscript/10.8.0/share/ghostscript/10.0.0/iccprofiles");
        fs::create_dir_all(&versioned).unwrap();
        fs::write(versioned.join("srgb.icc"), b"icc").unwrap();
        let found = srgb_profile_for(&gs).unwrap();
        assert!(
            found.ends_with("share/ghostscript/iccprofiles/srgb.icc"),
            "{found:?}"
        );

        let empty = tempfile::tempdir().unwrap();
        let gs_without = install_tree(empty.path(), None);
        assert_eq!(srgb_profile_for(&gs_without), None);

        let wrong_name = tempfile::tempdir().unwrap();
        let gs_wrong = install_tree(
            wrong_name.path(),
            Some("share/ghostscript/iccprofiles/readme.txt"),
        );
        assert_eq!(
            srgb_profile_for(&gs_wrong),
            None,
            "the profile must be a file named srgb.icc"
        );

        assert_eq!(
            srgb_profile_for(Path::new("/definitely/not/here/bin/gs")),
            None
        );
    }

    #[test]
    fn a_directory_named_srgb_icc_is_not_a_profile() {
        let dir = tempfile::tempdir().unwrap();
        let gs = install_tree(dir.path(), None);
        fs::create_dir_all(
            dir.path()
                .join("Cellar/ghostscript/10.8.0/share/ghostscript/iccprofiles/srgb.icc"),
        )
        .unwrap();
        assert_eq!(srgb_profile_for(&gs), None);
    }

    #[test]
    fn the_copy_fallback_never_overwrites_and_removes_the_staged_file() {
        let dir = tempfile::tempdir().unwrap();
        let staged = dir.path().join("staged.pdf");
        let dest = dir.path().join("dest.pdf");
        fs::write(&staged, b"result").unwrap();

        copy_then_delete(&staged, &dest).unwrap();
        assert_eq!(fs::read(&dest).unwrap(), b"result");
        assert!(!staged.exists());

        fs::write(&staged, b"second").unwrap();
        let err = copy_then_delete(&staged, &dest).unwrap_err();
        assert!(matches!(err, EngineError::Io(_)), "{err:?}");
        assert_eq!(
            fs::read(&dest).unwrap(),
            b"result",
            "the existing file is untouched"
        );
        assert!(staged.exists(), "a refused copy keeps the staged file");
    }

    #[test]
    fn the_final_move_refuses_an_output_created_in_the_meantime() {
        let dir = tempfile::tempdir().unwrap();
        let staged = dir.path().join("staged.pdf");
        let dest = dir.path().join("dest.pdf");
        fs::write(&staged, b"result").unwrap();
        fs::write(&dest, b"someone else's file").unwrap();
        let err = move_into_place(&staged, &dest).unwrap_err();
        assert!(err.to_string().contains("already exists"), "{err}");
        assert_eq!(fs::read(&dest).unwrap(), b"someone else's file");
    }

    #[test]
    fn inspection_answers_are_parsed_strictly() {
        let parsed =
            parse_inspection(&serde_json::from_str(VALID_OUTPUT_INSPECTION).unwrap()).unwrap();
        assert_eq!(parsed.pages, 3);
        assert_eq!(parsed.characters, 990);
        assert!(!parsed.locked);
        assert_eq!(parsed.signals.part, Some(2));
        assert_eq!(parsed.signals.conformance, Some('B'));
        assert!(parsed.signals.has_output_intent);

        let lower =
            serde_json::json!({"ok":true,"pages":1,"characters":0,"conformance":"b","part":2});
        let parsed = parse_inspection(&lower).unwrap();
        assert_eq!(parsed.signals.conformance, Some('B'), "upper-cased");
        assert!(!parsed.signals.has_output_intent);

        for bad_conformance in [
            serde_json::json!("BB"),
            serde_json::json!(""),
            serde_json::json!(5),
        ] {
            let value = serde_json::json!({"ok":true,"pages":1,"characters":0,"conformance":bad_conformance});
            assert_eq!(parse_inspection(&value).unwrap().signals.conformance, None);
        }
        let big_part = serde_json::json!({"ok":true,"pages":1,"characters":0,"part":300});
        assert_eq!(parse_inspection(&big_part).unwrap().signals.part, None);

        assert!(parse_inspection(&serde_json::json!({"ok":true,"characters":0})).is_err());
        assert!(parse_inspection(&serde_json::json!({"ok":true,"pages":1})).is_err());
        let failure =
            parse_inspection(&serde_json::json!({"ok":false,"error":"nope"})).unwrap_err();
        assert_eq!(failure, "nope");
    }

    #[test]
    fn bridge_failures_show_their_error_text() {
        assert_eq!(
            bridge_error_text(r#"{"ok":false,"error":"The PDF is protected by a password."}"#),
            "The PDF is protected by a password."
        );
        assert_eq!(bridge_error_text("plain stderr text"), "plain stderr text");
        assert_eq!(tail_chars("  short  ", 300), "short");
        let long = format!("{}END", "a".repeat(400));
        let tail = tail_chars(&long, 300);
        assert!(
            tail.ends_with("END") && tail.chars().count() <= 302,
            "{tail}"
        );
    }
}
