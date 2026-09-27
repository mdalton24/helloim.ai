//! Native microphone capture for push-to-talk, Linux only.
//!
//! **WHY THIS EXISTS AND WHY IT IS NOT IN `ui/index.html` ANY MORE.** The
//! first version of Linux voice input captured inside the webview:
//! `getUserMedia()` into a `ScriptProcessorNode`, raw PCM hand-encoded to a
//! WAV, handed to `stt::stt_transcribe`. It worked, repeatedly, in this
//! developer's own testing. **It did not survive Beck's whole-system
//! verification.** Her rig — a real fresh VM, a synthetic PulseAudio
//! microphone (`module-null-sink` + `module-remap-source`), an 11-second
//! known clip played while the push-to-talk chord was held — found:
//!
//! - An 11-second hold arriving at whisper as **0.6-0.7 seconds** of audio
//!   (`processing 'speech.wav' (8916 samples, 0.6 sec)` in whisper's own
//!   log), so no transcript and no reply, while whisper itself answered a
//!   direct full-file call correctly and the brain was healthy — the defect
//!   was specifically in what the webview handed whisper.
//! - The WebKitWebProcess leaking file descriptors **without bound** during
//!   listening — past the 1024 soft limit, still climbing past 65536 with it
//!   raised, VM load average around 45.
//! - `cannot create wakeup pipe` and poll assertions in the GStreamer log —
//!   GLib's own internal wakeup-pipe creation failing, the textbook shape of
//!   a process that has run out of file descriptors.
//!
//! **Re-run on a second fresh VM, same rig, twice (a single 11-second hold,
//! and five rapid presses back to back) — neither reproduced Beck's
//! numbers.** That is not the same as "fine". WebKitGTK's Web Audio
//! implementation is GStreamer underneath, `getUserMedia`'s own capture
//! pipeline plus a `ScriptProcessorNode` pulling samples out of it roughly
//! every 93ms is real, ongoing traffic across a boundary this crate does not
//! control on either side, and Beck's measurements are exactly the kind of
//! bug that is real, serious, and inconsistent to trigger — which is worse
//! than a bug that fails the same way every time, because it is the one that
//! still ships. Per the brief that authorized this file: rather than keep
//! hunting for the exact trigger inside a webview audio stack this crate
//! does not own, capture moved entirely out of it.
//!
//! **WHAT THIS FILE ACTUALLY DOES.** Opens the system's default audio input
//! device through `cpal` — which on this box's Linux talks to ALSA, and on
//! an ordinary desktop ALSA's own `default` device is routed through
//! PulseAudio's ALSA plugin, so `pactl set-default-source` (exactly what
//! Beck's synthetic-mic rig and this module's own test both use) is honoured
//! without this file doing anything PulseAudio-specific itself. Buffers raw
//! samples in memory for as long as a capture is held, and on stop:
//! downmixes to mono, resamples to 16kHz (same averaging-window algorithm as
//! the WAV path this replaces — ported, not reinvented, so the accuracy
//! characteristics whisper.cpp was already tuned against do not change),
//! encodes a WAV, and hands it straight to `stt::LocalWhisperProvider` —
//! the same provider `stt.rs`'s own (still-registered, no longer called from
//! the front end) `stt_transcribe` command already used. Nothing in this
//! file talks to WebKitGTK, GStreamer, or the webview's own audio stack at
//! all — that is the entire point.
//!
//! **ONE CAPTURE AT A TIME, ENFORCED.** `native_capture_start` refuses if a
//! capture is already running rather than stacking a second one — see
//! `CAPTURE`'s own doc. `native_capture_stop_and_transcribe` is the only way
//! to end one, and it always clears the slot first, even before the
//! transcription network call, so a slow or failed transcribe can never
//! leave the mic looking held open to a second call racing in.
//!
//! **THE STREAM LIVES ON ITS OWN THREAD.** `cpal::Stream` holds
//! platform-specific handles that are not `Send`, so it cannot be parked in
//! a `Mutex<Option<Stream>>` shared across the async Tauri command handlers
//! that start and stop a capture, which may run on different threads. A
//! dedicated OS thread builds the stream, calls `.play()`, blocks until told
//! to stop, and only then drops the stream — all on that one thread, which
//! is the real cpal contract for this type, not a workaround invented here.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Mutex;
use std::thread::JoinHandle;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

// `LocalWhisperProvider::transcribe` is a trait method (`SpeechToTextProvider`,
// stt.rs), not an inherent one -- Rust does not bring trait methods into scope
// just because the type implementing them is named, so this has to be
// imported even though nothing here ever spells `SpeechToTextProvider`
// itself. First caught by a real `cargo build` on the lab VM, 2026-09-16 --
// this file compiled in nobody's head until then, see this crate's own
// Cargo.toml comment for why that took until a Linux VM existed to fix.
use crate::stt::SpeechToTextProvider;

