use crate::core::policy::ext_lower;
use crate::engines::{bridge, DetectedTools, EngineError};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub fn convert(
    source: &Path,
    format: &str,
    output: &Path,
    tools: &DetectedTools,
    cancel: &Arc<AtomicBool>,
    unit_progress: &dyn Fn(u8),
) -> Result<(), EngineError> {
    let ext = ext_lower(source);
    let fmt = format.trim().to_ascii_uppercase();

    // Fast native VideoToolbox/AVFoundation/ExtAudioFile path when FFmpeg is absent
    let source_is_native_av = matches!(
        ext.as_str(),
        "mp4"
            | "mov"
            | "m4v"
            | "m4a"
            | "wav"
            | "aiff"
            | "aif"
            | "aac"
            | "caf"
            | "alac"
            | "flac"
            | "mp3"
    );

    if tools.ffmpeg.is_none() {
        if !source_is_native_av {
            return Err(EngineError::Unsupported(format!(
                "FFmpeg is required for .{ext} files. Install via `brew install ffmpeg`."
            )));
        }
        if matches!(fmt.as_str(), "M4A" | "MP4" | "MOV" | "M4V" | "GIF") {
            if fmt == "GIF" && !matches!(ext.as_str(), "mp4" | "mov" | "m4v") {
                return Err(EngineError::Unsupported(format!(
                    "Cannot convert audio .{ext} to animated GIF"
                )));
            }
            unit_progress(30);
            bridge::convert_av(source, &fmt, output, Some(cancel))?;
            unit_progress(100);
            return Ok(());
        }
        if matches!(fmt.as_str(), "WAV" | "AIFF" | "ALAC" | "CAF" | "FLAC") {
            return convert_via_native_audio(source, &fmt, output, cancel, unit_progress);
        }
        return Err(EngineError::Unsupported(format!(
            "FFmpeg is required to convert to {fmt}. Install via `brew install ffmpeg`."
        )));
    }

    let ffmpeg = tools.ffmpeg.as_ref().unwrap();
    let total_secs = probe_duration_secs(tools.ffprobe.as_deref(), source).unwrap_or(0.0);

    let mut args = vec![
        "-y".to_string(),
        "-nostdin".to_string(),
        "-hide_banner".to_string(),
        "-i".to_string(),
        source.to_string_lossy().into_owned(),
    ];

    match fmt.as_str() {
        "WAV" => args.extend([
            "-vn".into(),
            "-acodec".into(),
            "pcm_s16le".into(),
            "-ar".into(),
            "44100".into(),
            "-ac".into(),
            "2".into(),
        ]),
        "AIFF" => args.extend([
            "-vn".into(),
            "-acodec".into(),
            "pcm_s16be".into(),
            "-ar".into(),
            "44100".into(),
            "-ac".into(),
            "2".into(),
        ]),
        "ALAC" => args.extend(["-vn".into(), "-acodec".into(), "alac".into(), "-f".into(), "ipod".into()]),
        "CAF" => args.extend([
            "-vn".into(),
            "-acodec".into(),
            "pcm_s16le".into(),
            "-ar".into(),
            "44100".into(),
            "-ac".into(),
            "2".into(),
            "-f".into(),
            "caf".into(),
        ]),
        "MP3" => args.extend(["-vn".into(), "-acodec".into(), "libmp3lame".into(), "-q:a".into(), "2".into()]),
        "FLAC" => args.extend(["-vn".into(), "-acodec".into(), "flac".into()]),
        "OGG" => {
            if ffmpeg_has_encoder(ffmpeg, "libvorbis") {
                args.extend(["-vn".into(), "-acodec".into(), "libvorbis".into(), "-q:a".into(), "4".into()]);
            } else {
                args.extend(["-vn".into(), "-acodec".into(), "libopus".into(), "-b:a".into(), "128k".into()]);
            }
        }
        "OPUS" => args.extend(["-vn".into(), "-acodec".into(), "libopus".into(), "-b:a".into(), "128k".into()]),
        "AAC" => args.extend(["-vn".into(), "-acodec".into(), "aac".into(), "-b:a".into(), "192k".into()]),
        "M4A" => args.extend([
            "-vn".into(),
            "-acodec".into(),
            "aac".into(),
            "-b:a".into(),
            "192k".into(),
            "-f".into(),
            "ipod".into(),
        ]),
        "MP4" => args.extend([
            "-vf".into(),
            "scale=trunc(iw/2)*2:trunc(ih/2)*2".into(),
            "-c:v".into(),
            "libx264".into(),
            "-preset".into(),
            "fast".into(),
            "-crf".into(),
            "23".into(),
            "-pix_fmt".into(),
            "yuv420p".into(),
            "-c:a".into(),
            "aac".into(),
            "-b:a".into(),
            "160k".into(),
            "-movflags".into(),
            "+faststart".into(),
        ]),
        "MOV" => args.extend([
            "-vf".into(),
            "scale=trunc(iw/2)*2:trunc(ih/2)*2".into(),
            "-c:v".into(),
            "libx264".into(),
            "-preset".into(),
            "fast".into(),
            "-crf".into(),
            "23".into(),
            "-pix_fmt".into(),
            "yuv420p".into(),
            "-c:a".into(),
            "aac".into(),
            "-movflags".into(),
            "+faststart".into(),
        ]),
        "M4V" => args.extend([
            "-vf".into(),
            "scale=trunc(iw/2)*2:trunc(ih/2)*2".into(),
            "-c:v".into(),
            "libx264".into(),
            "-preset".into(),
            "fast".into(),
            "-crf".into(),
            "23".into(),
            "-pix_fmt".into(),
            "yuv420p".into(),
            "-c:a".into(),
            "aac".into(),
            "-f".into(),
            "ipod".into(),
        ]),
        "MKV" => args.extend([
            "-vf".into(),
            "scale=trunc(iw/2)*2:trunc(ih/2)*2".into(),
            "-c:v".into(),
            "libx264".into(),
            "-preset".into(),
            "fast".into(),
            "-crf".into(),
            "23".into(),
            "-c:a".into(),
            "aac".into(),
            "-b:a".into(),
            "160k".into(),
        ]),
        "WEBM" => args.extend([
            "-c:v".into(),
            "libvpx-vp9".into(),
            "-crf".into(),
            "32".into(),
            "-b:v".into(),
            "0".into(),
            "-deadline".into(),
            "realtime".into(),
            "-cpu-used".into(),
            "4".into(),
            "-c:a".into(),
            "libopus".into(),
            "-b:a".into(),
            "128k".into(),
        ]),
        "AVI" => args.extend([
            "-c:v".into(),
            "mpeg4".into(),
            "-q:v".into(),
            "5".into(),
            "-c:a".into(),
            "libmp3lame".into(),
            "-q:a".into(),
            "4".into(),
        ]),
        "GIF" => args.extend([
            "-t".into(),
            "15".into(),
            "-vf".into(),
            "fps=12,scale='min(480,iw)':-1:flags=lanczos,split[s0][s1];[s0]palettegen[p];[s1][p]paletteuse".into(),
            "-loop".into(),
            "0".into(),
        ]),
        _ => {
            return Err(EngineError::Unsupported(format!(
                "Unsupported media target format: {fmt}"
            )))
        }
    }

    args.extend(["-progress".into(), "pipe:1".into(), "-nostats".into()]);
    args.push(output.to_string_lossy().into_owned());

    run_ffmpeg_with_progress(ffmpeg, &args, total_secs, cancel, unit_progress)
}

