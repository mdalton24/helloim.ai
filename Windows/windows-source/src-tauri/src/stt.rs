//! Speech-to-text providers — a small trait mirroring the shape this crate
//! already uses for the BRAIN (`providers.rs`/`adapter.rs`: several "kinds"
//! behind one call site, so adding a second backend is a second `impl`, not
//! a rewrite). Voice OUTPUT already has that shape too (`tts.rs`, Kokoro).
//! Voice INPUT never has, because until now it never needed one — the whole
//! job was done by the browser.
//!
//! WHY THIS EXISTS — Phase 2 of `desktop/LINUX-APPLIANCE-INSTALLER-PLAN.md`.
//! WebKitGTK, the webview this app runs on Linux, ships no Web Speech API at
//! all: `window.SpeechRecognition` and `window.webkitSpeechRecognition` are
//! both `undefined` there (Chromium/WebView2 only — see `ui/index.html`'s own
//! `SpeechRecognitionCtor` comment, which already reads that absence
//! correctly and disables the mic rather than pretending). Until now nothing
//! stood behind that gate, so voice input on Linux was a dead end, full stop.
//! This module is that second thing: a local whisper.cpp backend the front
//! end can call instead, wired up ONLY where the browser engine is already
//! known to be missing (see `ui/index.html`'s `LOCAL_STT_AVAILABLE`) — a
//! Windows build with a real `SpeechRecognitionCtor` never calls this at all,
//! so its behaviour is unchanged.
//!
//! THE PATTERN THIS MIRRORS, DELIBERATELY, RATHER THAN INVENTING A NEW ONE:
//! `~/voice-line/client/ears.py`'s `transcribe_wav`, the production
//! speech-to-text call already running on this very box against
//! `voiceline-whisper.service` — whisper.cpp's own OpenAI-compatible
//! `POST /v1/audio/transcriptions` route, `multipart/form-data`, field name
//! `file`, `response_format=json`. Read there rather than re-derived. Two
//! things carried over on purpose: the 404-on-`/inference` warning (that
//! route exists on the same server and answers, but is not this one — a
//! whisper.cpp gotcha `ears.py`'s own header already paid for), and the "no
//! ffmpeg on the far end" constraint, which is why the caller must hand this
//! a real WAV rather than whatever `MediaRecorder` happens to produce (webm/
//! opus) — see `voice-visualizer/index.html`'s `encodeWav`, the browser-side
//! half of the same rule, which `ui/index.html`'s capture code mirrors for
//! the same reason.
//!
//! WHAT IS DELIBERATELY NOT HERE: streaming, interim results, wake-word,
//! continuous listening. whisper.cpp's REST route takes one finished clip
//! and hands back one finished transcript — a fundamentally different shape
//! from the Web Speech API's `continuous`/`interimResults` model the rest of
//! `ui/index.html` (the wake-word engine, the anchor-window logic) is built
//! around. Reproducing THAT shape on top of a batch endpoint — buffering,
//! voice-activity detection, partial-result painting, a wake-word model — is
//! Phase 3 of the same plan, not this. This module only ever answers "here
//! is a finished clip, what did it say", which is exactly what push-to-talk
//! needs and nothing more; wake-word stays gated on `SpeechRecognitionCtor`
//! exactly as it was before this file existed.

use std::io::Read;
use std::time::Duration;

/// One already-recorded utterance, ready to send. The caller does the
/// recording and the WAV encoding (`ui/index.html`) — this side only ever
/// receives finished bytes and hands back text, the same "front end owns
/// capture, Rust owns the network call" split `tts.rs` already uses for
/// output.
pub trait SpeechToTextProvider {
    /// `wav_bytes` must already be a well-formed WAV file — see this
    /// module's header on why. Returns the transcript, cleaned, or an error
    /// string that is safe to show a user directly (never a raw transport
    /// error or a URL).
    fn transcribe(&self, wav_bytes: &[u8]) -> Result<String, String>;
}

/// The only implementation today: whisper.cpp's OpenAI-compatible route,
/// reached over loopback by default. `endpoint` is the full URL including
/// the route, so a future settings screen — or a user pointing this at a
/// whisper server elsewhere on their own LAN — is a config value, never a
/// rebuild.
pub struct LocalWhisperProvider {
    endpoint: String,
    timeout: Duration,
}

impl LocalWhisperProvider {
    /// Mirrors `~/voice-line/client/config.py`: a `VOICE_LINE_WHISPER`-shaped
    /// env override (named for THIS app, not borrowed, since the two are
    /// separate services that happen to run the same software) defaulting to
    /// `127.0.0.1:2022` — the port `voiceline-whisper.service` already binds
    /// on this box (`~/.config/systemd/user/voiceline-whisper.service`) —
    /// plus the fixed OpenAI-shaped route appended once, here, so nobody
    /// downstream has to remember to append it.
    pub fn default_endpoint() -> String {
        let base = std::env::var("HELLOIM_WHISPER_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:2022".to_string());
        format!("{}/v1/audio/transcriptions", base.trim_end_matches('/'))
    }

