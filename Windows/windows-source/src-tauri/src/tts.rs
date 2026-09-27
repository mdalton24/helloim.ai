//! Kokoro, running on the user's own machine.
//!
//! Mark, 2026-08-26, after hearing what Windows offers: "yes include kokora and
//! also give the user selection of voices to use and a sample next to them on
//! the menu."
//!
//! WHY THIS EXISTS AT ALL. His Windows machine has exactly three speech voices —
//! David, Mark and Zira, the legacy SAPI set — and Windows offers no API to add
//! better ones. There was no software fix on that side. Kokoro is Apache 2.0, it
//! runs on ordinary CPU, it costs nothing per word, and it is what this whole
//! system already speaks with.
//!
//! IT IS NOT IN THE INSTALLER, AND THAT IS DELIBERATE. The model is 88 MB and
//! the voices 27 MB against a 4.5 MB installer. Bundling them would make every
//! download twenty-five times bigger for a feature not everybody wants, so it is
//! fetched once, on request, with progress — the same shape as the Claude Code
//! install, and for the same reason.
//!
//! WHAT LIVES WHERE:
//!   in the binary   the phoneme dictionary and the token vocabulary (see g2p)
//!   downloaded      kokoro-v1.0.int8.onnx, voices-v1.0.bin
//!   never           anything about what was spoken; nothing leaves the machine

use serde::Serialize;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};

const MODEL_URL: &str =
    "https://github.com/thewh1teagle/kokoro-onnx/releases/download/model-files-v1.0/kokoro-v1.0.int8.onnx";
const VOICES_URL: &str =
    "https://github.com/thewh1teagle/kokoro-onnx/releases/download/model-files-v1.0/voices-v1.0.bin";
/* ── WHAT THESE TWO HASHES ARE, AND WHAT THEY ARE NOT. ────────────────────────

   Cassandra, 2026-08-28: this path fetched ~115 MB from somebody else's GitHub
   release and verified NOTHING — no checksum, no signature, no pinned digest —
   then handed the bytes to onnxruntime. GitHub release assets are mutable by
   the repo owner, and `thewh1teagle/kokoro-onnx` is not ours. Her contrast is
   the argument: `install.rs` hashes its download as it streams and deletes it
   on a mismatch. This file now does the same thing, the same way, on purpose.

   THE HONEST PROVENANCE, because a checksum whose origin is unstated is worse
   than none — it reads as verified:

     * UPSTREAM PUBLISHES NO CHECKSUM. Checked 2026-08-28: the release body for
       `model-files-v1.0` carries none, and the GitHub API returns `digest:
       null` for all five assets (they were uploaded 2025-01-28, before GitHub
       started recording asset digests). There is no third-party attestation to
       pin, and inventing confidence we do not have is not on the table.

     * SO THESE ARE OUR OWN BYTES. They are the SHA-256 of the exact files
       NameOS fetched on 2026-08-26, vetted, and bundled into `installer/tts/`,
       which ships inside the installer that carries a minisign signature the
       updater checks before running it. So what this pin actually asserts is:
       **a runtime download must be byte-identical to the copy we ship and
       sign.** That is a real property and it is the one worth having.

     * WHAT IT DOES NOT PROVE: that upstream's file was ever what its author
       intended. We are pinning continuity with an artefact we reviewed, not
       vouching for its origin.

     * CORROBORATION, measured rather than assumed: an independent fresh
       download on 2026-08-28 matched the 2026-08-26 copy and the installed
       copy byte for byte. Three fetches, two days apart, same hash.

   IF YOU DELIBERATELY MOVE TO A NEWER MODEL, change the URL and the hash in the
   same commit, and re-derive the hash from the file you actually shipped —
   never from whatever the download happens to return that day. */
const KOKORO_SHA256: &str = "6e742170d309016e5891a994e1ce1559c702a2ccd0075e67ef7157974f6406cb";
const VOICES_SHA256: &str = "bca610b8308e8d99f32e6fe4197e7ec01679264efed0cac9140fe9c29f1fbf7d";

const SAMPLE_RATE: u32 = 24_000;
/// The model's positional encoding is fixed; longer text is spoken in pieces.
const MAX_TOKENS: usize = 510;
const NET_TIMEOUT: Duration = Duration::from_secs(1800);