fn convert_via_native_audio(
    source: &Path,
    fmt: &str,
    output: &Path,
    cancel: &Arc<AtomicBool>,
    unit_progress: &dyn Fn(u8),
) -> Result<(), EngineError> {
    let ext = ext_lower(source);
    let mut temp_m4a: Option<PathBuf> = None;
    let audio_input = if matches!(ext.as_str(), "mp4" | "mov" | "m4v") {
        let m4a = output.with_extension("m4a.tmp");
        unit_progress(25);
        bridge::convert_av(source, "M4A", &m4a, Some(cancel))?;
        temp_m4a = Some(m4a.clone());
        m4a
    } else {
        source.to_path_buf()
    };

    unit_progress(55);
    let res: Result<(), EngineError> = if fmt == "FLAC" {
        // Use Swift bridge ExtAudioFile encoder directly so <4608-frame clips select a smaller FLAC block size cleanly.
        bridge::convert_av(&audio_input, "FLAC", output, Some(cancel))
            .map(|_| ())
            .map_err(EngineError::from)
    } else {
        let afconvert = Path::new("/usr/bin/afconvert");
        if afconvert.is_file() {
            let mut args = match fmt {
                "WAV" => vec!["-f".into(), "WAVE".into(), "-d".into(), "LEI16@44100".into()],
                "AIFF" => vec!["-f".into(), "AIFF".into(), "-d".into(), "BEI16@44100".into()],
                "ALAC" => vec!["-f".into(), "m4af".into(), "-d".into(), "alac".into()],
                "CAF" => vec!["-f".into(), "caff".into(), "-d".into(), "LEI16".into()],
                _ => {
                    if let Some(tmp) = temp_m4a {
                        let _ = std::fs::remove_file(tmp);
                    }
                    return Err(EngineError::Unsupported(format!(
                        "Native audio unsupported: {fmt}"
                    )));
                }
            };
            args.push(audio_input.to_string_lossy().into_owned());
            args.push(output.to_string_lossy().into_owned());
            bridge::run_external_command(afconvert, &args, None, 120, Some(cancel))
                .map(|_| ())
                .map_err(EngineError::from)
        } else if fmt == "CAF" {
            bridge::convert_av(&audio_input, "CAF", output, Some(cancel))
                .map(|_| ())
                .map_err(EngineError::from)
        } else {
            Err(EngineError::Unsupported(
                "afconvert not found and FFmpeg is not installed".to_string(),
            ))
        }
    };

    if let Some(tmp) = temp_m4a {
        let _ = std::fs::remove_file(tmp);
    }
    res?;
    unit_progress(100);
    Ok(())
}

