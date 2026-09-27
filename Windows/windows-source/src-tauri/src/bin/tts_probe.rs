//! Prove Kokoro actually makes a sound, outside the app.
//!
//! `tts.rs` needs a Tauri AppHandle, so it cannot be run from a test. This
//! walks the same three steps with the same code — phonemize, pick the style
//! row, run the graph — and writes a WAV. It exists because a release build
//! finishing is not evidence that anything speaks.
//!
//!   cargo run --release --bin tts_probe -- <model.onnx> <voices.bin> <voice> "text" <out.wav>

#[path = "../g2p.rs"]
mod g2p;

use std::io::Read;

const STYLE_DIM: usize = 256;
const MAX_TOKENS: usize = 510;
const SAMPLE_RATE: u32 = 24_000;

fn style_for(pack: &str, voice: &str, token_len: usize) -> Result<Vec<f32>, String> {
    let file = std::fs::File::open(pack).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
    let mut entry = zip
        .by_name(&format!("{voice}.npy"))
        .map_err(|_| format!("No voice called {voice}."))?;
    let mut raw = Vec::new();
    entry.read_to_end(&mut raw).map_err(|e| e.to_string())?;
    if raw.len() < 10 || &raw[0..6] != b"\x93NUMPY" {
        return Err("not an npy".into());
    }
    let header_len = u16::from_le_bytes([raw[8], raw[9]]) as usize;
    let floats: Vec<f32> = raw[10 + header_len..]
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect();
    let rows = floats.len() / STYLE_DIM;
    if rows == 0 {
        return Err("empty voice".into());
    }
    let row = token_len.min(rows - 1);
    eprintln!("  style: {rows} rows, using row {row}");
    Ok(floats[row * STYLE_DIM..(row + 1) * STYLE_DIM].to_vec())
}

fn wav(samples: &[f32]) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + data_len as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    out.extend_from_slice(&(SAMPLE_RATE * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        let v = (s.clamp(-1.0, 1.0) * 32767.0) as i16;
        out.extend_from_slice(&v.to_le_bytes());
    }
    out
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let a: Vec<String> = std::env::args().collect();
    let (model, pack, voice, text, out) = (&a[1], &a[2], &a[3], &a[4], &a[5]);

    let phonemes = g2p::phonemize(text);
    eprintln!("text     : {text}");
    eprintln!("phonemes : {phonemes}");

    let mut session = ort::session::Session::builder()?.commit_from_file(model)?;
    eprintln!("inputs   : {:?}", session.inputs());
    eprintln!("outputs  : {:?}", session.outputs());

    let mut samples: Vec<f32> = Vec::new();
    for chunk in g2p::split_for_model(&phonemes, MAX_TOKENS) {
        let ids = g2p::tokens(&chunk);
        eprintln!("  chunk: {} tokens", ids.len());
        if ids.len() <= 2 {
            continue;
        }
        let style = style_for(pack, voice, ids.len() - 2)?;
        let tokens = ort::value::Tensor::from_array((vec![1_i64, ids.len() as i64], ids.clone()))?;
        let style_t = ort::value::Tensor::from_array((vec![1_i64, 256_i64], style))?;
        let speed_t = ort::value::Tensor::from_array((vec![1_i64], vec![1.0_f32]))?;
        let outputs = session.run(ort::inputs![
            "tokens" => tokens, "style" => style_t, "speed" => speed_t
        ])?;
        let (_, audio) = outputs["audio"].try_extract_tensor::<f32>()?;
        eprintln!("  got {} samples", audio.len());
        samples.extend_from_slice(audio);
    }

    // The check that matters: is it actually sound, or a silent file?
    let peak = samples.iter().fold(0f32, |m, s| m.max(s.abs()));
    let rms = (samples.iter().map(|s| s * s).sum::<f32>() / samples.len().max(1) as f32).sqrt();
    eprintln!(
        "AUDIO    : {} samples, {:.2}s, peak {:.4}, rms {:.4}",
        samples.len(),
        samples.len() as f32 / SAMPLE_RATE as f32,
        peak,
        rms
    );

    std::fs::write(out, wav(&samples))?;
    eprintln!("wrote    : {out}");
    if peak < 0.01 {
        eprintln!("SILENT — this is a failure, not a pass.");
        std::process::exit(1);
    }
    Ok(())
}
