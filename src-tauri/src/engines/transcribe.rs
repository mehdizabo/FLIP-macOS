//! Local speech-to-text: ffmpeg decodes to 16 kHz mono WAV, `whisper-cli` writes the transcript.
//!
//! The engine is pure: the tools and the model arrive as explicit paths and nothing is searched or
//! read from the environment. The whisper.cpp flags in `whisper_args` follow upstream's documented
//! command line, but they have never been run against a real binary in this repository (it is not
//! installed on the build machine), so they are kept in that single function to be corrected easily.

use crate::core::preferences::AppPreferences;
use crate::engines::bridge::{self, BridgeError};
use crate::engines::EngineError;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

/// The only supported model file name.
pub const MODEL_FILE_NAME: &str = "ggml-small.bin";

/// `<app support dir>/models/ggml-small.bin` (`AppPreferences::app_support_dir()`); does not check that the file exists.
pub fn model_path() -> PathBuf {
    AppPreferences::app_support_dir()
        .join("models")
        .join(MODEL_FILE_NAME)
}

/// Explicit paths to the external tools; this engine never searches for them.
pub struct TranscribeTools<'a> {
    /// The `whisper-cli` binary.
    pub whisper_cli: &'a Path,
    /// The multilingual `ggml-small.bin` model file.
    pub model: &'a Path,
    /// The ffmpeg binary used to decode the source to 16 kHz mono WAV.
    pub ffmpeg: &'a Path,
}

/// Safety bound for the ffmpeg decode step; long video files are allowed but never forever.
const FFMPEG_TIMEOUT_SECS: u64 = 2 * 60 * 60;
/// Safety bound for the whole whisper-cli run, as fixed by the design.
const WHISPER_TIMEOUT_SECS: u64 = 4 * 60 * 60;

/// Pause between checks on the running whisper-cli child (also the cancel reaction time).
const POLL_INTERVAL: Duration = Duration::from_millis(50);

/// Decode `source` with ffmpeg, run whisper-cli, write the transcript to `out_txt`. Errors: `EngineError`.
pub fn transcribe(
    source: &Path,
    out_txt: &Path,
    tools: &TranscribeTools<'_>,
    work_dir: &Path,
    cancel: &Arc<AtomicBool>,
    unit_progress: &dyn Fn(u8),
) -> Result<(), EngineError> {
    let wav = work_dir.join("transcribe-input.wav");
    let out_prefix = work_dir.join("transcript");
    let transcript_file = work_dir.join("transcript.txt");
    let out_existed = out_txt.exists();

    // A stale transcript from an earlier run must never be mistaken for this run's output.
    let _ = std::fs::remove_file(&wav);
    let _ = std::fs::remove_file(&transcript_file);

    let result: Result<(), EngineError> = (|| {
        decode_to_wav(source, &wav, tools.ffmpeg, cancel, unit_progress)?;
        run_whisper(&out_prefix, &wav, tools, cancel, unit_progress)?;

        let raw = std::fs::read_to_string(&transcript_file).map_err(|e| {
            EngineError::ConversionFailed(format!(
                "whisper-cli produced no transcript file `{}`: {e}",
                transcript_file.display()
            ))
        })?;
        let text = clean_transcript(&raw);
        if text.is_empty() {
            return Err(EngineError::ConversionFailed(
                "No speech was recognised in this file. Nothing was written.".to_string(),
            ));
        }

        let mut output = text;
        output.push('\n');
        std::fs::write(out_txt, output.as_bytes())?;
        unit_progress(100);
        Ok(())
    })();

    let _ = std::fs::remove_file(&wav);
    let _ = std::fs::remove_file(&transcript_file);
    if result.is_err() && !out_existed {
        let _ = std::fs::remove_file(out_txt);
    }
    result
}