/// (raw samples at the device's own rate, sample rate, channel count)
type CaptureResult = Result<(Vec<f32>, u32, u16), String>;

struct CaptureHandle {
    stop_tx: mpsc::Sender<()>,
    join: JoinHandle<CaptureResult>,
}

/// Holds the running capture, if any. A plain `Mutex`, not an `Arc` shared
/// further — every Tauri command that touches this takes the lock, does one
/// thing, and lets go; nothing holds it across an `.await`.
static CAPTURE: Mutex<Option<CaptureHandle>> = Mutex::new(None);

/// True for exactly as long as a capture is believed running — read-only
/// from outside this module today (nothing currently calls it; kept because
/// "is a capture actually active" is the kind of fact a future status
/// indicator should read from one place rather than infer from whether an
/// `invoke` happens to be in flight).
pub(crate) static CAPTURING: AtomicBool = AtomicBool::new(false);

/// Start listening. Blocks (briefly) until the microphone is confirmed
/// actually open — a caller that gets `Ok(())` back can trust a real stream
/// is running, not merely that a thread was asked to start one.
#[tauri::command]
pub fn native_capture_start() -> Result<(), String> {
    let mut guard = CAPTURE.lock().unwrap();
    if guard.is_some() {
        return Err("A capture is already running.".into());
    }

    let (ready_tx, ready_rx) = mpsc::channel::<Result<(), String>>();
    let (stop_tx, stop_rx) = mpsc::channel::<()>();

    let join = std::thread::Builder::new()
        .name("helloim-mic-capture".into())
        .spawn(move || -> CaptureResult { capture_thread_main(ready_tx, stop_rx) })
        .map_err(|e| format!("Could not start the microphone capture thread: {e}"))?;

    match ready_rx.recv() {
        Ok(Ok(())) => {
            *guard = Some(CaptureHandle { stop_tx, join });
            CAPTURING.store(true, Ordering::SeqCst);
            Ok(())
        }
        Ok(Err(e)) => Err(e),
        Err(_) => {
            Err("The microphone capture thread ended before it could start.".into())
        }
    }
}

/// The whole life of one capture thread: open the device, confirm success (or
/// failure) back to the caller, run until told to stop, then hand back
/// whatever was recorded. Split out of `native_capture_start` purely for
/// readability — every early return sends on `ready_tx` first, because that
/// channel is the ONLY way `native_capture_start` learns the stream failed;
/// an early return with nothing sent would hang that command forever.
fn capture_thread_main(
    ready_tx: mpsc::Sender<Result<(), String>>,
    stop_rx: mpsc::Receiver<()>,
) -> CaptureResult {
    let host = cpal::default_host();
    let device = match host.default_input_device() {
        Some(d) => d,
        None => {
            let msg = "No microphone was found on this machine.".to_string();
            let _ = ready_tx.send(Err(msg.clone()));
            return Err(msg);
        }
    };
    let supported = match device.default_input_config() {
        Ok(c) => c,
        Err(e) => {
            let msg = format!("Could not read the microphone's format: {e}");
            let _ = ready_tx.send(Err(msg.clone()));
            return Err(msg);
        }
    };

    let sample_format = supported.sample_format();
    let config: cpal::StreamConfig = supported.config();
    let sample_rate = config.sample_rate.0;
    let channels = config.channels;

    let samples = std::sync::Arc::new(Mutex::new(Vec::<f32>::new()));
    let err_fn = |e: cpal::StreamError| eprintln!("mic capture stream error: {e}");

    let stream_result = {
        let samples = samples.clone();
        match sample_format {
            cpal::SampleFormat::F32 => device.build_input_stream(
                &config,
                move |data: &[f32], _: &cpal::InputCallbackInfo| {
                    samples.lock().unwrap().extend_from_slice(data);
                },
                err_fn,
                None,
            ),
            cpal::SampleFormat::I16 => device.build_input_stream(
                &config,
                move |data: &[i16], _: &cpal::InputCallbackInfo| {
                    let mut s = samples.lock().unwrap();
                    s.extend(data.iter().map(|v| f32::from(*v) / 32768.0));
                },
                err_fn,
                None,
            ),
            cpal::SampleFormat::U16 => device.build_input_stream(
                &config,
                move |data: &[u16], _: &cpal::InputCallbackInfo| {
                    let mut s = samples.lock().unwrap();
                    s.extend(data.iter().map(|v| (f32::from(*v) - 32768.0) / 32768.0));
                },
                err_fn,
                None,
            ),
            other => {
                let msg = format!("This microphone's audio format ({other:?}) isn't supported yet.");
                let _ = ready_tx.send(Err(msg.clone()));
                return Err(msg);
            }
        }
    };

    let stream = match stream_result {
        Ok(s) => s,
        Err(e) => {
            let msg = format!("Could not open the microphone: {e}");
            let _ = ready_tx.send(Err(msg.clone()));
            return Err(msg);
        }
    };
    if let Err(e) = stream.play() {
        let msg = format!("Could not start the microphone: {e}");
        let _ = ready_tx.send(Err(msg.clone()));
        return Err(msg);
    }

    // Confirmed open. From here, this thread's only job is to keep the
    // stream alive (audio keeps flowing into `samples` for exactly as long
    // as this blocks) until told to stop.
    let _ = ready_tx.send(Ok(()));
    let _ = stop_rx.recv();
    drop(stream);

    let out = samples.lock().unwrap().clone();
    Ok((out, sample_rate, channels))
}

