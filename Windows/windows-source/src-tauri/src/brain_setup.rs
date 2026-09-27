//! What this machine can actually run, and which brain to put on it.
//!
//! Written 2026-08-27 in response to a feature request: give the installer
//! real options for which local brain to run, instead of a single "Get
//! Ollama" button with no idea what the machine underneath it can hold — and
//! when local is chosen, test the hardware first and recommend from that.
//!
//! WHAT WAS THERE BEFORE THIS FILE, because it explains the shape of it: the
//! entire local-brain flow was three states that all ended in opening a web
//! page — `Get Ollama` to the download page, `Get a model` to the library, and
//! a form. Detection stopped at "is something answering on 11434". So the
//! product asked a customer to choose a model with no idea what their machine
//! could hold, which on a 8 GB laptop means downloading twenty gigabytes to
//! find out it swaps.
//!
//! **THE RULE THIS FILE INHERITS IS ALREADY WRITTEN IN `providers.rs`: DETECT,
//! NEVER ASSUME.** That comment exists because the premise error had already
//! bitten this product once — "embeddings from the local model" described the
//! development box, which has a warm 5090, rather than the customer's machine.
//! Everything here is measured on the machine in front of the user or reported
//! as unmeasured. There is no default that quietly describes this box.
//!
//! **"COULD NOT DETERMINE" IS AN ANSWER AND IT IS NEVER "NONE".** A missing
//! `nvidia-smi` means we did not find out; it does not mean there is no GPU,
//! and rendering it as "no GPU detected" would recommend a CPU-sized model to
//! somebody holding a 4090. Every unknown travels as `None` with a sentence in
//! `notes` saying what could not be read and why, in the same discipline Argus
//! works under: report what IS, and always say what you could not determine.
//!
//! NO NEW DEPENDENCIES ON PURPOSE. `sysinfo` would be the obvious crate and it
//! would mean editing `Cargo.toml`, which currently carries another session's
//! uncommitted work. Everything below is std plus commands that ship with the
//! operating system, which also keeps the probe honest: it reads exactly what a
//! person would read if they looked themselves.

use serde::{Deserialize, Serialize};
use std::process::Command;

// --- WHAT WE FOUND ----------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Gpu {
    pub name: String,
    /// None means we could not read it, NOT that the card has no memory.
    pub vram_mb: Option<u64>,
    /// Which probe produced this, so a surprising number can be chased to the
    /// command that said it rather than argued about.
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HardwareProfile {
    pub os: String,
    pub cores: Option<usize>,
    pub ram_mb: Option<u64>,
    pub gpus: Vec<Gpu>,
    pub disk_free_mb: Option<u64>,
    /// Plain sentences about what could not be determined. This is shown to the
    /// user, not logged and forgotten — a recommendation made on partial
    /// information has to say which part was missing.
    pub notes: Vec<String>,
}

impl HardwareProfile {
    /// The biggest single card, which is what one model can actually use.
    ///
    /// **NOT the sum.** Two 8 GB cards are not one 16 GB card: a model has to
    /// fit in one device's memory unless it is explicitly split, and adding
    /// them is the arithmetic that recommends a model which then fails to
    /// load. Summing here would be inventing capacity the machine does not
    /// have, which is the same class of error as assuming a GPU exists.
    pub fn best_vram_mb(&self) -> Option<u64> {
        self.gpus.iter().filter_map(|g| g.vram_mb).max()
    }
}

// --- THE CATALOG ------------------------------------------------------------
//
// CURATED AND VERSIONED HERE, NOT SCRAPED. The registry can list what exists;
// it cannot say which of them is a sane choice for a machine with 8 GB of VRAM,
// and that judgement IS the feature. A scraped list would also change under the
// product without anyone deciding it had.
//
// EVERY SIZE IS MEASURED FROM THE REAL MANIFEST, and the first version of this
// list is why that is stated so loudly. It was seeded from the models that
// happen to be pulled on THIS box — four of five were Qwen — and two of its
// tags, `qwen3.6:4b` and `qwen3.6:8b`, did not exist at all. `qwen3.6` is a
// real model that ships only at 27b and 35b, so the wizard would have handed a
// customer `ollama pull qwen3.6:4b` and watched it fail. Written the same hour
// as a module header promising nothing here is defaulted from the machine that
// wrote it.
//
// So: tags and byte sizes come from registry.ollama.ai manifests, checked
// 2026-08-27, and the requirements are DERIVED from them rather than typed
// beside them. A download size is not a memory requirement — the KV cache and
// compute buffers live on the card too — but that relationship is a rule, and a
// rule belongs in a function where it can be read once.
//
// The families are chosen so a buyer recognises the names: Llama, Gemma,
// Mistral, Phi, DeepSeek, plus one vision model and one for code. Curated, not
// scraped: the registry can list what exists, and cannot say which is a sane
// choice for a machine with 8 GB.

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Model {
    pub tag: &'static str,
    pub label: &'static str,
    /// **MEASURED, NOT ESTIMATED.** Every number below is the sum of the
    /// `image.model` layers in that tag's real manifest, read from
    /// registry.ollama.ai on 2026-08-27. Nothing here is remembered.
    pub download_mb: u64,
    pub good_for: &'static str,
    pub caveat: Option<&'static str>,
}

/// What this model needs resident on the card.
///
/// **DERIVED FROM ONE MEASURED NUMBER, NOT TYPED.** The first version of this
/// file carried hand-written `min_vram_mb` and `min_ram_mb` beside each entry,
/// and hand-written numbers are exactly where the errors were: two of the five
/// tags did not exist at all, so their requirements had been invented to match
/// a download size that was also invented. One real measurement and a stated
/// rule cannot drift apart that way.
///
/// The rule: the weights have to be resident, and the KV cache and compute
/// buffers live on the card too. A fifth over the weights plus a gigabyte is
/// the conservative approximation, and conservative is the right direction —
/// being told a model is processor-only when it would just fit costs some
/// speed, and the reverse wedges the display.
///
/// **TAKES THE RAW MEGABYTES, NOT A `Model`** — split out 2026-08-31 so
/// `providers.rs` can size an ALREADY-INSTALLED model (whose size comes from
/// Ollama's own `/api/tags`, a `u64` at runtime) against this exact rule
/// instead of a second, hand-copied one. Two formulas for "does this fit"
/// drifting apart is exactly the class of bug the catalog's own header warns
/// about — one measured number and one stated rule, never two.
pub fn needs_vram_mb_for(download_mb: u64) -> u64 {
    download_mb * 6 / 5 + 1024
}

/// What it needs in system memory to run on the processor instead.
///
/// The weights still have to be somewhere, and the operating system needs to
/// keep running around them. Slower by a lot, and said in those words rather
/// than hidden behind a smaller number.
pub fn needs_ram_mb_for(download_mb: u64) -> u64 {
    download_mb + 4096
}

pub const CATALOG: &[Model] = &[
    Model { tag: "llama3.2:3b", label: "Llama 3.2 3B", download_mb: 1926,
        good_for: "The smallest one worth having. Runs on almost any machine.",
        caveat: Some("Small models get details confidently wrong. Fine for drafting, \
                      not for anything you would act on unchecked.") },
    Model { tag: "phi4-mini:3.8b", label: "Phi-4 Mini", download_mb: 2376,
        good_for: "Punches above its size on a modest laptop.", caveat: None },
    Model { tag: "gemma3:4b", label: "Gemma 3 4B", download_mb: 3184,
        good_for: "Google's small one. Good at doing what it is told.", caveat: None },
    Model { tag: "mistral:7b", label: "Mistral 7B", download_mb: 4170,
        good_for: "A well-liked all-rounder that has aged well.", caveat: None },
    // HERMES — added 2026-08-27 after a request asking why it was missing:
    // it was popular and had been named in an earlier list, but not the
    // marketplace one.
    //
    // The earlier list was this one, not the marketplace's — providers.rs
    // records a request for "gemini for cloud, or claude, or local hermes
    // agent". That matters, because Hermes is a MODEL and not an agent
    // framework — it belongs here, in the things a local brain can run, rather
    // than beside CrewAI and LangGraph. Putting it in the marketplace would
    // have filed a brain under tools.
    Model { tag: "hermes3:8b", label: "Hermes 3 8B", download_mb: 4445,
        good_for: "Follows instructions closely and stays in character.",
        caveat: None },
    Model { tag: "llama3.1:8b", label: "Llama 3.1 8B", download_mb: 4693,
        good_for: "The one most people look for first, and a sensible default.",
        caveat: None },
    Model { tag: "deepseek-r1:8b", label: "DeepSeek-R1 8B", download_mb: 4983,
        good_for: "Works through a problem step by step before answering.",
        caveat: Some("It thinks out loud, so answers take noticeably longer.") },
    Model { tag: "qwen2.5vl:7b", label: "Qwen 2.5 VL 7B (sees images)", download_mb: 5693,
        good_for: "Reads screenshots and photographs, not just text.",
        caveat: Some("Vision costs memory. Pick this only if you need pictures read.") },
    Model { tag: "phi4:14b", label: "Phi-4 14B", download_mb: 8634,
        good_for: "Strong reasoning without needing a very large card.", caveat: None },
    Model { tag: "gpt-oss:20b", label: "GPT-OSS 20B", download_mb: 13154,
        good_for: "Longer reasoning, and it holds a thread better.", caveat: None },
    Model { tag: "gemma3:27b", label: "Gemma 3 27B", download_mb: 16591,
        good_for: "The most capable general model that still fits one big card.",
        caveat: None },
    Model { tag: "qwen3-coder:30b", label: "Qwen3 Coder 30B", download_mb: 17697,
        good_for: "Code. Reads a file and changes it without losing the shape.",
        caveat: None },
];