fn decode_to_wav(
    source: &Path,
    wav: &Path,
    ffmpeg: &Path,
    cancel: &Arc<AtomicBool>,
    unit_progress: &dyn Fn(u8),
) -> Result<(), EngineError> {
    unit_progress(5);
    let args = vec![
        "-hide_banner".to_string(),
        "-loglevel".to_string(),
        "error".to_string(),
        "-nostdin".to_string(),
        "-y".to_string(),
        "-i".to_string(),
        source.to_string_lossy().into_owned(),
        "-vn".to_string(),
        "-ac".to_string(),
        "1".to_string(),
        "-ar".to_string(),
        "16000".to_string(),
        "-c:a".to_string(),
        "pcm_s16le".to_string(),
        wav.to_string_lossy().into_owned(),
    ];
    bridge::run_external_command(ffmpeg, &args, None, FFMPEG_TIMEOUT_SECS, Some(cancel))
        .map(|_| ())
        .map_err(ffmpeg_error)?;
    unit_progress(30);
    Ok(())
}

fn ffmpeg_error(err: BridgeError) -> EngineError {
    match err {
        BridgeError::Cancelled => EngineError::Cancelled,
        BridgeError::CommandFailed(label, detail) => EngineError::ConversionFailed(format!(
            "ffmpeg ({label}) failed: {}",
            last_non_empty_line(&detail)
        )),
        BridgeError::TimedOut(label, secs) => EngineError::ConversionFailed(format!(
            "ffmpeg ({label}) exceeded the {secs} second safety timeout"
        )),
        other => EngineError::Bridge(other),
    }
}

/// The whisper.cpp command line, kept in one place so the flags can be corrected together.
fn whisper_args(model: &Path, wav: &Path, out_prefix: &Path) -> Vec<String> {
    vec![
        "-m".to_string(),
        model.to_string_lossy().into_owned(),
        "-f".to_string(),
        wav.to_string_lossy().into_owned(),
        "-l".to_string(),
        "auto".to_string(),
        "-otxt".to_string(),
        "-of".to_string(),
        out_prefix.to_string_lossy().into_owned(),
        // `-pp` makes whisper.cpp print its `progress = NN%` lines.
        "-pp".to_string(),
    ]
}

fn run_whisper(
    out_prefix: &Path,
    wav: &Path,
    tools: &TranscribeTools<'_>,
    cancel: &Arc<AtomicBool>,
    unit_progress: &dyn Fn(u8),
) -> Result<(), EngineError> {
    let args = whisper_args(tools.model, wav, out_prefix);
    let mut child = Command::new(tools.whisper_cli)
        .args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    // Progress lines are read from stderr; stdout is drained in its own thread so a full pipe can
    // never block the child, and its lines are scanned too in case the tool writes them there.
    let (tx, rx) = mpsc::channel::<u8>();
    let stdout = child.stdout.take();
    let stdout_tx = tx.clone();
    let stdout_handle = std::thread::spawn(move || {
        if let Some(out) = stdout {
            let reader = BufReader::new(out);
            for line in reader.lines().map_while(Result::ok) {
                if let Some(pct) = parse_progress_line(&line) {
                    let _ = stdout_tx.send(pct);
                }
            }
        }
    });

    let last_stderr_line = Arc::new(Mutex::new(String::new()));
    let stderr_state = last_stderr_line.clone();
    let stderr = child.stderr.take();
    let stderr_handle = std::thread::spawn(move || {
        if let Some(err) = stderr {
            let reader = BufReader::new(err);
            for line in reader.lines().map_while(Result::ok) {
                if let Some(pct) = parse_progress_line(&line) {
                    let _ = tx.send(pct);
                }
                let trimmed = line.trim();
                if !trimmed.is_empty() {
                    if let Ok(mut slot) = stderr_state.lock() {
                        *slot = trimmed.chars().take(300).collect();
                    }
                }
            }
        }
    });

    let mut last_pct = 30u8;
    let mut emit = |pct: u8| {
        let mapped = 30 + (u16::from(pct) * 65 / 100) as u8;
        if mapped > last_pct {
            last_pct = mapped;
            unit_progress(mapped);
        }
    };

    let start = Instant::now();
    let status = loop {
        for pct in rx.try_iter() {
            emit(pct);
        }
        if cancel.load(Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            let _ = stdout_handle.join();
            let _ = stderr_handle.join();
            return Err(EngineError::Cancelled);
        }
        if let Some(st) = child.try_wait()? {
            break st;
        }
        if start.elapsed() >= Duration::from_secs(WHISPER_TIMEOUT_SECS) {
            let _ = child.kill();
            let _ = child.wait();
            let _ = stdout_handle.join();
            let _ = stderr_handle.join();
            let hours = WHISPER_TIMEOUT_SECS / 3600;
            return Err(EngineError::ConversionFailed(format!(
                "whisper-cli exceeded the {hours}-hour safety timeout"
            )));
        }
        std::thread::sleep(POLL_INTERVAL);
    };

    let _ = stdout_handle.join();
    for pct in rx.try_iter() {
        emit(pct);
    }
    let _ = stderr_handle.join();

    if !status.success() {
        let detail = last_stderr_line
            .lock()
            .map(|slot| slot.clone())
            .unwrap_or_default();
        let detail = if detail.is_empty() {
            "no error output".to_string()
        } else {
            detail
        };
        return Err(EngineError::ConversionFailed(format!(
            "whisper-cli failed ({}): {detail}",
            status.code().unwrap_or(-1)
        )));
    }
    Ok(())
}