#[derive(Default)]
pub struct Tts {
    /// The ONNX session, kept warm. Loading an 88 MB graph takes long enough
    /// that doing it per sentence would make every reply start late.
    session: Mutex<Option<ort::session::Session>>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TtsStatus {
    pub installed: bool,
    pub download_mb: u64,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Voice {
    pub id: String,
    /// "Bella", not "af_bella" — the id is for us, the label is for a person.
    pub label: String,
    /// "American" / "British", from the id's first letter.
    pub accent: String,
    /// "f" / "m", from the second.
    pub gender: String,
}

#[derive(Clone, Serialize)]
struct Progress {
    text: String,
}

/// **THE CSP HAS TO ALLOW `blob:` UNDER `media-src` OR NONE OF THIS MAKES A
/// SOUND, and there is nowhere in `tauri.conf.json` to write that down.**
/// Mark reported the voices silent twice, 2026-08-26. The engine was never at
/// fault -- measured here at 2.70 seconds of audio, peak 0.6452 -- and the WAV
/// reaches the window as a blob URL, which `default-src 'self'` refuses. A
/// blocked media source raises nothing anyone is watching: the element simply
/// never becomes playable, `play()` reports success, and the room stays quiet.
/// Proven by serving this page under both policies: the old one gives
/// MEDIA_ERR_SRC_NOT_SUPPORTED, the new one loads.
/// If somebody tightens that line again, this is the file that goes mute.
fn dir(app: &AppHandle) -> Result<PathBuf, String> {
    let d = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("no data directory: {e}"))?
        .join("tts");
    std::fs::create_dir_all(&d).map_err(|e| format!("could not create {d:?}: {e}"))?;
    Ok(d)
}

/// The copy that ships INSIDE the installer, beside the executable.
///
/// Mark, 2026-08-26: "can we package this into our installer so they dont have
/// to go and download." He is right that a 115 MB download standing between
/// somebody and the good voices is a step most people will not take — and the
/// ones who would take it are not the ones this product is for.
///
/// Read-only and shared by every user on the machine, which is why it lives in
/// the program directory rather than being copied into each profile: 115 MB
/// per person, for a file nobody edits, is waste that shows up as a full disk
/// months later.
fn bundled(name: &str) -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let p = exe.parent()?.join("tts").join(name);
    p.is_file().then_some(p)
}

/// Where to READ a model from: the downloaded copy first, then the bundled one.
///
/// Downloaded wins on purpose. If someone has fetched a newer model, that is a
/// deliberate act and the shipped copy must not silently override it.
fn find(app: &AppHandle, name: &str) -> Option<PathBuf> {
    let own = dir(app).ok()?.join(name);
    if own.is_file() {
        return Some(own);
    }
    bundled(name)
}

fn model_path(app: &AppHandle) -> Result<PathBuf, String> {
    find(app, "kokoro.onnx").ok_or_else(|| "The voice model is not installed.".to_string())
}
fn voices_path(app: &AppHandle) -> Result<PathBuf, String> {
    find(app, "voices.bin").ok_or_else(|| "The voice pack is not installed.".to_string())
}

/// Where to WRITE a download. Always the per-user directory — the program
/// directory needs administrator rights and the app is not running with them.
fn download_target(app: &AppHandle, name: &str) -> Result<PathBuf, String> {
    Ok(dir(app)?.join(name))
}

fn say(app: &AppHandle, text: &str) {
    let _ = app.emit("tts:progress", Progress { text: text.to_string() });
}

#[tauri::command]
pub fn tts_status(app: AppHandle) -> TtsStatus {
    // is_ok() rather than is_file(): the paths above already resolved to a
    // real file, in whichever of the two places it turned out to be.
    let ok = model_path(&app).is_ok() && voices_path(&app).is_ok();
    TtsStatus { installed: ok, download_mb: 115 }
}