    pub fn new(endpoint: String) -> Self {
        Self { endpoint, timeout: Duration::from_secs(30) }
    }
}

impl Default for LocalWhisperProvider {
    fn default() -> Self {
        Self::new(Self::default_endpoint())
    }
}

// A fixed multipart boundary rather than a random one. Nothing in this app
// ever has two of these requests in flight at once — the front end serialises
// push-to-talk itself (one utterance at a time; see `ui/index.html`'s
// `pttActive` guard, which refuses a second start while one is already
// running) — and a boundary string that plain PCM-in-a-WAV essentially never
// contains is one fewer thing to get wrong for something this small. If it
// ever collided, whisper.cpp's own multipart parser would fail exactly the
// way any parser does on a bad boundary, and that surfaces as an ordinary,
// reported transcribe error rather than a silently corrupted transcript.
const BOUNDARY: &str = "----helloimWhisperBoundary7Q2x9f";

fn build_multipart_body(wav_bytes: &[u8]) -> Vec<u8> {
    let mut body = Vec::with_capacity(wav_bytes.len() + 256);
    body.extend_from_slice(
        format!(
            "--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"speech.wav\"\r\nContent-Type: audio/wav\r\n\r\n"
        )
        .as_bytes(),
    );
    body.extend_from_slice(wav_bytes);
    body.extend_from_slice(b"\r\n");
    // response_format/temperature mirror ears.py's own call, so this route is
    // asked for exactly the shape ears.py already gets back (plain JSON with
    // a "text" field) instead of trusting whisper-server's own default.
    for (name, value) in [("response_format", "json"), ("temperature", "0")] {
        body.extend_from_slice(
            format!("--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n")
                .as_bytes(),
        );
    }
    body.extend_from_slice(format!("--{BOUNDARY}--\r\n").as_bytes());
    body
}

impl SpeechToTextProvider for LocalWhisperProvider {
    fn transcribe(&self, wav_bytes: &[u8]) -> Result<String, String> {
        if wav_bytes.is_empty() {
            return Err("No audio was captured.".into());
        }
        let body = build_multipart_body(wav_bytes);
        let agent = ureq::AgentBuilder::new().timeout(self.timeout).build();
        let resp = agent
            .post(&self.endpoint)
            .set("Content-Type", &format!("multipart/form-data; boundary={BOUNDARY}"))
            .send_bytes(&body);
        let resp = match resp {
            Ok(r) => r,
            // Same trap ears.py's header already names: /inference is a
            // real route on the same server and answers, but it is not the
            // OpenAI-shaped one this call needs.
            Err(ureq::Error::Status(404, _)) => {
                return Err(format!(
                    "The local speech server at {} answered 404. It is running but not \
                     exposing the OpenAI-style transcription route.",
                    self.endpoint
                ));
            }
            Err(ureq::Error::Status(code, _)) => {
                return Err(format!("The local speech server rejected the request ({code})."));
            }
            Err(ureq::Error::Transport(_)) => {
                return Err(
                    "Could not reach the local speech service. Is it running?".to_string()
                );
            }
        };
        let mut text = String::new();
        resp.into_reader()
            .take(1024 * 1024)
            .read_to_string(&mut text)
            .map_err(|_| "The local speech service sent back something unreadable.".to_string())?;
        // whisper-server answers JSON ({"text": "..."}) for response_format=json;
        // fall back to the raw body if it ever doesn't, same defensiveness as
        // ears.py's own `resp.json().get("text", "")` vs `resp.text` split.
        let out = match serde_json::from_str::<serde_json::Value>(&text) {
            Ok(v) => v.get("text").and_then(|t| t.as_str()).unwrap_or(&text).to_string(),
            Err(_) => text,
        };
        Ok(clean_transcript(&out))
    }
}