/// Number before `%` on a whisper.cpp `progress = NN%` line.
fn parse_progress_line(line: &str) -> Option<u8> {
    if !line.to_ascii_lowercase().contains("progress") {
        return None;
    }
    let before_pct = &line[..line.find('%')?];
    let digits: String = before_pct
        .chars()
        .rev()
        .skip_while(|c| c.is_whitespace())
        .take_while(char::is_ascii_digit)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    digits.parse::<u8>().ok()
}

/// Trimmed transcript text without the lines that hold only a whisper placeholder tag.
fn clean_transcript(raw: &str) -> String {
    raw.lines()
        .filter(|line| {
            let trimmed = line.trim();
            !trimmed.is_empty() && !is_placeholder_line(trimmed)
        })
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string()
}

fn is_placeholder_line(line: &str) -> bool {
    line.len() > 2
        && ((line.starts_with('[') && line.ends_with(']'))
            || (line.starts_with('(') && line.ends_with(')')))
}

fn last_non_empty_line(text: &str) -> String {
    let line = text
        .lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .trim();
    line.chars().take(300).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn fake_tool(dir: &Path, name: &str, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join(name);
        fs::write(&path, body).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    fn fake_ffmpeg(args_log: Option<&Path>) -> String {
        let log = args_log
            .map(|p| format!("printf '%s\\n' \"$@\" > '{}'\n", p.display()))
            .unwrap_or_default();
        format!("#!/bin/sh\n{log}for last do :; done\nprintf 'RIFFfake' > \"$last\"\n")
    }

    fn fake_whisper_writing(content_file: &Path) -> String {
        format!(
            "#!/bin/sh\nprefix=\"\"\nwhile [ $# -gt 0 ]; do\n  case \"$1\" in\n    -of) prefix=\"$2\"; shift 2 ;;\n    *) shift ;;\n  esac\ndone\ncat '{}' > \"$prefix.txt\"\n",
            content_file.display()
        )
    }

    struct Env {
        _dir: tempfile::TempDir,
        bin: PathBuf,
        work: PathBuf,
        model: PathBuf,
        source: PathBuf,
        out: PathBuf,
    }

    fn env() -> Env {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("bin");
        let work = dir.path().join("work");
        fs::create_dir(&bin).unwrap();
        fs::create_dir(&work).unwrap();
        let model = dir.path().join(MODEL_FILE_NAME);
        fs::write(&model, b"fake-model").unwrap();
        let source = dir.path().join("audio.mp3");
        fs::write(&source, b"fake audio").unwrap();
        let out = dir.path().join("audio.txt");
        Env {
            _dir: dir,
            bin,
            work,
            model,
            source,
            out,
        }
    }

    fn no_cancel() -> Arc<AtomicBool> {
        Arc::new(AtomicBool::new(false))
    }

    #[test]
    fn transcribes_with_fake_tools_and_cleans_temporary_files() {
        let e = env();
        let args_log = e.work.parent().unwrap().join("ffmpeg-args.txt");
        let ffmpeg = fake_tool(&e.bin, "ffmpeg", &fake_ffmpeg(Some(&args_log)));
        let content = e.work.parent().unwrap().join("whisper-out.txt");
        fs::write(&content, "Ciao, questo è un test.\n").unwrap();
        let whisper = fake_tool(&e.bin, "whisper-cli", &fake_whisper_writing(&content));

        let tools = TranscribeTools {
            whisper_cli: &whisper,
            model: &e.model,
            ffmpeg: &ffmpeg,
        };
        transcribe(&e.source, &e.out, &tools, &e.work, &no_cancel(), &|_| {}).unwrap();

        assert_eq!(
            fs::read_to_string(&e.out).unwrap(),
            "Ciao, questo è un test.\n"
        );
        assert!(!e.work.join("transcribe-input.wav").exists());
        assert!(!e.work.join("transcript.txt").exists());

        let logged = fs::read_to_string(&args_log).unwrap();
        assert!(logged
            .lines()
            .any(|line| line == e.source.to_string_lossy()));
    }

    #[test]
    fn reports_monotonic_progress_from_whisper_output() {
        let e = env();
        let ffmpeg = fake_tool(&e.bin, "ffmpeg", &fake_ffmpeg(None));
        let whisper = fake_tool(
            &e.bin,
            "whisper-cli",
            r#"#!/bin/sh
prefix=""
while [ $# -gt 0 ]; do
  case "$1" in
    -of) prefix="$2"; shift 2 ;;
    *) shift ;;
  esac
done
printf 'progress =   0%%\n' >&2
printf 'progress =  25%%\n' >&2
printf 'progress =  50%%\n' >&2
printf 'progress =  75%%\n' >&2
printf 'progress = 100%%\n' >&2
printf 'Testo.\n' > "$prefix.txt"
"#,
        );

        let tools = TranscribeTools {
            whisper_cli: &whisper,
            model: &e.model,
            ffmpeg: &ffmpeg,
        };
        let seen = std::cell::RefCell::new(Vec::<u8>::new());
        transcribe(&e.source, &e.out, &tools, &e.work, &no_cancel(), &|pct| {
            seen.borrow_mut().push(pct)
        })
        .unwrap();

        let seen = seen.into_inner();
        assert!(seen.windows(2).all(|w| w[0] <= w[1]), "{seen:?}");
        assert_eq!(seen.first(), Some(&5));
        assert_eq!(seen.last(), Some(&100));
        let before_final = &seen[..seen.len() - 1];
        assert!(
            before_final.iter().any(|&v| v >= 95),
            "progress is not live: {seen:?}"
        );
    }

    #[test]
    fn cancel_kills_whisper_quickly_without_leaving_a_child() {
        let e = env();
        let ffmpeg = fake_tool(&e.bin, "ffmpeg", &fake_ffmpeg(None));
        let pid_file = e.work.parent().unwrap().join("whisper.pid");
        let whisper = fake_tool(
            &e.bin,
            "whisper-cli",
            &format!(
                "#!/bin/sh\necho $$ > '{}'\nexec sleep 60\n",
                pid_file.display()
            ),
        );

        let tools = TranscribeTools {
            whisper_cli: &whisper,
            model: &e.model,
            ffmpeg: &ffmpeg,
        };
        let cancel = no_cancel();
        // Cancel once whisper-cli is really running, so the test can never hit the ffmpeg step.
        let flag = cancel.clone();
        let cancel_at = Arc::new(Mutex::new(None::<Instant>));
        let cancel_at_state = cancel_at.clone();
        let pid_wait = pid_file.clone();
        let setter = std::thread::spawn(move || {
            let deadline = Instant::now();
            while !pid_wait.exists() && deadline.elapsed() < Duration::from_secs(10) {
                std::thread::sleep(Duration::from_millis(5));
            }
            if let Ok(mut slot) = cancel_at_state.lock() {
                *slot = Some(Instant::now());
            }
            flag.store(true, Ordering::Relaxed);
        });

        let err = transcribe(&e.source, &e.out, &tools, &e.work, &cancel, &|_| {})
            .expect_err("cancelled run fails");
        setter.join().unwrap();

        assert!(matches!(err, EngineError::Cancelled), "{err:?}");
        let cancelled_at = cancel_at.lock().unwrap().expect("cancel was requested");
        assert!(
            cancelled_at.elapsed() < Duration::from_secs(5),
            "cancel latency: {:?}",
            cancelled_at.elapsed()
        );
        assert!(!e.out.exists());
        assert!(!e.work.join("transcribe-input.wav").exists());
        assert!(!e.work.join("transcript.txt").exists());

        let pid = fs::read_to_string(&pid_file).unwrap().trim().to_string();
        let alive = Command::new("/bin/kill")
            .args(["-0", &pid])
            .status()
            .unwrap();
        assert!(!alive.success(), "whisper child {pid} still exists");
    }

    #[test]
    fn empty_transcript_is_an_error_and_leaves_no_output() {
        let e = env();
        let ffmpeg = fake_tool(&e.bin, "ffmpeg", &fake_ffmpeg(None));
        let content = e.work.parent().unwrap().join("empty.txt");
        fs::write(&content, "").unwrap();
        let whisper = fake_tool(&e.bin, "whisper-cli", &fake_whisper_writing(&content));

        let tools = TranscribeTools {
            whisper_cli: &whisper,
            model: &e.model,
            ffmpeg: &ffmpeg,
        };
        let err = transcribe(&e.source, &e.out, &tools, &e.work, &no_cancel(), &|_| {})
            .expect_err("empty transcript fails");

        assert!(matches!(err, EngineError::ConversionFailed(_)), "{err:?}");
        assert_eq!(
            err.to_string(),
            "No speech was recognised in this file. Nothing was written."
        );
        assert!(!e.out.exists());
        assert!(!e.work.join("transcribe-input.wav").exists());
        assert!(!e.work.join("transcript.txt").exists());
    }

    #[test]
    fn blank_audio_only_transcript_is_an_error() {
        let e = env();
        let ffmpeg = fake_tool(&e.bin, "ffmpeg", &fake_ffmpeg(None));
        let content = e.work.parent().unwrap().join("blank.txt");
        fs::write(&content, "[BLANK_AUDIO]\n").unwrap();
        let whisper = fake_tool(&e.bin, "whisper-cli", &fake_whisper_writing(&content));

        let tools = TranscribeTools {
            whisper_cli: &whisper,
            model: &e.model,
            ffmpeg: &ffmpeg,
        };
        let err = transcribe(&e.source, &e.out, &tools, &e.work, &no_cancel(), &|_| {})
            .expect_err("blank audio fails");

        assert!(matches!(err, EngineError::ConversionFailed(_)), "{err:?}");
        assert!(err.to_string().contains("No speech was recognised"));
        assert!(!e.out.exists());
    }

    #[test]
    fn whisper_failure_names_the_tool_and_quotes_stderr() {
        let e = env();
        let ffmpeg = fake_tool(&e.bin, "ffmpeg", &fake_ffmpeg(None));
        let whisper = fake_tool(
            &e.bin,
            "whisper-cli",
            "#!/bin/sh\nprintf 'loading model\\n' >&2\nprintf 'boom: model not found\\n' >&2\nexit 1\n",
        );

        let tools = TranscribeTools {
            whisper_cli: &whisper,
            model: &e.model,
            ffmpeg: &ffmpeg,
        };
        let err = transcribe(&e.source, &e.out, &tools, &e.work, &no_cancel(), &|_| {})
            .expect_err("whisper failure fails");

        assert!(matches!(err, EngineError::ConversionFailed(_)), "{err:?}");
        let message = err.to_string();
        assert!(message.contains("whisper"), "{message}");
        assert!(message.contains("boom: model not found"), "{message}");
        assert!(!e.out.exists());
        assert!(!e.work.join("transcribe-input.wav").exists());
    }

    #[test]
    fn ffmpeg_failure_names_the_tool_and_quotes_stderr() {
        let e = env();
        let ffmpeg = fake_tool(
            &e.bin,
            "ffmpeg",
            "#!/bin/sh\nprintf 'boom: no such input\\n' >&2\nexit 1\n",
        );
        let whisper = fake_tool(&e.bin, "whisper-cli", "#!/bin/sh\nexit 0\n");

        let tools = TranscribeTools {
            whisper_cli: &whisper,
            model: &e.model,
            ffmpeg: &ffmpeg,
        };
        let err = transcribe(&e.source, &e.out, &tools, &e.work, &no_cancel(), &|_| {})
            .expect_err("ffmpeg failure fails");

        assert!(matches!(err, EngineError::ConversionFailed(_)), "{err:?}");
        let message = err.to_string();
        assert!(message.contains("ffmpeg"), "{message}");
        assert!(message.contains("boom: no such input"), "{message}");
        assert!(!e.out.exists());
    }

    #[test]
    fn handles_accents_and_spaces_in_paths() {
        let e = env();
        let args_log = e.work.parent().unwrap().join("ffmpeg-args.txt");
        let ffmpeg = fake_tool(&e.bin, "ffmpeg", &fake_ffmpeg(Some(&args_log)));
        let content = e.work.parent().unwrap().join("accented.txt");
        fs::write(&content, "Perché no? È così.\n").unwrap();
        let whisper = fake_tool(&e.bin, "whisper-cli", &fake_whisper_writing(&content));

        let source = e.work.parent().unwrap().join("Il mio audio àèìòù.mp3");
        fs::write(&source, b"fake audio").unwrap();
        let out = e.work.parent().unwrap().join("Il mio audio àèìòù.txt");

        let tools = TranscribeTools {
            whisper_cli: &whisper,
            model: &e.model,
            ffmpeg: &ffmpeg,
        };
        transcribe(&source, &out, &tools, &e.work, &no_cancel(), &|_| {}).unwrap();

        assert_eq!(fs::read_to_string(&out).unwrap(), "Perché no? È così.\n");
        let logged = fs::read_to_string(&args_log).unwrap();
        assert!(logged.lines().any(|line| line == source.to_string_lossy()));
    }

    #[test]
    fn whisper_arguments_and_model_path_follow_the_contract() {
        let model = Path::new("/models/ggml-small.bin");
        let wav = Path::new("/work/transcribe-input.wav");
        let prefix = Path::new("/work/transcript");
        let args = whisper_args(model, wav, prefix);

        let model_s = model.to_string_lossy().into_owned();
        let wav_s = wav.to_string_lossy().into_owned();
        let prefix_s = prefix.to_string_lossy().into_owned();
        assert!(args.windows(2).any(|w| w[0] == "-m" && w[1] == model_s));
        assert!(args.windows(2).any(|w| w[0] == "-f" && w[1] == wav_s));
        assert!(args.windows(2).any(|w| w[0] == "-l" && w[1] == "auto"));
        assert!(args.iter().any(|a| a == "-otxt"));
        assert!(args.windows(2).any(|w| w[0] == "-of" && w[1] == prefix_s));
        assert!(args.iter().any(|a| a == "-pp"));

        let path = model_path();
        assert!(path.ends_with(Path::new("Library/Application Support/FLIP/models/ggml-small.bin")));
        assert_eq!(path.file_name().and_then(|n| n.to_str()), Some(MODEL_FILE_NAME));
    }
}