/// How a model sits on a given machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Fit {
    /// Fits on the card with headroom left. This is the one to recommend.
    Fast,
    /// Will run on the processor. Honest word, not a hidden downgrade.
    Slow,
    /// Neither. Shown anyway, greyed, with the reason — a hidden option looks
    /// like a missing feature, and the reason is what teaches the next choice.
    TooBig,
    /// We could not measure enough to say. NOT a synonym for TooBig.
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Recommendation {
    pub tag: String,
    pub label: String,
    pub fit: Fit,
    pub download_mb: u64,
    pub good_for: String,
    /// One sentence a person can act on: why this fits, or why it does not.
    pub why: String,
    pub caveat: Option<String>,
    /// True when the download will not fit in the space left on disk.
    pub needs_disk: bool,
}

/// LEAVE A QUARTER OF THE CARD ALONE.
///
/// Same ceiling `ask-local.sh` enforces on the box this was written on, and it
/// was bought with an outage: a machine here went down after something asked a
/// 24 GB card for 30, and a saturated GPU is not a slow GPU — it needs somebody
/// to walk over and hold the power button. The desktop and the driver need
/// memory too, so a model that exactly fills VRAM is a model that wedges the
/// display it is supposed to be drawing into.
const VRAM_HEADROOM: f64 = 0.75;

/// Same reasoning one level out: an operating system that has given every byte
/// to a language model is an operating system that swaps.
const RAM_HEADROOM: f64 = 0.80;

/// Same split as `needs_vram_mb_for` above and for the same reason: the sizing
/// RULE lives here once, on raw megabytes, so an installed model (a runtime
/// `u64`) and a catalog entry (a `'static` `Model`) are judged by the same
/// arithmetic instead of two copies that can quietly disagree.
pub fn classify_size(download_mb: u64, p: &HardwareProfile) -> Fit {
    let vram = p.best_vram_mb();
    let ram = p.ram_mb;

    if let Some(v) = vram {
        if (v as f64 * VRAM_HEADROOM) >= needs_vram_mb_for(download_mb) as f64 {
            return Fit::Fast;
        }
    }
    if let Some(r) = ram {
        if (r as f64 * RAM_HEADROOM) >= needs_ram_mb_for(download_mb) as f64 {
            return Fit::Slow;
        }
    }
    // Nothing measured at all: say so. Guessing here is exactly the premise
    // error this whole file exists to avoid.
    if vram.is_none() && ram.is_none() {
        return Fit::Unknown;
    }
    Fit::TooBig
}

pub fn classify(m: &Model, p: &HardwareProfile) -> Fit {
    classify_size(m.download_mb, p)
}

pub fn recommend(p: &HardwareProfile) -> Vec<Recommendation> {
    let mut out: Vec<Recommendation> = CATALOG
        .iter()
        .map(|m| {
            let fit = classify(m, p);
            let why = match fit {
                Fit::Fast => match p.best_vram_mb() {
                    Some(v) => format!(
                        "Fits on your {} GB card with room to spare.",
                        (v as f64 / 1024.0).round() as u64
                    ),
                    None => "Fits on your graphics card.".to_string(),
                },
                Fit::Slow => "Too big for the graphics card, but it will run on the \
                              processor. Expect it to be several times slower."
                    .to_string(),
                Fit::TooBig => "Bigger than this machine can hold.".to_string(),
                Fit::Unknown => "Could not measure this machine's memory, so this is \
                                 not a recommendation either way."
                    .to_string(),
            };
            Recommendation {
                tag: m.tag.to_string(),
                label: m.label.to_string(),
                fit,
                download_mb: m.download_mb,
                good_for: m.good_for.to_string(),
                why,
                caveat: m.caveat.map(String::from),
                needs_disk: p.disk_free_mb.is_some_and(|d| d < m.download_mb),
            }
        })
        .collect();

    // Best fit first, then the biggest model within that fit — on a machine
    // that can hold more, the better model is the right default. Ordering is
    // stable so the list does not reshuffle between two identical probes.
    out.sort_by_key(|r| {
        let rank = match r.fit {
            Fit::Fast => 0,
            Fit::Slow => 1,
            Fit::Unknown => 2,
            Fit::TooBig => 3,
        };
        (rank, std::cmp::Reverse(r.download_mb))
    });
    out
}

// --- READING THE MACHINE ----------------------------------------------------
//
// Every parser below is separate from the command that feeds it, for one
// reason: a parser can be tested and a `Command` cannot. The disconnect bug on
// 2026-08-27 shipped precisely because its decision lived inside a function
// that shelled out, so there was no way to exercise it without the hardware.

/// `nvidia-smi --query-gpu=name,memory.total --format=csv,noheader,nounits`
pub fn parse_nvidia_smi(out: &str) -> Vec<Gpu> {
    out.lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| {
            let (name, mem) = l.rsplit_once(',')?;
            Some(Gpu {
                name: name.trim().to_string(),
                // A card that reports an unparseable size is a card whose size
                // we do not know. Not zero — zero would read as "no memory".
                vram_mb: mem.trim().parse::<u64>().ok(),
                source: "nvidia-smi".to_string(),
            })
        })
        .collect()
}

/// `MemTotal:       65780208 kB` out of /proc/meminfo.
pub fn parse_meminfo_mb(out: &str) -> Option<u64> {
    out.lines()
        .find(|l| l.starts_with("MemTotal:"))
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|kb| kb.parse::<u64>().ok())
        .map(|kb| kb / 1024)
}

/// PowerShell `(Get-CimInstance Win32_ComputerSystem).TotalPhysicalMemory`
pub fn parse_bytes_mb(out: &str) -> Option<u64> {
    out.trim().parse::<u64>().ok().map(|b| b / 1024 / 1024)
}

/// `df -Pk <dir>` — the POSIX output, whose fourth column is free 1K blocks.
///
/// `-P` is not decoration. Without it a long device name wraps onto its own
/// line and the columns move, which is the classic way a df parser reads the
/// mount point as a number and reports nonsense free space.
pub fn parse_df_kb_mb(out: &str) -> Option<u64> {
    out.lines()
        .nth(1)?
        .split_whitespace()
        .nth(3)?
        .parse::<u64>()
        .ok()
        .map(|kb| kb / 1024)
}

fn run(cmd: &str, args: &[&str]) -> Option<String> {
    // THE SAME FLASHING WINDOWS AS `ssh_run` BELOW, ON THE MORE COMMON PATH.
    // Reported 2026-08-27 against the REMOTE probe: each console command it
    // shells out to briefly pops its own console window on Windows — six
    // commands, six windows. `ssh_run` was fixed the same day. This is its
    // twin for the LOCAL machine, called by `probe_hardware` (nvidia-smi,
    // then two separate `powershell` calls for RAM and disk on Windows) every
    // time the brain-setup wizard runs, which is why the same flashing showed
    // up here too: it was never given the same fix `ssh_run` got, so the twin
    // kept flashing.
    let mut cmd = Command::new(cmd);
    cmd.args(args);
    crate::hide_console(&mut cmd);
    let out = cmd.output().ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).to_string())
}

/// What is actually in front of the user, asked at click time.
#[tauri::command(async)]
pub fn probe_hardware() -> HardwareProfile {
    let mut p = HardwareProfile {
        os: std::env::consts::OS.to_string(),
        ..Default::default()
    };

    // GPU. nvidia-smi ships with the driver on every platform, so it is the
    // one probe that reads the same everywhere.
    match run(
        "nvidia-smi",
        &[
            "--query-gpu=name,memory.total",
            "--format=csv,noheader,nounits",
        ],
    ) {
        Some(o) => p.gpus = parse_nvidia_smi(&o),
        None => p.notes.push(
            "Could not run nvidia-smi, so the graphics card was not measured. \
             That is not the same as there being no card — if you have one, the \
             recommendations below are more cautious than they need to be."
                .into(),
        ),
    }

    // RAM and cores.
    p.cores = std::thread::available_parallelism().ok().map(|n| n.get());
    p.ram_mb = match std::env::consts::OS {
        "linux" => std::fs::read_to_string("/proc/meminfo")
            .ok()
            .and_then(|s| parse_meminfo_mb(&s)),
        "windows" => run(
            "powershell",
            &[
                "-NoProfile",
                "-Command",
                "(Get-CimInstance Win32_ComputerSystem).TotalPhysicalMemory",
            ],
        )
        .and_then(|s| parse_bytes_mb(&s)),
        "macos" => run("sysctl", &["-n", "hw.memsize"]).and_then(|s| parse_bytes_mb(&s)),
        _ => None,
    };
    if p.ram_mb.is_none() {
        p.notes
            .push("Could not read how much memory this machine has.".into());
    }

    // Free space where Ollama actually puts the weights, not on the system
    // drive by assumption. A 18 GB download that runs out of disk at 90% is a
    // worse experience than being told up front it will not fit.
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".into());
    p.disk_free_mb = match std::env::consts::OS {
        "windows" => run(
            "powershell",
            &[
                "-NoProfile",
                "-Command",
                "(Get-PSDrive -Name (Split-Path -Qualifier $env:USERPROFILE).TrimEnd(':')).Free",
            ],
        )
        .and_then(|s| parse_bytes_mb(&s)),
        _ => run("df", &["-Pk", &home]).and_then(|s| parse_df_kb_mb(&s)),
    };
    if p.disk_free_mb.is_none() {
        p.notes
            .push("Could not read how much disk space is free, so download sizes \
                   below are not checked against it."
                .into());
    }

    p
}