/// SHA-256 of a file already on disk, streamed rather than read into memory.
///
/// 115 MB held in a `Vec` to check a hash would be 115 MB of resident memory on
/// a machine we know nothing about, for no gain. Same reasoning as the
/// hash-as-you-write below, pointed at a file that is already there.
fn hash_file(path: &std::path::Path) -> Result<String, String> {
    use sha2::{Digest, Sha256};
    let mut f = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let n = f.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// Download the model and the voice packs, once — and prove they are the files
/// we mean before either of them is ever loaded.
#[tauri::command(async)]
pub fn tts_install(app: AppHandle) -> Result<(), String> {
    let agent = ureq::AgentBuilder::new().timeout(NET_TIMEOUT).build();

    for (url, path, label, mb, want) in [
        (MODEL_URL, download_target(&app, "kokoro.onnx")?, "the voice model", 88u64, KOKORO_SHA256),
        (VOICES_URL, download_target(&app, "voices.bin")?, "the voices", 27u64, VOICES_SHA256),
    ] {
        /* A FILE THAT IS ALREADY THERE STILL HAS TO PROVE ITSELF, and this
           branch used to skip on existence alone. Anything downloaded before
           this check existed arrived unverified, and this is the only moment
           we can put that right without hashing 115 MB on the speaking path.
           A mismatch here is not fatal: we delete it and fetch it again. */
        if path.is_file() {
            match hash_file(&path) {
                Ok(h) if h == want => continue, // genuinely ours; never re-download
                Ok(_) => {
                    say(&app, &format!("The copy of {label} on this machine was not the \
                                        one we ship. Replacing it."));
                    let _ = std::fs::remove_file(&path);
                }
                // Could not read it to check. Treat unreadable as unverified —
                // "I could not tell" is not "it is fine".
                Err(_) => {
                    let _ = std::fs::remove_file(&path);
                }
            }
        }

        say(&app, &format!("Downloading {label} — about {mb} MB."));
        let resp = agent.get(url).call().map_err(|e| match e {
            ureq::Error::Status(c, _) => format!("The download service answered {c}."),
            ureq::Error::Transport(t) => format!("Could not reach the download service: {t}"),
        })?;

        /* WRITTEN TO A .part AND RENAMED ONLY WHEN COMPLETE. A download killed
           half way through would otherwise leave a file that exists, passes the
           "installed" check, and fails at load with something unreadable about
           protobuf.

           THE CHECKSUM IS WHY THE .part MATTERS TWICE OVER NOW: the file only
           takes its real name once the bytes have been proven, so a download
           that does not match never becomes something `find()` can load. */
        let part = path.with_extension("part");
        let mut file = std::fs::File::create(&part).map_err(|e| e.to_string())?;
        let mut reader = resp.into_reader();
        let mut buf = vec![0u8; 1 << 16];
        let mut total: u64 = 0;
        // Hashed AS IT IS WRITTEN — the pattern install.rs already uses, not a
        // second one invented here.
        let mut hasher = <sha2::Sha256 as sha2::Digest>::new();
        loop {
            let n = reader.read(&mut buf).map_err(|e| format!("The download stopped: {e}"))?;
            if n == 0 { break; }
            sha2::Digest::update(&mut hasher, &buf[..n]);
            file.write_all(&buf[..n]).map_err(|e| e.to_string())?;
            let before = total / (10 << 20);
            total += n as u64;
            if total / (10 << 20) != before {
                say(&app, &format!("{} MB of {label}…", total / (1 << 20)));
            }
        }
        drop(file);

        let got: String = sha2::Digest::finalize(hasher).iter().map(|b| format!("{b:02x}")).collect();
        if got != want {
            /* Bytes that are not the ones we vetted do not get loaded and then
               apologised for. Delete them, and tell the person what actually
               happened — they still have working speech, because the copy in
               the installer is untouched. */
            let _ = std::fs::remove_file(&part);
            return Err(format!(
                "{label} did not match the copy we published, so it was deleted rather \
                 than used. Nothing on your machine changed and the built-in voices \
                 still work. This usually means the download was interrupted or \
                 something on the network altered it — worth trying again."
            ));
        }
        std::fs::rename(&part, &path).map_err(|e| e.to_string())?;
    }
    say(&app, "Verified. Ready.");
    Ok(())
}

/// Read the voice names out of the pack.
///
/// The pack is a numpy .npz, which is a zip — so the names come from the zip
/// index alone and none of the 27 MB of weights is touched to draw a menu.
#[tauri::command(async)]
pub fn tts_voices(app: AppHandle) -> Result<Vec<Voice>, String> {
    let path = voices_path(&app)?;
    if !path.is_file() {
        return Ok(Vec::new());
    }
    let file = std::fs::File::open(&path).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| format!("The voice pack is unreadable: {e}"))?;

    let mut out = Vec::new();
    for i in 0..zip.len() {
        let name = match zip.by_index_raw(i) {
            Ok(f) => f.name().to_string(),
            Err(_) => continue,
        };
        let id = name.trim_end_matches(".npy").to_string();
        // The ids are coded: <accent><gender>_<name>. af_bella is an American
        // woman called Bella. Shown as "Bella", because nobody wants to read a
        // filename in a menu.
        let (prefix, given) = id.split_once('_').unwrap_or(("", id.as_str()));
        let accent = match prefix.chars().next() {
            Some('a') => "American", Some('b') => "British", Some('e') => "Spanish",
            Some('f') => "French",   Some('h') => "Hindi",   Some('i') => "Italian",
            Some('j') => "Japanese", Some('p') => "Portuguese", Some('z') => "Chinese",
            _ => "",
        };
        let gender = match prefix.chars().nth(1) {
            Some('f') => "f", Some('m') => "m", _ => "",
        };
        let mut label = given.trim_start_matches("v0").to_string();
        if let Some(c) = label.get_mut(0..1) { c.make_ascii_uppercase(); }
        out.push(Voice { id, label, accent: accent.into(), gender: gender.into() });
    }
    // Grouped the way somebody would look for them, not the way a zip lists them.
    out.sort_by(|a, b| a.accent.cmp(&b.accent).then(a.label.cmp(&b.label)));
    Ok(out)
}