/// Stop listening and transcribe whatever was captured. Always clears the
/// running-capture slot first — see `CAPTURE`'s own doc — so a caller never
/// has to guess whether a failed transcribe left the microphone open.
#[tauri::command]
pub fn native_capture_stop_and_transcribe() -> Result<String, String> {
    let handle = CAPTURE.lock().unwrap().take();
    CAPTURING.store(false, Ordering::SeqCst);
    let Some(handle) = handle else {
        return Err("No capture was running.".into());
    };

    // Best-effort: the receiver may already be gone if the thread somehow
    // exited on its own, and that is exactly what the `.join()` below will
    // surface honestly rather than this call pretending to know why.
    let _ = handle.stop_tx.send(());
    let (samples, sample_rate, channels) = handle
        .join
        .join()
        .map_err(|_| "The microphone capture thread panicked.".to_string())??;

    if samples.is_empty() {
        // Released before any audio actually arrived -- not an error, the
        // same "nothing to send" shape the webview version of this used.
        return Ok(String::new());
    }

    let mono = to_mono(&samples, channels);
    let resampled = downsample_to_16k(&mono, sample_rate);
    let wav = encode_wav_16k_mono(&resampled);
    crate::stt::LocalWhisperProvider::default().transcribe(&wav)
}

/// Average all channels down to one. A no-op copy when the device is already
/// mono, which is the common case for a plain microphone.
fn to_mono(samples: &[f32], channels: u16) -> Vec<f32> {
    if channels <= 1 {
        return samples.to_vec();
    }
    let ch = channels as usize;
    samples
        .chunks(ch)
        .map(|frame| frame.iter().sum::<f32>() / ch as f32)
        .collect()
}

/// Direct port of `ui/index.html`'s own `downsampleTo16k` (same averaging-
/// window algorithm, not a reinvention) — kept in step deliberately: this is
/// the exact shape whisper.cpp was already being fed by the webview path,
/// and changing the resampling algorithm at the same time as moving where it
/// runs would make a regression here impossible to tell apart from one in
/// the move itself.
fn downsample_to_16k(input: &[f32], from_rate: u32) -> Vec<f32> {
    const TARGET: u32 = 16000;
    if TARGET >= from_rate {
        return input.to_vec();
    }
    let ratio = f64::from(from_rate) / f64::from(TARGET);
    let out_len = (input.len() as f64 / ratio).floor() as usize;
    let mut out = Vec::with_capacity(out_len);
    for i in 0..out_len {
        let start = (i as f64 * ratio).floor() as usize;
        let end = (((i + 1) as f64) * ratio).floor() as usize;
        let end = end.min(input.len());
        if start >= end {
            out.push(0.0);
            continue;
        }
        let sum: f32 = input[start..end].iter().sum();
        out.push(sum / (end - start) as f32);
    }
    out
}