// --- THE MACHINE THAT IS NOT THIS ONE ---------------------------------------
//
// Written 2026-08-27 in response to a request to support probing a remote
// machine as well as the local one — ask for local or remote, and if remote,
// ask for the address and credentials, then read that machine's hardware.
//
// **THE PROBE IS SEPARATE FROM THE TRANSPORT, and that split is the whole
// design.** `profile_from_probe` is handed a closure that runs one command
// somewhere and returns its output; it does not know or care whether that is
// SSH, a local shell, or a test. So the thing worth testing — did we read the
// machine correctly, and did we say so honestly when we could not — is
// testable with no network, no second machine, and no credentials. Only the
// closure needs a real connection, and a closure is a much smaller thing to be
// unsure about than a probe.
//
// This is the same lesson as the disconnect bug: a decision buried inside a
// function that talks to the outside world is a decision nobody can test.
//
// **READ-ONLY, BY CONSTRUCTION.** The rule for the remote path is
// *probe only, never install*: helloim.ai does not change a machine it does not
// live on. Every command below reads. If Ollama is missing on the far side,
// that is reported with the exact line for the user to run themselves — we do
// not run it for them.

/// Which family of commands the far machine understands.
///
/// **THIS IS NOT A DETAIL — A LIKELY REMOTE TARGET IS A WINDOWS BOX.**
/// Measured 2026-08-27 over SSH to a Windows machine on the LAN: `nvidia-smi`
/// answers perfectly, and `nproc` comes back *"'nproc' is not recognized as an
/// internal or external command"*. A Unix-only probe would therefore have
/// reported that machine's RTX 5080 and then claimed its memory could not be
/// measured, which is true-but-useless when one PowerShell call would have
/// read it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RemoteOs {
    Unix,
    Windows,
}

/// The commands the remote probe runs. Every one is read-only, and they are
/// listed here rather than inline so that claim can be checked at a glance
/// instead of taken on trust — there is a test that greps them.
pub fn remote_probe_commands(os: RemoteOs) -> Vec<(&'static str, &'static str)> {
    // nvidia-smi is the same everywhere, which is why the GPU — the field that
    // actually decides the recommendation — survives even when the OS guess is
    // wrong.
    let gpu = (
        "gpu",
        "nvidia-smi --query-gpu=name,memory.total --format=csv,noheader,nounits",
    );
    let ollama = ("ollama", "ollama --version");
    match os {
        RemoteOs::Unix => vec![
            gpu,
            ("ram", "cat /proc/meminfo"),
            ("cores", "nproc"),
            ("disk", "df -Pk $HOME"),
            ollama,
        ],
        RemoteOs::Windows => vec![
            gpu,
            ("ram", "powershell -NoProfile -Command \"(Get-CimInstance Win32_ComputerSystem).TotalPhysicalMemory\""),
            ("cores", "powershell -NoProfile -Command \"(Get-CimInstance Win32_ComputerSystem).NumberOfLogicalProcessors\""),
            ("disk", "powershell -NoProfile -Command \"(Get-PSDrive -Name C).Free\""),
            ollama,
        ],
    }
}

/// Did we fail to REACH that machine, or reach it and fail to run something?
///
/// **THE DISTINCTION A REAL FIRST ATTEMPT NEEDED — 2026-08-27, trying a
/// remote host on the LAN.** SSH refused the connection with `Permission
/// denied (publickey,password)` because the calling machine had no key on the
/// machine it was calling — no console flash worth mentioning, no password
/// prompt, and no obvious sign anything had gone wrong at all. The probe
/// turned that into six separate "could not measure X" notes and no statement
/// that the connection had never happened at all — six symptoms of one cause,
/// none of them naming it.
///
/// A connection that was refused and a machine missing `nvidia-smi` are
/// completely different problems with completely different fixes, and the user
/// is owed the one sentence that says which they have.
pub fn is_unreachable(err: &str) -> bool {
    let e = err.to_lowercase();
    [
        "permission denied",
        "connection refused",
        "connection closed",
        "no route to host",
        "could not resolve",
        "name or service not known",
        "timed out",
        "host key verification failed",
        "connection timed out",
        "network is unreachable",
        "could not start ssh",
    ]
    .iter()
    .any(|m| e.contains(m))
}

/// What to do about it, in the user's own next action.
///
/// The chosen flow for this is "password once, then install a key", and that
/// needs an SSH library this crate deliberately does not have (no async
/// runtime — see the module notes). So the honest interim is not a password
/// box that does nothing: it is the exact command that fixes it, the same way
/// the remote path already hands over an install line rather than running it.
pub fn key_hint(user: &str, host: &str, port: u16) -> String {
    let p = if port == 22 {
        String::new()
    } else {
        format!(" -p {port}")
    };
    // **NAME THE MACHINE, NOT "THIS COMPUTER" — a real host-mixup bug, found
    // 2026-08-27.** This text used to say "run this ONCE from this computer".
    // Read on the Windows box helloim.ai is running the wizard from, but with
    // two machines in play and one of them the subject of the sentence, "this
    // computer" is genuinely ambiguous — a reader can reasonably run it on the
    // OTHER machine instead. On Linux the PowerShell line is meaningless, so
    // the pipe appended nothing — while the ssh connection itself SUCCEEDED
    // and printed no error. It looked like it had worked.
    //
    // The failure mode is the worst kind: an instruction that half-executes
    // and reports nothing. So the sentence now names both machines and says
    // which one is which, rather than relying on the reader's idea of "this".
    //
    // NAME THE SHELL for the same reason. `$env:` and `type` are PowerShell,
    // and pasting them into cmd.exe fails in a way that looks like the
    // instruction was wrong rather than typed into the wrong window.
    //
    // AND THE KEYGEN LINE IS CONDITIONAL ON PURPOSE. Run unguarded against an
    // existing key it prompts to overwrite, and somebody following an
    // instruction from an error dialog will press y — destroying every other
    // machine's access from this one to fix access to this one. `-f` with a
    // Test guard makes it a no-op when a key is already there.
    format!(
        "{host} refused the login. helloim.ai signs in with an SSH key and never \
         asks for a password, and {host} has not been told to trust this \
         computer yet.\n\n\
         RUN THESE ON THE COMPUTER HELLOIM.AI IS RUNNING ON — the one you are \
         reading this on right now, NOT on {host}. Open PowerShell here and \
         run both lines. The second asks for {host}'s password once; helloim.ai \
         stores nothing either way:\n\n\
         if (-not (Test-Path \"$HOME\\.ssh\\id_ed25519\")) {{ ssh-keygen -t ed25519 \
         -f \"$HOME\\.ssh\\id_ed25519\" -N '\"\"' }}\n\
         type \"$HOME\\.ssh\\id_ed25519.pub\" | ssh{p} {user}@{host} \
         \"mkdir -p ~/.ssh && cat >> ~/.ssh/authorized_keys\"\n\n\
         Then press Look at that machine again.\n\n\
         (On macOS or Linux the second line is just: ssh-copy-id{p} {user}@{host})"
    )
}

/// Ask the far machine what it is, rather than assuming.
pub fn detect_remote_os<F>(run_remote: &F) -> RemoteOs
where
    F: Fn(&str) -> Result<String, String>,
{
    match run_remote("uname -s") {
        Ok(o) if !o.trim().is_empty() => RemoteOs::Unix,
        // `uname` failing is what a Windows shell does with it. Defaulting to
        // Windows here is safe in the way that matters: nvidia-smi is in both
        // sets, so the field the recommendation hangs on is read either way.
        _ => RemoteOs::Windows,
    }
}