/// One voice's style matrix: 510 rows of 256 floats, indexed by token count.
fn style_for(app: &AppHandle, voice: &str, token_len: usize) -> Result<Vec<f32>, String> {
    let file = std::fs::File::open(voices_path(app)?).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
    let mut entry = zip
        .by_name(&format!("{voice}.npy"))
        .map_err(|_| format!("No voice called {voice}."))?;
    let mut raw = Vec::new();
    entry.read_to_end(&mut raw).map_err(|e| e.to_string())?;

    /* .npy: a 6-byte magic, two version bytes, then a header length and an
       ASCII dict. The data is plain little-endian f32 after that. Parsing the
       header length rather than assuming 128 bytes is the difference between
       this working and producing noise. */
    if raw.len() < 10 || &raw[0..6] != b"\x93NUMPY" {
        return Err("The voice pack is not in the expected format.".into());
    }
    let header_len = u16::from_le_bytes([raw[8], raw[9]]) as usize;
    let start = 10 + header_len;
    let floats: Vec<f32> = raw[start..]
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect();

    const STYLE_DIM: usize = 256;
    let rows = floats.len() / STYLE_DIM;
    if rows == 0 {
        return Err("That voice is empty.".into());
    }
    /* THE ROW IS CHOSEN BY LENGTH. Kokoro ships a style vector per token count;
       using row 0 for everything is the classic mistake and gives speech with
       the wrong pacing for every sentence that is not tiny. */
    let row = token_len.min(rows - 1);
    Ok(floats[row * STYLE_DIM..(row + 1) * STYLE_DIM].to_vec())
}

