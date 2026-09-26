//! The AI Boardroom — convening the person's own connected brains on one
//! question, the way our own AI Agent Mastermind convenes specialists.
//! `HAI-APP-IA-SPEC.md`'s flagship feature, and Iris's
//! `AI-BOARDROOM-how-it-works.md` (`~/Documents/Iris/hai-ia-v2-2026-09-05/`)
//! is the design this file exists to implement. Read that file's own "The
//! flow" section before changing the shape below — this is a transcription
//! of it, not a fresh design.
//!
//! ## Why this is its own module, and not a JS-side loop of individual sends
//!
//! Convening is one round trip out of the app's own engine seam per seat, per
//! round — the SAME `Engine::start`/`TurnSink` machinery `main.rs::send`
//! already drives, called directly instead of through a chat turn, because a
//! Boardroom question is not a conversation with anybody in particular and
//! must not become one of the transcripts History lists. Reusing the seam
//! rather than re-implementing a second way to call a brain is the whole
//! point of `engine::mod`'s existing seam: "any engine that can produce
//! [the five-shape events] is drop-in."
//!
//! ## The shape on the wire
//!
//! `rounds[] -> { round, question, seats: [{ seat, name, role, text, stance,
//! error, ts }] }, synthesis: { seat, name, text, ts } | null`. This is the
//! real Mastermind's own `~/voice-line/logs/mastermind-thread.json` shape —
//! `seat`/`name`/`text`/`error`/`ts` are its field names verbatim — with
//! `role` and `stance` added, because a Boardroom seat carries a
//! Primary/Secondary designation the Mastermind's do not.
//!
//! ## What a seat is never allowed to do
//!
//! **"The Boardroom only reads and reasons — it never acts,"** per the design
//! doc's own Privacy & Safe Agency section. Three separate things enforce
//! that, because no single one of them can promise it alone:
//!
//! 1. `Provider::allow_shell` / `allow_agency` are forced OFF on the seat's
//!    own clone before it is ever handed to `engine::for_provider` — see
//!    `neutered` below — so `Bash` and the three agency tools
//!    (`OpenUrl`/`LaunchApp`/`OpenSettingsPage`) are never even offered to
//!    the NATIVE engine, whatever the person allowed that same brain to do
//!    in ordinary chat. `engine::native::tools::definitions` reads exactly
//!    these two flags off the `Provider` inside the `TurnRequest`, so this is
//!    the one point of control that actually reaches it.
//! 2. `Permission::Ask` on every seat. In headless `-p` mode `claude.exe` has
//!    nobody to answer an approval prompt, so `--permission-mode default`
//!    REFUSES rather than hangs — this is not a guess; it is the same
//!    behaviour the app's own Safe Agency copy already states for the "Ask
//!    first" mode (see `ui/index.html`'s `SAFE_OPTS`, sourced from Beck's
//!    2026-08-28 NO-GO). So even Claude Code's OWN built-in tools, which
//!    exist independently of any `--mcp-config`, are refused before they run.
//! 3. Every seat runs inside `scratch_dir` (below) — an app-owned, disposable
//!    folder, never the person's real project. `engine::native`'s Read/Write/
//!    Edit/Glob/Grep are NOT gated by `allow_shell`/`allow_agency` at all
//!    (only `Bash` and the three agency tools are — see
//!    `native::tools::definitions`'s own doc), so mitigation 1 above does not
//!    reach them. Confining the workdir to a folder with nothing real in it
//!    is what keeps a native seat's Read/Write/Edit from ever touching
//!    anything that matters, even if the model tries one uninstructed.
//!
//! **Point 3 is a real, stated limitation, not a solved problem** — a native
//! seat COULD write a file inside its own scratch folder if it ignored the
//! system prompt telling it this is reasoning-only. It cannot reach anything
//! outside that folder, and nothing outside `app_config_dir` is ever at
//! risk, but "cannot leave the box" is a different, weaker claim than "cannot
//! act at all," and the difference is written here so nobody re-derives a
//! false stronger one from this file's own confidence.

use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use tauri::{AppHandle, Manager, State};

use crate::engine::{self, Permission, TurnRequest, TurnSink};
use crate::providers::{self, Provider, Providers};

/// How long one seat gets to answer before it is counted as a timeout and
/// dropped from the vote — "mark them, don't hang," per the design doc's own
/// "A brain fails or times out" state. Generous on purpose: a local model's
/// first answer after a cold load can take the better part of a minute (see
/// the app's own "a local model's first test can take a minute" copy on the
/// Brain sheet), and a Boardroom round is something the person asked for and
/// is waiting on, not a background poll.
const SEAT_TIMEOUT: Duration = Duration::from_secs(180);