/// OpenSSH prints client-side advisories to stderr, and they are not the error.
///
/// **THE FAULT THIS FIXES, found by running it against a real Windows box on
/// the LAN:** a failing
/// command came back with three lines of *"WARNING: connection is not using a
/// post-quantum key exchange algorithm"* ahead of the one line that said what
/// actually went wrong. Handing that to a user as the reason their setup
/// failed is an error message that reports without informing — the exact thing
/// the interface rules here forbid.
pub fn clean_ssh_stderr(err: &str) -> String {
    err.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with("**"))
        .collect::<Vec<_>>()
        .join(" ")
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteProfile {
    pub hardware: HardwareProfile,
    /// Whether Ollama is already there. None means we could not tell.
    pub ollama_present: Option<bool>,
    /// What the user should run THEMSELVES if it is missing. We never run it.
    pub install_hint: Option<String>,
    /// Whether we got in at all. `false` means nothing below was measured
    /// because the connection never happened — a completely different problem
    /// from a machine that answered and lacked a command.
    #[serde(default)]
    pub reachable: bool,
    /// The one sentence, when `reachable` is false: what went wrong and the
    /// exact command that fixes it. Six vague notes are not an error message.
    pub problem: Option<String>,
    /// Sized against THAT machine, never this one.
    ///
    /// **THIS FIELD EXISTS BECAUSE THE ALTERNATIVE WAS A LIE.** The screen
    /// needs a recommendation next to the hardware it just measured, and the
    /// only recommendation available was `brain_advice`, which measures the
    /// box helloim.ai is running on. Reusing it would have put "fits on your 24 GB
    /// card" beside a remote machine with 8 GB. The recommendation belongs to
    /// the profile it was computed from, so it travels with it.
    pub models: Vec<Recommendation>,
}

/// Build a profile from whatever the runner can tell us about a machine.
///
/// The runner returns Err for "that command did not work", which is different
/// from returning empty output. Both end up as an unmeasured field with a
/// sentence attached, never as a zero.
pub fn profile_from_probe<F>(os_hint: &str, run_remote: F) -> RemoteProfile
where
    F: Fn(&str) -> Result<String, String>,
{
    // KNOCK ONCE BEFORE ASKING ANYTHING. If the door does not open, every
    // question after it produces its own little failure note and the user is
    // left assembling one cause out of six symptoms. `uname` is the cheapest
    // knock and it is needed anyway to pick the command table.
    if let Err(e) = run_remote("uname -s") {
        if is_unreachable(&e) {
            return RemoteProfile {
                hardware: HardwareProfile {
                    os: String::new(),
                    ..Default::default()
                },
                ollama_present: None,
                install_hint: None,
                reachable: false,
                problem: Some(e),
                // No recommendations. Sizing a model against a machine we
                // never reached would be a confident answer about nothing.
                models: vec![],
            };
        }
    }

    let remote_os = detect_remote_os(&run_remote);
    let table = remote_probe_commands(remote_os);
    let mut p = HardwareProfile {
        os: match remote_os {
            RemoteOs::Unix => os_hint.to_string(),
            RemoteOs::Windows => "windows".to_string(),
        },
        ..Default::default()
    };
    let cmd = |k: &str| table.iter().find(|(n, _)| *n == k).unwrap().1;

    match run_remote(cmd("gpu")) {
        Ok(o) if !o.trim().is_empty() => p.gpus = parse_nvidia_smi(&o),
        _ => p.notes.push(
            "Could not run nvidia-smi on that machine, so its graphics card was \
             not measured. That is not the same as it having no card."
                .into(),
        ),
    }
    // Each OS reports these in its own units, and mixing the parsers up is how
    // a machine with 32 GB gets recorded as having 32 MB.
    p.ram_mb = run_remote(cmd("ram")).ok().and_then(|o| match remote_os {
        RemoteOs::Unix => parse_meminfo_mb(&o),
        RemoteOs::Windows => parse_bytes_mb(&o),
    });
    if p.ram_mb.is_none() {
        p.notes
            .push("Could not read that machine's memory size.".into());
    }
    p.cores = run_remote(cmd("cores"))
        .ok()
        .and_then(|o| o.trim().parse::<usize>().ok());
    p.disk_free_mb = run_remote(cmd("disk")).ok().and_then(|o| match remote_os {
        RemoteOs::Unix => parse_df_kb_mb(&o),
        RemoteOs::Windows => parse_bytes_mb(&o),
    });
    if p.disk_free_mb.is_none() {
        p.notes.push(
            "Could not read free disk space on that machine, so download sizes \
             are not checked against it."
                .into(),
        );
    }

    // Present is a fact; absent is only a fact if the command ran and said so.
    // A transport failure must not be reported as "Ollama is not installed".
    // Present is a fact. Absent is a fact ONLY when the shell said so in as
    // many words — measured against a real Windows box on the LAN, where a
    // missing binary exits non-zero with "is not recognized" on stderr, so
    // treating every error as "unknown" meant a Windows box could never be
    // offered the install line.
    // Anything else (link down, auth refused) stays genuinely unknown.
    let ollama_present = match run_remote(cmd("ollama")) {
        Ok(o) if o.to_lowercase().contains("version") => Some(true),
        Ok(_) => Some(false),
        Err(e) => {
            let e = e.to_lowercase();
            if e.contains("not recognized") || e.contains("command not found") {
                Some(false)
            } else {
                None
            }
        }
    };
    let install_hint = match (ollama_present, remote_os) {
        (Some(false), RemoteOs::Unix) => {
            Some("curl -fsSL https://ollama.com/install.sh | sh".to_string())
        }
        // Piping a shell script into a Windows box is not an instruction, it is
        // a wrong answer that looks like one.
        (Some(false), RemoteOs::Windows) => {
            Some("winget install Ollama.Ollama".to_string())
        }
        _ => None,
    };

    let models = recommend(&p);
    RemoteProfile {
        hardware: p,
        ollama_present,
        install_hint,
        reachable: true,
        problem: None,
        models,
    }
}