/// Speak. Returns a 24 kHz mono WAV.
///
/// AS RAW BYTES, NOT A `Vec<u8>`. Tauri serialises a plain Vec as a JSON array
/// of numbers, so seven seconds of speech crosses the bridge as roughly a
/// million characters of decimal text and arrives as a JS array that then has to
/// be copied into a buffer. `Response` hands the same bytes over as an
/// ArrayBuffer, which is what the audio element wants anyway.
#[tauri::command(async)]
pub fn tts_speak(
    app: AppHandle,
    state: tauri::State<'_, Tts>,
    text: String,
    voice: String,
    speed: Option<f32>,
) -> Result<tauri::ipc::Response, String> {
    let model = model_path(&app)?;
    if !model.is_file() {
        return Err("The voice is not installed yet.".into());
    }

    let mut guard = state.session.lock().map_err(|_| "busy".to_string())?;
    if guard.is_none() {
        let s = ort::session::Session::builder()
            .map_err(|e| e.to_string())?
            .commit_from_file(&model)
            .map_err(|e| format!("Could not load the voice model: {e}"))?;
        *guard = Some(s);
    }
    let session = guard.as_mut().unwrap();

    let phonemes = crate::g2p::phonemize(&text);
    let mut samples: Vec<f32> = Vec::new();

    for chunk in crate::g2p::split_for_model(&phonemes, MAX_TOKENS) {
        let ids = crate::g2p::tokens(&chunk);
        if ids.len() <= 2 {
            continue;   // nothing but the pads — silence, not an error
        }
        // The style row is indexed by the phoneme count WITHOUT the two pads.
        let style = style_for(&app, &voice, ids.len() - 2)?;

        let tokens = ort::value::Tensor::from_array((vec![1_i64, ids.len() as i64], ids.clone()))
            .map_err(|e| e.to_string())?;
        let style_t = ort::value::Tensor::from_array((vec![1_i64, 256_i64], style))
            .map_err(|e| e.to_string())?;
        let speed_t = ort::value::Tensor::from_array((vec![1_i64], vec![speed.unwrap_or(1.0)]))
            .map_err(|e| e.to_string())?;

        let outputs = session
            .run(ort::inputs!["tokens" => tokens, "style" => style_t, "speed" => speed_t])
            .map_err(|e| format!("The voice model failed: {e}"))?;
        let (_, audio) = outputs["audio"]
            .try_extract_tensor::<f32>()
            .map_err(|e| e.to_string())?;
        samples.extend_from_slice(audio);
    }

    if samples.is_empty() {
        return Err("There was nothing to say.".into());
    }
    Ok(tauri::ipc::Response::new(wav(&samples)))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A pasted hash with a character missing verifies nothing and looks
    /// exactly like one that verifies everything.
    #[test]
    fn the_pinned_hashes_are_well_formed_and_distinct() {
        for (name, h) in [("kokoro", KOKORO_SHA256), ("voices", VOICES_SHA256)] {
            assert_eq!(h.len(), 64, "{name}: a SHA-256 is 64 hex characters");
            assert!(
                h.chars().all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c)),
                "{name}: lowercase hex only — the comparison is a string compare"
            );
        }
        assert_ne!(KOKORO_SHA256, VOICES_SHA256, "both files pinned to one hash");
    }

    /// THE DRIFT THIS CATCHES: somebody bumps the bundled model and does not
    /// bump the constant, so the installer ships one file while the download
    /// path insists on another — and the mismatch only shows up on a customer's
    /// machine, as speech that will not install.
    ///
    /// Conditional by necessity: the bundled copies are 115 MB and are fetched
    /// by `ship-windows.sh` at build time, so they are not in a fresh checkout.
    /// It SAYS SO OUT LOUD when it skips rather than passing quietly — a test
    /// that reports success for having done nothing is the exact fault this
    /// review was called in to fix.
    #[test]
    fn the_pins_match_the_copies_we_actually_bundle() {
        // src-tauri/../installer/tts — where ship-windows.sh puts them.
        let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("installer")
            .join("tts");
        let mut checked = 0;
        for (name, want) in [("kokoro.onnx", KOKORO_SHA256), ("voices.bin", VOICES_SHA256)] {
            let p = base.join(name);
            if !p.is_file() {
                eprintln!(
                    "SKIPPED for {name}: {p:?} is not present, so the pin was NOT verified \
                     against a real file in this run."
                );
                continue;
            }
            assert_eq!(
                hash_file(&p).unwrap(),
                want,
                "{name} in the installer does not match the pinned hash in tts.rs"
            );
            checked += 1;
        }
        eprintln!("pins verified against {checked} of 2 bundled files");
    }
}

/// A 16-bit PCM WAV, written by hand.
///
/// A dependency for 44 bytes of header would be a package to keep working
/// forever in exchange for nothing.
fn wav(samples: &[f32]) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + data_len as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());       // PCM header size
    out.extend_from_slice(&1u16.to_le_bytes());        // PCM
    out.extend_from_slice(&1u16.to_le_bytes());        // mono
    out.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    out.extend_from_slice(&(SAMPLE_RATE * 2).to_le_bytes()); // byte rate
    out.extend_from_slice(&2u16.to_le_bytes());        // block align
    out.extend_from_slice(&16u16.to_le_bytes());       // bits
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        // Clamped, not wrapped: a sample over 1.0 wrapping to full negative is
        // an audible click, and the model does occasionally overshoot.
        let v = (s.clamp(-1.0, 1.0) * 32767.0) as i16;
        out.extend_from_slice(&v.to_le_bytes());
    }
    out
}