/// Strip whisper's own bracketed non-speech markers ([BLANK_AUDIO], [SIGHS],
/// ...) and collapse whitespace — the same cleanup `ears.py::clean` already
/// does server-side for the voice line, repeated here because this path
/// never goes through that file. Deliberately does NOT repeat ears.py's
/// "punctuation-only collapses to empty string" step: that exists there to
/// stop an open, VAD-triggered mic from firing a turn on noise, and
/// push-to-talk has no VAD — a person held the button on purpose, so an odd
/// punctuation-only transcript is reported honestly rather than silently
/// swallowed.
fn clean_transcript(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_bracket = false;
    for ch in text.chars() {
        match ch {
            '[' => in_bracket = true,
            ']' => in_bracket = false,
            _ if in_bracket => {}
            _ => out.push(ch),
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The Tauri command `ui/index.html` calls. Base64 in, because a Tauri
/// command argument is JSON — the same encoding `email.rs`/`update.rs`
/// already use for a binary payload crossing this exact boundary, so this
/// is not a new convention. Decoding, provider selection and the network
/// call all happen off the UI thread (`async`), the same as every other
/// network-backed command in this crate (`tts_speak`, `tts_install`, ...).
#[tauri::command(async)]
pub fn stt_transcribe(audio_base64: String) -> Result<String, String> {
    use base64::{engine::general_purpose::STANDARD, Engine as _};
    let wav_bytes = STANDARD
        .decode(audio_base64.as_bytes())
        .map_err(|_| "The recorded audio could not be read.".to_string())?;
    let provider = LocalWhisperProvider::default();
    provider.transcribe(&wav_bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_endpoint_with_and_without_an_override() {
        // ONE TEST, NOT TWO — `cargo test` runs tests in parallel in the same
        // process, and `std::env::set_var`/`remove_var` are process-global.
        // Two separate #[test] fns here raced on this exact env var (caught
        // on the lab VM build, 2026-09-14: the no-override assertion read
        // back a value the OTHER test had just set), so both cases live in
        // one test function where the ordering is ours to control instead of
        // the scheduler's.
        std::env::remove_var("HELLOIM_WHISPER_URL");
        assert_eq!(
            LocalWhisperProvider::default_endpoint(),
            "http://127.0.0.1:2022/v1/audio/transcriptions",
            "no override set: must stay in step with voiceline-whisper.service's bound port \
             (see FACTS.md and the unit file itself) — silent drift here is a Linux build \
             that points at nothing and calls it working"
        );

        std::env::set_var("HELLOIM_WHISPER_URL", "http://192.168.1.50:2022/");
        assert_eq!(
            LocalWhisperProvider::default_endpoint(),
            "http://192.168.1.50:2022/v1/audio/transcriptions",
            "override set: must be honoured and still get the route appended exactly once"
        );
        std::env::remove_var("HELLOIM_WHISPER_URL");
    }

    #[test]
    fn clean_transcript_strips_bracketed_markers_and_collapses_space() {
        assert_eq!(
            clean_transcript("  [BLANK_AUDIO]  hello   there [SIGHS] "),
            "hello there"
        );
    }

    #[test]
    fn multipart_body_carries_the_boundary_the_field_name_and_the_wav_bytes() {
        let wav = b"RIFF....WAVEfmt fake-pcm-bytes-here";
        let body = build_multipart_body(wav);
        let text = String::from_utf8_lossy(&body);
        assert!(text.contains(BOUNDARY));
        assert!(text.contains("name=\"file\""));
        assert!(text.contains("audio/wav"));
        assert!(text.contains("response_format"));
        assert!(body.windows(wav.len()).any(|w| w == wav));
    }

    #[test]
    fn empty_audio_is_refused_before_any_network_call() {
        let provider = LocalWhisperProvider::new("http://127.0.0.1:1".to_string());
        let err = provider.transcribe(&[]).unwrap_err();
        assert!(err.contains("No audio"));
    }

    /// The one test in this file that is not synthetic — it needs a real
    /// whisper.cpp server actually running at `HELLOIM_WHISPER_URL`/the
    /// default loopback address, which is why it is `#[ignore]`d rather than
    /// part of the ordinary suite: `cargo test` must still pass on a machine
    /// with nothing listening on :2022. Run it deliberately, with a server
    /// up, via `cargo test --bin remembrancer stt:: -- --ignored`.
    ///
    /// `fixtures/jfk.wav` is whisper.cpp's own bundled sample
    /// (`samples/jfk.wav` in the upstream repo, 16kHz mono PCM, public
    /// domain) — a known clip with a known correct transcript, copied in
    /// rather than synthesised, so this test proves something a
    /// hand-crafted WAV of silence could not: that `build_multipart_body`'s
    /// bytes are accepted by REAL whisper.cpp (not just parsed by our own
    /// code) and that `transcribe`'s JSON-unwrapping actually recovers real
    /// speech, not just a fixture string we chose ourselves.
    ///
    /// Verified 2026-09-14 against a CPU-only whisper.cpp build (ggml-base.en)
    /// on a lab VM mirroring voiceline-whisper.service's own
    /// invocation (`--inference-path /v1/audio/transcriptions`): this exact
    /// test, run against that exact server, returned "And so my fellow
    /// Americans, ask not what your country can do for you, ask what you can
    /// do for your country." — matching the known JFK inaugural line.
    #[test]
    #[ignore]
    fn transcribes_a_known_wav_against_a_real_running_whisper_server() {
        let wav = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/fixtures/jfk.wav"))
            .expect("fixtures/jfk.wav should be checked in");
        let provider = LocalWhisperProvider::default();
        let text = provider.transcribe(&wav).expect("the local whisper server should answer");
        let lower = text.to_lowercase();
        assert!(
            lower.contains("ask not what your country"),
            "unexpected transcript from a known clip: {text:?}"
        );
    }
}