/// Run one command on another machine over the SSH already on this system.
///
/// **KEY AUTH ONLY, AND `BatchMode=yes` IS WHAT ENFORCES IT.** With batch mode
/// on, ssh fails rather than prompting, so this can never block the app waiting
/// on a password nobody can see. The requested password bootstrap -- give it
/// once, install a key, discard it -- is NOT here yet and deliberately so:
/// see the note in the module docs about what it costs to add.
pub fn ssh_run(host: &str, user: &str, port: u16, cmd: &str) -> Result<String, String> {
    let target = format!("{user}@{host}");
    let mut c = Command::new("ssh");
    // **THE FLASHING WINDOWS, found 2026-08-27.** Every other place in this
    // app that spawns a process calls this, and this function was the only
    // one that did not. On Windows a console program pops a black console
    // window as it starts and exits, so one probe — six commands — is six
    // windows flashing at somebody who is only trying to read their own
    // hardware.
    crate::hide_console(&mut c);
    let out = c
        .args([
            "-o", "BatchMode=yes",
            "-o", "ConnectTimeout=10",
            // Reading a machine's specs is not a reason to permanently trust a
            // host key, but refusing outright makes first contact impossible.
            // accept-new trusts a NEW host and still screams if a known one
            // changed, which is the case that actually matters.
            "-o", "StrictHostKeyChecking=accept-new",
            "-p", &port.to_string(),
            &target,
            cmd,
        ])
        .output()
        .map_err(|e| format!("Could not start ssh: {e}"))?;
    if !out.status.success() {
        let why = clean_ssh_stderr(&String::from_utf8_lossy(&out.stderr));
        return Err(if why.is_empty() {
            "That machine refused the command.".into()
        } else {
            why
        });
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

#[tauri::command(async)]
pub fn probe_remote(host: String, user: String, port: Option<u16>) -> RemoteProfile {
    let port = port.unwrap_or(22);
    let mut r = profile_from_probe("linux", |c| ssh_run(&host, &user, port, c));
    if !r.reachable {
        // ssh's own words first — it knows why far better than we do — then
        // the fix, which is the part the user actually needs. The raw line is
        // kept because "Permission denied (publickey)" and "No route to host"
        // send somebody to completely different places.
        let raw = r.problem.clone().unwrap_or_default();
        r.problem = Some(format!("{}\n\n{raw}", key_hint(&user, &host, port)));
    }
    r
}

// --- WHAT THE RECOMMENDED SCREEN OFFERS -------------------------------------
//
// Raised 2026-08-28: Claude was missing from the recommended setup screen,
// and the request was for a Cloud AI section listing Claude alongside other
// cloud brains, plus a Local AI section for several local install options.
//
// THE REPORT WAS RIGHT THAT CLAUDE WAS MISSING, AND THE REASON IS A CATEGORY
// SPLIT THAT ONLY MADE SENSE FROM THE INSIDE. The Recommended grid lists MCP
// connectors — Gmail, Drive, Cloudflare — things the assistant can USE. The
// brain that does the thinking was configured somewhere else entirely, so the
// one connection without which nothing works was the one connection that
// screen never mentioned.
//
// **IT WAS SIX AND SIX FOR ONE DAY.** Later the same day the direction
// changed: take out all cloud-based AI except Claude, and leave the local
// option and local check. So it is ONE AND ONE — Claude, and Ollama. Both
// headings stay, and that is a deliberate choice rather than an oversight:
// what the two sections teach is the distinction that matters most, which is
// whether the conversation leaves the machine. Collapsing them into a single
// "AI" list would hide exactly that, to make a screen look fuller.
//
// **WHAT WENT, so it can come back rather than be re-researched:** OpenAI,
// Google Gemini, Groq, DeepSeek and Mistral from CLOUD_AI, and LM Studio, Jan,
// llama.cpp, GPT4All and LocalAI from LOCAL_AI. The whole `BrainPreset`
// mechanism went with them — `BRAIN_PRESETS`, the `brain_presets` command and
// the "Other brains you can connect" picker in the sheet — because with the
// cloud kinds gone it was a list of one, pointing at a wizard on the same
// panel. **Every cloud URL in it had been REQUESTED before being written
// down** (each `/models` answered 401, which is the right answer: the endpoint
// is real and wants a key; Google's answers 404 unauthenticated and 400 with a
// bearer token, so it took the second request to confirm). That verification
// is worth more than the constants and it is why they should be restored from
// `3a328ff` rather than retyped from memory — two invented model tags shipped
// out of this very file once, and a plausible-looking base URL is the same
// failure one layer up.

#[derive(Debug, Clone, Serialize)]
pub struct AiOption {
    pub id: &'static str,
    pub name: &'static str,
    /// Single letter for the tile, same lettermark treatment the app grid uses
    /// rather than traced logos.
    pub letter: &'static str,
    pub colour: &'static str,
    /// "claude" | "local" — what providers.rs will route. Checked by a test
    /// against `ROUTABLE_KINDS`, because a tile naming a kind the backend
    /// cannot route is a card that opens a form the backend then refuses.
    pub kind: &'static str,
    /// Empty for Claude, whose setup is its own flow rather than a base URL.
    pub base_url: &'static str,
    pub note: &'static str,
    /// Where to get it, for the local ones. Empty for cloud.
    pub download_url: &'static str,
}

pub const CLOUD_AI: &[AiOption] = &[
    // CLAUDE FIRST AND ALWAYS. It is the brain the product ships with, and the
    // binary every other brain runs through — a local or compatible provider is
    // environment variables on the same spawned process.
    AiOption { id: "claude", name: "Claude", letter: "C", colour: "#d97757",
        kind: "claude", base_url: "",
        // "SHIPS WITH" WAS WRONG AND IT MATTERED — corrected 2026-08-29 after a
        // compliance review. helloim.ai ships NO Claude Code binary: the installer
        // payload is helloim.ai.exe, WebView2, DirectML and the voice model, and
        // `install.rs` downloads Claude Code from Anthropic at first run.
        // "Ships with" reads as bundled redistribution, which is the one thing
        // this product deliberately does not do — and Anthropic's own terms
        // permit saying a product "runs Claude Code" while treating bundling as
        // a different act. Every other string in the app was already accurate;
        // this one and its twin in the sign-in sheet were the odd ones out.
        note: "Use your Claude account. Install Claude Code and sign in — no API key needed.",
        download_url: "" },
    // OPENAI, BACK ON THE SCREEN 2026-08-29 — requested five times before it
    // was added. It rides the `openai-compatible` kind through helloim.ai's own
    // translator (`adapter.rs`); the gate is `providers::OPENAI_ENABLED` and
    // this tile must move with it in the same change, or the backend routes a
    // kind no screen offers, or a screen offers a kind the backend refuses.
    //
    // THE NOTE SAYS "YOUR OWN KEY" BECAUSE THAT IS THE DIFFERENCE THAT MATTERS
    // AT THE MOMENT OF CHOOSING. Claude is install-and-sign-in; this one needs
    // them to go and fetch something from another company's dashboard first,
    // and finding that out after picking is the kind of small betrayal that
    // makes a person distrust the next screen too.
    AiOption { id: "openai", name: "OpenAI", letter: "O", colour: "#10a37f",
        kind: "openai-compatible", base_url: "https://api.openai.com/v1",
        note: "Your own OpenAI key. Connects directly, without Claude; OpenAI bills your usage.",
        download_url: "" },
    // GEMINI, ADDED 2026-09-02 — same request as OpenAI's, same answer:
    // whether helloim.ai could connect to "each ai a user uses" (2026-08-27,
    // the ruling that justified building `adapter.rs` at all — see that
    // file's header) was raised again for Gemini specifically, to ask whether
    // it needed its own path. It does not.
    //
    // **RIDES THE EXACT SAME `openai-compatible` KIND AS THE TILE ABOVE, NOT
    // A NEW ONE.** Verified live, not assumed from Google's docs alone:
    // Google's own OpenAI-compatibility endpoint
    // (https://generativelanguage.googleapis.com/v1beta/openai/) answers
    // `/chat/completions` in the identical shape `adapter.rs` already reads —
    // a real key, a real tool-call request, a real
    // `choices[0].message.tool_calls[].function.{name,arguments}` back,
    // `finish_reason: "tool_calls"`, same as OpenAI's own. So this is not a
    // second translator, or a second `providers.rs` kind, or a new arm in
    // `apply_env` — it is one more row of the leverage `adapter.rs`'s header
    // already claims ("Gemini is one row of it"), proven true for the first
    // time by actually calling it.
    //
    // `is_reasoning_model`'s prefix list does not match any Gemini name, so
    // a Gemini row always takes the plain `max_tokens` / sampling-params path
    // — verified live: Gemini's endpoint accepts `max_tokens` (not
    // `max_completion_tokens`) and a real `temperature`/`top_p` without
    // complaint on the same request shape this translator already sends.
    //
    // **ONE OPEN GAP, NOT SILENTLY PAPERED OVER:** Gemini's own answer (see
    // `desktop/provider-auth-answers.md`) names an aggressive default safety
    // filter that can return an empty candidate list with `finishReason:
    // SAFETY` instead of a message. `openai_to_anthropic_response` today
    // reads that shape as the generic "The provider replied, but with no
    // message in it." — honest and not a hang, but it does not NAME safety
    // as the reason, because reproducing a live safety block to verify the
    // exact field name was judged not worth spending against a real account
    // for one error string. Worth a follow-up once someone can trigger it.
    AiOption { id: "gemini", name: "Gemini", letter: "G", colour: "#4285f4",
        kind: "openai-compatible", base_url: "https://generativelanguage.googleapis.com/v1beta/openai/",
        note: "Your own Gemini key, from Google AI Studio. On the free tier Google may train \
               on your conversations — turn on pay-as-you-go billing there for privacy.",
        download_url: "" },
    // OPENROUTER, ADDED 2026-09-05 — a NAMED choice for a path that already
    // worked. OpenRouter's API is OpenAI-shaped, so before this row a user
    // could already reach it by taking the OpenAI tile and overriding its
    // Advanced base URL to `https://openrouter.ai/api/v1`. That was proven on
    // this box (the engine round-trips it through `adapter.rs`), and this tile
    // is the difference between "possible if you know the trick" and "offered".
    //
    // **RIDES THE EXACT SAME `openai-compatible` KIND AS OPENAI AND GEMINI, NOT
    // A NEW ONE, AND NOT `anthropic-compatible`.** OpenRouter also speaks
    // Anthropic's native `/v1/messages` (that is how the real `claude` CLI
    // drives it for our own failover), but the anthropic-skin kind stays gated
    // by a locked product decision — and it is not needed here, because the
    // OpenAI-compatible wire is agnostic-sufficient: one key, one base URL, the
    // translator `adapter.rs` already reads. Same routing, same gate
    // (`providers::OPENAI_ENABLED`), same translator. Nothing new in
    // `providers.rs` or `apply_env`.
    //
    // **THE DEFAULT MODEL IS `openai/gpt-6-astra`, PROVEN LIVE 2026-09-05
    // AGAINST THE REAL KEY** — base + model returned real text with
    // `finish_reason: stop`, billed on OpenRouter (`is_byok:false`), so the
    // call went through rather than being answered locally. Chosen because it
    // is the one OpenRouter model verified end to end on this box; a cheaper
    // default I only reasoned about would be a tile whose default I could not
    // stand behind. It is a reasoning model, but `is_reasoning_model`'s prefix
    // list (o1/o3/o4/gpt-5) does not match `gpt-6-astra`, so this row takes the
    // plain `max_tokens` / sampling-params path — which is correct HERE,
    // because OpenRouter accepts `max_tokens` universally (unlike OpenAI's own
    // reasoning endpoints, which 400 it). The user can change the model; the
    // point of a default is that the first press answers.
    AiOption { id: "openrouter", name: "OpenRouter", letter: "R", colour: "#6566f1",
        kind: "openai-compatible", base_url: "https://openrouter.ai/api/v1",
        note: "Your own OpenRouter key — one key, hundreds of models from many \
               vendors. Connects directly, without Claude; pricing depends on your model.",
        download_url: "" },
];

// FIVE MORE STOOD HERE UNTIL 2026-08-28 — LM Studio, Jan, llama.cpp, GPT4All
// and LocalAI — and they were LOCAL tools, which is the part of the removal
// that genuinely costs something. They went because all five rode the
// `openai-compatible` kind, and a kind that accepts any base URL is a cloud
// door whatever a particular person points it at. Ollama is the local path
// that was named, and it is the one helloim.ai can actually measure a machine
// for.
//
// THE PORT IS OLLAMA'S DOCUMENTED DEFAULT AND IT IS ALSO `save_provider`'S OWN
// DEFAULT, deliberately the same string in both places: a tile that seeds an
// address the backend would have filled in differently is a bug waiting for
// somebody to press Test. (It read `.../v1` until 2026-08-28 — harmless only
// because this tile short-circuits into the wizard and never used it, but the
// Anthropic path appends `/v1/messages`, so that value would have produced
// `/v1/v1/messages`, a 404, and an error telling the user to update software
// that was already new enough.)
pub const LOCAL_AI: &[AiOption] = &[
    AiOption { id: "ollama", name: "Ollama", letter: "O", colour: "#6fd08c",
        kind: "local", base_url: "http://127.0.0.1:11434",
        note: "The one helloim.ai knows best — it measures your machine and only offers models that fit.",
        download_url: "https://ollama.com/download" },
];

#[derive(Debug, Clone, Serialize)]
pub struct RecommendedAi {
    pub cloud: Vec<AiOption>,
    pub local: Vec<AiOption>,
}

#[tauri::command(async)]
pub fn recommended_ai() -> RecommendedAi {
    RecommendedAi {
        cloud: CLOUD_AI.to_vec(),
        local: LOCAL_AI.to_vec(),
    }
}

/// Everything the setup screen needs in one call: what the machine is, and what
/// to put on it. One command rather than two so the recommendations can never
/// be rendered against a different probe than the one shown beside them.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrainAdvice {
    pub hardware: HardwareProfile,
    pub models: Vec<Recommendation>,
}

#[tauri::command(async)]
pub fn brain_advice() -> BrainAdvice {
    let hardware = probe_hardware();
    let models = recommend(&hardware);
    BrainAdvice { hardware, models }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile(vram: Option<u64>, ram: Option<u64>) -> HardwareProfile {
        HardwareProfile {
            os: "linux".into(),
            cores: Some(24),
            ram_mb: ram,
            gpus: vram
                .map(|v| {
                    vec![Gpu {
                        name: "Test".into(),
                        vram_mb: Some(v),
                        source: "test".into(),
                    }]
                })
                .unwrap_or_default(),
            disk_free_mb: Some(500_000),
            notes: vec![],
        }
    }

    #[test]
    fn it_reads_what_nvidia_smi_really_prints() {
        // Verbatim shape from the 5090 in this box.
        let got = parse_nvidia_smi("NVIDIA GeForce RTX 5090 Laptop GPU, 24463\n");
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].name, "NVIDIA GeForce RTX 5090 Laptop GPU");
        assert_eq!(got[0].vram_mb, Some(24463));
    }

    #[test]
    fn two_cards_are_two_rows_and_never_one_big_one() {
        let p = HardwareProfile {
            gpus: parse_nvidia_smi("A, 8192\nB, 8192\n"),
            ..profile(None, Some(64000))
        };
        // 8+8 is not 16. Summing here is how you recommend a model that then
        // fails to load.
        assert_eq!(p.best_vram_mb(), Some(8192));
    }

    #[test]
    fn an_unreadable_size_is_unknown_and_not_zero() {
        let got = parse_nvidia_smi("Weird Card, [N/A]\n");
        assert_eq!(got[0].vram_mb, None);
        // and it must not then read as a card with no memory
        let p = HardwareProfile {
            gpus: got,
            ..profile(None, None)
        };
        assert_eq!(p.best_vram_mb(), None);
    }

    /// Not a test — a diagnostic, ignored by default so it never gates CI on
    /// whatever hardware happens to be running it.
    ///
    ///   cargo test --bins show_this_machine -- --ignored --nocapture
    ///
    /// Every other test here runs against fixtures, which proves the parsers
    /// agree with strings I wrote down. This is the one that proves they agree
    /// with the machine, and those are different claims.
    #[test]
    #[ignore]
    fn show_this_machine() {
        let a = brain_advice();
        println!("{:#?}", a.hardware);
        for r in &a.models {
            println!("{:?}  {:<28} {:>6} MB  {}", r.fit, r.label, r.download_mb, r.why);
        }
    }

    /// A fake machine, so the probe is testable without a second computer.
    fn fake(answers: Vec<(&'static str, Result<&'static str, &'static str>)>)
        -> impl Fn(&str) -> Result<String, String> {
        move |cmd: &str| {
            for (frag, ans) in &answers {
                if cmd.contains(frag) {
                    return ans.map(String::from).map_err(String::from);
                }
            }
            Err("no such command".into())
        }
    }

    #[test]
    fn it_reads_a_remote_machine() {
        let r = profile_from_probe("linux", fake(vec![
            ("uname -s", Ok("Linux\n")),
            ("nvidia-smi", Ok("NVIDIA GeForce RTX 4090, 24564\n")),
            ("meminfo", Ok("MemTotal:       32000000 kB\n")),
            ("nproc", Ok("16\n")),
            ("df", Ok("Filesystem 1024-blocks Used Available Capacity Mounted\n\
                       /dev/sda1  100000000  1000000 90000000 2% /\n")),
            ("ollama --version", Ok("ollama version 0.33.0\n")),
        ]));
        assert_eq!(r.hardware.best_vram_mb(), Some(24564));
        assert_eq!(r.hardware.cores, Some(16));
        assert_eq!(r.ollama_present, Some(true));
        assert!(r.install_hint.is_none());
        assert!(r.hardware.notes.is_empty(), "nothing was unmeasured");
    }

    #[test]
    fn the_recommendations_describe_that_machine_not_this_one() {
        // THE BUG THIS EXISTS TO STOP, caught while wiring the wizard: the
        // screen had nothing to show beside a remote probe except
        // brain_advice, which measures the box helloim.ai runs on. That would put
        // "fits on your 24 GB card" next to a laptop with 6.
        let small = profile_from_probe("linux", fake(vec![
            ("uname -s", Ok("Linux\n")),
            ("nvidia-smi", Ok("Tiny Card, 6144\n")),
            ("meminfo", Ok("MemTotal: 16000000 kB\n")),
        ]));
        let coder = small.models.iter().find(|m| m.tag == "qwen3-coder:30b").unwrap();
        assert_ne!(coder.fit, Fit::Fast, "sized against the wrong machine");
        // 6 GB * 0.75 = 4608 MB of usable card, so the little ones fit and the
        // big ones must not. Asserting BOTH halves matters: "nothing fits" and
        // "everything fits" are each a way of being sized against the wrong
        // computer, and only one of them looks obviously wrong.
        let fast: Vec<_> = small.models.iter().filter(|m| m.fit == Fit::Fast).collect();
        assert!(!fast.is_empty(), "a 6 GB card can hold the small models");
        assert!(
            fast.iter().all(|m| m.download_mb < 3000),
            "only the smallest belong on a 6 GB card, got {:?}",
            fast.iter().map(|m| &m.tag).collect::<Vec<_>>()
        );
        assert!(small.models.iter().any(|m| m.fit == Fit::Slow), "and RAM runs the rest");
    }

    #[test]
    fn a_refused_login_says_so_once_instead_of_six_times() {
        // MARK'S FIRST REMOTE ATTEMPT, reproduced: ssh answers "Permission
        // denied (publickey,password)" because the calling machine has no key
        // on the target. Before this, the probe carried on and produced a note
        // per unanswered command — six symptoms, no cause.
        let r = profile_from_probe("linux", fake(vec![
            ("uname -s", Err("user@192.0.2.10: Permission denied (publickey,password).")),
        ]));
        assert!(!r.reachable);
        assert!(r.problem.is_some(), "a refused connection needs a stated cause");
        assert!(r.hardware.notes.is_empty(), "no per-field noise on top of it");
        // And nothing may be recommended off a machine we never reached.
        assert!(r.models.is_empty(), "sizing against nothing is a confident lie");
        assert_eq!(r.ollama_present, None);
    }

    #[test]
    fn a_missing_command_is_not_a_refused_login() {
        // The other side of the line, and the one that would break the Windows
        // path if this were sloppy: `uname` failing because the shell has no
        // such command means we DID get in.
        assert!(!is_unreachable("'uname' is not recognized as an internal or external command"));
        assert!(!is_unreachable("bash: nvidia-smi: command not found"));
        // And the real refusals, verbatim from ssh.
        assert!(is_unreachable("user@192.0.2.10: Permission denied (publickey,password)."));
        assert!(is_unreachable("ssh: connect to host 192.0.2.30 port 22: No route to host"));
        assert!(is_unreachable("ssh: Could not resolve hostname nope: Name or service not known"));
        assert!(is_unreachable("Host key verification failed."));
    }

    #[test]
    fn the_fix_is_a_command_the_user_can_actually_run() {
        let h = key_hint("user", "192.0.2.10", 22);
        assert!(h.contains("ssh-keygen"), "it must say how to make a key");
        assert!(h.contains("user@192.0.2.10"), "and name the machine");
        assert!(!h.contains("-p 22"), "the default port is noise");
        assert!(h.contains("PowerShell"), "an unnamed shell is a failed paste");
        // AMBIGUOUS "THIS COMPUTER" WORDING IS WHAT CAUSED THE HOST-MIXUP BUG.
        // The text has to name the target and say plainly that the commands
        // do NOT run there, because a reader holding two machines in mind
        // will otherwise pick the wrong one — and the wrong one fails
        // silently.
        assert!(h.contains("NOT on 192.0.2.10"), "it must say where NOT to run it");
        assert!(h.matches("192.0.2.10").count() >= 3, "name the machine, repeatedly");
        // THE DESTRUCTIVE CASE: unguarded, ssh-keygen prompts to overwrite, and
        // somebody following an error dialog presses y — losing every other
        // machine's access from this one in order to gain this one.
        assert!(h.contains("Test-Path"), "keygen must not clobber an existing key");
        // A non-default port has to survive into the command, or the fix fails
        // for exactly the people who need it spelled out.
        assert!(key_hint("u", "h", 2222).contains("-p 2222"));
    }

    #[test]
    fn a_dead_connection_is_never_reported_as_missing_hardware() {
        // THE FAILURE THIS GUARDS. Every command errors because the link is
        // down. Reporting that as "no GPU, no Ollama" would tell somebody
        // their 4090 box is unsuitable because the network hiccuped.
        let r = profile_from_probe("linux", fake(vec![]));
        assert_eq!(r.hardware.best_vram_mb(), None);
        assert_eq!(r.ollama_present, None, "unknown, NOT false");
        assert!(r.install_hint.is_none(), "never advise an install we cannot justify");
        assert!(r.hardware.notes.len() >= 3, "and it says what it could not read");
        // And nothing may be recommended off a profile like this.
        for rec in recommend(&r.hardware) {
            assert_eq!(rec.fit, Fit::Unknown);
        }
    }

    #[test]
    fn ollama_genuinely_absent_gets_a_command_the_user_runs_themselves() {
        let r = profile_from_probe("linux", fake(vec![
            ("uname -s", Ok("Linux\n")),
            ("meminfo", Ok("MemTotal: 32000000 kB\n")),
            ("ollama --version", Ok("bash: ollama: command not found\n")),
        ]));
        assert_eq!(r.ollama_present, Some(false));
        // Probe only, never install: we hand over the line, we do not run it.
        assert!(r.install_hint.unwrap().contains("ollama.com/install.sh"));
    }

    #[test]
    fn every_remote_command_is_read_only() {
        // The claim "we never change a machine we do not live on" should be
        // checkable, not promised in a comment. Both tables, so a Windows
        // command cannot quietly be the one that mutates.
        for os in [RemoteOs::Unix, RemoteOs::Windows] {
            for (_, c) in remote_probe_commands(os) {
                let c = c.to_lowercase();
                for bad in ["install", "rm ", "apt", "curl", "sh -c", " > ", "start-", "remove-"] {
                    assert!(!c.contains(bad), "{c} is not read-only ({bad})");
                }
            }
        }
    }

    #[test]
    fn a_windows_remote_is_read_with_windows_commands() {
        // Measured shapes: nvidia-smi is identical, everything else is not.
        let r = profile_from_probe("linux", fake(vec![
            ("uname -s", Err("'uname' is not recognized")),
            ("nvidia-smi", Ok("NVIDIA GeForce RTX 5080, 16303\n")),
            ("TotalPhysicalMemory", Ok("34123464704\n")),
            ("NumberOfLogicalProcessors", Ok("32\n")),
            ("Get-PSDrive", Ok("512000000000\n")),
            ("ollama --version", Ok("ollama version 0.33.0\n")),
        ]));
        assert_eq!(r.hardware.os, "windows");
        assert_eq!(r.hardware.best_vram_mb(), Some(16303));
        // 34123464704 / 1048576 = 32542.99..., and it truncates. Getting this
        // wrong by one is how a "needs 32 GB" check quietly goes the wrong way.
        assert_eq!(r.hardware.ram_mb, Some(32542)); // bytes, not kB
        assert_eq!(r.hardware.cores, Some(32));
        assert!(r.hardware.notes.is_empty());
    }

    #[test]
    fn a_shell_saying_no_such_command_is_absence_but_a_dead_link_is_not() {
        // Measured against 192.0.2.20: a missing binary exits non-zero with
        // "is not recognized" on stderr, so this has to be read as absent or
        // no Windows box could ever be offered the install line.
        let absent = profile_from_probe("linux", fake(vec![
            ("uname -s", Err("'uname' is not recognized")),
            ("ollama --version", Err("'ollama' is not recognized as an internal")),
        ]));
        assert_eq!(absent.ollama_present, Some(false));
        assert!(absent.install_hint.is_some());

        // But a refused connection is still not evidence of anything.
        let dead = profile_from_probe("linux", fake(vec![
            ("uname -s", Err("Connection refused")),
            ("ollama --version", Err("Connection refused")),
        ]));
        assert_eq!(dead.ollama_present, None);
        assert!(dead.install_hint.is_none());
    }

    #[test]
    fn a_windows_remote_is_never_told_to_pipe_a_shell_script() {
        let r = profile_from_probe("linux", fake(vec![
            ("uname -s", Err("nope")),
            ("ollama --version", Ok("not recognized")),
        ]));
        let hint = r.install_hint.unwrap();
        assert!(!hint.contains("curl"), "curl|sh is not a Windows instruction");
        assert!(hint.contains("winget"));
    }

    #[test]
    fn the_post_quantum_warning_is_not_the_error() {
        // Verbatim from a real failed probe against 192.0.2.20.
        let real = "** WARNING: connection is not using a post-quantum key exchange algorithm.\n\
                    ** This session may be vulnerable to \"store now, decrypt later\" attacks.\n\
                    ** The server may need to be upgraded. See https://openssh.com/pq.html\n\
                    'nproc' is not recognized as an internal or external command,\n";
        let got = clean_ssh_stderr(real);
        assert!(got.starts_with("'nproc' is not recognized"));
        assert!(!got.contains("post-quantum"));
        // An advisory-only stderr must not become an error with no text.
        assert_eq!(clean_ssh_stderr("** WARNING: something\n"), "");
    }

    /// The remote counterpart of `show_this_machine`, and it needs a real
    /// second computer, so it is ignored by default:
    ///
    ///   BRAIN_HOST=192.0.2.20 BRAIN_USER=user \
    ///     cargo test --bins show_that_machine -- --ignored --nocapture
    #[test]
    #[ignore]
    fn show_that_machine() {
        let (host, user) = match (std::env::var("BRAIN_HOST"), std::env::var("BRAIN_USER")) {
            (Ok(h), Ok(u)) => (h, u),
            _ => {
                println!("set BRAIN_HOST and BRAIN_USER to run this");
                return;
            }
        };
        let r = probe_remote(host, user, None);
        println!("{:#?}", r);
        // READ WHAT THE PROBE RETURNED, not a fresh recommendation computed
        // here. Calling recommend() again printed eleven "Unknown" rows under
        // a machine that was never reached — a screenful of model names beside
        // a failed login, which is precisely the picture the real UI refuses
        // to draw. A diagnostic that shows what the product would not show is
        // worse than no diagnostic.
        for rec in &r.models {
            println!("{:?}  {:<28} {}", rec.fit, rec.label, rec.why);
        }
        if !r.reachable {
            println!("(no models — the machine was never reached)");
        }
    }

    #[test]
    fn it_reads_df_and_survives_the_wrapped_line() {
        // Real `df -Pk ~` shape.
        let real = "Filesystem     1024-blocks      Used Available Capacity Mounted on\n\
                    /dev/nvme0n1p2  1921726488 251464016 1572571216      14% /\n";
        assert_eq!(parse_df_kb_mb(real), Some(1535714));
        // A header with no data row is not a zero-byte disk.
        assert_eq!(parse_df_kb_mb("Filesystem 1024-blocks Used Available\n"), None);
    }

    #[test]
    fn it_reads_meminfo() {
        assert_eq!(
            parse_meminfo_mb("MemFree: 100 kB\nMemTotal:       65780208 kB\n"),
            Some(64238)
        );
        assert_eq!(parse_meminfo_mb("nothing useful here"), None);
    }

    #[test]
    fn the_quarter_of_the_card_is_actually_left_alone() {
        // 24 GB card, model wanting 21 GB: 24 * 0.75 = 18, so NOT Fast. This is
        // the outage rule, and a test that let it through would be the outage.
        let coder = CATALOG.iter().find(|m| m.tag == "qwen3-coder:30b").unwrap();
        assert_ne!(classify(coder, &profile(Some(24463), Some(64000))), Fit::Fast);
        // Same model on a 32 GB card: 32 * 0.75 = 24, comfortably over 21.
        assert_eq!(classify(coder, &profile(Some(32768), Some(64000))), Fit::Fast);
    }

    #[test]
    fn no_card_but_plenty_of_ram_is_slow_not_impossible() {
        let small = CATALOG.iter().find(|m| m.tag == "llama3.2:3b").unwrap();
        assert_eq!(classify(small, &profile(None, Some(32000))), Fit::Slow);
    }

    #[test]
    fn a_small_machine_is_told_the_truth() {
        let big = CATALOG.iter().find(|m| m.tag == "qwen3-coder:30b").unwrap();
        assert_eq!(classify(big, &profile(Some(4096), Some(8000))), Fit::TooBig);
    }

    #[test]
    fn measuring_nothing_is_unknown_and_never_too_big() {
        // THE WHOLE POINT OF THE FILE. A machine we could not measure must not
        // be told its hardware is insufficient — that is the "no GPU detected"
        // failure recommending a tiny model to somebody holding a 4090.
        for m in CATALOG {
            assert_eq!(classify(m, &profile(None, None)), Fit::Unknown);
        }
    }

    #[test]
    fn the_best_fitting_biggest_model_leads() {
        // 24 GB card, 64 GB RAM — the box this was written on.
        let recs = recommend(&profile(Some(24463), Some(64000)));
        assert_eq!(recs[0].fit, Fit::Fast);
        // Nothing that fits fast may sort below something that does not.
        let first_slow = recs.iter().position(|r| r.fit != Fit::Fast);
        if let Some(i) = first_slow {
            assert!(recs[i..].iter().all(|r| r.fit != Fit::Fast));
        }
        // Within Fast, bigger first.
        let fast: Vec<u64> = recs
            .iter()
            .filter(|r| r.fit == Fit::Fast)
            .map(|r| r.download_mb)
            .collect();
        let mut sorted = fast.clone();
        sorted.sort_by_key(|d| std::cmp::Reverse(*d));
        assert_eq!(fast, sorted);
    }

    #[test]
    fn a_full_disk_is_flagged_without_hiding_the_model() {
        let p = HardwareProfile {
            disk_free_mb: Some(3000),
            ..profile(Some(24463), Some(64000))
        };
        let recs = recommend(&p);
        let coder = recs.iter().find(|r| r.tag == "qwen3-coder:30b").unwrap();
        assert!(coder.needs_disk);
        // Still listed, and still honest about fitting the card — running out
        // of disk is a different problem from the machine being too small.
        assert_eq!(recs.iter().filter(|r| r.tag == "qwen3-coder:30b").count(), 1);
    }

    #[test]
    fn the_recommended_screen_offers_one_and_one_and_leads_with_claude() {
        // IT WAS SIX AND SIX FOR ONE DAY, then cut to just Claude on the
        // cloud side and Ollama on the local side, the same day. The counts
        // are asserted rather than left loose so that
        // restoring a tile is a deliberate act with a test to update, not
        // something that drifts back in beside an unrelated change.
        // FOUR SINCE 2026-09-05: Claude, OpenAI, Gemini and OpenRouter. Three
        // since 2026-09-02, two since 2026-08-29, one for a day before that,
        // six before that. The count is asserted rather than left loose so that
        // adding or removing a tile is a deliberate act with a test to update,
        // not something that drifts in beside an unrelated change — and it has
        // now caught it in every direction.
        assert_eq!(CLOUD_AI.len(), 4, "Claude, OpenAI, Gemini and OpenRouter");
        assert_eq!(LOCAL_AI.len(), 1, "Ollama, the requested local path");
        // Claude first: it is the brain the product runs, and the binary every
        // other brain runs through. FIRST IS NOT DEFAULT — nothing on the setup
        // screen is preselected, and `providers.rs` hands out no brain until
        // somebody picks one.
        assert_eq!(CLOUD_AI[0].id, "claude");
        assert!(CLOUD_AI.iter().any(|o| o.id == "openai"), "OpenAI is offered");
        assert!(CLOUD_AI.iter().any(|o| o.id == "gemini"), "Gemini is offered");
        assert!(CLOUD_AI.iter().any(|o| o.id == "openrouter"), "OpenRouter is offered");
        // Gemini and OpenRouter ride the SAME kind as OpenAI, deliberately —
        // see each tile's own comment. A future reader "fixing" either to a
        // dedicated kind would be re-inventing `adapter.rs`, not fixing
        // anything. OpenRouter also speaks Anthropic's `/v1/messages`, but the
        // anthropic-skin kind stays gated and is not needed: the OpenAI wire is
        // agnostic-sufficient.
        assert_eq!(
            CLOUD_AI.iter().find(|o| o.id == "gemini").unwrap().kind,
            "openai-compatible"
        );
        assert_eq!(
            CLOUD_AI.iter().find(|o| o.id == "openrouter").unwrap().kind,
            "openai-compatible"
        );
        assert_eq!(LOCAL_AI[0].id, "ollama");
        for o in CLOUD_AI.iter().chain(LOCAL_AI) {
            // THE ONE LIST, read from providers.rs rather than repeated here.
            // A tile naming a kind the backend cannot route is a card that
            // opens a form the backend then refuses — which is how a removed
            // kind stays reachable from a screen nobody re-read.
            assert!(
                crate::providers::kind_is_routable(o.kind),
                "{} has kind {}, which providers.rs cannot route",
                o.id, o.kind
            );
            assert!(!o.name.is_empty() && !o.note.is_empty(), "{} is unlabelled", o.id);
            assert_eq!(o.letter.chars().count(), 1, "{} needs a single lettermark", o.id);
        }
        // Cloud tiles are https or, for Claude, no URL at all — its setup is a
        // sign-in, not an endpoint. A cleartext cloud URL would put a key on
        // the wire.
        for o in CLOUD_AI {
            assert!(
                o.base_url.is_empty() || o.base_url.starts_with("https://"),
                "{} is not https", o.id
            );
        }
        // Local tiles are loopback ONLY. A "local" option pointing off-machine
        // would send someone's conversation to a stranger while the screen
        // said the word local.
        for o in LOCAL_AI {
            assert!(o.base_url.starts_with("http://127.0.0.1:"), "{} is not loopback", o.id);
            assert!(!o.download_url.is_empty(), "{} gives nowhere to get it", o.id);
            // AND IT MUST BE THE ADDRESS THE BACKEND WOULD HAVE PICKED. This
            // tile seeds a provider row; seeding one the backend defaults
            // differently is a Test that fails for a reason nobody can see.
            // It read `.../v1` until 2026-08-28, which appends to
            // `/v1/v1/messages` on the Anthropic path.
            assert!(!o.base_url.ends_with("/v1"), "{} would double the /v1", o.id);
        }
    }

    #[test]
    fn every_catalog_entry_is_internally_sane() {
        for m in CATALOG {
            // Weights need room for context on top, always — so a requirement
            // can never be smaller than the download it has to hold.
            assert!(needs_vram_mb_for(m.download_mb) > m.download_mb, "{} vram too cheap", m.tag);
            assert!(needs_ram_mb_for(m.download_mb) > m.download_mb, "{} ram too cheap", m.tag);
            assert!(!m.good_for.is_empty());
            // A tag with no colon is a name, not a version, and `ollama pull`
            // would silently take whatever "latest" happens to be that week.
            assert!(m.tag.contains(':'), "{} is not a pinned tag", m.tag);
        }
    }

    /// THE TEST THAT WOULD HAVE CAUGHT THE INVENTED TAGS, and it is ignored by
    /// default because it needs the network:
    ///
    ///   cargo test --bins every_tag_is_real -- --ignored --nocapture
    ///
    /// The first catalogue shipped `qwen3.6:4b` and `qwen3.6:8b`, which do not
    /// exist — `qwen3.6` is real and ships only at 27b and 35b. Nothing in the
    /// suite could tell, because every other test asked whether the numbers
    /// were consistent with each other rather than with the registry. Run this
    /// whenever the catalogue changes.
    #[test]
    #[ignore]
    fn every_tag_is_real() {
        let mut bad = vec![];
        for m in CATALOG {
            let (name, tag) = m.tag.split_once(':').unwrap();
            let url = format!("https://registry.ollama.ai/v2/library/{name}/manifests/{tag}");
            let body = match ureq::get(&url).call() {
                Ok(r) => r.into_string().unwrap_or_default(),
                Err(e) => {
                    bad.push(format!("{} — {e}", m.tag));
                    continue;
                }
            };
            // The model layers are the download. Summing them is how the
            // numbers in the catalogue were produced in the first place.
            let mb: u64 = body
                .split("\"size\":")
                .skip(1)
                .filter_map(|s| s.split(|c: char| !c.is_ascii_digit()).next())
                .filter_map(|n| n.parse::<u64>().ok())
                .max()
                .unwrap_or(0)
                / 1048576;
            let drift = (mb as i64 - m.download_mb as i64).abs();
            println!("{:<22} catalogue {:>6} MB   registry {:>6} MB", m.tag, m.download_mb, mb);
            if drift > 200 {
                bad.push(format!("{} says {} MB, registry says {} MB", m.tag, m.download_mb, mb));
            }
        }
        assert!(bad.is_empty(), "{bad:#?}");
    }
}