/// Direct port of `ui/index.html`'s own `encodeWav16kMono` — same fixed
/// 16-bit mono, 16kHz header, same reason: whisper.cpp has no ffmpeg in
/// front of it (see `stt.rs`'s own header), so it has to be handed a WAV it
/// can decode directly, byte for byte the same shape as before this moved.
fn encode_wav_16k_mono(samples: &[f32]) -> Vec<u8> {
    const RATE: u32 = 16000;
    let mut buf = Vec::with_capacity(44 + samples.len() * 2);
    buf.extend_from_slice(b"RIFF");
    buf.extend_from_slice(&(36 + samples.len() as u32 * 2).to_le_bytes());
    buf.extend_from_slice(b"WAVE");
    buf.extend_from_slice(b"fmt ");
    buf.extend_from_slice(&16u32.to_le_bytes()); // fmt chunk size
    buf.extend_from_slice(&1u16.to_le_bytes()); // PCM
    buf.extend_from_slice(&1u16.to_le_bytes()); // mono
    buf.extend_from_slice(&RATE.to_le_bytes());
    buf.extend_from_slice(&(RATE * 2).to_le_bytes()); // byte rate (16-bit mono)
    buf.extend_from_slice(&2u16.to_le_bytes()); // block align
    buf.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    buf.extend_from_slice(b"data");
    buf.extend_from_slice(&(samples.len() as u32 * 2).to_le_bytes());
    for &s in samples {
        let clamped = s.clamp(-1.0, 1.0);
        let v = if clamped < 0.0 {
            (clamped * 32768.0) as i16
        } else {
            (clamped * 32767.0) as i16
        };
        buf.extend_from_slice(&v.to_le_bytes());
    }
    buf
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn to_mono_averages_interleaved_channels() {
        // Two channels, three frames: (1,3) (2,4) (0,0) -> averages 2,3,0
        let stereo = vec![1.0, 3.0, 2.0, 4.0, 0.0, 0.0];
        assert_eq!(to_mono(&stereo, 2), vec![2.0, 3.0, 0.0]);
    }

    #[test]
    fn to_mono_is_a_plain_copy_for_a_mono_device() {
        let mono = vec![0.1, -0.2, 0.3];
        assert_eq!(to_mono(&mono, 1), mono);
    }

    #[test]
    fn downsample_halves_a_double_rate_input() {
        let input: Vec<f32> = (0..8).map(|i| i as f32).collect();
        let out = downsample_to_16k(&input, 32000);
        assert_eq!(out.len(), 4);
        // Each output sample is the average of a 2-sample window: (0+1)/2, (2+3)/2, ...
        assert_eq!(out, vec![0.5, 2.5, 4.5, 6.5]);
    }

    #[test]
    fn downsample_is_a_no_op_at_or_below_target_rate() {
        let input = vec![0.1, 0.2, 0.3];
        assert_eq!(downsample_to_16k(&input, 16000), input);
        assert_eq!(downsample_to_16k(&input, 8000), input);
    }

    #[test]
    fn wav_header_reports_correct_sizes_and_a_16k_mono_pcm_format() {
        let samples = vec![0.0f32; 100];
        let wav = encode_wav_16k_mono(&samples);
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
        assert_eq!(&wav[12..16], b"fmt ");
        let channels = u16::from_le_bytes([wav[22], wav[23]]);
        let rate = u32::from_le_bytes([wav[24], wav[25], wav[26], wav[27]]);
        let bits = u16::from_le_bytes([wav[34], wav[35]]);
        assert_eq!(channels, 1);
        assert_eq!(rate, 16000);
        assert_eq!(bits, 16);
        let data_size = u32::from_le_bytes([wav[40], wav[41], wav[42], wav[43]]);
        assert_eq!(data_size as usize, samples.len() * 2);
        assert_eq!(wav.len(), 44 + samples.len() * 2);
    }

    #[test]
    fn wav_samples_clamp_and_round_trip_at_full_scale() {
        let wav = encode_wav_16k_mono(&[1.0, -1.0, 0.0]);
        let s0 = i16::from_le_bytes([wav[44], wav[45]]);
        let s1 = i16::from_le_bytes([wav[46], wav[47]]);
        let s2 = i16::from_le_bytes([wav[48], wav[49]]);
        assert_eq!(s0, 32767);
        assert_eq!(s1, -32768);
        assert_eq!(s2, 0);
    }

    #[test]
    fn starting_a_second_capture_while_one_is_running_is_refused() {
        // Does not touch real hardware -- forces the guard's own early-return
        // path directly, the same shape connectors.rs's own busy-guard tests
        // use, rather than actually opening a device in a test run (CI/this
        // box may have none).
        let (stop_tx, _stop_rx) = mpsc::channel();
        let join = std::thread::spawn(|| -> CaptureResult { Ok((vec![], 16000, 1)) });
        *CAPTURE.lock().unwrap() = Some(CaptureHandle { stop_tx, join });
        let err = native_capture_start().unwrap_err();
        assert!(err.contains("already running"));
        // Clean up so this test does not leak state into whichever test
        // runs next in the same process.
        let handle = CAPTURE.lock().unwrap().take().unwrap();
        let _ = handle.stop_tx.send(());
        let _ = handle.join.join();
    }

    #[test]
    fn stopping_with_nothing_running_is_a_named_error_not_a_panic() {
        // Only safe to assert when nothing else in this process left a
        // capture running -- guarded by taking the slot first and putting
        // back whatever was already there, so this test cannot clobber a
        // real one if test execution order ever changes.
        let prior = CAPTURE.lock().unwrap().take();
        let err = native_capture_stop_and_transcribe().unwrap_err();
        assert_eq!(err, "No capture was running.");
        *CAPTURE.lock().unwrap() = prior;
    }
}