/// Minimum seats to convene at all — "one brain is not a boardroom," per the
/// design doc. Below this the room routes back to normal chat instead of
/// pretending to debate with itself.
const MIN_SEATS: usize = 2;

const REASONING_SYSTEM: &str = "\
You are one seat at an AI Boardroom: several AI brains a person has connected \
to their own assistant, being asked one question together so they get more \
than a single opinion. You are being asked to REASON ONLY. Do not attempt to \
use a file, shell, browser or settings tool even if one appears to be \
available -- there is nothing in this folder to read or change, only a \
question to answer in words.\n\n\
Answer in exactly this shape, nothing before it:\n\
STANCE: <2 to 6 words that sum up your position>\n\
<a blank line, then 2 to 5 sentences of reasoning>\n\n\
Be direct. If you are only agreeing with what you expect another brain to \
say, say so plainly rather than padding -- a bare restatement is not a \
contribution.";

const CROSS_READ_SYSTEM: &str = "\
Same rules as before: REASON ONLY, no tools, the exact STANCE-then-reasoning \
shape. You have now seen how every other seat answered independently. Hold \
your position, refine it, or change your mind -- and say briefly which of \
the three you did, before the reasoning.";

const SYNTHESIS_SYSTEM: &str = "\
You are the Primary brain at an AI Boardroom, or standing in for a Primary \
that could not answer. Every seat, including you, has answered the same \
question independently and then seen every other seat's answer once. Read \
every seat's FINAL position below and write ONE recommendation for the \
person who asked.\n\n\
Rules: name the single clear move. State any real condition on it. Say \
plainly, by name, where the room agreed and where it split -- a 2-1 split is \
a 2-1 split, never launder it into a fake consensus, and never silently pick \
a winner without saying that you did and why. One short paragraph, direct, \
no headings.";

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SeatAnswer {
    /// The provider's own id — `mastermind-thread.json`'s `seat` field,
    /// renamed for `serde`'s camelCase convention but the same key the TODO
    /// comment beside `#iaViewBoardroom` names.
    pub seat: String,
    pub name: String,
    /// "primary" | "secondary" | "" — never shown as a role a Mastermind
    /// transcript never had reason to carry.
    pub role: String,
    /// The reasoning, with the `STANCE:` line already stripped off — empty
    /// when the seat errored or timed out rather than left holding stale
    /// text from an earlier round.
    pub text: String,
    /// The 2-6 word summary a seat was asked to lead with. Empty when the
    /// seat did not follow the shape, or errored before producing anything.
    pub stance: String,
    /// `None` on a real answer. `Some(reason)` means this seat contributed
    /// nothing to this round and is why a round can have fewer voices than
    /// seats — "never silently drop a seat," per the design doc.
    pub error: Option<String>,
    pub ts: u64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BoardroomRound {
    pub round: u32,
    pub question: String,
    pub seats: Vec<SeatAnswer>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Synthesis {
    pub seat: String,
    pub name: String,
    pub text: String,
    pub ts: u64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BoardroomResult {
    pub question: String,
    /// Exactly two: round 1 (independent) and round 2 (cross-read). Capped
    /// at one revision round on purpose — the design doc's own reasoning:
    /// "endless rounds cost money and converge on nothing."
    pub rounds: Vec<BoardroomRound>,
    pub synthesis: Option<Synthesis>,
    /// Set when the room ran but could not produce a synthesis (neither
    /// Primary nor Secondary answered either round) — the design doc's
    /// "Deadlock" state. `None` on an ordinary, fully-synthesised result.
    pub note: Option<String>,
}

fn now_ts() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// A folder this app owns, holds nothing of the person's, and is safe to
/// hand to Read/Write/Edit/Glob/Grep — see this module's own header, point 3.
/// Created once and reused; never deleted, because it is empty in the normal
/// case and cleaning up after a model that ignored its instructions is a
/// smaller job than reasoning about whether a delete race is safe.
fn scratch_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_config_dir()
        .map_err(|e| format!("no config directory: {e}"))?
        .join("boardroom-scratch");
    std::fs::create_dir_all(&dir).map_err(|e| format!("could not create {dir:?}: {e}"))?;
    Ok(dir)
}

/// A seat that can never act, whatever the person allowed this same brain to
/// do in ordinary chat — see this module's own header, mitigation 1.
fn neutered(p: &Provider) -> Provider {
    let mut p = p.clone();
    p.allow_shell = false;
    p.allow_agency = false;
    p
}

/// The two facts `STANCE: <line>\n\n<body>` splits into. Tolerant of a model
/// that drops the blank line, and of one that ignores the shape entirely —
/// the whole text becomes the body with an empty stance rather than this
/// function ever failing the turn over formatting.
fn parse_stance(raw: &str) -> (String, String) {
    let raw = raw.trim();
    let Some(rest) = raw.strip_prefix("STANCE:") else {
        return (String::new(), raw.to_string());
    };
    match rest.split_once('\n') {
        Some((first_line, body)) => (first_line.trim().to_string(), body.trim().to_string()),
        // No body followed on its own line -- treat the whole remainder as
        // the stance rather than losing it.
        None => (rest.trim().to_string(), String::new()),
    }
}

/// Collects one turn's answer instead of streaming it to a window — the
/// Boardroom's stand-in for `main.rs::AppSink`. `Send + Sync + 'static` for
/// free: every field is a `Mutex` around plain data.
struct CollectingSink {
    tx: Mutex<Option<mpsc::Sender<()>>>,
    text: Mutex<String>,
    error: Mutex<Option<String>>,
}

impl CollectingSink {
    fn new() -> (Arc<Self>, mpsc::Receiver<()>) {
        let (tx, rx) = mpsc::channel();
        (
            Arc::new(CollectingSink {
                tx: Mutex::new(Some(tx)),
                text: Mutex::new(String::new()),
                error: Mutex::new(None),
            }),
            rx,
        )
    }
}

impl TurnSink for CollectingSink {
    fn event(&self, value: serde_json::Value) {
        match value.get("type").and_then(|t| t.as_str()) {
            Some("assistant") => {
                if let Some(blocks) = value.pointer("/message/content").and_then(|c| c.as_array()) {
                    let mut buf = self.text.lock().unwrap_or_else(|e| e.into_inner());
                    for block in blocks {
                        if block.get("type").and_then(|t| t.as_str()) == Some("text") {
                            if let Some(t) = block.get("text").and_then(|t| t.as_str()) {
                                buf.push_str(t);
                            }
                        }
                    }
                }
            }
            Some("result") => {
                if value.get("is_error").and_then(|b| b.as_bool()) == Some(true) {
                    let msg = value
                        .get("result")
                        .and_then(|r| r.as_str())
                        .unwrap_or("The brain reported an error.")
                        .to_string();
                    *self.error.lock().unwrap_or_else(|e| e.into_inner()) = Some(msg);
                }
            }
            _ => {}
        }
    }

    fn raw(&self, _line: String) {
        // Non-JSON stdout noise. The Mastermind-shaped result has nowhere to
        // put this and it is not diagnostic of a failure on its own — see
        // `failure` below for the channel that IS.
    }

    fn failure(&self, line: String) {
        // A stderr-shaped diagnostic. Kept only if nothing more specific
        // (a real `result` error) has landed yet — that one names what
        // actually went wrong; this is a fallback for when it never arrives.
        let mut err = self.error.lock().unwrap_or_else(|e| e.into_inner());
        if err.is_none() {
            *err = Some(line);
        }
    }

    fn finished(&self, _code: i32) {
        if let Some(tx) = self.tx.lock().unwrap_or_else(|e| e.into_inner()).take() {
            let _ = tx.send(());
        }
    }
    // confirm() is left at the trait default (`false`, deny) — see this
    // module's own header. A seat with allow_shell/allow_agency forced off
    // and Permission::Ask should never reach it, and if something upstream
    // ever changes that, silent refusal is still the correct answer for a
    // sink with no person behind it to ask.
}

/// Run one seat, once, and return its answer -- never `Err` outward. A seat
/// that could not be reached is a `SeatAnswer` with `error: Some(..)`, never
/// a reason to fail the whole round; see this module's header docstring and
/// the design doc's "never silently drop a seat."
fn ask_seat(
    provider: &Provider,
    role: &str,
    system_prompt: &str,
    prompt: String,
    workdir: &Path,
) -> SeatAnswer {
    let seat = provider.id.clone();
    let name = provider.name.clone();
    let provider = neutered(provider);
    let eng = engine::for_provider(&provider);
    let (sink, rx) = CollectingSink::new();

    let req = TurnRequest {
        prompt,
        workdir: workdir.to_path_buf(),
        // The strictest choice. Headless `-p` has nobody to answer an
        // approval prompt, so this REFUSES any real action rather than
        // hanging — see this module's own header, mitigation 2.
        permission: Permission::Ask,
        system_prompt: system_prompt.to_string(),
        // Never resumed and never stored -- a Boardroom question is not a
        // conversation with anybody in particular. See `store_dir` below.
        resume: None,
        mcp_config: None,
        mcp_env: Vec::new(),
        allowed_tools: Vec::new(),
        // No transcript file for this turn -- it is not a conversation and
        // must never appear in History, which reads exactly this directory.
        store_dir: None,
        provider,
    };

    let turn = match eng.start(&req, sink.clone()) {
        Ok(t) => t,
        Err(e) => {
            return SeatAnswer {
                seat,
                name,
                role: role.to_string(),
                text: String::new(),
                stance: String::new(),
                error: Some(e),
                ts: now_ts(),
            }
        }
    };

    match rx.recv_timeout(SEAT_TIMEOUT) {
        Ok(()) => {
            let text = sink.text.lock().unwrap_or_else(|e| e.into_inner()).clone();
            let error = sink.error.lock().unwrap_or_else(|e| e.into_inner()).clone();
            if text.trim().is_empty() {
                let error = error.or_else(|| Some("answered with nothing".to_string()));
                return SeatAnswer { seat, name, role: role.to_string(), text: String::new(), stance: String::new(), error, ts: now_ts() };
            }
            let (stance, body) = parse_stance(&text);
            SeatAnswer { seat, name, role: role.to_string(), text: body, stance, error, ts: now_ts() }
        }
        Err(_) => {
            // Timed out. Cancel so the process does not keep running after
            // the room has already moved past it, then report it as a
            // dropped vote rather than blocking the whole convene on one
            // slow seat.
            let _ = turn.cancel();
            SeatAnswer {
                seat,
                name,
                role: role.to_string(),
                text: String::new(),
                stance: String::new(),
                error: Some("didn't answer in time".to_string()),
                ts: now_ts(),
            }
        }
    }
}

fn seat_role(store: &providers::Store, id: &str) -> &'static str {
    if id == store.active {
        "primary"
    } else if !store.secondary.is_empty() && id == store.secondary {
        "secondary"
    } else {
        ""
    }
}

/// One full round, every seat answered in parallel. Threads rather than
/// async: this crate's HTTP is `ureq` (blocking) throughout, and
/// `#[tauri::command(async)]` already moves the whole call off the UI
/// thread — see `stop`'s own doc in `main.rs` for the same pattern.
fn run_round(
    round_no: u32,
    question: &str,
    seats: &[Provider],
    store: &providers::Store,
    system_prompt: &str,
    prompt: &str,
    workdir: &Path,
) -> BoardroomRound {
    let answers: Vec<SeatAnswer> = std::thread::scope(|scope| {
        let handles: Vec<_> = seats
            .iter()
            .map(|p| {
                let role = seat_role(store, &p.id);
                scope.spawn(move || ask_seat(p, role, system_prompt, prompt.to_string(), workdir))
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap_or_else(|_| SeatAnswer {
            seat: "unknown".into(),
            name: "unknown".into(),
            role: "".into(),
            text: String::new(),
            stance: String::new(),
            error: Some("crashed while answering".into()),
            ts: now_ts(),
        })).collect()
    });
    BoardroomRound { round: round_no, question: question.to_string(), seats: answers }
}

/// Every seat's compact, attributed position, for feeding back into the next
/// round or into synthesis. Kept short on purpose — this text becomes part of
/// EVERY seat's next prompt, so its cost multiplies by the seat count.
fn format_positions(round: &BoardroomRound) -> String {
    let mut out = String::new();
    for s in &round.seats {
        if let Some(err) = &s.error {
            out.push_str(&format!("- {} ({}): did not answer — {}\n", s.name, s.seat, err));
            continue;
        }
        if s.stance.is_empty() {
            out.push_str(&format!("- {}: {}\n", s.name, s.text));
        } else {
            out.push_str(&format!("- {} — {}: {}\n", s.name, s.stance, s.text));
        }
    }
    out
}

/// A seat's FINAL position: round 2 if it answered there, else round 1 if it
/// answered there, else dropped. "A brain fails or times out ... it is
/// dropped from the vote" — but only once it has had both its chances.
fn final_positions(round1: &BoardroomRound, round2: &BoardroomRound) -> Vec<SeatAnswer> {
    round1
        .seats
        .iter()
        .map(|r1| match round2.seats.iter().find(|r2| r2.seat == r1.seat) {
            Some(r2) if r2.error.is_none() => r2.clone(),
            _ if r1.error.is_none() => r1.clone(),
            Some(r2) => r2.clone(),
            None => r1.clone(),
        })
        .collect()
}

/// Convene the person's own connected brains on one question.
///
/// `seats`: the ids the person toggled in on screen. Empty means "everyone
/// currently eligible" — the design doc's own default ("defaults to all
/// connected").
#[tauri::command(async)]
pub fn boardroom_convene(
    app: AppHandle,
    state: State<Providers>,
    question: String,
    seats: Vec<String>,
) -> Result<BoardroomResult, String> {
    let question = question.trim().to_string();
    if question.is_empty() {
        return Err("Ask the room something first.".into());
    }

    let store = providers::list_providers(app.clone(), state);
    let airgapped = providers::is_airgapped(&app);

    let mut eligible: Vec<Provider> = store
        .providers
        .iter()
        .filter(|p| providers::is_eligible(p))
        .filter(|p| seats.is_empty() || seats.contains(&p.id))
        // AIR-GAPPED MEANS ONLY OFFLINE SEATS — the design doc's own state:
        // "restricts seats to offline brains, so a private question never
        // leaves the machine to be debated." `kind == "local"` is the one
        // genuinely offline kind this build has (Ollama on loopback).
        .filter(|p| !airgapped || p.kind == "local")
        .cloned()
        .collect();
    // Primary first, then Secondary, then everyone else — cosmetic only
    // (every seat gets the identical prompt); it just means the room reads
    // top-to-bottom the way a person would expect it to.
    eligible.sort_by_key(|p| match seat_role(&store, &p.id) {
        "primary" => 0,
        "secondary" => 1,
        _ => 2,
    });

    if eligible.len() < MIN_SEATS {
        return Err(if airgapped {
            "Air-gap is on, so the Boardroom can only seat brains on this machine \
             — right now there's only one (or none) available. Connect a second \
             local brain under Settings → AI Brain, or turn off air-gap."
                .into()
        } else {
            "The Boardroom needs at least two connected brains to convene — right \
             now there's only one. Connect a second brain under Settings → AI \
             Brain, or just ask this one directly in chat."
                .into()
        });
    }

    let workdir = scratch_dir(&app)?;

    // ---- Round 1: independent. Nobody has seen anybody else's answer. ----
    let round1 = run_round(1, &question, &eligible, &store, REASONING_SYSTEM, &question, &workdir);

    // ---- Round 2: cross-read, capped at one revision round. ----
    let cross_prompt = format!(
        "The original question was:\n{question}\n\nEvery seat in the room answered independently, including you. Here is what everyone said:\n\n{}",
        format_positions(&round1)
    );
    let round2 = run_round(2, &question, &eligible, &store, CROSS_READ_SYSTEM, &cross_prompt, &workdir);

    // ---- Synthesis: Primary reads every final position; Secondary stands
    // in if Primary itself could not answer either round. ----
    let finals = final_positions(&round1, &round2);
    let synth_provider = finals
        .iter()
        .find(|s| s.error.is_none() && s.role == "primary")
        .or_else(|| finals.iter().find(|s| s.error.is_none() && s.role == "secondary"))
        .and_then(|s| eligible.iter().find(|p| p.id == s.seat));

    let (synthesis, note) = match synth_provider {
        None => (
            None,
            Some(
                "Neither your Primary nor Secondary brain could answer, so the room \
                 has no synthesis this time — every seat's own answer above is still \
                 real, there is just nobody to reconcile them."
                    .to_string(),
            ),
        ),
        Some(p) => {
            let synth_prompt = format!(
                "The question:\n{question}\n\nEvery seat's final position:\n\n{}",
                finals
                    .iter()
                    .map(|s| if let Some(err) = &s.error {
                        format!("- {} ({}): did not answer — {}\n", s.name, s.seat, err)
                    } else if s.stance.is_empty() {
                        format!("- {}: {}\n", s.name, s.text)
                    } else {
                        format!("- {} — {}: {}\n", s.name, s.stance, s.text)
                    })
                    .collect::<String>()
            );
            let answer = ask_seat(p, seat_role(&store, &p.id), SYNTHESIS_SYSTEM, synth_prompt, &workdir);
            if answer.error.is_some() || answer.text.trim().is_empty() {
                (
                    None,
                    Some(format!(
                        "{} was going to synthesise and could not this time ({}) — \
                         every seat's own answer above is still real.",
                        answer.name,
                        answer.error.as_deref().unwrap_or("answered with nothing")
                    )),
                )
            } else {
                (
                    Some(Synthesis { seat: p.id.clone(), name: p.name.clone(), text: answer.text, ts: now_ts() }),
                    None,
                )
            }
        }
    };

    Ok(BoardroomResult { question, rounds: vec![round1, round2], synthesis, note })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seat(id: &str, error: Option<&str>, text: &str) -> SeatAnswer {
        SeatAnswer {
            seat: id.into(),
            name: id.into(),
            role: String::new(),
            text: text.into(),
            stance: String::new(),
            error: error.map(str::to_string),
            ts: 0,
        }
    }

    #[test]
    fn stance_and_body_split_on_the_asked_shape() {
        let (stance, body) = parse_stance("STANCE: fixed-price\n\nQuote it clean.");
        assert_eq!(stance, "fixed-price");
        assert_eq!(body, "Quote it clean.");
    }

    /// A model that ignores the shape entirely must still be usable — the
    /// whole reply becomes the body, not a formatting failure that drops the
    /// seat.
    #[test]
    fn text_with_no_stance_line_is_all_body() {
        let (stance, body) = parse_stance("Just take the fixed price deal.");
        assert_eq!(stance, "");
        assert_eq!(body, "Just take the fixed price deal.");
    }

    /// A stance with nothing after it on its own line -- the whole remainder
    /// is kept as the stance rather than silently discarded.
    #[test]
    fn a_stance_with_no_following_body_is_not_lost() {
        let (stance, body) = parse_stance("STANCE: fixed-price");
        assert_eq!(stance, "fixed-price");
        assert_eq!(body, "");
    }

    /// Round 2 wins when it genuinely answered — the seat got to revise.
    #[test]
    fn final_position_prefers_round_two_when_it_answered() {
        let r1 = BoardroomRound { round: 1, question: "q".into(), seats: vec![seat("a", None, "first")] };
        let r2 = BoardroomRound { round: 2, question: "q".into(), seats: vec![seat("a", None, "revised")] };
        let finals = final_positions(&r1, &r2);
        assert_eq!(finals[0].text, "revised");
    }

    /// A seat that timed out on the cross-read still has its round-1 answer
    /// counted — one bad round must not erase a real one.
    #[test]
    fn final_position_falls_back_to_round_one_when_round_two_failed() {
        let r1 = BoardroomRound { round: 1, question: "q".into(), seats: vec![seat("a", None, "first")] };
        let r2 = BoardroomRound { round: 2, question: "q".into(), seats: vec![seat("a", Some("didn't answer in time"), "")] };
        let finals = final_positions(&r1, &r2);
        assert_eq!(finals[0].text, "first");
        assert!(finals[0].error.is_none(), "a working round-1 answer must not read as an error");
    }

    /// A seat that never answered either round stays a dropped vote, not a
    /// fabricated one.
    #[test]
    fn final_position_reports_a_seat_that_never_answered_at_all() {
        let r1 = BoardroomRound { round: 1, question: "q".into(), seats: vec![seat("a", Some("boom"), "")] };
        let r2 = BoardroomRound { round: 2, question: "q".into(), seats: vec![seat("a", Some("boom"), "")] };
        let finals = final_positions(&r1, &r2);
        assert!(finals[0].error.is_some());
    }

    /// `neutered` must turn both agency flags off regardless of what the
    /// real, saved provider carries — see this module's own header,
    /// mitigation 1. Proven able to fail: removing either assignment inside
    /// `neutered` fails this.
    #[test]
    fn neutered_always_strips_shell_and_agency() {
        let mut p = Provider {
            id: "x".into(), kind: "local".into(), name: "X".into(), base_url: String::new(),
            model: "m".into(), builtin: false, disconnected: false, connected: true,
            checked_at: String::new(), last_error: String::new(), has_secret: false,
            caveat: String::new(), migration_note: String::new(), allow_shell: true, allow_agency: true,
        };
        let n = neutered(&p);
        assert!(!n.allow_shell && !n.allow_agency);
        p.allow_shell = false;
        p.allow_agency = false;
        assert!(!neutered(&p).allow_shell && !neutered(&p).allow_agency);
    }
}