fn ffmpeg_has_encoder(ffmpeg: &Path, encoder: &str) -> bool {
    let Ok((out, _)) = bridge::run_external_command(
        ffmpeg,
        &["-hide_banner".to_string(), "-encoders".to_string()],
        None,
        5,
        None,
    ) else {
        return false;
    };
    out.lines().any(|line| {
        line.split_whitespace()
            .nth(1)
            .is_some_and(|name| name == encoder)
    })
}

fn probe_duration_secs(ffprobe: Option<&Path>, source: &Path) -> Option<f64> {
    let probe = ffprobe?;
    let args = vec![
        "-v".to_string(),
        "error".to_string(),
        "-show_entries".to_string(),
        "format=duration".to_string(),
        "-of".to_string(),
        "default=noprint_wrappers=1:nokey=1".to_string(),
        source.to_string_lossy().into_owned(),
    ];
    let (out, _) = bridge::run_external_command(probe, &args, None, 10, None).ok()?;
    out.trim().parse::<f64>().ok().filter(|d| *d > 0.0)
}

fn run_ffmpeg_with_progress(
    ffmpeg: &Path,
    args: &[String],
    total_secs: f64,
    cancel: &Arc<AtomicBool>,
    unit_progress: &dyn Fn(u8),
) -> Result<(), EngineError> {
    let mut child = Command::new(ffmpeg)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    unit_progress(5);
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();

    let err_handle = std::thread::spawn(move || {
        use std::io::Read;
        let mut s = String::new();
        if let Some(mut r) = stderr {
            let _ = r.read_to_string(&mut s);
        }
        s
    });

    let (tx, rx) = std::sync::mpsc::channel::<u8>();
    let out_handle = std::thread::spawn(move || {
        if let Some(out) = stdout {
            let reader = BufReader::new(out);
            for line in reader.lines().map_while(Result::ok) {
                if total_secs > 0.0 {
                    if let Some(us_str) = line.strip_prefix("out_time_us=") {
                        if let Ok(us) = us_str.trim().parse::<f64>() {
                            let pct = ((us / 1_000_000.0) / total_secs * 100.0)
                                .round()
                                .clamp(5.0, 98.0) as u8;
                            let _ = tx.send(pct);
                        }
                    }
                }
            }
        }
    });

    let start = Instant::now();
    let timeout = Duration::from_secs(600);
    let status = loop {
        for pct in rx.try_iter() {
            unit_progress(pct);
        }
        if cancel.load(Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            let _ = out_handle.join();
            let _ = err_handle.join();
            return Err(EngineError::Cancelled);
        }
        if let Some(st) = child.try_wait()? {
            break st;
        }
        if start.elapsed() >= timeout {
            let _ = child.kill();
            let _ = child.wait();
            let _ = out_handle.join();
            let _ = err_handle.join();
            return Err(EngineError::ConversionFailed(
                "FFmpeg exceeded 10-minute safety timeout".to_string(),
            ));
        }
        std::thread::sleep(Duration::from_millis(25));
    };

    let _ = out_handle.join();
    for pct in rx.try_iter() {
        unit_progress(pct);
    }
    let err_out = err_handle.join().unwrap_or_default();
    if !status.success() {
        let tail: String = err_out.chars().rev().take(600).collect::<Vec<_>>().into_iter().rev().collect();
        return Err(EngineError::ConversionFailed(format!(
            "FFmpeg failed ({}): {}",
            status.code().unwrap_or(-1),
            tail.trim()
        )));
    }
    unit_progress(100);
    Ok(())
}
