// Jarvis Desktop — slice one.
//
// One job: put a window on this machine that launches Claude Code as a child
// process and streams what it does, live, as it happens. No web board, no
// server, no bundler. The window IS the product.
//
// The streaming contract is Claude Code's own:
//   claude -p <prompt> --output-format stream-json --verbose
// which emits one JSON object per line on stdout. We forward each line to the
// front end untouched and let the front end decide how to draw it. Parsing it
// in Rust would mean re-deriving their schema in two places, and it changes.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::process::{Command, Output, Stdio};
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

// API providers and MCP servers, the keys they need, and the real round trip
// that decides whether the indicator is allowed to be green.
mod connectors;
mod email;
mod google_email;
mod ms_graph;
mod google_calendar;
mod computer_time;
mod google_policy;

// The swappable brain: Claude by default, Ollama on the user's own machine,
// any endpoint that natively speaks Anthropic's /v1/messages shape, or any
// OpenAI-compatible endpoint through the in-process translator below. The
// same earn-your-green honesty system as the connectors, pointed at the one
// choice that changes everything else.
mod boardroom;
mod providers;

// Autonomous background tasks -- give it a task, it runs many turns
// unattended while the app stays open, and reports back. Built on the same
// `engine::for_provider`/`TurnSink` seam `boardroom.rs` already proves out,
// bypassing `Session::turn` entirely the same way Boardroom does, for the
// same reason: a worker running alongside ordinary chat must not block it or
// be cancelled by it. See that file's own header before this one's.
mod worker;

// WHO RUNS THE AGENT LOOP. Today there is exactly one answer — spawn Claude
// Code and forward its stream — and this module is that answer put behind a
// trait so a second one can exist.
//
// Mark, 2026-08-31: "we need to make each of the providers stand alone without
// claude." Every brain in `providers.rs` currently reaches the model through a
// single spawn of somebody else's binary, so "pick a local model" still means
// "download 214 MB from Anthropic first". This is stage one of removing that,
// and stage one changes nothing a customer can observe: `send()` below builds
// an `engine::TurnRequest` instead of a `Command`, and `ClaudeCodeEngine` does
// what these lines used to do, in the same order.
mod engine;

// The OpenAI translator. **REWIRED 2026-08-29 AND NOW REACHABLE** — Mark took
// the cloud kinds out on 2026-08-28 ("we will have to work on that later") and
// then asked for a Claude-or-OpenAI choice at setup, so the wiring came back:
// `init()` is called from the setup hook below, and `ensure_running()` has
// exactly one caller, the `openai-compatible` arm of `providers::apply_env`.
//
// **THIS COMMENT SAID THE LISTENER NEVER BINDS BECAUSE `OPENAI_ENABLED` IS
// `false`. IT HAS BEEN `true` SINCE 2026-08-29** (`providers.rs:163`) and this
// paragraph went on contradicting the constant it names — corrected 2026-08-31.
// A reader trusting it would conclude the OpenAI path is dead when it routes,
// which is the exact way somebody builds on a false premise. `adapter.rs:11`
// carried the same wrong claim and is corrected too.
//
// What makes it safe is not the gate being shut, because it is not: it is that
// the dot has to be EARNED on the user's own machine before the row can be
// used. `test_provider` reaches OpenAI for real with their key and then spawns
// Claude Code through `apply_env` into the translator, so going green is a real
// end-to-end round trip, and `may_select` refuses a row that has not gone
// green. The gate is still a `const` rather than an environment variable — this
// is a loopback relay to a paid account, and a switch a stray export could flip
// is not a switch — and flipping it back is the one line that takes the
// translator off every screen again. Read the checklist on
// `providers::OPENAI_ENABLED`; that file's header is the whole design.
mod adapter;

// The one-time move of retired "api" connectors into the Brain sheet, run at
// setup before either store's first read. The whole design — why the secret
// is never touched, why the green never travels — is at the top of migrate.rs.
mod migrate;

// "About you" — who the user is, their goal, and the one thing they always
// want remembered. Written into the working folder's CLAUDE.md so it genuinely
// reaches the model rather than sitting in a settings file.
mod profile;

// Skills — the things you taught it to do. A folder with a SKILL.md in it;
// this module creates and reads exactly that, and invents nothing on top.
mod skills;

// Signing in to a hosted connector from inside the app: dynamic client
// registration, PKCE, and a one-shot loopback redirect. No developer account,
// no broker, no API key to go and find.
mod oauth;

// Memory. Tier 2 of the memory-engine brief -- the session bridge, so picking
// work back up does not mean explaining it again. Plain text in the working
// folder, because the site promises exactly that in public.
mod memory;

// Folder trust. "Point it at a folder" is the pitch, and a folder can arrive
// carrying instructions -- a CLAUDE.md, a .claude/settings.local.json, a
// .mcp.json -- which a headless `claude -p` run obeys with no prompt anywhere.
// The whole argument, with the vendor's own wording, is at the top of the file.
mod folder_trust;

// Memory tier 3 -- the facts, preferences and decisions it keeps. The
// markdown in the user's folder is the store; SQLite with FTS5 is its
// rebuildable search index.
mod facts;

// "Add a local file for context" -- copies a file the person picks from
// anywhere on their disk into `workdir/context/`, so the model's ordinary
// read/glob/grep tools cover it like everything else already in the folder.
// A copy, not a new store: see the module's own header for why this is not
// Tier 2 or Tier 3 and does not live beside them under `.helloim/`.
mod context_files;

// Slice 3 of the memory engine: the MCP server that makes memory a TOOL the
// model asks for, spawned by Claude Code as `remembrancer mcp-memory`. Lives
// in this same binary so nothing new ships.
mod mcp;

// Installing Claude Code for the user, rather than printing a command at them.
mod install;

// Text -> the IPA phonemes Kokoro speaks. Dictionary-based on purpose; see
// the note at the top of the file about espeak-ng's licence.
mod g2p;

// Kokoro, on the user's own machine. Downloaded on request, never bundled.
mod tts;

// Asking whether there is a newer helloim.ai, and installing it when somebody says
// so. Nothing here runs on a timer and nothing installs itself.
mod update;

// Suggestions & Feedback: a native POST to helloim.ai/api/feedback, because the
// webview CSP allows nameos.ai and not helloim.ai. See feedback.rs for why the
// endpoint is not moved onto the CSP-allowed host, and why the trust boundary
// stays on the server.
mod feedback;

// Signing in to helloim.ai from the app, and syncing settings with it --
// desktop/WEBAPP-SYNC-SPEC.md. A SECOND, entirely optional account space from
// `providers.rs`'s nameos.ai update/entitlement path, added on top of it and
// never touching it. Native POST, same CSP reasoning as feedback.rs above;
// the session token lives in its own OS keyring namespace, never the one
// `connectors::KEYRING_SERVICE` already uses for real secrets. See
// account.rs's own header for the full design and what it deliberately does
// not do.
mod account;

// The network half of shared-memory sync (GET/POST/DELETE
// /api/memory/facts) -- the local half (dedupe, delete-safety, the
// remote_fact_id/remote_seq bookkeeping) stays in facts.rs, untouched, per
// that file's own note on why the two are deliberately separate modules.
mod memory_sync;

// How the assistant sounds before the user has told us anything. Appended to
// the system prompt on every turn, because the alternative — a bare model with
// no instructions — answers like a support ticket.
mod voice;

// "Hear the difference" — the payoff on the "In your own words" pane. A
// deliberately separate, toolless, one-shot path: `send()` runs in the person's
// folder, which now carries their voice card, so it cannot produce a draft that
// genuinely lacks it. Read the module note before touching it — every flag in
// there is answering a specific failure.
mod voice_demo;

// What THIS machine can hold, and which local brain to put on it. The old flow
// offered one button — "Get Ollama" — and left the customer to guess a model
// against hardware nobody had measured.
mod brain_setup;

// The agent marketplace, and which button each entry gets. One agent per
// category runs at a time, and that rule lives here rather than in the page
// because it is a safety property, not a presentation choice.
mod marketplace;

// The summonable companion window and the global hotkey that raises it —
// the second Tauri window this app has ever had, and the first thing here
// that has to work while the app has no focus at all. See the module's own
// header for why it is a second window rather than a resize of `main`.
mod companion;
mod mic_hotkeys;

// What this app does when it cannot draw a window. It used to do nothing at
// all -- launch, stay alive, map nothing, print nothing, on a release build
// that has no stderr to print to anyway. Every diagnostic in this file is
// written down BEFORE the window is attempted, because the failure being
// diagnosed is the window never arriving.
mod startup;
// The other half of "there's no crash or hang report" (Beck's v1.0.1 report):
// a panic was already caught by startup.rs before this module existed, but
// nothing caught a hang -- the window still up, nothing panicked, and
// nothing answering -- and there was no way for a person to ever see any of
// it. See this module's own header for the full design and why it is not
// telemetry.
mod diagnostics;
mod folder_features;

// Local speech-to-text for the window WebKitGTK gives us none — see the
// module's own header for the full story. Declared here rather than beside
// `tts.rs` (voice OUTPUT) only because that is where every other feature
// module in this file already sits alphabetically-ish by arrival date, not
// by symmetry; the two are siblings in every way that matters.
mod stt;

// Where the microphone is actually opened on Linux, as of 2026-09-14 — see
// `native_capture.rs`'s own header for the incident (Beck's synthetic-mic
// rig: an 11s hold arriving at whisper as 0.6-0.7s, unbounded WebKitWebProcess
// fd growth, `cannot create wakeup pipe` in the GStreamer log) that moved
// capture out of the webview and in here. `stt.rs` above still does the
// actual HTTP call to whisper.cpp — this module only replaces WHERE THE
// AUDIO COMES FROM, feeding it the same `LocalWhisperProvider`.
//
// Linux-only at the `mod` level, not just inside the file: `cpal` is a
// Linux-only Cargo dependency (see Cargo.toml's own comment by its `cpal =
// "0.15"` line) — including this module in a Windows build would fail to
// link a dependency that build never pulls in. Windows keeps its existing,
// working `SpeechRecognitionCtor` path and never touches this file at all,
// so `native_capture_start`/`native_capture_stop_and_transcribe` need a
// same-named stand-in there purely so `generate_handler!` below has one
// symbol to register on every platform — see those two `#[cfg(not(target_os
// = "linux"))]` stubs further down, same shape as `pick_folder`/`pick_files`
// already use in this file for the identical reason.
#[cfg(target_os = "linux")]
mod native_capture;
#[cfg(target_os = "linux")]
use native_capture::{native_capture_start, native_capture_stop_and_transcribe};

/// Windows/macOS never call this — `ui/index.html` only reaches
/// `native_capture_start`/`stop_and_transcribe` down the `LOCAL_STT_AVAILABLE`
/// branch, which is false wherever `SpeechRecognitionCtor` (the browser's own
/// Web Speech API, present on WebView2) is real. It exists only so these two
/// command names resolve on every platform for `generate_handler!` — see
/// `native_capture.rs`'s own header and the comment above this module's
/// Linux-only `mod` line.
#[cfg(not(target_os = "linux"))]
#[tauri::command]
fn native_capture_start() -> Result<(), String> {
    Err("Native microphone capture is only built into the Linux app.".into())
}

#[cfg(not(target_os = "linux"))]
#[tauri::command]
fn native_capture_stop_and_transcribe() -> Result<String, String> {
    Err("Native microphone capture is only built into the Linux app.".into())
}

// On Windows, spawning a console program (claude) pops a black console window
// every single turn — it flashes open and shut as the child starts and exits.
// CREATE_NO_WINDOW suppresses it. On every other platform there is no console to
// suppress, so this is a no-op. (The app itself is already windowless via the
// windows_subsystem attribute above; this is about the CHILD process.)
#[cfg(windows)]
pub(crate) fn hide_console(cmd: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    cmd.creation_flags(CREATE_NO_WINDOW);
}
#[cfg(not(windows))]
fn hide_console(_cmd: &mut Command) {}

/// Everything the window owns about the run currently in flight.
///
/// `session_id` is the reason this is state at all rather than a local: Claude
/// Code hands one back on its first `system/init` line, and passing it to
/// `--resume` on the next turn is what turns a series of one-shot calls into a
/// conversation. Losing it silently would look exactly like a model with no
/// memory, which is the failure this whole product exists to fix.
/// `turn` USED TO BE `child: Mutex<Option<Child>>` AND THE CHANGE IS THE WHOLE
/// SEAM. A `Child` is one engine's idea of a running turn; a native engine has
/// no child process to hold. What the app actually needs from a turn in flight
/// is two answers — is it still going, and stop it — which is what
/// `engine::RunningTurn` is and all it is.
#[derive(Default)]
pub(crate) struct Session {
    /// **THE `u64` IS A GENERATION, NOT DECORATION — added 2026-09-04,
    /// Cassandra's race 2 of 2 on the deadlock fix.** `cancel_running_turn`
    /// empties this slot the MOMENT it `take()`s a turn out, well before
    /// that turn's own `cancel()` (a real kill, real time) returns. A
    /// `send()` landing in that exact window reads the slot as free and
    /// starts a brand new turn — correctly. The generation is what lets
    /// that OLD turn's own, still-in-flight `finished()` callback (see
    /// `clear_turn_if_still_current`) tell "the slot still holds me" apart
    /// from "a newer turn has since taken my place," which mere presence or
    /// absence cannot do: a blind `*guard = None` would silently untrack the
    /// NEW turn just because the slot happened to be occupied when the OLD
    /// one's callback finally ran.
    turn: Mutex<Option<(u64, Box<dyn engine::RunningTurn>)>>,
    /// The next generation to hand out. See `turn`'s own doc above.
    next_generation: std::sync::atomic::AtomicU64,
    // pub(crate): providers::select_provider clears it on a real switch, so a
    // conversation is never resumed into a different brain.
    pub(crate) session_id: Mutex<Option<String>>,
    /// **SERIALIZES CONCURRENT CANCELLERS — added 2026-09-04, Cassandra's
    /// race 1 of 2.** See `cancel_running_turn`'s own doc for the orphaned-
    /// process race this closes: Stop and window-close can both call it at
    /// once, and only ONE of them may be mid-`turn.cancel()` (a real kill)
    /// at a time, with the second one WAITING for the first to genuinely
    /// finish before it can conclude "nothing left to do."  A different lock
    /// from `turn` above, on purpose: nothing `turn.cancel()`'s own callback
    /// chain ever touches this one, so holding it for the whole call cannot
    /// reintroduce the reentrant-mutex deadlock the `take()` rewrite exists
    /// to fix.
    cancel_gate: Mutex<()>,
}

#[derive(Clone, Serialize)]
struct Finished {
    code: i32,
    session_id: Option<String>,
}

#[derive(Clone, Serialize)]
struct Line {
    text: String,
}

/// **THE SAFE AGENCY LAYER'S PER-CALL CONFIRM — `OpenUrl` only,
/// `SAFE-AGENCY-SPEC.md` room amendment 2.** `TurnSink::confirm` needs to
/// block the running turn's own thread on a real answer from the person
/// looking at the window; this is that answer's postbox. One pending
/// confirmation per id, and an id exists only between the moment `confirm`
/// registers it and the moment it is answered or times out — nothing here
/// survives past one round trip.
#[derive(Default)]
struct PendingConfirms {
    next_id: std::sync::atomic::AtomicU64,
    waiting: Mutex<HashMap<u64, std::sync::mpsc::Sender<bool>>>,
}

/// The payload behind `claude:confirm` — `index.html`'s own confirm sheet
/// reads `id` back verbatim on the button press, which is the only thing
/// that lets `answer_open_url_confirm` find the right postbox above.
#[derive(Clone, Serialize)]
struct ConfirmRequest {
    id: u64,
    prompt: String,
}

/// How long a confirm prompt waits for a person before it is treated as
/// declined. Long enough that reading a URL and deciding is not a race —
/// this is a security prompt, not a toast — short enough that a turn nobody
/// is at the keyboard for does not hang the window indefinitely: `Ending`
/// still owns the turn, but nothing else advances while this thread blocks.
const CONFIRM_TIMEOUT: Duration = Duration::from_secs(120);

/// The window's end of `engine::TurnSink::confirm` — see that method's own
/// doc for why the default (every OTHER sink) is `false` and this is the one
/// override with a real answer behind it.
///
/// **THIS COULD NOT BE VERIFIED FROM THIS BOX, SAID PLAINLY.** Compiled and
/// type-checked (`cargo check`, and cross-checked for the Windows target),
/// never run: there is no Tauri window to click a button in here. What Beck
/// needs to prove on the real Windows box, specifically: the `claude:confirm`
/// event actually reaches `index.html`, the sheet it opens actually shows the
/// real URL, pressing Allow/Cancel actually calls `answer_open_url_confirm`
/// with the matching id, and a call left unanswered actually times out at
/// `CONFIRM_TIMEOUT` rather than hanging the turn.
fn confirm_via_window(app: &AppHandle, window_label: &str, prompt: &str, cancelled: Option<&std::sync::atomic::AtomicBool>) -> bool {
    let state = app.state::<PendingConfirms>();
    let id = state.next_id.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let (tx, rx) = std::sync::mpsc::channel();
    state.waiting.lock().unwrap().insert(id, tx);

    let sent = app.emit_to(
        window_label,
        "claude:confirm",
        ConfirmRequest { id, prompt: prompt.to_string() },
    );
    // The event could not even be SENT (a closed window, most plausibly) --
    // nobody will ever answer this id, so waiting out the full timeout would
    // just be a slow way of finding out what is already known. Clean up the
    // postbox immediately rather than leaving a dead entry for the next
    // `answer_open_url_confirm` call to find and be confused by.
    if sent.is_err() {
        state.waiting.lock().unwrap().remove(&id);
        return false;
    }

    // FAIL CLOSED ON A TIMEOUT OR A DROPPED SENDER, NEVER OPEN. A person who
    // never answers has not said yes; `recv_timeout`'s `Err` covers both "the
    // clock ran out" and "the sender was dropped without sending" (which
    // cannot currently happen — nothing removes an entry except this
    // function and `answer_open_url_confirm` — but the failure mode of a
    // channel error is the same "no answer" either way, and treating it as
    // anything but a decline would be trusting an absence of a signal).
    let deadline = std::time::Instant::now() + CONFIRM_TIMEOUT;
    let answer = loop {
        if cancelled.map(|c| c.load(std::sync::atomic::Ordering::SeqCst)).unwrap_or(false)
            || std::time::Instant::now() >= deadline { break false; }
        match rx.recv_timeout(Duration::from_millis(50)) {
            Ok(answer) => break answer,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
            Err(_) => break false,
        }
    };
    let _ = app.emit_to(window_label, "claude:confirm-ended", serde_json::json!({"id":id}));
    state.waiting.lock().unwrap().remove(&id);
    answer
}

/// The button press in `index.html`'s confirm sheet lands here. **Silently
/// does nothing for an id that is not waiting** — answered already, timed
/// out already, or a stale id from a previous turn's sheet the person had
/// left open — rather than returning an error the front end has no
/// meaningful way to act on; the turn that mattered has already moved on
/// either way.
#[tauri::command]
fn answer_open_url_confirm(state: State<PendingConfirms>, id: u64, allow: bool) {
    if let Some(tx) = state.waiting.lock().unwrap().remove(&id) {
        let _ = tx.send(allow);
    }
}

/// Claude Code's own harmless "I don't recognise this model ID" diagnostic.
///
/// **THIS IS EXPECTED ON EVERY SINGLE LOCAL-BRAIN TURN, NOT A SIGN ANYTHING IS
/// WRONG.** Verified against Anthropic's own docs, 2026-08-31
/// (code.claude.com/docs/en/errors#unrecognized-model-id-on-a-request):
/// *"This warning appears when Claude Code encounters a model ID it doesn't
/// recognize in its internal model registry... does not prevent the request
/// from completing... Claude Code proceeds to send the request to the
/// configured API endpoint... The warning can appear regardless of whether
/// ANTHROPIC_BASE_URL points to Anthropic's API or a custom/non-Anthropic
/// endpoint."* An Ollama tag like `llama3.2:latest` is never going to be in
/// that internal registry — no local or third-party model ever will be — so
/// this line fires on literally every turn a local brain answers, and the
/// answer still comes.
///
/// **THIS WAS BEING FORWARDED AS `claude:stderr`, WHICH THE WINDOW PAINTS
/// RED AND READS AS A FAILURE.** Mark's first message to a local brain came
/// back as this diagnostic in a red bubble before a word of the real answer
/// had arrived, and the only sane-looking response to what LOOKS like a
/// broken app — pressing Stop — killed the run that was about to answer him
/// correctly. A brain that is working must not look broken on its first
/// message. See the stderr thread in `send()`, and `providers::test_binary`,
/// which had the same shape of bug in the other direction (discarding this
/// line rather than acting on it).
///
/// **b17 SHIPPED THIS AS AN EXACT-EQUALITY CHECK AND IT NEVER MATCHED —
/// Beck, 2026-08-31.** The marker was assumed to be its own whole line, with
/// the `{"model":...}` detail on a SEPARATE line beneath it, because that is
/// how it read in the screenshot Mark sent. It is not two lines. Beck spawned
/// the real `claude.exe` 2.1.251 nine times, reproducing `send()`'s exact
/// child process — same args, same six `provider_env()` variables, stdin
/// closed — and every run wrote **one** stderr line: the marker, a single
/// space, then the JSON detail, no newline between them. An exact-equality
/// check against a 33-character marker can never match a longer line it is
/// only ever a PREFIX of, so `swallow_detail` never armed and every local
/// turn painted this notice red. The predicate now matches the marker as a
/// prefix. **The old two-line shape is kept working too** — a different
/// `claude` build may still split it, and Beck could not confirm which
/// version ships on Mark's own machine (his account, not hers) — so a bare
/// marker line with nothing following it still arms `swallow_detail` for the
/// next line, exactly as before.
pub(crate) const UNRECOGNIZED_MODEL_MARKER: &str = "[claude-code:unrecognized_model]";

pub(crate) fn is_unrecognized_model_notice(line: &str) -> bool {
    line.trim_start().starts_with(UNRECOGNIZED_MODEL_MARKER)
}

/// The line beneath the notice above, when the notice occupies a line on its
/// own — its OWN detail, `{"model":"llama3.2:latest","query_source":"sdk"}`,
/// not a second, unrelated stderr message. Only swallowed immediately after a
/// notice line that carried nothing past the marker itself (see
/// `is_unrecognized_model_notice`'s doc comment for why that is now the
/// less common of the two real shapes), and only when it is actually shaped
/// like that detail (a bare JSON object naming a model), so a real error
/// that merely happens to start with `{` is never eaten by mistake.
pub(crate) fn is_unrecognized_model_detail(line: &str) -> bool {
    let t = line.trim();
    t.starts_with('{') && t.contains("\"model\"")
}

/// Resolve the `claude` binary. `Command` uses the inherited PATH, and a
/// desktop app launched from a file manager does not get the shell's PATH — so
/// the usual install location is checked by hand before giving up. Reporting
/// "not found" when it is sitting in ~/.local/bin is the kind of confident
/// wrong answer that costs an afternoon.
///
/// `REMEMBRANCER_CLAUDE` overrides everything, for anyone who installed it
/// somewhere we do not think to look.
pub(crate) fn claude_binary() -> String {
    if let Some(explicit) = std::env::var_os("REMEMBRANCER_CLAUDE") {
        return explicit.to_string_lossy().into_owned();
    }
    /* THIS USED TO READ `HOME` AND NOTHING ELSE, WHICH IS NEVER SET ON WINDOWS
       — so on the platform this ships to it fell straight through to a bare
       "claude" and depended entirely on PATH. That fails in the exact moment it
       matters most: right after the installer runs, when the new PATH entry has
       not reached this already-running process. The app would install Claude
       successfully and then report it missing.
       home_dir() knows about USERPROFILE, and both of the installer's landing
       spots are checked by hand. */
    if let Some(home) = home_dir() {
        let candidates: &[&str] = if cfg!(windows) {
            &[".local/bin/claude.exe", ".claude/bin/claude.exe", "AppData/Roaming/npm/claude.cmd"]
        } else {
            &[".local/bin/claude", ".claude/bin/claude"]
        };
        for rel in candidates {
            let p = home.join(rel);
            if p.is_file() {
                return p.to_string_lossy().into_owned();
            }
        }
    }
    "claude".to_string()
}

/// What the window needs to know before it lets anyone type: is the thing this
/// app is a front end for actually here and runnable.
#[derive(Clone, Serialize)]
struct Preflight {
    ready: bool,
    /// Which of the two failures it is, so the window can say something useful
    /// rather than "an error occurred": "missing" or "broken".
    problem: Option<String>,
    path: String,
    version: Option<String>,
    detail: Option<String>,
    /// Is the binary actually SIGNED IN — Mark, 2026-08-27: "bella was setup,
    /// but she said 'please run login'. However, in top left, claude is
    /// showing green."
    ///
    /// **`--version` SUCCEEDING WAS BEING READ AS "CONNECTED", AND IT IS NOT
    /// THE SAME CLAIM.** A logged-out Claude Code answers `--version`
    /// perfectly and then refuses the first real request with "please run
    /// /login". So the chip went green on a machine that could not think, and
    /// the first thing the product did was contradict its own status light —
    /// the precise failure this codebase's honesty rules exist to prevent:
    /// green is earned by a real round trip, never by a string being
    /// non-empty.
    ///
    /// `claude auth status` reports it as JSON. None means we could not tell
    /// (an older binary without the subcommand), which is deliberately NOT
    /// false: "we could not ask" and "it is signed out" send somebody to
    /// different places.
    logged_in: Option<bool>,
}

/// How long `preflight()` and `claude_logged_in()` will wait for the binary
/// before giving up on it.
///
/// **THE RESIDUAL OF W5 -- Jarvis, 2026-08-29.** W5 changed the wizard and
/// tour to wait for preflight to have ANSWERED, not for it to have answered
/// "yes" -- but that only helps if preflight actually answers. `Command::
/// output()` has no timeout of its own: a wedged binary, a PATH entry that
/// resolves to something that reads stdin forever despite `Stdio::null()`, a
/// broken symlink into an interactive shell -- any of it and the Rust command
/// simply never returns. `claudeState` stays `null` in the window forever,
/// which is a WORSE failure than the one W5 fixed: not "not ready" but
/// *unknown, silently, permanently* -- no wizard, no tour, no sign-in sheet,
/// nothing on screen saying why, because every one of those now correctly
/// waits for an answer that is never coming. Ten seconds is generous for a
/// version check and short enough that a genuinely wedged binary reads as
/// broken within one human's patience, not one coffee break.
const PREFLIGHT_TIMEOUT: Duration = Duration::from_secs(10);

/// Runs `cmd` to completion, but never for longer than `timeout`. Kills it and
/// returns a `TimedOut` error otherwise.
///
/// READS STDOUT AND STDERR ON THEIR OWN THREADS WHILE POLLING FOR EXIT,
/// rather than after the process ends. An OS pipe has a finite buffer (64 KiB
/// on Windows); a wait loop that only calls `try_wait()` and reads afterwards
/// would deadlock against a child that writes more than that before exiting --
/// the very bug this function exists to remove, rebuilt one layer down.
/// `claude --version` and `claude auth status --json` print at most a few
/// hundred bytes today, so that could not actually be provoked here, but a
/// timeout helper that is only safe for small output is a trap for whoever
/// reaches for it next expecting a general-purpose one.
///
/// Killing the child closes its end of both pipes, which is what lets the two
/// reader threads see EOF and return promptly once the deadline is hit --
/// `join()` below is not itself bounded, and does not need to be for that
/// reason.
fn run_with_timeout(mut cmd: Command, timeout: Duration) -> std::io::Result<Output> {
    cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd.spawn()?;
    let stdout_pipe = child.stdout.take();
    let stderr_pipe = child.stderr.take();
    let out_reader = thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(mut s) = stdout_pipe {
            let _ = s.read_to_end(&mut buf);
        }
        buf
    });
    let err_reader = thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(mut s) = stderr_pipe {
            let _ = s.read_to_end(&mut buf);
        }
        buf
    });

    let start = Instant::now();
    let status = loop {
        match child.try_wait()? {
            Some(status) => break status,
            None => {
                if start.elapsed() >= timeout {
                    // Best-effort: a child that has already exited between the
                    // check above and here makes `kill()` fail harmlessly, and
                    // `wait()` reaps whichever of the two actually happened.
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::TimedOut,
                        format!("did not answer within {timeout:?}"),
                    ));
                }
                thread::sleep(Duration::from_millis(25));
            }
        }
    };
    // .unwrap_or_default() rather than propagating a reader thread's panic:
    // losing the captured output on the (currently theoretical) chance a
    // reader thread panicked is a smaller failure than tearing down the
    // caller over it.
    let stdout = out_reader.join().unwrap_or_default();
    let stderr = err_reader.join().unwrap_or_default();
    Ok(Output { status, stdout, stderr })
}

/// Ask the binary whether it is signed in. Cheap, local, no model call.
///
/// A timeout collapses into the same `None` the doc comment on `Preflight::
/// logged_in` already defines for "we could not tell" -- no new state to
/// invent, because that is exactly what a timeout is.
fn claude_logged_in(path: &str) -> Option<bool> {
    let mut cmd = Command::new(path);
    cmd.args(["auth", "status", "--json"]);
    hide_console(&mut cmd);
    let out = run_with_timeout(cmd, PREFLIGHT_TIMEOUT).ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    // Parsed, not string-matched: "loggedIn": false contains the word too.
    let v: serde_json::Value = serde_json::from_str(text.trim()).ok()?;
    v.get("loggedIn").and_then(|b| b.as_bool())
}

/// Run `claude --version` and find out for real.
///
/// This exists because the alternative is what the app did before: let someone
/// type their first request, press Send, and get a raw spawn error in red. On a
/// machine where Claude Code was never installed that is the entire first
/// impression of the product — an app that looks broken, for a reason that is
/// not its fault and that it never explains.
///
/// It deliberately does NOT try to answer "are they signed in". Nothing cheap
/// proves that; the only honest test is a real request, and running one behind
/// their back would spend their money to draw a screen. That case is caught at
/// run time instead, from what the failure actually says.
#[tauri::command(async)]
fn preflight() -> Preflight {
    let path = claude_binary();

    let mut cmd = Command::new(&path);
    cmd.arg("--version");
    /* PLUGINS, THE WAY CLAUDE CODE ALREADY TAKES THEM -- Mark, 2026-08-26:
       `claude --plugin-dir ./connect-apps-plugin`. Verified before this was
       written: loading that directory really does turn `connect-apps:setup`
       into a command the model can run.
       One flag per plugin, every turn, from the folder the Skills view
       installs into. Nothing is bundled and nothing is generated -- the
       capability is the binary's own. */
    for dir in skills::plugin_dirs() {
        cmd.arg("--plugin-dir").arg(dir);
    }

    hide_console(&mut cmd);
    // BOUNDED, NOT `cmd.output()` -- Jarvis, 2026-08-29 (the W5 residual). See
    // `run_with_timeout`'s own comment. A timeout surfaces as `Err` with kind
    // `TimedOut`, which the `Err(e)` arm below already treats as "broken" with
    // a real detail string -- the exact "say so on screen" fallback asked
    // for, reusing the path that already existed for a binary that will not
    // run, rather than inventing a fourth state for the window to learn.
    let out = run_with_timeout(cmd, PREFLIGHT_TIMEOUT);

    match out {
        Ok(o) if o.status.success() => Preflight {
            ready: true,
            problem: None,
            logged_in: claude_logged_in(&path),
            path,
            version: Some(String::from_utf8_lossy(&o.stdout).trim().to_string())
                .filter(|v| !v.is_empty()),
            detail: None,
        },
        // It is there and it will not run — a broken install, wrong
        // architecture, missing runtime. Different problem, different advice.
        Ok(o) => Preflight {
            ready: false,
            problem: Some("broken".into()),
            logged_in: None,
            path,
            version: None,
            detail: Some(String::from_utf8_lossy(&o.stderr).trim().to_string())
                .filter(|d: &String| !d.is_empty()),
        },
        Err(e) => Preflight {
            ready: false,
            problem: Some(if e.kind() == std::io::ErrorKind::NotFound {
                "missing".into()
            } else {
                "broken".into()
            }),
            logged_in: None,
            path,
            version: None,
            detail: Some(e.to_string()),
        },
    }
}

/// `pub(crate)`, not private — `providers::select_provider` needs the same
/// answer `send()` already refuses a second turn on, for the same reason. See
/// its call site for the race this closes.
pub(crate) fn alive(session: &Session) -> bool {
    match session.turn.lock().unwrap().as_ref() {
        Some((_, turn)) => turn.is_alive(),
        None => false,
    }
}

/// Which OS this build actually is, told to the front end so it can offer
/// Linux's account-optional path (`ui/index.html`'s `ACCOUNT_OPTIONAL`,
/// `skipAccountGate()`) without one `index.html` having to guess from a
/// browser fingerprint (`navigator.platform`/UA sniffing) — this crate
/// already cross-compiles per target (plain `cargo build` here vs `cargo
/// xwin build` for Windows, see `desktop/ship-windows.sh`), so `cfg!` is
/// baked in at compile time and is always correct for the binary actually
/// running, never a guess about the machine underneath it.
///
/// **WINDOWS IS UNCHANGED BY THIS COMMAND EXISTING.** It answers `false`
/// there, `ui/index.html` never shows the skip control, and every path
/// through the gate — the unconditional `/api/auth/me` check, the front
/// porch, the login/signup forms — runs exactly as it did before this file
/// existed. Mark, 2026-09-02, on the very feature the gate sits behind: "the
/// subscription IS the gate... do not weaken or remove the account/licensing
/// system" — that instruction was never about Linux, which had no working
/// voice input to gate access TO until stt.rs; extending Linux specifically,
/// 2026-09-14, is a separate, later decision, not a reversal of the first.
#[tauri::command]
fn is_linux_build() -> bool {
    cfg!(target_os = "linux")
}

#[tauri::command]
fn is_running(state: State<Session>) -> bool {
    alive(&state)
}

#[tauri::command]
fn current_session(state: State<Session>) -> Option<String> {
    state.session_id.lock().unwrap().clone()
}

/// Drop the remembered session id, so the next prompt starts a fresh thread.
#[tauri::command]
fn new_conversation(state: State<Session>) {
    *state.session_id.lock().unwrap() = None;
}

/// "Clear conversation history" — the one-click privacy control, added
/// 2026-09-03 in the overnight hardening pass.
///
/// **WHAT THIS ACTUALLY REMOVES, SAID ONCE SO THE BUTTON'S OWN DESCRIPTION
/// AND THIS FUNCTION CANNOT DRIFT APART: every `<id>.json` transcript this app
/// has saved under `conversations_dir`, FOR ANY BRAIN** — not native-engine
/// only, not anymore. Until 2026-09-05 that WAS the whole of it, because
/// `engine::native` was the only thing that ever wrote here (see that
/// function's own doc history). `Engine::persists_own_history` and
/// `AppSink`'s generic capture (`main.rs::capture_turn_into_history`) now
/// write OUR OWN copy of a Claude turn's prompt and answer into this exact
/// directory too, in the identical `<id>.json` shape, keyed on `claude.exe`'s
/// own real session id rather than one of ours — and this sweep never had to
/// change to cover it, because it was never native-specific to begin with: it
/// already deletes every `.json` file in the folder, no id-prefix check (see
/// `clear_conversation_files`'s own body). **And the remembered `--resume`
/// session id** — the same one `new_conversation` above already drops for
/// "start a fresh thread". Both together, because leaving the id behind would
/// let the very next turn, on either engine, resume a conversation whose
/// transcript this button just told the person it deleted.
///
/// **WHAT THIS DELIBERATELY DOES NOT TOUCH, and each is a decision recorded
/// where it lives, not an oversight here:**
/// - **`claude.exe`'s OWN internal session storage** — its account-scoped
///   files outside this app's `app_config_dir` entirely, a genuinely
///   different thing from OUR copy above. Reaching into another program's
///   data directory from a button labelled for OUR history is exactly the
///   kind of scope creep a privacy control must not have, and staying out of
///   it is the whole reason `AppSink` captures its own copy from the event
///   stream rather than reading anything `claude.exe` itself wrote to disk.
///   A person who presses this button gets OUR record of what was said
///   erased; `claude.exe`'s own separate copy, wherever the vendor binary
///   keeps it, is untouched — same boundary as always, narrower than it
///   might sound.
/// - **Tier 3 memory (`facts.rs`, `.helloim/memory/facts.md`).** Its own
///   module doc: "Delete a line here and it is forgotten... Forgetting is
///   line-granular, on purpose", with `facts::forget` already the tool for
///   it. A bulk wipe from an unrelated button would defeat a design chosen
///   deliberately, and worse, do it silently.
/// - **Tier 2, the session bridge (`memory.rs`, `.helloim/memory/` INSIDE the
///   customer's own working folder).** Its own module doc: "cancel and it is
///   still yours" — that file is the person's, living in a project folder
///   they chose, not app cache. A folder-scoped delete from an app-wide
///   button would need to know which of possibly many working folders to
///   reach into, and guessing wrong deletes the wrong project's memory.
///
/// Returns how many transcript files were actually removed, so the screen
/// can say a true number rather than "Done."
///
/// **THE ACTUAL SWEEP IS `clear_conversation_files` BELOW, NOT HERE — same
/// reason `cancel_running_turn` was pulled out from under `stop` a screen
/// down.** `State<Session>` has no public constructor outside a running app,
/// so a test that wants to prove files are genuinely gone from disk needs a
/// path and nothing else; this command's only job is handing it one.
#[tauri::command]
fn clear_conversations(app: AppHandle, state: State<Session>) -> Result<usize, String> {
    // Same drop as `new_conversation`, run unconditionally and first: an
    // in-memory write that cannot itself fail, so a filesystem error below
    // still leaves this half done rather than none of it.
    *state.session_id.lock().unwrap() = None;

    // No directory resolved, or none ever created, means nothing was ever
    // stored — zero removed is the true answer, not a reason to error the
    // button a person just pressed hoping for a clean slate.
    let Some(dir) = conversations_dir(&app) else { return Ok(0) };
    clear_conversation_files(&dir)
}

/// Delete every conversation transcript in `dir` and say how many were
/// actually removed. Pure filesystem work, no `AppHandle`, no `State` — see
/// `clear_conversations` above for why that split exists.
fn clear_conversation_files(dir: &std::path::Path) -> Result<usize, String> {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        // A directory `conversations_dir` itself just create_dir_all'd
        // failing to open for reading is a genuinely strange machine state
        // (permissions changed underneath us, a drive unmounted) — report it
        // rather than silently claiming success on a folder we could not see
        // into.
        Err(e) => return Err(format!("could not read {dir:?}: {e}")),
    };
    let mut removed = 0usize;
    for entry in entries.flatten() {
        let path = entry.path();
        // ONLY our own `<id>.json` transcripts. `conversations_dir` is a
        // folder this app owns and creates, so nothing else belongs in it —
        // but filtering by extension costs nothing and means a stray file
        // dropped in by hand (an editor swap file, a `.DS_Store`) is left
        // alone instead of being swept up by an unqualified "delete
        // everything in this directory."
        if path.extension().and_then(|e| e.to_str()) == Some("json") && std::fs::remove_file(&path).is_ok()
        {
            removed += 1;
        }
    }
    Ok(removed)
}

/// The actual work behind the Stop button, and also behind closing the window
/// (see `on_window_event` in `main()`) -- pulled out of the `#[tauri::command]`
/// below so it takes a plain `&Session` instead of a `tauri::State`. `State`
/// has no public constructor outside a running app (its one field is private,
/// and `StateManager::new` is `pub(crate)`), so a test that only ever gets
/// `&Session` is the only way this behaviour is checkable by `cargo test` at
/// all.
///
/// **THE PROOF THAT IT REALLY KILLS THE PROCESS MOVED WITH THE PROCESS.** The
/// window-close bug (NameOS.exe alive 187s after every window was gone, Beck
/// b21/b20) would have looked identical to a correct fix if nothing external to
/// the Rust `Child` handle was ever checked, so that `/proc/<pid>` test still
/// exists -- it is `engine::claude_code::tests::cancel_actually_terminates_the_
/// process`, which is where the `Child` now lives.
/// `cancel_running_turn_calls_through_to_the_engine` below covers this half.
///
/// **THE SLOT IS CLEARED ONLY ON SUCCESS**, which is the pre-seam behaviour: a
/// turn we failed to stop is still out there, and forgetting the handle is the
/// one response guaranteed to make it unstoppable.
///
/// **THE LOCK IS NEVER HELD WHILE `cancel()` RUNS — fixed 2026-09-04, and this
/// is the single most important line in this function.** The old body took
/// `session.turn.lock()` as a `MutexGuard` and held it across the WHOLE call
/// to `turn.cancel()`. For `ClaudeCodeRun` that was harmless — its `cancel()`
/// is pure `Child::kill()`/`wait()`, nothing else. It is NOT harmless for
/// `NativeRun`: `NativeRun::cancel()` -> `Ending::close()` -> unconditionally
/// -> `sink.finished()`, and the real sink in this app, `AppSink::finished`
/// (below), re-locks `state.turn` to clear it. `std::sync::Mutex` is not
/// reentrant, so a thread already holding that lock blocking on it again is a
/// same-thread, permanent self-deadlock — and because `stop` and the
/// `CloseRequested` handler both ran this on the MAIN thread (see both below),
/// that deadlock froze the entire window. Diagnosed from a live report: the
/// transcript showed "Stopped. Nothing from this turn was saved to the
/// conversation." (the exact text `NativeRun::cancel`'s own closure emits,
/// which runs and reaches the screen BEFORE the fatal second lock attempt),
/// then the app went unresponsive with no further event ever arriving — the
/// signature of a hang, not a crash.
///
/// The fix: `take()` the turn OUT of the slot first — the guard that produces
/// is a temporary, dropped at the end of this statement, so the lock is
/// released before `cancel()` is even called. `cancel()` then runs with
/// nothing of ours locked, so whatever it calls back into (including a second
/// visit to this exact mutex) finds it free. "Cleared only on success" still
/// holds: on `Err` the turn is put back rather than left forgotten, the same
/// outcome the old code produced, just without the reentrancy hazard on the
/// way there.
///
/// **THAT FIX OPENED TWO RACES OF ITS OWN — found by Cassandra, 2026-09-04,
/// reviewing it, both closed the same day, neither a design change from the
/// shape above:**
///
/// 1. **Stop+close could orphan `claude.exe`.** `take()` empties the slot
///    the instant it runs, well before `turn.cancel()` (a real kill) returns.
///    Nothing used to stop a SECOND caller — window-close, racing a Stop
///    already in flight — from finding the slot already empty, returning
///    `Ok(())` immediately, and letting `on_window_event` proceed straight to
///    `app_handle.exit(0)` while the FIRST caller's kill was still running.
///    Exiting mid-kill is the b20/b21 orphan bug back under a race. Closed by
///    `session.cancel_gate` below, held for the WHOLE call: a second caller
///    now genuinely waits for the first's real kill work to finish before it
///    can conclude there is nothing left to do.
/// 2. **A blind clear/restore could wipe or overwrite a NEWER turn.** While
///    the slot reads empty (between `take()` and `cancel()` returning), a
///    `send()` can start turn B — correctly, since nothing is running. But
///    turn A's own `finished()` (if `cancel()` succeeds) or this function's
///    own put-back (if it fails) used to touch the slot unconditionally,
///    which would silently clear or overwrite B. `finished()` is closed by
///    `clear_turn_if_still_current`'s generation check; the put-back below is
///    closed by checking the slot is still genuinely empty before restoring.
fn cancel_running_turn(session: &Session) -> Result<(), String> {
    // See point 1 above. Held for the WHOLE call, including `turn.cancel()`
    // — a DIFFERENT lock from `session.turn`, so holding it here cannot
    // reintroduce the reentrant-mutex deadlock the `take()` rewrite exists
    // to fix, because nothing `turn.cancel()`'s own callback chain ever
    // touches `cancel_gate`.
    let _serialize = session.cancel_gate.lock().unwrap_or_else(|e| e.into_inner());

    let taken = session.turn.lock().unwrap().take();
    let Some((generation, turn)) = taken else { return Ok(()) };
    match turn.cancel() {
        Ok(()) => Ok(()),
        Err(e) => {
            // See point 2 above. `cancel_gate` rules out another CANCELLER
            // racing this one, but not a brand new turn: `send()` reads the
            // slot as free the moment `take()` ran, above, and has no reason
            // to wait on `cancel_gate` — starting a turn and cancelling one
            // are different operations. Restoring THIS turn is only safe if
            // nothing has claimed the slot since; if it has, that turn wins
            // and ours is left to be forgotten rather than overwriting it.
            let mut guard = session.turn.lock().unwrap();
            if guard.is_none() {
                *guard = Some((generation, turn));
            }
            Err(e)
        }
    }
}

/// Clear the tracked turn ONLY IF it is still the exact one this call
/// believes it is closing — identified by `generation`, never by mere
/// presence. See `cancel_running_turn`'s own doc, point 2, for the race this
/// closes: the slot reads empty for the whole span between `take()` and
/// `cancel()` returning, a `send()` landing in that window correctly starts
/// a brand new turn, and the OLD turn's own `finished()` callback running
/// late must not blindly clear a slot that by then belongs to someone else.
fn clear_turn_if_still_current(session: &Session, generation: u64) {
    let mut guard = session.turn.lock().unwrap();
    if matches!(guard.as_ref(), Some((g, _)) if *g == generation) {
        *guard = None;
    }
}

/// **`(async)` IS NOT DECORATION — same reasoning `pick_folder`'s own doc
/// comment gives, restated here because this is the second place it mattered
/// and the first time it actually bit.** A plain `#[tauri::command]` runs on
/// the MAIN thread; `stop` calls straight into `cancel_running_turn`, which
/// calls into an engine's `cancel()` — and `cancel()` is a trait method this
/// file does not control the runtime of (a wire's own cleanup, a child
/// process's `kill()`/`wait()`, whatever a future engine adds). The lock fix
/// above closes the ONE known way that could hang forever; `(async)` is the
/// second, independent net — even a merely SLOW cancel (a wedged `wait()`, an
/// OS scheduler hiccup) no longer freezes the window, because it now runs off
/// the thread that pumps it.
#[tauri::command(async)]
fn stop(state: State<Session>) -> Result<(), String> {
    cancel_running_turn(&state)
}

/// Where the generated `--mcp-config` lives. The app's own config directory,
/// not the working folder: it is ours, it is rewritten every turn, and putting
/// a generated file inside somebody's project would be litter they did not ask
/// for and would eventually commit.
fn mcp_config_path(app: &AppHandle) -> Result<std::path::PathBuf, String> {
    let dir = app
        .path()
        .app_config_dir()
        .map_err(|e| format!("no config directory: {e}"))?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("could not create {dir:?}: {e}"))?;
    Ok(dir.join("mcp-launch.json"))
}

/// Where OUR OWN conversation transcripts live — every engine's, as of
/// 2026-09-05.
///
/// **THE APP'S DIRECTORY, NOT THE PERSON'S PROJECT FOLDER**, for the same reason
/// `mcp-launch.json` is: it is ours, and a generated file inside somebody's
/// repository is litter they did not ask for and would eventually commit.
///
/// `None` on failure rather than an error: a person's words are worth more than
/// our bookkeeping, so the turn still runs and the engine says on screen that it
/// will not be remembered.
///
/// **USED TO SAY "the native engine's own", AND FOR OVER A DAY THAT WAS THE
/// WHOLE PROBLEM.** `engine::native` writes here from inside its own `drive`
/// loop (`Plan::store_dir` → `store::Conversation::save`); `ClaudeCodeEngine`
/// itself still ignores this value completely — it is a spawn of somebody
/// else's program and its own transcripts belong to `claude.exe`.
/// But `send`'s `AppSink` now writes here too, for any engine answering
/// `Engine::persists_own_history() == false`, capturing what it already sees
/// cross the wire rather than reading anything `claude.exe` itself wrote —
/// see that method's own doc and `capture_turn_into_history` for the design.
/// So this directory is genuinely shared now: some files were written from
/// inside an engine, some from the sink above it, and `list_conversation_
/// files` below does not, and must not, need to tell which is which.
pub(crate) fn conversations_dir(app: &AppHandle) -> Option<std::path::PathBuf> {
    let dir = app.path().app_config_dir().ok()?.join("conversations");
    match std::fs::create_dir_all(&dir) {
        Ok(()) => Some(dir),
        Err(e) => {
            eprintln!("could not create the conversations folder: {e}");
            None
        }
    }
}

/// One row in the History tab — Iris's IA spec, "Same layout as SupportAssist's
/// History... user can DELETE them, and filter over HOW MANY DAYS."
///
/// **EVERY BRAIN, AS OF 2026-09-05 — CORRECTED THE SAME DAY IT SHIPPED WRONG.**
/// This used to say "native-engine conversations only," on the reasoning that
/// `conversations_dir` only ever held `engine::native::store`'s own files, and
/// called the resulting empty History tab for anyone on the built-in Claude
/// brain "honestly empty rather than wrong." It was wrong in a different way:
/// Claude is the brain every fresh install starts on, so for the common case
/// the tab was empty on every visit, forever, which read to a real person —
/// Mark, 2026-09-05, running the shipped app — as "history doesn't even
/// work," not as an honest limitation. `Engine::persists_own_history` and
/// `AppSink`'s generic capture (`capture_turn_into_history`, a screen up) now
/// write our own copy of ANY engine's turn into this same directory, so this
/// list covers every brain a person might have chosen, not only the one that
/// happened to keep its own store from day one.
///
/// **WHAT DID NOT CHANGE:** this still never reaches into `claude.exe`'s own
/// account-scoped storage — see `capture_turn_into_history`'s own doc. The row
/// below comes from OUR copy of what was said, captured off the same event
/// stream the window itself renders, never from reading another program's
/// files.
///
/// **THE UI'S EMPTY-STATE COPY STILL SAYS THE OLD THING AND IS NOW WRONG —
/// FLAGGED FOR WREN, NOT FIXED HERE.** `ui/index.html`'s `iaRenderHistory`
/// tells an empty-list person "A cloud brain like Claude keeps its own
/// history separately — this list will be empty while that is the brain in
/// use." That sentence was true when it was written this same morning and is
/// false the moment this change ships: a Claude turn now DOES appear here.
/// The empty state a Claude-brain person with a genuinely fresh install sees
/// needs its own honest copy — "no conversations yet," not "not for your
/// brain" — and that is Wren's screen, not this file's.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ConversationSummary {
    id: String,
    /// First line of the first thing the PERSON typed, not the assistant's
    /// reply — that is what somebody scanning a list is trying to recognise
    /// this conversation by, never what it answered.
    title: String,
    snippet: String,
    /// Unix seconds, off the file's own mtime. `engine::native::store::
    /// Message` carries no per-message timestamp (see that module's own doc
    /// on what is deliberately not stored), so the file itself — rewritten
    /// atomically on every `commit` — is the only clock this has.
    updated_at: u64,
    message_count: usize,
}

/// First line, trimmed to a screen-width summary. Never empty: a
/// conversation whose only message so far is blank still needs a row that
/// does not read as a rendering bug.
fn summarize(text: &str, max_chars: usize) -> String {
    let first_line = text.lines().next().unwrap_or("").trim();
    if first_line.is_empty() {
        return "(no text)".to_string();
    }
    let truncated: String = first_line.chars().take(max_chars).collect();
    if first_line.chars().count() > max_chars {
        format!("{truncated}…")
    } else {
        truncated
    }
}

/// List every stored conversation, newest first. `days`: keep only rows
/// touched within the last N days; `None` returns everything. Never errors —
/// no folder, an unreadable folder, or a folder with nothing safe to read in
/// it are all an empty list, the same "nothing was ever stored" answer
/// `clear_conversations` gives for the identical cases.
///
/// **THE REAL WORK IS `list_conversation_files` BELOW, NOT HERE — same split
/// as `clear_conversations`/`clear_conversation_files` a screen up, and for
/// the identical reason.** `AppHandle` has no public constructor outside a
/// running app, so a test that wants to prove the day-cutoff and the sort
/// order against real files on a real clock needs a path and a `now` and
/// nothing else; this command's only job is handing those over.
#[tauri::command]
fn list_conversations(app: AppHandle, days: Option<u32>) -> Vec<ConversationSummary> {
    let Some(dir) = conversations_dir(&app) else { return Vec::new() };
    list_conversation_files(&dir, days, std::time::SystemTime::now())
}

/// Pure filesystem work: no `AppHandle`, no `Tauri` anything. See
/// `list_conversations` above for why this split exists. `now` is threaded
/// through rather than read here so a test can pick a fixed instant and
/// place a fixture's mtime relative to it, instead of racing the real clock.
fn list_conversation_files(
    dir: &std::path::Path,
    days: Option<u32>,
    now: std::time::SystemTime,
) -> Vec<ConversationSummary> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let cutoff = days.map(|d| now.checked_sub(Duration::from_secs(u64::from(d) * 86_400)).unwrap_or(now));

    let mut out = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        // Same filter `clear_conversation_files` already uses: only our own
        // `<id>.json` transcripts. This also naturally excludes the
        // `<id>.json.writing` scratch file `Conversation::save` briefly
        // creates — its extension is `writing`, not `json`.
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else { continue };
        if !engine::native::store::id_is_safe(stem) {
            continue;
        }
        let Some(convo) = engine::native::store::Conversation::load(dir, stem) else { continue };
        let modified = std::fs::metadata(&path).and_then(|m| m.modified()).unwrap_or(now);
        if let Some(cut) = cutoff {
            if modified < cut {
                continue;
            }
        }
        let updated_at = modified
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let first_user = convo
            .messages
            .iter()
            .find(|m| m.role == engine::native::store::Role::User)
            .map(|m| m.text.as_str())
            .unwrap_or("");
        let last = convo.messages.last().map(|m| m.text.as_str()).unwrap_or("");
        out.push(ConversationSummary {
            id: convo.id,
            title: summarize(first_user, 64),
            snippet: summarize(last, 140),
            updated_at,
            message_count: convo.messages.len(),
        });
    }
    out.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    out
}

/// Delete one conversation. Idempotent — deleting one already gone (a double
/// click, a stale list) is `Ok(())`, not an error the person did nothing to
/// deserve.
#[tauri::command]
fn delete_conversation(app: AppHandle, state: State<Session>, id: String) -> Result<(), String> {
    if !engine::native::store::id_is_safe(&id) {
        return Err("That isn't a conversation this app made.".into());
    }
    // If the conversation on screen right now is the one being deleted, drop
    // the remembered session id too — same reasoning as `clear_conversations`:
    // leaving it behind would let the very next turn `--resume` a transcript
    // this button just told the person it deleted. A DIFFERENT engine's id
    // (Claude Code's) never carries this prefix, so this can never clear
    // somebody else's live session by accident.
    {
        let mut guard = state.session_id.lock().unwrap();
        if guard.as_deref() == Some(id.as_str()) {
            *guard = None;
        }
    }
    let Some(dir) = conversations_dir(&app) else { return Ok(()) };
    let path = dir.join(format!("{id}.json"));
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("could not delete {path:?}: {e}")),
    }
}

/// `window: tauri::WebviewWindow` IS LOAD-BEARING, NOT DECORATION -- Tauri
/// injects whichever window actually called `invoke('send', ...)`, and its
/// label is the one thing downstream needs to answer "who should hear this
/// turn." See the `AppSink` doc comment below for why that question exists
/// at all now.
#[tauri::command]
fn send(
    app: AppHandle,
    window: tauri::WebviewWindow,
    prompt: String,
    workdir: String,
    mode: String,
    // A folder-feature chip's name, when this turn was started by one, so the
    // matching preamble rides the system prompt for this turn only. `None` (the
    // ordinary case, and every call that predates the chips) means no preamble.
    // It is a lookup KEY, never trusted text — see `folder_features` for why an
    // unknown value can only ever mean "no preamble", not "inject this".
    preset: Option<String>,
    computer_time: Option<computer_time::Context>,
) -> Result<(), String> {
    let prompt = prompt.trim().to_string();
    if prompt.is_empty() {
        return Err("Nothing to send.".into());
    }

    let state = app.state::<Session>();
    if alive(&state) {
        return Err("Still working on the last one.".into());
    }

    let dir = std::path::PathBuf::from(&workdir);
    if !dir.is_dir() {
        return Err(format!("{} is not a folder on this machine.", workdir));
    }

    /* THE TRUST BOUNDARY, AND IT IS HERE RATHER THAN IN THE WINDOW ON PURPOSE.
       A gate that only exists in the front end is not a gate — anything that
       can reach this command walks straight past it. The window shows the
       screen; this refuses to spawn.

       What is being refused: a folder carrying instruction-bearing content the
       user has not agreed to. Claude Code obeys that content on turn one in a
       `-p` run with no dialog of its own — its own `--help` says the workspace
       trust dialog is skipped in non-interactive mode — and the app's default
       mode is now "Allow edits", so the two together are the whole hole.

       THERE IS NO "WORK HERE BUT IGNORE ITS INSTRUCTIONS" THIRD OPTION, and it
       is not an oversight. `--setting-sources user` would drop the settings
       files and `.mcp.json`, but nothing short of `--bare` or `--safe-mode`
       drops CLAUDE.md, and both of those would also drop this app's own
       channels — profile.rs writes "About you" into that very file. A button
       promising to ignore the folder's instructions could not keep the promise.

       A CLEAN FOLDER SAILS THROUGH. `needs_decision` is only set when something
       was actually found, or when the folder could not be read at all — and an
       unreadable folder is refused rather than assumed innocent. So the common
       case, including the app's own seeded working folder, never sees this.

       IT STAYS ON THE NATIVE ENGINE TOO, AND THAT IS NOT AN OVERSIGHT — checked
       deliberately on 2026-08-31 when the native gate opened. The obvious
       reading is that a tool-less engine cannot be hurt by folder content, so
       the gate could be skipped for it and a local-brain user spared a review
       screen. THAT READING IS WRONG, and it is wrong for a specific reason:
       `memory::bridge_prompt(&workdir, ..)` reads a notes file OUT OF THE
       WORKING FOLDER and puts its contents into the system prompt, on every
       engine. So folder content still crosses into the model on a native turn.
       The blast radius is genuinely smaller — with no tools an injection can
       change what the model SAYS but not what it DOES — and "smaller" is not
       "none". `memory.rs` already frames that block as untrusted between markers
       precisely because Cassandra proved the unframed version exploitable.
       Skipping this gate would be trusting that framing on its own.
       `profile::system_prompt_block` rides the same door below, for the same
       reason — see the note beside where `system` is assembled. */
    let trust = folder_trust::gate(&app, &workdir);
    if trust.needs_decision {
        // The window gets the full probe so it can draw the review screen
        // instead of leaving somebody at a dead end with an error. Targeted
        // at the CALLING window specifically -- same reasoning as AppSink
        // below: `app.emit` would also hand the probe to the hidden,
        // always-created `companion` window, which runs this exact page too
        // and would silently eat the folder-review state without anyone
        // ever seeing its screen.
        let _ = app.emit_to(window.label(), "folder:untrusted", trust.clone());
        return Err(
            "This folder hasn't been reviewed yet. It contains instructions that would \
             run as soon as you send this, so helloim.ai stopped. Open the folder button to \
             see what is in there and decide."
                .into(),
        );
    }

    /* WHICH BRAIN, AND THEREFORE WHICH ENGINE — BOTH DECIDED HERE, BEFORE
       ANYTHING IS BUILT FOR THEM.

       IF NOBODY HAS CHOSEN A BRAIN THIS REFUSES RATHER THAN PICKING ONE.
       `active_provider` answers `None` on a machine that has never been through
       setup; it used to answer the built-in Claude row, which meant a person who
       skipped the question was quietly put on an Anthropic account. The refusal
       names the screen that fixes it — a dead end here is what sends somebody
       back to the download page.

       This stays HERE rather than moving into the engine: "has this person
       chosen a brain" is a question about the product, and every engine has to
       ask it. Where that choice is then WIRED UP is the engine's business —
       `ClaudeCodeEngine` hands it to `providers::apply_env`, which sets the
       routing variables and removes any inherited `ANTHROPIC_API_KEY` so a
       user's real Anthropic credential is never forwarded to a third-party
       endpoint.

       IT MOVED UP HERE ON 2026-08-31 AND THE MOVE IS LOAD-BEARING, not tidying:
       two decisions below now depend on which engine is driving, and both were
       being made on a fact about a *file* instead. A refusal also now happens
       before an MCP config is written for a turn that was never going to run,
       which is strictly better. */
    let chosen = providers::active_provider(&app, &app.state::<providers::Providers>())
        .ok_or(providers::NO_BRAIN_YET)?;

    // AIR-GAP REFUSES BEFORE ANYTHING IS BUILT FOR THE TURN, same standing as
    // the folder-trust gate a few lines up: a decision about whether this
    // turn may leave the machine belongs before the MCP config and the
    // system prompt are assembled for a send that was never going anywhere.
    // `kind == "local"` is the one genuinely offline kind today (Ollama on
    // loopback) — "claude" and "openai-compatible" are cloud by definition,
    // whatever address happens to be configured on the row.
    if providers::is_airgapped(&app) && chosen.kind != "local" {
        return Err(
            "Air-gap is on, so only a brain on this machine may answer right now. \
             Turn it off in Settings → Internet connection, or switch to a local \
             brain to keep going offline."
                .into(),
        );
    }
    let engine = engine::for_provider(&chosen);

    // THE FOLDER-FEATURE PROMISE VS "ALLOW EVERYTHING" -- Mastermind round 64,
    // Cassandra/Vance's finding, 2026-09-06/07: on `ClaudeCodeEngine` with
    // `bypassPermissions`, nothing in this app confines what the model writes,
    // so a preset's "stays in your folder" claim would be false. See
    // `folder_features::preset_blocked_by_permission`'s own doc for the full
    // reasoning and why it takes plain booleans rather than `engine::Permission`
    // itself. Computed here, before anything else is built for this turn --
    // same standing as the folder-trust and air-gap refusals above -- and
    // `permission` is kept for `req.permission` below rather than recomputed
    // from `mode` a second time.
    let permission = engine::Permission::from_window(&mode);
    if folder_features::preset_blocked_by_permission(
        preset.as_deref(),
        engine.needs_vendor_binary(),
        permission == engine::Permission::Full,
    ) {
        return Err(folder_features::PERMISSION_REFUSAL.into());
    }

    // A CONVERSATION IS ONLY RESUMED BY THE ENGINE THAT STARTED IT. The two
    // engines share this one id slot and their ids are not interchangeable:
    // `claude.exe --resume helloim-chat-…` fails outright, and a Claude Code
    // session id reaching the native engine would name one of our conversation
    // files after somebody else's transcript. A foreign id is dropped, so the
    // turn starts a clean conversation instead of failing.
    let resume = state
        .session_id
        .lock()
        .unwrap()
        .clone()
        .filter(|id| engine.owns_session(id));

    // THE CONNECTORS ACTUALLY REACH THE MODEL HERE, and until 2026-08-26 they
    // did not: they were saved, handshake-tested and shown with a green dot,
    // and this file never mentioned MCP at all. A person could add a server,
    // watch it pass a real test, and then find the assistant had never heard
    // of it.
    //
    // The config is written fresh every turn rather than kept in step with
    // edits, because "the file on disk agrees with the connector list" is one
    // more thing that can silently stop being true.
    //
    // The SECRETS are not in that file — see `connectors::launch_config`. They
    // go into this child's environment, which the MCP servers inherit, and the
    // file only names the variables.
    // THE MEMORY SERVER RIDES THE SAME FILE AS THE CONNECTORS — one config,
    // one flag, synthesized even when no connectors exist, so there is no
    // reliance on repeated --mcp-config flags. The entry points Claude Code
    // back at this very binary's `mcp-memory` subcommand. Scope note: no
    // --scope is passed here yet — the product has no specialist concept at
    // this spawn site, so every memory lands in the global scope. The
    // isolation mechanics in mcp.rs are ready for the day it does.
    //
    // THIS BLOCK RUNS BEFORE THE PROMPT IS ASSEMBLED, deliberately: the
    // memory guidance (slice 4a) and the bridge's promote line (4b) are only
    // appended when the memory server is genuinely attached this turn.
    // Instructing tools that do not exist teaches the model to emit calls
    // that go nowhere.
    let (mut cfg, mcp_env) =
        match if providers::is_airgapped(&app) { None } else { connectors::launch_config(&app, &app.state::<connectors::Connectors>()) } {
            Some((json, env)) => (
                serde_json::from_str::<serde_json::Value>(&json)
                    .unwrap_or_else(|_| serde_json::json!({ "mcpServers": {} })),
                env,
            ),
            None => (serde_json::json!({ "mcpServers": {} }), Vec::new()),
        };
    let mut has_memory = false;
    match std::env::current_exe() {
        Ok(exe) => {
            if let Some(servers) = cfg.get_mut("mcpServers").and_then(|v| v.as_object_mut()) {
                servers.insert(
                    "nameos-memory".into(),
                    serde_json::json!({
                        "type": "stdio",
                        "command": exe.to_string_lossy(),
                        "args": ["mcp-memory", "--workdir", &workdir],
                    }),
                );
                has_memory = true;
            }
        }
        // Memory becomes a tool the model simply does not have this turn;
        // the bridge in --append-system-prompt still carries tier 2.
        Err(e) => eprintln!("could not resolve own path for the memory server: {e}"),
    }
    let mcp_path = match mcp_config_path(&app).and_then(|p| {
        std::fs::write(&p, cfg.to_string()).map(|_| p).map_err(|e| e.to_string())
    }) {
        Ok(path) => Some(path),
        // A config that cannot be written is not a reason to refuse the
        // whole request. The turn runs without it, and the person's own
        // words are worth more than our config file.
        Err(e) => {
            eprintln!("could not write the MCP config, continuing without it: {e}");
            None
        }
    };
    // No config file, no --mcp-config flag, no memory tools — whatever the
    // entry above said.
    if mcp_path.is_none() {
        has_memory = false;
    }
    /* AND NO TOOLS AT ALL MEANS NO MEMORY EITHER, WHICHEVER WAY THE CONFIG
       WENT. Until 2026-08-31 `has_memory` was a fact about whether a FILE had
       been written, and the guidance appended below is prose telling the model
       to call `memory_search` and `memory_save`. An engine with no MCP cannot
       offer either, so on that path every one of those instructions is a lie the
       model then tries to act on — which is this file's own warning, four
       comments up, arriving through the one door it did not cover:
       "instructing tools that do not exist teaches the model to emit calls that
       go nowhere." */
    if !engine.supports_tools() {
        has_memory = false;
    }

    // THE VOICE, THE PROFILE (ON AN ENGINE WITH NO CLAUDE.md OF ITS OWN), THE
    // MEMORY GUIDANCE, AND WHERE THEY LEFT OFF, in that order. A memory store
    // nothing reads is a diary; this is the door it goes through, present on
    // the very first turn before anybody has typed.
    let mut system = voice::VOICE.to_string();
    system.push_str(google_policy::GUIDANCE);
    if let Some(context) = computer_time { system.push_str(&context.prompt()); }
    /* `ClaudeCodeEngine` NEVER NEEDS THIS: it spawns a real `claude.exe` in
       `req.workdir`, and the vendor binary reads CLAUDE.md off disk itself —
       which is the whole reason `profile.rs` writes "About you" there instead
       of somewhere `send` would have to forward. The native engine runs no
       process in the folder and opens no file; it is one HTTP call with
       whatever string it is handed. So without this, the memory field's own
       promise in `to_block()` — "This one does not lapse. It applies to every
       conversation, including this one." — was false, unconditionally, on
       every native turn. Found by Wren, 2026-09-01, tracing the write side;
       confirmed by tracing the read side and finding none for this engine.

       THIS IS SYSTEM-PROMPT TEXT, NOT A TOOL. `profile::system_prompt_block`
       reuses `to_block()`'s own markdown rather than deriving a second
       version, and the native engine gains no capability from it — still no
       Read, no Write, no MCP. See `engine::mod.rs`'s `NATIVE_ENABLED` header:
       giving this engine tools is separate, undone work, and this is not it.

       Gated on `!supports_tools()` — the same signal `has_memory` already uses
       three lines up, and for the same reason: the day a second tool-less
       engine exists, it inherits this for free instead of needing its own
       `if provider.kind == ...`. */
    if !engine.supports_tools() {
        if let Some(block) = profile::system_prompt_block(&app) {
            system.push_str(&block);
        }
    }
    if has_memory {
        system.push_str(memory::GUIDANCE);
    }
    /* TIER 3, FOR THE SAME TURNS THAT JUST LOST THE PROFILE BLOCK'S TOOL.
       `memory_search` is how Tier 3 (facts.rs — the durable preferences,
       decisions and project facts, not the session bridge below) normally
       reaches the model, and an engine with no MCP was never going to get one.
       Without this, everything the person told a previous, tool-bearing
       session to remember is invisible on this path — not degraded, gone —
       which is the exact silent-capability-loss `engine::NATIVE_ENABLED`'s own
       header says must never ship without a word. One bounded search, keyed on
       THIS turn's own prompt, gated the same way the line above it is: a turn
       with tools already reaches Tier 3 through `memory_search` and does not
       need a second copy of it stapled onto the system prompt as well. See
       `tier3_prompt`'s own doc for the sanitizing and the framing. */
    if !engine.supports_tools() {
        if let Some(facts) = memory::tier3_prompt(&workdir, &prompt) {
            system.push_str(&facts);
        }
    }
    if let Some(bridge) = memory::bridge_prompt(&workdir, has_memory) {
        system.push_str(&bridge);
    }
    // LAST, SO IT IS THE MOST SPECIFIC THING THE MODEL READS THIS TURN. A
    // folder-feature chip's preamble is engine-agnostic by construction: it goes
    // into the same `system` both engines read, and it only ever asks for the
    // Read/Write/Edit/Grep tools every engine already offers. An absent or
    // unknown preset leaves `system` untouched (see `folder_features`).
    folder_features::apply_preset(&mut system, preset.as_deref());

    // EVERYTHING ABOVE IS ENGINE-AGNOSTIC AND EVERYTHING BELOW IS THE SEAM.
    // The folder gate, the MCP config, the system prompt and the brain choice
    // are decisions about this product; how a turn is actually driven is not.
    let req = engine::TurnRequest {
        prompt,
        workdir: dir,
        permission,
        system_prompt: system,
        resume,
        mcp_config: mcp_path.clone(),
        mcp_env,
        // THE MEMORY TOOLS ARE PRE-APPROVED, and this is why memory works at
        // all in the default permission mode. Found by the slice-4 observed run
        // (2026-08-27): in headless -p mode nobody can answer a permission
        // prompt, so an unapproved MCP tool is denied — the model CALLED
        // memory_save exactly as the guidance asked, got permission_denied, and
        // no fact landed. Slice 3's own tests drove the server over raw pipes
        // and could never see this. Pre-approving exactly our own two tools, by
        // full name, is the app trusting its own memory feature — the server
        // forces provenance, refuses verified, has no forget or edit, and trips
        // on credential-shaped values. Nothing else widens: connector tools keep
        // their normal permission treatment.
        allowed_tools: { let mut allowed = if has_memory {
            vec![
                "mcp__nameos-memory__memory_search".to_string(),
                "mcp__nameos-memory__memory_save".to_string(),
            ]
        } else {
            Vec::new()
        };
        if mcp_path.is_some() { allowed.extend(google_policy::cli_tools(&cfg)); }
        allowed },
        // Ours, not the working folder. `ClaudeCodeEngine` ignores it; the
        // native engine keeps its transcripts there because `--resume` belongs
        // to the vendor binary and it has none.
        store_dir: conversations_dir(&app),
        provider: chosen,
    };

    // ON `Err` NOTHING HAS STARTED AND NOTHING HAS BEEN BILLED, which is what
    // lets this go back to the window as words. A silent fallback to the
    // Anthropic cloud would bill an account the user chose not to use.
    // A FRESH GENERATION PER TURN -- see `Session::turn`'s own doc for the
    // race this closes. Allocated before `engine.start` so `AppSink` (which
    // needs to know its own identity for `finished`'s later check) can carry
    // it from the moment it exists, rather than the turn being stored under
    // one generation and the sink believing a different one.
    let generation = state.next_generation.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    // GENERIC HISTORY CAPTURE'S OWN COPIES, taken NOW, before `req` is handed
    // to `engine.start` below by reference and then dropped at the end of
    // this function. `AppSink` outlives `req` -- it is still alive answering
    // `event`/`finished` long after `send` has returned -- so it needs its
    // own `String`s, not a borrow into a request that will not be there.
    // `capture_history` is asked of `engine` once, here, rather than
    // re-asked inside the sink later: see `Engine::persists_own_history`'s
    // own doc for why the sink has no engine of its own to ask again.
    let capture_history = !engine.persists_own_history();
    let sink = std::sync::Arc::new(AppSink {
        app: app.clone(),
        window_label: window.label().to_string(),
        generation,
        prompt: req.prompt.clone(),
        provider_id: req.provider.id.clone(),
        model: req.provider.model.clone(),
        store_dir: req.store_dir.clone(),
        capture_history,
        pending_answer: std::sync::Mutex::new(String::new()),
    });
    let turn = engine.start(&req, sink)?;
    *state.turn.lock().unwrap() = Some((generation, turn));

    Ok(())
}

/// The window's end of `engine::TurnSink`: four callbacks onto the four Tauri
/// events `index.html` already listens for.
///
/// **THE EVENT NAMES DO NOT CHANGE AND MUST NOT.** `claude:event`,
/// `claude:raw`, `claude:stderr` and `claude:done` are the contract with 15,240
/// hand-written lines of front end and no bundler to rename anything for us.
/// They are named after the engine that shipped; that is a wart, and it is
/// cheaper than a front-end migration in the same change as an engine
/// refactor.
///
/// **`window_label` IS WHY MARK HEARD EVERY ANSWER TWICE, reported and fixed
/// 2026-09-03.** `tauri.conf.json` declares a second window, `companion`
/// (`visible: false`, always-on-top, summoned later by a hotkey) -- and
/// Tauri creates a config-declared window and starts running its page at
/// app launch REGARDLESS of `visible`, hidden or not. `companion` loads this
/// exact `index.html` (`?surface=companion` only tags a dataset attribute
/// nothing reads yet -- Wren/Vega's own layout for it is still unbuilt) --
/// so it registers the identical `listen('claude:event', ...)` -> `speak()`
/// path `main` does. `AppSink::event/raw/failure/finished` used to call
/// plain `self.app.emit(...)`, and Tauri's own docs are explicit that `emit`
/// -- called on ANY handle, app or window -- broadcasts to every window in
/// the app; only `emit_to(label, ...)` is scoped. So one assistant turn
/// produced one `claude:event` broadcast, two listening webviews, and two
/// independent calls to `speak()` -- one audible stream per window, which is
/// exactly "heard twice, only ever shown once" (only `main` renders a visible
/// transcript; `companion` has no chat UI of its own yet, so its copy of the
/// turn was heard and never seen). `window_label` is the label of whichever
/// window actually invoked `send` (Tauri injects the calling window when a
/// command takes a `WebviewWindow`/`Window` parameter), captured once at
/// dispatch and carried for the life of the turn. This is also the more
/// correct shape going forward, not just the narrower one: if `companion`
/// ever grows its own composer, a turn started there should answer there,
/// not in `main` -- routing by the window that actually asked is right in
/// both directions, "always main" would just be swapping which window goes
/// silent.
struct AppSink {
    app: AppHandle,
    window_label: String,
    /// This turn's own identity in `Session.turn`'s generation counter.
    /// Set once, at `send()`, from the same counter that stamped the slot
    /// when this turn was stored — see that field's own doc, and
    /// `clear_turn_if_still_current`'s, for the race this exists to close.
    generation: u64,

    // -- GENERIC HISTORY CAPTURE, added 2026-09-05 for "history doesn't even
    // work" -- see `engine::Engine::persists_own_history`'s own doc for the
    // whole design. Everything below is inert (`capture_history: false`) for
    // an engine that already keeps its own store, which today is only
    // `engine::native` -- so this costs that path nothing, and the fields
    // still have to exist on every `AppSink` because one struct serves every
    // engine `send` might have chosen for THIS turn. --
    /// What the person actually typed, cloned out of `TurnRequest` before it
    /// was handed to the engine. `store::Conversation::commit` wants both
    /// halves of the exchange at once (see that module's own commit rule),
    /// and by the time `finished` runs, `req` is long gone.
    prompt: String,
    /// Recorded into the conversation file purely for the same honesty
    /// `store::Conversation`'s own doc already gives its `provider_id`/
    /// `model` fields — nothing reads them back yet.
    provider_id: String,
    model: String,
    /// `None` on a machine where `conversations_dir` itself could not be
    /// created — see that function's own doc. Capture is simply skipped
    /// then, the same way `engine::native::mod`'s own `drive` skips its
    /// internal save on the identical case.
    store_dir: Option<std::path::PathBuf>,
    /// `!engine.persists_own_history()` for whichever engine `send` chose
    /// for this turn, decided once, before the turn starts, and carried
    /// rather than re-asked — the sink has no `Engine` of its own to ask
    /// again once the turn is running.
    capture_history: bool,
    /// The latest COMPLETE text an `assistant` event has carried this turn,
    /// overwritten as later ones arrive, never appended.
    ///
    /// **OVERWRITE, NOT APPEND, AND THAT IS THE WHOLE OF WHY THIS ISN'T A
    /// `Vec`.** A tool-using turn's own shape is text-then-tool_use-then-text
    /// -- reasoning bubbles that precede a call, the call itself, and only
    /// then the real answer -- and `store::Conversation::commit`'s own
    /// contract (see that module's header) commits ONE exchange: the
    /// question and the actual final answer, never the tool round in
    /// between. Keeping only the most recent text block reproduces that
    /// exactly, engine-agnostic, with no tool-shape parsing of its own.
    pending_answer: std::sync::Mutex<String>,
}

/// The session id carried on a `system` line, if this is one.
///
/// **PURE, AND SEPARATE FROM THE SINK, BECAUSE THE SINK NEEDS A LIVE TAURI APP
/// AND THIS DECISION DOES NOT.** Passing the id to the next turn is what turns
/// a series of one-shot calls into a conversation; losing it silently looks
/// exactly like a model with no memory, which is the failure this whole product
/// exists to fix. Before the seam that made it a claim nothing in `cargo test`
/// could check — it lived inside a closure in a `#[tauri::command]`.
///
/// It reads the shared five-shape contract rather than anything about how a
/// particular engine produced it, which is why it lives on this side of the
/// seam and not in `engine/claude_code.rs`.
fn session_id_of(value: &serde_json::Value) -> Option<&str> {
    if value.get("type").and_then(|t| t.as_str()) != Some("system") {
        return None;
    }
    value.get("session_id").and_then(|s| s.as_str())
}

/// The text of a completed `assistant` event's TEXT content blocks, joined —
/// `None` for a `system`/`result`/`user` line, and `None` for an assistant
/// bubble that is pure `tool_use` with no text alongside it.
///
/// **`session_id_of`'s TWIN, AND FOR THE SAME REASON.** The generic History
/// capture in `AppSink` below has no engine of its own to ask "what did you
/// just say" — same as `session_id_of` has none to ask "what conversation is
/// this" — because both read the one shared five-shape stream `index.html`
/// already parses, rather than anything a specific engine produced. See
/// `engine::native::mod.rs`'s own header for that shared contract, and
/// `engine::Engine::persists_own_history`'s doc for why this function exists
/// at all: it is what lets `main.rs` record a turn's real answer without
/// knowing which engine actually generated it.
fn assistant_text_of(value: &serde_json::Value) -> Option<String> {
    if value.get("type").and_then(|t| t.as_str()) != Some("assistant") {
        return None;
    }
    let content = value.get("message")?.get("content")?.as_array()?;
    let text: String = content
        .iter()
        .filter(|block| block.get("type").and_then(|t| t.as_str()) == Some("text"))
        .filter_map(|block| block.get("text").and_then(|t| t.as_str()))
        .collect();
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

impl engine::TurnSink for AppSink {
    fn event(&self, value: serde_json::Value) {
        // Catch the session id on the way past.
        if let Some(id) = session_id_of(&value) {
            let state = self.app.state::<Session>();
            *state.session_id.lock().unwrap() = Some(id.to_string());
        }
        // Catch the latest real answer text on the way past too, for an
        // engine that will not save this turn itself — see `capture_history`
        // and `pending_answer`'s own docs on the struct above.
        if self.capture_history {
            if let Some(text) = assistant_text_of(&value) {
                *self.pending_answer.lock().unwrap() = text;
            }
        }
        let _ = self.app.emit_to(&self.window_label, "claude:event", value);
    }

    fn raw(&self, line: String) {
        let _ = self.app.emit_to(&self.window_label, "claude:raw", Line { text: line });
    }

    fn failure(&self, line: String) {
        let _ = self.app.emit_to(&self.window_label, "claude:stderr", Line { text: line });
    }

    fn finished(&self, code: i32) {
        // Cleared, then the id read, then emitted — the same order the pre-seam
        // reaper used.
        //
        // **THE CLEAR USED TO BE UNCONDITIONAL, AND THAT WAS LOAD-BEARING IN
        // THE WRONG DIRECTION — Cassandra, 2026-09-04, race 2 of the deadlock
        // fix.** This comment used to call the clear "belt and braces rather
        // than load-bearing, because a finished turn already answers
        // `is_alive()` false and `cancel()` Ok" — true of THIS turn, and
        // silent about what an unconditional `*guard = None` does to a
        // DIFFERENT, newer one that may by now occupy the slot (see
        // `clear_turn_if_still_current`'s own doc for exactly how that
        // happens). It was load-bearing after all, just for the wrong turn.
        let state = self.app.state::<Session>();
        clear_turn_if_still_current(&state, self.generation);
        let session_id = state.session_id.lock().unwrap().clone();

        // GENERIC HISTORY CAPTURE. Same commit rule `store.rs`'s own module
        // header states for the native engine's internal save: nothing is
        // written unless there is a real answer, so a failed or cancelled
        // turn (empty `pending_answer`, exactly as a native failure leaves
        // `plan.convo` uncommitted) leaves no trace and a retry is a clean
        // first attempt — this reuses that store, so it inherits that
        // contract rather than re-deciding it.
        //
        // THE REAL WORK IS `capture_turn_into_history` BELOW, NOT HERE — same
        // split `list_conversation_files`/`clear_conversation_files` already
        // draw a screen up, and for the identical reason: `AppHandle` has no
        // public constructor outside a running app, so a test proving this
        // actually lands on disk (and that `list_conversation_files` then
        // finds it) needs a path and a session id and nothing else.
        if self.capture_history {
            let answer = std::mem::take(&mut *self.pending_answer.lock().unwrap());
            if !answer.is_empty() {
                if let (Some(id), Some(dir)) = (session_id.as_deref(), self.store_dir.as_deref()) {
                    if let Err(e) = capture_turn_into_history(
                        dir,
                        id,
                        self.prompt.clone(),
                        answer,
                        self.provider_id.clone(),
                        self.model.clone(),
                    ) {
                        self.raw(format!(
                            "This turn answered, but it could not be saved, so the next \
                             message will not remember it ({e})."
                        ));
                    }
                }
            }
        }

        let _ = self.app.emit_to(&self.window_label, "claude:done", Finished { code, session_id });
    }

    fn confirm(&self, prompt: &str) -> bool {
        confirm_via_window(&self.app, &self.window_label, prompt, None)
    }
    fn confirm_cancellable(&self, prompt: &str, cancelled: &std::sync::atomic::AtomicBool) -> bool {
        confirm_via_window(&self.app, &self.window_label, prompt, Some(cancelled))
    }
}

/// The real work behind `AppSink::finished`'s generic History capture — pure
/// filesystem and `store` calls, no `AppHandle`, so a test can drive it
/// directly. See `engine::Engine::persists_own_history`'s own doc for the
/// whole design and `AppSink::finished`'s call site for why this is split out
/// at all.
///
/// Loads any existing conversation at `id` first rather than always minting
/// one, so a second turn in an already-started conversation appends to it
/// instead of overwriting the first exchange with itself — the same reason
/// `engine::native::mod.rs::conversation_for` loads before minting.
///
/// **SILENTLY DOES NOTHING FOR AN UNSAFE ID, RATHER THAN ERRORING.** This is
/// never a person's input the way `delete_conversation`'s id is — it is our
/// own session-id bookkeeping, defensively re-checked because
/// `store::CHAT_ID_PREFIX`'s own doc says the prefix means "minted by
/// `engine::native`," never "safe to use as a filename," and a foreign
/// engine's session id has never been checked against `id_is_safe` before
/// today. A real `claude.exe` id is a lowercase UUID and passes cleanly; if
/// some future engine ever hands back something that does not, the honest
/// answer is the turn simply is not added to History, not a message on
/// screen about an internal id nobody typed.
fn capture_turn_into_history(
    dir: &std::path::Path,
    id: &str,
    prompt: String,
    answer: String,
    provider_id: String,
    model: String,
) -> Result<(), String> {
    if !engine::native::store::id_is_safe(id) {
        return Ok(());
    }
    let mut convo = engine::native::store::Conversation::load(dir, id)
        .unwrap_or_else(|| engine::native::store::Conversation::new(id.to_string(), provider_id, model));
    convo.commit(prompt, answer);
    convo.save(dir)
}

/// The user's home, on the platform we are actually running on.
///
/// THIS USED TO BE `std::env::var("HOME")` WITH `"/"` AS THE FALLBACK, AND THAT
/// IS A WINDOWS BUG THAT SHIPPED. Windows does not set HOME — it sets
/// USERPROFILE — so on the platform this product is actually installed on, the
/// working-folder box came up containing `/`, which is not a folder there. The
/// field painted itself red on a machine where the user had done nothing wrong.
/// It looked like the app was broken on first launch, and it was.
pub(crate) fn home_dir() -> Option<std::path::PathBuf> {
    #[cfg(windows)]
    {
        if let Some(p) = std::env::var_os("USERPROFILE") {
            if !p.is_empty() {
                return Some(std::path::PathBuf::from(p));
            }
        }
        // Fallback for a profile-less session: HOMEDRIVE + HOMEPATH.
        if let (Some(d), Some(p)) = (std::env::var_os("HOMEDRIVE"), std::env::var_os("HOMEPATH")) {
            let mut joined = std::ffi::OsString::from(d);
            joined.push(&p);
            return Some(std::path::PathBuf::from(joined));
        }
    }
    std::env::var_os("HOME")
        .filter(|h| !h.is_empty())
        .map(std::path::PathBuf::from)
}

/// Ask Windows where Documents actually lives, rather than assuming the
/// obvious path.
///
/// THIS EXISTS BECAUSE THE OBVIOUS PATH IS WRONG ON A LOT OF REAL MACHINES.
/// When Documents is OneDrive-redirected — the default on a lot of consumer
/// Windows installs — `%USERPROFILE%\Documents` and the OneDrive-synced
/// Documents are BOTH real folders on disk, and a literal join on the home
/// directory silently picks the unsynced one. Nobody sees an error; the
/// roaming-persona feature (set up on one machine, point it at a folder in a
/// cloud drive, it is already there on the second machine) just quietly does
/// nothing in exactly the case it exists for.
///
/// `SHGetKnownFolderPath` with `FOLDERID_Documents` is the documented way to
/// ask instead of assume — it returns the redirected path when redirection is
/// in force, and the plain path otherwise. `KF_FLAG_DEFAULT` (no flags) is
/// deliberate: no create, no cache-only, no alias-following override — just
/// what the shell would show you if you clicked Documents right now.
///
/// Returns `None` on any failure, by design — a locked-down profile, a
/// redirect pointed at an offline network share, a shell component that
/// didn't answer. The caller falls back to the literal-join behaviour that
/// shipped before this existed; a missing Documents folder must never make
/// the app unusable.
#[cfg(windows)]
fn windows_known_documents_dir() -> Option<std::path::PathBuf> {
    use windows_sys::Win32::UI::Shell::{FOLDERID_Documents, KF_FLAG_DEFAULT, SHGetKnownFolderPath};

    // SAFETY: `out` is a valid, aligned, writable `*mut PWSTR` for the
    // duration of the call, which is all the API requires of it. A null
    // token means "the calling user", which is correct here — this is never
    // run impersonating anyone else. On success the shell hands back a
    // NUL-terminated UTF-16 buffer it owns; we read it and free it with
    // `CoTaskMemFree` before returning, so nothing escapes this function
    // still holding a foreign allocation.
    unsafe {
        let mut out: *mut u16 = std::ptr::null_mut();
        let hr = SHGetKnownFolderPath(
            &FOLDERID_Documents,
            KF_FLAG_DEFAULT as u32,
            std::ptr::null_mut(),
            &mut out,
        );
        if hr < 0 || out.is_null() {
            return None;
        }
        let len = (0..).take_while(|&i| *out.offset(i) != 0).count();
        let wide = std::slice::from_raw_parts(out, len);
        let path = std::path::PathBuf::from(
            <std::ffi::OsString as std::os::windows::ffi::OsStringExt>::from_wide(wide),
        );
        windows_sys::Win32::System::Com::CoTaskMemFree(out as *const std::ffi::c_void);
        if path.as_os_str().is_empty() { None } else { Some(path) }
    }
}

/// Pick the folder `default_workdir` seeds into, given what the platform
/// layer found.
///
/// Split out from `default_workdir` so the DECISION — trust a known-folder
/// answer when we have one, otherwise fall back to the literal join, otherwise
/// fall back to home — can be exercised on any OS with a fake answer standing
/// in for the real Windows call, the same way `folder_trust::probe_at` takes
/// a stand-in record path and `profile::adopted_value` takes a stand-in claim.
/// The actual `SHGetKnownFolderPath` call cannot run outside Windows; this
/// logic can be proven everywhere.
fn choose_documents_parent(
    home: &std::path::Path,
    known_documents: Option<std::path::PathBuf>,
    literal_docs_is_dir: bool,
) -> std::path::PathBuf {
    if let Some(known) = known_documents {
        return known;
    }
    if literal_docs_is_dir {
        home.join("Documents")
    } else {
        home.to_path_buf()
    }
}

/// The folder the app works in on a machine that has never run it.
///
/// IT IS CREATED, AND IT HAS REAL FILES IN IT — Mark's instruction: "a folder,
/// with files should be created and be default." An empty path in that box, or
/// a valid-but-empty folder, both hand a new user the same question on their
/// first ten seconds: what am I supposed to point this at? So the app answers
/// it by making somewhere to work and putting something real in it.
///
/// IT NEVER OVERWRITES. If the folder is already there, its files are the
/// user's, and this returns the path without touching a single one. The starter
/// files are written ONLY into a folder this function has just created — a
/// welcome note that reappears after someone deletes it would be the app
/// arguing with them.
///
/// A failure here is not fatal. If the folder cannot be made — a locked-down
/// profile, a full disk, a redirected Documents folder on a dead network share
/// — this falls back to the home directory, which is the old behaviour and
/// still better than an empty box.
/// (`app` is injected by Tauri, not passed from the window — the JS call is
/// still `invoke("default_workdir")`. It is here so a folder this function
/// CREATES can be recorded as trusted; see `folder_trust::trust_our_own`.)
#[tauri::command]
fn default_workdir(app: AppHandle) -> String {
    let Some(home) = home_dir() else {
        return String::new();
    };

    // On Windows, ask the OS where Documents really is (this is what honours
    // OneDrive redirection — see `windows_known_documents_dir`). Everywhere
    // else, and if that ask fails for any reason, fall back to the literal
    // join when it exists, and to the home folder otherwise, rather than
    // creating a Documents folder on a system that has deliberately not got
    // one.
    #[cfg(windows)]
    let known_documents = windows_known_documents_dir();
    #[cfg(not(windows))]
    let known_documents: Option<std::path::PathBuf> = None;

    let parent = choose_documents_parent(&home, known_documents, home.join("Documents").is_dir());
    let workdir = parent.join("helloim.ai");

    if workdir.is_dir() {
        return workdir.to_string_lossy().into_owned();
    }
    if std::fs::create_dir_all(&workdir).is_err() {
        return home.to_string_lossy().into_owned();
    }
    seed_workdir(&workdir);
    /* THE ONE FOLDER THIS APP MAY TRUST ON SOMEBODY'S BEHALF, and the reason it
       is safe is the two lines above: the folder did not exist a millisecond
       ago and every file in it was written by `seed_workdir`. Nothing can have
       arrived with it. Note where this sits — INSIDE the `is_dir()` early
       return above, so a folder that was already there is never auto-trusted,
       even at this same path. Best effort: if the record cannot be written the
       user is simply asked once, which is the safe direction to fail. */
    folder_trust::trust_our_own(&app, &workdir);
    workdir.to_string_lossy().into_owned()
}

/// The starter files. Written once, into a folder that did not exist a moment
/// ago.
///
/// THEY ARE NOT PLACEHOLDERS. Each one is either useful on its own or teaches
/// the one thing a new user needs to know, because a folder full of `TODO` is
/// worse than an empty one — it looks like the product shipped unfinished.
/// Markdown throughout: it is what the notes panel reads, and it opens in
/// anything.
fn seed_workdir(dir: &std::path::Path) {
    let files: &[(&str, &str)] = &[
        (
            "Start here.md",
            "# Start here\n\n\
             This folder is where helloim.ai works. Anything you ask for happens in \
             here — files it writes, changes it makes, notes it keeps.\n\n\
             You can move it or point at a different one any time, with the \
             folder button in the left rail.\n\n\
             ## Try asking for one of these\n\n\
             - *Summarise every file in this folder.*\n\
             - *Make me a note called Groceries and put milk and coffee in it.*\n\
             - *What changed in here since yesterday?*\n\n\
             You do not have to phrase it carefully. Ask the way you would ask a \
             person.\n",
        ),
        (
            "Notes/Welcome.md",
            "# Welcome\n\n\
             Notes you keep in here can link to each other with double brackets, \
             like [[Start here]].\n\n\
             helloim.ai reads that structure, so the more you link, the better it \
             understands how your work fits together. It reads the titles and \
             the links — never the contents of your notes — to draw that \
             picture.\n",
        ),
        (
            "Notes/Scratch.md",
            "# Scratch\n\n\
             Somewhere to put a thought before it goes anywhere else.\n",
        ),
    ];

    for (rel, body) in files {
        let path = dir.join(rel);
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        // create_new: never clobber. If something is already there — a folder
        // restored from backup, a sync client that got there first — it wins.
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            use std::io::Write;
            let _ = f.write_all(body.as_bytes());
        }
    }
}

/// Does that path exist and is it a folder? The window asks on every keystroke
/// so the field can say "no" while there is still time to fix it. Finding out
/// at Send is finding out too late — the run does not start and the reason
/// arrives as an error, which reads as the app failing rather than the path
/// being wrong.
#[tauri::command]
fn is_folder(path: String) -> bool {
    !path.trim().is_empty() && std::path::Path::new(path.trim()).is_dir()
}

#[derive(Clone, Serialize)]
struct NoteNode {
    id: String,    // the note's stem — Obsidian resolves a [[link]] by name, not path
    path: String,  // relative to workdir, for a hover/label only — never sent further
}

#[derive(Clone, Serialize)]
struct NoteEdge {
    a: String,
    b: String,
}

#[derive(Clone, Serialize)]
struct NotesGraph {
    nodes: Vec<NoteNode>,
    edges: Vec<NoteEdge>,
    truncated: bool, // the walk hit MAX_FILES — the picture is real but partial
}

/// The living-brain panel's data source. Reads the CURRENT working folder —
/// the one already chosen in the toolbar, not a second "vault path" the user
/// would have to configure — and returns real markdown notes plus their real
/// [[wikilink]] structure between them. Point it at an actual Obsidian vault
/// and this is that vault's real graph; point it at an ordinary code folder
/// and it comes back sparse or empty, which is the honest answer, not a
/// reason to invent edges to fill the picture out.
///
/// STRUCTURE ONLY, NEVER CONTENT — the same rule `vault-graph.py` already
/// banked for markdaltoncom's Neural Core, which this panel is the
/// hand-written-canvas sibling of: titles and link structure leave this
/// function, note bodies never do. That matters more here, not less — this
/// graph renders straight into a UI a shared screen could show, where the
/// vault-graph build only ever fed a poll on a private box.
///
/// Bounded on purpose: MAX_FILES caps the walk so a huge repo can't turn a
/// folder pick into a stall, MAX_DEPTH keeps it out of deep trees even before
/// the skip-list below catches the obvious ones by name.
#[tauri::command(async)]
fn scan_notes(workdir: String) -> NotesGraph {
    const MAX_FILES: usize = 200;
    const MAX_DEPTH: usize = 6;
    const SKIP: &[&str] = &[
        ".git", ".obsidian", ".trash", "node_modules", "target",
        "venv", ".venv", "__pycache__", "dist", "build",
    ];

    let root = std::path::PathBuf::from(workdir.trim());
    let mut files: Vec<std::path::PathBuf> = Vec::new();
    let mut stack: Vec<(std::path::PathBuf, usize)> = vec![(root.clone(), 0)];

    while let Some((dir, depth)) = stack.pop() {
        if files.len() >= MAX_FILES {
            break;
        }
        if depth > MAX_DEPTH {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if SKIP.contains(&name.as_str()) || name.starts_with('.') && name != "." {
                continue;
            }
            let path = entry.path();
            if path.is_dir() {
                stack.push((path, depth + 1));
            } else if path.extension().map(|e| e == "md").unwrap_or(false) {
                files.push(path);
                if files.len() >= MAX_FILES {
                    break;
                }
            }
        }
    }
    let truncated = files.len() >= MAX_FILES;

    let mut by_stem: HashMap<String, ()> = HashMap::new();
    let mut nodes = Vec::with_capacity(files.len());
    for path in &files {
        let stem = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let rel = path
            .strip_prefix(&root)
            .unwrap_or(path)
            .to_string_lossy()
            .into_owned();
        by_stem.entry(stem.clone()).or_insert(());
        nodes.push(NoteNode { id: stem, path: rel });
    }

    let mut edges = Vec::new();
    let mut seen: HashSet<(String, String)> = HashSet::new();
    for path in &files {
        let Ok(text) = std::fs::read_to_string(path) else {
            continue;
        };
        let from = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        for target in wikilink_targets(&text) {
            if target == from || !by_stem.contains_key(&target) {
                continue;
            }
            let key = if from < target {
                (from.clone(), target)
            } else {
                (target, from.clone())
            };
            if seen.insert(key.clone()) {
                edges.push(NoteEdge { a: key.0, b: key.1 });
            }
        }
    }

    NotesGraph { nodes, edges, truncated }
}

/// Hand-rolled instead of pulling in the `regex` crate for one bracket
/// pattern. Finds every `[[Target]]`, `[[Target|alias]]` and
/// `[[Target#heading]]`, returning just Target — Obsidian's own resolution
/// rule, and the same one `vault-graph.py` already uses.
fn wikilink_targets(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0;
    while i + 1 < bytes.len() {
        if bytes[i] == b'[' && bytes[i + 1] == b'[' {
            if let Some(end_rel) = text[i + 2..].find("]]") {
                let inner = &text[i + 2..i + 2 + end_rel];
                let target = inner.split(['|', '#']).next().unwrap_or("").trim();
                if !target.is_empty() {
                    out.push(target.to_string());
                }
                i += 2 + end_rel + 2;
                continue;
            }
        }
        i += 1;
    }
    out
}

/// The native folder chooser.
///
/// `command(async)` IS THE WHOLE BUG FIX AND IT MUST NOT BE DROPPED. A plain
/// `#[tauri::command]` on a non-async function runs on the MAIN thread. This
/// one then blocks on `recv()` waiting for the folder — and the main thread is
/// exactly what GTK needs to draw the dialog and deliver the answer. So the
/// picker never appears, the callback never fires, and the whole window is
/// dead: no typing, no buttons, nothing. Pressing "Choose…" killed the app.
///
/// It compiled clean and was only caught by clicking the button in a real
/// window and watching it freeze. `(async)` on a sync function is Tauri's
/// documented way to force it onto a worker thread instead.
///
/// ---
///
/// LINUX DRAWS ITS OWN DIALOG, AND THAT IS NOT NOT-INVENTED-HERE. The obvious
/// route — `tauri-plugin-dialog` — leaks a whole dialog on every single pick.
/// Measured here, not suspected: fifteen picks left fifteen live GTK windows
/// and about 1.6 MB each.
///
/// The cause is two lines down in `rfd`: it builds the chooser with
/// `gtk_file_chooser_native_new`, which hands back a reference it now owns, and
/// on drop it calls `gtk_native_dialog_destroy` — which hides the dialog and
/// **explicitly does not release that reference**. Nothing ever unrefs it, so
/// every pick strands one. It is inside a dependency of a dependency, so there
/// is nothing to pass or configure to avoid it.
///
/// So on Linux we build a plain `FileChooserDialog` and destroy it in the
/// response handler. gtk-rs unrefs on drop, which is the half rfd is missing.
/// Every other platform keeps the plugin, which does not have this problem.
#[cfg(target_os = "linux")]
#[tauri::command(async)]
fn pick_folder(app: AppHandle, current: String) -> Option<String> {
    use gtk::prelude::*;

    let (tx, rx) = std::sync::mpsc::channel();
    let start = std::path::PathBuf::from(current.trim());

    // GTK is only safe to touch from the thread running its main loop.
    let queued = app.run_on_main_thread(move || {
        let dialog = gtk::FileChooserDialog::new(
            Some("Which folder should it work in?"),
            None::<&gtk::Window>,
            gtk::FileChooserAction::SelectFolder,
        );
        dialog.add_button("Cancel", gtk::ResponseType::Cancel);
        // Not "Open". They are not opening it, they are choosing where the
        // agent will work, and the button is the last chance to say so.
        dialog.add_button("Use this folder", gtk::ResponseType::Accept);

        // Open where they already are. A picker that always starts at $HOME
        // makes them re-walk the same tree every time they change their mind.
        if start.is_dir() {
            let _ = dialog.set_current_folder(&start);
        }

        dialog.connect_response(move |d, response| {
            let chosen = if response == gtk::ResponseType::Accept {
                d.filename()
            } else {
                None
            };
            // THE LINE THE LEAK WAS ABOUT. Without it the window merely hides
            // and the object lives forever.
            unsafe {
                d.destroy();
            }
            let _ = tx.send(chosen);
        });

        dialog.show_all();
    });

    // If it could not even be queued there is no dialog and nobody will ever
    // send, so do not sit on the channel until the app closes.
    if queued.is_err() {
        return None;
    }

    rx.recv()
        .ok()
        .flatten()
        .map(|p| p.to_string_lossy().into_owned())
}

/// Grant the webview microphone access, and nothing else.
///
/// Voice input (`getUserMedia({ audio: true })`) was landing with a silent
/// `NotAllowedError` on every platform, because neither webview backend asks
/// the OS for a microphone unless the HOST explicitly answers the request —
/// there is no "just works" default here, on either platform, and the two
/// backends don't even fail the same way.
///
/// LINUX: WebKitGTK denies every media permission request unless the host
/// connects a handler to the webview's own `permission-request` signal.
/// Confirmed against this project's actual vendored dependency source
/// (`webkit2gtk` 2.0.2, the exact version wry 0.55.1 already resolves to —
/// checked in `Cargo.lock` before pinning it in `Cargo.toml`), not just the
/// crate's docs: `WebViewExt::connect_permission_request` and
/// `PermissionRequestExt::{allow,deny}` are unconditional, no version
/// feature gate. Signal semantics per WebKitGTK's own reference:
/// https://webkitgtk.org/reference/webkit2gtk/2.40.4/signal.WebView.permission-request.html
///
/// WINDOWS: WebView2 raises `ICoreWebView2::PermissionRequested` for the same
/// call, but "leave it unhandled and WebView2 shows its own prompt" is not a
/// safe assumption to build on — WebView2Feedback has a live report of
/// microphone requests being silently auto-denied with no event ever firing:
/// https://github.com/MicrosoftEdge/WebView2Feedback/issues/1462
/// Handling it ourselves removes that runtime-version-dependent guesswork,
/// and answers Mark's actual question ("does Windows need anything") with a
/// yes rather than a hope. API confirmed against this project's vendored
/// `webview2-com`/`webview2-com-sys` 0.38.2 source (`add_PermissionRequested`,
/// `COREWEBVIEW2_PERMISSION_KIND_MICROPHONE`, `COREWEBVIEW2_PERMISSION_STATE_*`)
/// and against wry 0.55.1's own use of the identical handler for clipboard
/// permission (`src/webview2/mod.rs`), which is the proof this pattern
/// compiles and runs in exactly this dependency tree.
///
/// Both branches grant AUDIO ONLY, never a blanket "allow everything" — a
/// combined audio+video request is refused rather than silently handing out
/// the camera too, same "never silently widen what's allowed" reasoning as
/// `permission_mode` above.
#[cfg(target_os = "linux")]
fn grant_microphone_permission(webview: tauri::webview::PlatformWebview) {
    use webkit2gtk::glib::Cast;
    use webkit2gtk::{PermissionRequestExt, UserMediaPermissionRequestExt, WebViewExt};

    // `webkit2gtk::WebView` is a GObject reference, so `.inner()` is a
    // pointer copy, not a second webview.
    webview.inner().connect_permission_request(|_view, request| {
        match request.downcast_ref::<webkit2gtk::UserMediaPermissionRequest>() {
            Some(media) if media.is_for_audio_device() && !media.is_for_video_device() => {
                media.allow();
            }
            // Either a video (or audio+video) request, or some other media
            // shape entirely — deny rather than guess at intent.
            Some(media) => media.deny(),
            // Not a media request at all (geolocation, notifications,
            // clipboard, ...). We only ever asked for the microphone.
            None => request.deny(),
        }
        // Handled: stop WebKit's own default handler, which denies
        // everything, from running as well.
        true
    });
}

/// Pick FILES to hand to the model. Mark, 2026-08-27: "on the place to type we
/// need to add a way to attach files".
///
/// **"ATTACH" HERE MEANS "NAME IT", NOT "UPLOAD IT", and that is the whole
/// point of this product.** Nothing is copied, nothing is sent anywhere: the
/// path goes into the prompt and Claude Code reads the file off the disk it is
/// already sitting on. An attach button that quietly uploaded somebody's
/// documents would contradict the sentence on our own front page — "nothing to
/// upload, nothing to migrate, nothing to hand over".
///
/// Multi-select on purpose: attaching three files one at a time is three
/// dialogs to answer the same question.
#[cfg(windows)]
#[tauri::command(async)]
fn pick_files(app: AppHandle) -> Vec<String> {
    use tauri_plugin_dialog::DialogExt;

    let (tx, rx) = std::sync::mpsc::channel();
    app.dialog()
        .file()
        .set_title("Which files should it look at?")
        .pick_files(move |chosen| {
            let _ = tx.send(chosen);
        });

    rx.recv()
        .ok()
        .flatten()
        .map(|paths| {
            paths
                .into_iter()
                .filter_map(|p| p.into_path().ok())
                .map(|p| p.to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default()
}


/// The same picker on Linux, where the dialog plugin is not a dependency and
/// GTK is what the folder picker already uses. See the note above it for why
/// this route exists at all.
#[cfg(target_os = "linux")]
#[tauri::command(async)]
fn pick_files(app: AppHandle) -> Vec<String> {
    use gtk::prelude::*;

    let (tx, rx) = std::sync::mpsc::channel();
    let _ = app.run_on_main_thread(move || {
        let dialog = gtk::FileChooserDialog::new(
            Some("Which files should it look at?"),
            None::<&gtk::Window>,
            gtk::FileChooserAction::Open,
        );
        dialog.add_button("Cancel", gtk::ResponseType::Cancel);
        dialog.add_button("Use these", gtk::ResponseType::Accept);
        dialog.set_select_multiple(true);
        let picked = if dialog.run() == gtk::ResponseType::Accept {
            dialog
                .filenames()
                .into_iter()
                .map(|p| p.to_string_lossy().into_owned())
                .collect()
        } else {
            Vec::new()
        };
        unsafe { dialog.destroy() };
        let _ = tx.send(picked);
    });
    rx.recv().unwrap_or_default()
}

#[cfg(windows)]
fn grant_microphone_permission(webview: tauri::webview::PlatformWebview) {
    use webview2_com::Microsoft::Web::WebView2::Win32::{
        COREWEBVIEW2_PERMISSION_KIND, COREWEBVIEW2_PERMISSION_KIND_MICROPHONE,
        COREWEBVIEW2_PERMISSION_STATE_ALLOW, COREWEBVIEW2_PERMISSION_STATE_DENY,
    };

    let controller = webview.controller();
    let Ok(core) = (unsafe { controller.CoreWebView2() }) else {
        // No CoreWebView2 yet — nothing to attach to. Voice input will fail
        // the same way it did before this fix; better than a panic on an
        // otherwise-working window.
        return;
    };

    let mut token = 0i64;
    let handler = webview2_com::PermissionRequestedEventHandler::create(Box::new(
        |_sender, args| {
            let Some(args) = args else { return Ok(()) };
            let mut kind = COREWEBVIEW2_PERMISSION_KIND::default();
            unsafe { args.PermissionKind(&mut kind) }?;
            let state = if kind == COREWEBVIEW2_PERMISSION_KIND_MICROPHONE {
                COREWEBVIEW2_PERMISSION_STATE_ALLOW
            } else {
                COREWEBVIEW2_PERMISSION_STATE_DENY
            };
            unsafe { args.SetState(state) }
        },
    ));
    let _ = unsafe { core.add_PermissionRequested(&handler, &mut token) };
}

// macOS/iOS: not a build target of this project yet (see `bundle.targets` in
// tauri.conf.json — deb and appimage only; Windows ships as a raw cross
// build, not a bundle, and macOS isn't wired up at all). WKWebView's mic
// prompt is gated by an Info.plist entitlement (NSMicrophoneUsageDescription)
// plus a `WKUIDelegate` permission callback, not by anything reachable from
// here — genuinely different mechanism, left for whenever macOS shipping is
// actually on the table rather than guessed at now.
#[cfg(not(any(target_os = "linux", windows)))]
fn grant_microphone_permission(_webview: tauri::webview::PlatformWebview) {}

/// The same thing everywhere else, where the plugin is the right answer.
#[cfg(not(target_os = "linux"))]
#[tauri::command(async)]
fn pick_folder(app: AppHandle, current: String) -> Option<String> {
    use tauri_plugin_dialog::DialogExt;

    let (tx, rx) = std::sync::mpsc::channel();
    let mut dialog = app
        .dialog()
        .file()
        .set_title("Which folder should it work in?");

    let start = std::path::PathBuf::from(current.trim());
    if start.is_dir() {
        dialog = dialog.set_directory(start);
    }

    dialog.pick_folder(move |chosen| {
        let _ = tx.send(chosen);
    });

    rx.recv()
        .ok()
        .flatten()
        .and_then(|p| p.into_path().ok())
        .map(|p| p.to_string_lossy().into_owned())
}

fn main() {
    // THE MCP MEMORY SERVER LIVES IN THIS SAME BINARY. Dispatch BEFORE Tauri
    // boots: Claude Code spawns `remembrancer mcp-memory --workdir <dir>` as
    // a headless subprocess with piped stdio, and this path must never
    // initialise a webview or touch the display.
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("mcp-memory") {
        std::process::exit(mcp::run(&args[2..]));
    }

    if args.get(1).map(String::as_str) == Some("gmail-mcp") { std::process::exit(google_email::run()); }
    if args.get(1).map(String::as_str) == Some("ms-mcp") { std::process::exit(ms_graph::run()); }
    if args.get(1).map(String::as_str) == Some("email-mcp") {
        std::process::exit(email::run(&args[2..]));
    }

    // SEND-SAFETY (Cassandra re-review residual, 2026-09-09): the consequential
    // arming signal is an INHERITED env var. The four MCP-server subcommands
    // above have already exited, so only the MAIN APP process reaches here --
    // before Tauri boots and long before any turn can spawn the Claude-CLI
    // engine (which does NOT env_clear its child, so claude.exe + its MCP
    // children would inherit this). Scrub it once so a CLI-engine server can
    // only ever be armed by the native path's config injection, never by a
    // poisoned parent environment. The native path re-injects the flag per
    // child from config after env_clear, so this does not touch native arming.
    std::env::remove_var("NAMEOS_ALLOW_CONSEQUENTIAL");

    /* EVERYTHING BELOW CAN FAIL WITH NOTHING ON SCREEN, so the ability to say
       so is installed first. The order is the design:

         1. The log forwarder, first of all -- everything after this line
            that calls into tauri or wry can now have ITS internal errors
            reach our log too, not just our own `note()` calls. See
            `install_log_forwarder`'s doc: a failed window/webview creation
            has always logged exactly one line, straight into a sink nothing
            was listening to.
         2. The panic hook, because the failure MEASURED on 2026-08-28 is a
            panic inside tao's GTK init -- `run()` never returns, so the
            `Result` arm at the bottom of this function never sees it.
         3. A line saying we started, so "launched and vanished" and "never
            launched" stop looking identical in the log.
         4. The webview preflight, which on Windows is a real answer to "is
            there a WebView2 runtime" and on Linux is only a breadcrumb. It
            does not return if the answer is no.
         5. The WebView2/WebKitGTK data-directory check -- logs the exact
            path resolved (so the next report of "can't read and write to
            its data directory" is diagnosable from our own log, which it
            never was before), and proves that path is genuinely writable,
            with one bounded recovery attempt, before Tauri gets anywhere
            near it. See `ensure_webview_data_dir_ready`'s doc.

       None of it touches the mcp-memory path above: that is a headless stdio
       subprocess Claude Code spawns many times a session, and it must stay
       silent, log nothing, and never raise a dialog. */
    startup::install_log_forwarder();
    startup::install_panic_hook();
    // ONE-TIME MIGRATION, BEFORE THE FIRST NEW LINE — Beck's 1.0.2 GO report:
    // a log written by 1.0.0/1.0.1 still has C:\Users\<name> in it, sitting
    // right under the new Troubleshooting log button. Runs before the line
    // below so this launch's own "starting helloim.ai" line lands in an
    // already-clean file rather than racing its own migration.
    startup::redact_existing_log_once();
    startup::note(&format!("starting helloim.ai {}", env!("CARGO_PKG_VERSION")));
    startup::preflight();
    startup::ensure_webview_data_dir_ready();

    let builder = tauri::Builder::default();
    #[cfg(not(target_os = "linux"))]
    let builder = builder.plugin(tauri_plugin_dialog::init());
    let builder = builder.plugin(tauri_plugin_updater::Builder::new().build());
    // Added 2026-09-23, release v1.0.1 -- the app had no way to open a link
    // in the system browser at all. Confirmed by reading the code, not by
    // clicking anything on Windows (no Windows box to click it on from
    // here): no `tauri-plugin-opener` or `tauri-plugin-shell` was
    // registered anywhere, and `<a target="_blank">` alone is a well-known
    // trap in a Tauri webview -- Tauri's own issue tracker (#4756, #5931,
    // #7285) confirms it frequently just does nothing rather than opening
    // the OS browser, because the webview treats it as in-app navigation
    // first. This plugin, plus the `opener:default` capability below, plus
    // an explicit `invoke('plugin:opener|open_url', ...)` call from the
    // click handler (see ui/index.html) rather than trusting the bare
    // anchor tag, is the combination Tauri's own docs give for this.
    let builder = builder.plugin(tauri_plugin_opener::init());
    // Registers the extension trait (`app.global_shortcut()`) that
    // `companion::register` needs. This call alone registers no chord and
    // shows no window — see that module for the actual hotkey.
    let builder = builder.plugin(tauri_plugin_global_shortcut::Builder::new().build());

    builder
        .manage(Session::default())
        .manage(connectors::Connectors::default())
        .manage(google_email::Login::default())
        .manage(ms_graph::Login::default())
        .manage(providers::Providers::default())
        .manage(tts::Tts::default())
        .manage(update::Pending::default())
        .manage(marketplace::Market::default())
        // The safe agency layer's per-call confirm postbox -- see
        // `PendingConfirms`'s own doc.
        .manage(PendingConfirms::default())
        // Its own flag, never `Session`'s. Sharing that mutex is precisely what
        // would let the payoff panel block the person's real conversation.
        .manage(voice_demo::Demo::default())
        // Background tasks -- its own registry, never `Session`'s, for the
        // same reason `voice_demo::Demo` gets its own flag above: sharing
        // `Session.turn` would let a worker block or be blocked by ordinary
        // chat. See `worker.rs`'s own header.
        .manage(worker::Workers::default())
        .invoke_handler(tauri::generate_handler![
            send,
            stop,
            answer_open_url_confirm,
            is_linux_build,
            is_running,
            current_session,
            new_conversation,
            clear_conversations,
            list_conversations,
            delete_conversation,
            default_workdir,
            is_folder,
            pick_folder,
            pick_files,
            preflight,
            scan_notes,
            connectors::list_connectors,
            connectors::save_connector,
            google_email::sign_in_google,
            google_email::cancel_google_sign_in,
            ms_graph::sign_in_microsoft,
            ms_graph::cancel_microsoft_sign_in,
            email::test_email_account,
            email::save_email_account,
            connectors::delete_connector,
            connectors::test_connector,
            connectors::sign_in_connector,
            connectors::disconnect_connector,
            connectors::open_url,
            connectors::list_live_servers,
            connectors::claude_mcp_auth,
            providers::list_providers,
            providers::save_provider,
            providers::delete_provider,
            providers::test_provider,
            providers::select_provider,
            providers::set_secondary_provider,
            providers::disconnect_provider,
            providers::reconnect_provider,
            providers::detect_local_brain,
            providers::get_airgap,
            providers::set_airgap,
            boardroom::boardroom_convene,
            worker::worker_start,
            worker::worker_list,
            worker::worker_get,
            worker::worker_answer_confirm,
            worker::worker_cancel,
            worker::worker_resume,
            brain_setup::probe_hardware,
            brain_setup::brain_advice,
            brain_setup::probe_remote,
            // brain_setup::brain_presets went with the cloud kinds on
            // 2026-08-28 — the UI feature-detects it and simply draws no
            // picker, which is right: with Claude built in and Ollama offered
            // by the wizard, the list it rendered was a list of one.
            brain_setup::recommended_ai,
            marketplace::market_list,
            marketplace::market_launch,
            marketplace::market_stop,
            profile::load_profile,
            profile::save_profile,
            profile::sync_profile,
            profile::save_voice_card,
            profile::save_persona,
            voice_demo::hear_the_difference,
            skills::list_skills,
            skills::save_skill,
            skills::delete_skill,
            skills::find_skills_in_repo,
            skills::list_plugins,
            skills::install_plugin,
            skills::delete_plugin,
            memory::write_bridge,
            memory::read_bridge,
            folder_features::list_folder_features,
            folder_features::folder_feature_preset,
            folder_trust::folder_trust_probe,
            folder_trust::mark_folder_trusted,
            folder_trust::forget_folder_trust,
            facts::remember,
            facts::list_facts,
            facts::forget,
            facts::edit_fact,
            facts::retrieve_relevant_memory,
            facts::search_memory,
            facts::export_memory,
            facts::memory_stats,
            // The per-fact "remember everywhere" toggle -- the only door to
            // it, see facts.rs's own doc on why remember()/edit_fact() must
            // never be able to set it implicitly.
            facts::set_everywhere,
            context_files::add_context_file,
            context_files::list_context_files,
            context_files::remove_context_file,
            memory_sync::pull_memory_facts,
            memory_sync::push_memory_facts,
            memory_sync::push_memory_delete,
            install::install_claude,
            install::open_claude_login,
            install::open_claude_console_login,
            install::open_node_download,
            install::open_voice_settings,
            tts::tts_status,
            tts::tts_install,
            tts::tts_voices,
            tts::tts_speak,
            update::check_update,
            update::install_update,
            feedback::submit_feedback,
            account::account_sign_in,
            account::account_sign_out,
            account::account_status,
            account::push_settings,
            account::pull_settings,
            // Shared persona sync (GET/PUT /api/persona) -- separate from
            // push_settings/pull_settings above, see account.rs's own
            // "Shared persona sync" section header for why the two routes
            // are not the same call with a different name.
            account::pull_persona,
            account::push_persona,
            // The cross-device sync toggle (GET/POST /api/sync/pref) --
            // see account.rs's own "Sync preference" section header. Gates
            // pull_persona/push_persona above and memory_sync's own three
            // fire-points; none of those touch the network unless this has
            // been asked and answered "on."
            account::sync_pref_get,
            account::sync_pref_set,
            // The launch-gate additions for the nameos.ai -> helloim.ai
            // account migration, 2026-09-23 -- see account.rs's own doc on
            // each for why the gate needed all four rather than reusing
            // account_status (which never asks the server anything).
            account::account_verify,
            account::account_license,
            account::account_signup,
            account::account_reset_request,
            companion::show_companion,
            companion::hide_companion,
            companion::toggle_companion,
            companion::set_companion_hotkey,
            // The push-to-talk chord onboarding Step 3's recorder actually
            // calls (`invoke('register_global_shortcut', { chord })` in
            // ui/index.html) -- see mic_hotkeys.rs's own doc on that command
            // for the collision and never-leave-them-with-nothing rules it
            // enforces that companion::set_companion_hotkey did not need to.
            mic_hotkeys::register_global_shortcut,
            // The other half of that same recorder: RegisterHotKey delivers
            // WM_HOTKEY, never a keydown, to every window including this
            // one's webview -- so while mute/push-to-talk/companion are
            // registered with the OS, the recorder's own keydown listener
            // never fires for a chord that collides with one of them. These
            // two suspend all three before the recorder starts listening and
            // put back whatever should be active once it stops -- see
            // mic_hotkeys.rs's own doc on both for why resume re-reads the
            // live trackers instead of a snapshot taken at suspend time.
            mic_hotkeys::suspend_global_shortcuts_for_recording,
            mic_hotkeys::resume_global_shortcuts_after_recording,
            // Voice input's second path — see stt.rs's header. ui/index.html
            // calls this ONLY when window.SpeechRecognition is absent (the
            // Linux/WebKitGTK case today); a Windows build with the browser
            // API present never reaches it, so this addition changes nothing
            // there.
            stt::stt_transcribe,
            // Where that audio now comes from on Linux — see
            // native_capture.rs's header and the module-declaration comment
            // above. Bare names here because both are in scope either via
            // the `use native_capture::{...}` re-export (Linux) or the
            // same-named stub defined directly in this file (everywhere
            // else) — same pattern as pick_folder/pick_files.
            native_capture_start,
            native_capture_stop_and_transcribe,
            // The crash/hang report -- see diagnostics.rs's own header.
            // `heartbeat` is called on a timer from ui/index.html;
            // `open_crash_log` is the Settings -> Help & support button.
            diagnostics::heartbeat,
            diagnostics::open_crash_log
        ])
        .setup(|app| {
            // Retired "api" connectors move to the Brain sheet BEFORE either
            // store's first read — commands cannot fire until the page loads,
            // so this runs against files nothing else has opened yet. Best
            // effort: a failed write leaves both files untouched and the next
            // launch retries; it can never drop a row or touch a credential,
            // because migrate.rs contains neither operation.
            migrate::run(app.handle());
            // THE TRANSLATOR KNOWS WHERE providers.json LIVES AGAIN — restored
            // 2026-08-29 with the `openai-compatible` kind it exists to serve.
            //
            // THIS DOES NOT START ANYTHING. `init` hands over a path and
            // nothing else; the listener is only bound by `ensure_running()`,
            // which is called from exactly one place — `providers::apply_env`'s
            // openai arm — and that arm is unreachable while
            // `providers::OPENAI_ENABLED` is false. So on this build the relay
            // still never binds a port. Three things had to come back together
            // or the failure would have been silent, and this is the third:
            // ROUTABLE_KINDS' gated half, apply_env's arm, and this call.
            //
            // Best effort by design: if the config directory cannot be
            // resolved, the adapter simply stays uninitialised and refuses to
            // start, which is the same safe state it has been in since
            // 2026-08-28. A brain that cannot launch says so; one that launches
            // into a half-configured relay does not.
            match app.path().app_config_dir() {
                Ok(dir) => adapter::init(dir),
                Err(e) => eprintln!("adapter: no config dir, translator stays off: {e}"),
            }
            // Voice input needs the mic granted before the front end ever
            // calls getUserMedia() — see `grant_microphone_permission` for
            // why neither webview backend does this on its own. Best-effort:
            // a window that somehow isn't ready yet just keeps the old
            // (silently-denied) behaviour rather than failing startup over a
            // feature that isn't the app's core job.
            if let Some(window) = app.get_webview_window("main") {
                if let Err(e) = window.with_webview(grant_microphone_permission) {
                    eprintln!("could not wire up microphone permission: {e}");
                }
            }
            // The window object exists by now. Whether anything ever appears on
            // screen is a different question, and this is what asks it.
            startup::watch_for_a_window_that_never_appears(app.handle().clone());
            // The other half of the same report: a window that DID appear and
            // has since stopped responding. See diagnostics.rs's own header.
            diagnostics::install_hang_watchdog();
            // The companion's summon/dismiss chord. Best-effort — see the
            // function's own doc for why a failed registration must not stop
            // the app from starting.
            companion::register_default_hotkey(app.handle());
            // The mic kill switch and push-to-talk hold — overnight hardening
            // batch #2, item 3. Same shared `global_shortcut` manager as the
            // line above; see mic_hotkeys.rs's header for why neither this
            // call nor companion's may ever go back to `unregister_all()`.
            mic_hotkeys::register_default_hotkeys(app.handle());
            // Any worker task still marked `running`/`needsInput` on disk at
            // this point means the process that owned it exited without
            // finishing -- closed, crashed, or killed. Retitle it
            // `needsResume` and stop there: this call NEVER re-starts a
            // task's supervisor thread, only rewrites its status, so an
            // unattended task with real tool access is never picked back up
            // without a person's own click. See `worker.rs`'s own header and
            // `flag_orphans_on_launch`'s doc for why that line matters.
            worker::flag_orphans_on_launch(app.handle());
            Ok(())
        })
        // The first honest evidence that a webview exists and ran something.
        // Nothing in the front end had to change for this: it is a Rust-side
        // hook, so it cannot be lost by an edit to the page.
        .on_page_load(|_, _| startup::page_loaded())
        // CLOSING THE WINDOW LEFT NameOS.exe RUNNING -- Beck, b21/b20,
        // 2026-08-31: window destroyed, every WebView2 child gone, the
        // process itself still alive 187 seconds later. It happened to Mark
        // the same day -- he closed the app and it kept talking to him with
        // nothing on screen to show it or stop it.
        //
        // There was no `on_window_event` handler at all before this. Tauri's
        // own default -- exit once every window is gone -- assumes nothing
        // else is keeping the process alive, and that assumption is false
        // here: `state.turn` (managed in `Session`, started by `send()`) is
        // a real OS process running Claude Code, quite possibly mid-turn
        // under `--permission-mode bypassPermissions` with real tool access,
        // piped stdio nobody is reading anymore. Tauri has no way to know it
        // exists, so its own teardown never touches it.
        //
        // THE ANSWER IS: CLOSING MEANS EXIT, FULL STOP. No tray icon, no
        // "keep listening in the background" -- that would need a visible
        // indicator and a way to stop it (a wake word is not implemented
        // anywhere in this codebase today, checked by grep before writing
        // this), and inventing that under this bug would be a feature no one
        // asked for standing in front of a fix that is. So: kill the running
        // turn first, with the exact same `cancel_running_turn()` the Stop
        // button already calls (see `stop`, above) -- reused here rather
        // than duplicated, so nothing keeps running or speaking once the
        // window everyone reads it from is gone. Then call `exit(0)` on the
        // app handle directly, rather than trusting the default post-close
        // teardown to get there on its own -- this bug is the proof that
        // trust was misplaced once already, and an explicit exit costs
        // nothing when the default was already about to do the same thing.
        //
        // `CloseRequested` fires for every way this window can close --
        // the titlebar's own close control, Alt+F4, the taskbar, and the
        // front end's own `getCurrentWindow().close()` alike -- because all
        // of them go through the same OS close request.
        //
        // **THIS HANDLER USED TO RUN `cancel_running_turn` INLINE, ON THIS
        // CLOSURE'S OWN THREAD, AND THAT THREAD IS THE MAIN THREAD — fixed
        // 2026-09-04, alongside the identical fault in `stop` (see
        // `cancel_running_turn`'s own doc for the deadlock this was hitting).**
        // A window event callback cannot be marked `async` the way a
        // `#[tauri::command]` can, so the fix here is the same idea by a
        // different mechanism: `prevent_close()` unconditionally, do the real
        // work (cancel, then exit) on a plain OS thread that owns none of
        // this window's message pump, and let THAT thread be the one that
        // finally calls `exit(0)`. The window still closes and the process
        // still exits exactly as before -- it just cannot freeze getting
        // there, even if a future engine's `cancel()` is slow rather than
        // instant.
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                // THE COMPANION NEVER CLOSES, ONLY HIDES. It is the
                // summonable thing — the whole point of it is that the
                // hotkey can always bring it back. Letting its close control
                // (or Alt+F4, or anything else that sends the OS a close
                // request) fall through to the logic below would destroy
                // that window the first time anyone used it, and the next
                // press of the hotkey would find nothing to show.
                // `api.prevent_close()` stops the close the OS was about to
                // do; `hide()` gives the same visible result — it is gone —
                // without tearing the webview down or touching `state.turn`,
                // which `main` closing is what actually needs to do.
                if window.label() == companion::LABEL {
                    api.prevent_close();
                    let _ = window.hide();
                    return;
                }
                api.prevent_close();
                let app_handle = window.app_handle().clone();
                // A THREAD THAT WILL NOT START IS NOT A REASON TO HANG THE
                // CLOSE -- same shape `NativeEngine::start`'s own comment
                // already argues for a turn's thread; here the fallback is
                // simpler because there is no sink relying on an ending: just
                // do the same two steps inline. Slower to close in that one
                // case, never stuck.
                if std::thread::Builder::new()
                    .name("nameos-close-cleanup".into())
                    .spawn(move || {
                        let _ = cancel_running_turn(&app_handle.state::<Session>());
                        app_handle.exit(0);
                    })
                    .is_err()
                {
                    let app_handle = window.app_handle();
                    let _ = cancel_running_turn(&app_handle.state::<Session>());
                    app_handle.exit(0);
                }
            }
        })
        .run(tauri::generate_context!())
        // NOT `.expect(...)`. On a release build there is no stderr for a panic
        // message to reach, so the old line reported this failure to nobody at
        // all -- see the header of startup.rs. `fatal` writes it down and, on
        // Windows, says it out loud.
        .unwrap_or_else(|e| {
            startup::fatal(
                "helloim.ai can't start",
                &format!(
                    "helloim.ai could not open its window on this PC.\n\n\
                     Nothing you've saved is affected. This is almost always the \
                     Microsoft Edge WebView2 Runtime being missing or damaged: \
                     reinstall it from\n\
                     https://developer.microsoft.com/microsoft-edge/webview2/\n\
                     or run the helloim.ai installer again, then start helloim.ai.\n\n\
                     Technical detail: {e}"
                ),
            )
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// CLOSING THE WINDOW LEFT NameOS.exe RUNNING -- Beck, b21/b20,
    /// 2026-08-31, on real hardware: every WebView2 child gone, the process
    /// itself still alive 187 seconds later. This proves the actual fix, not
    /// the appearance of one -- an earlier version of this kind of change
    /// could pass by only checking that the tracked turn was set back to
    /// `None`, which a version that merely dropped the `Child` handle
    /// (forgetting the pid without ever signalling it) would also satisfy.
    /// Rust drops an unwaited `Child` without killing it -- that is
    /// documented `std::process` behaviour, not a bug -- so "the field reads
    /// `None` again" and "the process is actually dead" are two different
    /// claims, and only checking the first is exactly how this bug shipped
    /// once already.
    ///
    /// **IT STILL CHECKS THE SECOND CLAIM, THROUGH THE SEAM.** The `Child` now
    /// lives inside `engine::claude_code::ClaudeCodeRun`, so this drives the
    /// real one -- a genuine `sleep 30` boxed as a `RunningTurn` -- and asks
    /// the kernel, not Rust, whether it died. Nothing about the strength of the
    /// proof changed; only which module owns the process. The unit-level twin
    /// is `engine::claude_code::tests::cancel_actually_terminates_the_process`,
    /// and this one additionally covers the wiring: that `cancel_running_turn`
    /// reaches the engine at all and clears the slot afterwards.
    ///
    /// `#[cfg(unix)]` for the same reason as the timeout tests below -- `sleep`
    /// is not a Windows binary and this box cannot run one to test with
    /// regardless; neither `cancel_running_turn` nor `ClaudeCodeRun::cancel`
    /// has platform-specific code in it (plain `std::process::Child::kill` /
    /// `wait`), so proving the mechanism here is proving the mechanism itself.
    #[cfg(unix)]
    #[test]
    fn cancel_running_turn_calls_through_to_the_engine() {
        let session = Session::default();
        let mut cmd = Command::new("sleep");
        cmd.arg("30"); // long enough that a handle merely being dropped would leave it running
        let child = cmd.spawn().expect("failed to spawn sleep");
        let pid = child.id();
        *session.turn.lock().unwrap() =
            Some((1, Box::new(engine::claude_code::ClaudeCodeRun::new(child))));

        assert!(alive(&session), "a sleeping child must read as a live turn");

        cancel_running_turn(&session).expect("cancel_running_turn should succeed");

        assert!(
            session.turn.lock().unwrap().is_none(),
            "cancel_running_turn must clear the tracked turn"
        );
        assert!(
            !std::path::Path::new(&format!("/proc/{pid}")).exists(),
            "the child process (pid {pid}) is still alive after cancel_running_turn -- \
             it was forgotten, not killed, which is the exact shape of the window-close bug"
        );
        assert!(!alive(&session), "a cancelled turn must not read as alive");
    }

    /// **THE REGRESSION THIS EXISTS TO CATCH: the same test above, but for
    /// the engine whose `cancel()` chain actually re-enters `Session.turn` —
    /// `ClaudeCodeRun`'s never has, which is exactly why that test alone let
    /// the 2026-09-04 deadlock ship.** `NativeRun::cancel()` runs through
    /// `Ending::close()`, which unconditionally calls `sink.finished()`; the
    /// real sink in this app, `AppSink::finished`, re-locks `state.turn` to
    /// clear it. The old `cancel_running_turn` held that lock across the
    /// whole `cancel()` call, so the second lock attempt landed on a mutex
    /// this exact thread already owned — a same-thread, permanent deadlock.
    ///
    /// **This test cannot build a real `AppHandle`** — `tauri`'s `"test"`
    /// feature is not enabled in this build, and adding it just to construct
    /// one window for one test is a bigger footprint than the fault being
    /// proven needs. What actually mattered was never the Tauri emit calls;
    /// it was one specific lock being taken twice. So `ReLockingSink` does
    /// only that one thing `AppSink::finished` does — re-lock `session.turn`
    /// from inside `finished()` — which is the exact and only shape the real
    /// deadlock needed.
    ///
    /// **RUN ON ITS OWN THREAD WITH A HARD DEADLINE, DELIBERATELY.** A true
    /// regression here does not return an error, it never returns at all —
    /// which is precisely what froze Mark's window. Asserting on a return
    /// value from a call that might never make one would just move the hang
    /// from the app into `cargo test`. `recv_timeout` turns "hangs forever"
    /// into "fails loudly within 5 seconds" instead, which is the whole
    /// point of a regression test for a deadlock: the FAILURE has to be
    /// something a CI run can actually observe and report.
    ///
    /// **PROVEN ABLE TO FAIL**: reverting `cancel_running_turn` to hold its
    /// `MutexGuard` across `turn.cancel()` (the pre-2026-09-04 shape) makes
    /// this test time out every run, checked by hand before writing this
    /// comment.
    #[test]
    fn cancel_running_turn_does_not_deadlock_a_native_run() {
        struct ReLockingSink {
            session: std::sync::Arc<Session>,
        }
        impl engine::TurnSink for ReLockingSink {
            fn event(&self, _value: serde_json::Value) {}
            fn raw(&self, _line: String) {}
            fn failure(&self, _line: String) {}
            fn finished(&self, _code: i32) {
                // The exact re-entry `AppSink::finished` performs, above:
                // clear the tracked turn under the SAME lock
                // `cancel_running_turn` itself takes.
                *self.session.turn.lock().unwrap() = None;
            }
        }

        let session = std::sync::Arc::new(Session::default());
        let sink: std::sync::Arc<dyn engine::TurnSink> =
            std::sync::Arc::new(ReLockingSink { session: session.clone() });
        let run = engine::native::NativeRun::new_for_test(sink);
        *session.turn.lock().unwrap() = Some((1, Box::new(run)));

        assert!(alive(&session), "a fresh native run must read as a live turn");

        let session_for_thread = session.clone();
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let result = cancel_running_turn(&session_for_thread);
            let _ = done_tx.send(result);
        });

        let result = done_rx.recv_timeout(std::time::Duration::from_secs(5)).expect(
            "cancel_running_turn did not return within 5s -- this IS the deadlock this test \
             exists to catch: NativeRun::cancel -> Ending::close -> sink.finished() re-locking \
             Session.turn while cancel_running_turn is still holding it",
        );
        assert!(result.is_ok(), "cancel_running_turn should succeed: {result:?}");
        assert!(
            session.turn.lock().unwrap().is_none(),
            "cancel_running_turn must clear the tracked turn on success"
        );
        assert!(!alive(&session), "a cancelled turn must not read as alive");
    }

    // -- the two races Cassandra found in the deadlock fix -------------------

    /// A trivial `RunningTurn` for the races below -- `is_alive` is never
    /// read by anything these tests check, so it just answers `true`.
    struct AlwaysAliveRun;
    impl engine::RunningTurn for AlwaysAliveRun {
        fn is_alive(&self) -> bool {
            true
        }
        fn cancel(&self) -> Result<(), String> {
            Ok(())
        }
    }

    /// **RACE 1: STOP AND WINDOW-CLOSE RACING EACH OTHER MUST NOT ORPHAN THE
    /// CHILD — Cassandra, 2026-09-04.** The `take()` rewrite that closed the
    /// reentrant-lock deadlock removed the one thing that used to serialize
    /// two concurrent cancellers: the old code held `session.turn`'s own
    /// lock across the whole `cancel()` call, so a second caller simply
    /// blocked on that mutex until the first was done. `take()` empties the
    /// slot immediately, so a second caller now sees `None` and returns
    /// `Ok(())` at once -- and `on_window_event` calls `cancel_running_turn`
    /// then `app_handle.exit(0)`, so a window-close racing an in-flight Stop
    /// could exit the whole process WHILE the Stop's own `turn.cancel()` (a
    /// real `Child::kill()`/`wait()`, real time) was still running. That is
    /// the b20/b21 orphan bug (`claude.exe` alive after every window is
    /// gone) back under a race.
    ///
    /// **What this proves, concretely, without a real child process:** a
    /// `RunningTurn` whose `cancel()` takes a real, measurable amount of time
    /// records WHEN it finished; the second caller (simulating window-close)
    /// records WHEN its own call to `cancel_running_turn` RETURNED. The
    /// invariant `session.cancel_gate` exists to guarantee is that the
    /// second timestamp can never be earlier than the first — if it were,
    /// `app_handle.exit(0)` would already be racing a kill still in flight.
    ///
    /// **PROVEN ABLE TO FAIL**: removing the `session.cancel_gate` lock from
    /// `cancel_running_turn` makes the second caller return almost
    /// immediately (it finds the slot already `None` and takes the early
    /// `Ok(())` return) — checked by hand, well before the slow cancel's own
    /// 150ms sleep elapses.
    #[test]
    fn a_second_canceller_waits_for_a_concurrent_cancels_real_kill_to_finish() {
        struct SlowCancelRun {
            finished_at: std::sync::Arc<Mutex<Option<std::time::Instant>>>,
        }
        impl engine::RunningTurn for SlowCancelRun {
            fn is_alive(&self) -> bool {
                true
            }
            fn cancel(&self) -> Result<(), String> {
                // Long enough that a second caller returning early (the bug)
                // and a second caller genuinely waiting (the fix) are not a
                // photo finish -- the whole point is to give a WIDE margin.
                std::thread::sleep(std::time::Duration::from_millis(150));
                *self.finished_at.lock().unwrap() = Some(std::time::Instant::now());
                Ok(())
            }
        }

        let session = std::sync::Arc::new(Session::default());
        let finished_at: std::sync::Arc<Mutex<Option<std::time::Instant>>> =
            std::sync::Arc::new(Mutex::new(None));
        *session.turn.lock().unwrap() =
            Some((1, Box::new(SlowCancelRun { finished_at: finished_at.clone() })));

        // The "Stop" caller: grabs the turn and starts its slow, real kill.
        let s1 = session.clone();
        let stop_thread = std::thread::spawn(move || cancel_running_turn(&s1));
        // Give it a head start to win `cancel_gate` and start sleeping,
        // before the "window-close" caller below even tries.
        std::thread::sleep(std::time::Duration::from_millis(30));

        // The "window-close" caller: in the real app, this is immediately
        // followed by `app_handle.exit(0)` -- what matters is WHEN this call
        // returns, not its result (it will be `Ok(())`, having found the
        // slot already emptied by Stop).
        let s2 = session.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        let close_thread = std::thread::spawn(move || {
            let result = cancel_running_turn(&s2);
            let _ = tx.send(std::time::Instant::now());
            result
        });

        stop_thread.join().unwrap().expect("Stop's own cancel must succeed");
        let close_returned_at = rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("window-close's cancel_running_turn never returned");
        close_thread.join().unwrap().expect("window-close's cancel_running_turn must succeed");

        let kill_finished_at =
            finished_at.lock().unwrap().expect("the slow cancel never recorded its own completion");
        assert!(
            close_returned_at >= kill_finished_at,
            "window-close's cancel_running_turn returned at {close_returned_at:?}, BEFORE the \
             concurrent Stop's real kill work finished at {kill_finished_at:?} -- app_handle.exit(0) \
             would have raced an in-flight kill, which is the b20/b21 orphan bug under a race"
        );
    }

    /// **RACE 2a: A LATE `finished()` CALL MUST NEVER WIPE A NEWER TURN OUT
    /// OF THE SLOT — Cassandra, 2026-09-04.** While a turn's own `cancel()`
    /// is in flight, the slot reads `None` (see `cancel_running_turn`'s own
    /// doc), and a `send()` landing in that exact window correctly starts a
    /// brand new turn there. The OLD turn's `finished()` callback, arriving
    /// late, used to clear the slot unconditionally -- silently untracking
    /// the NEW turn: `is_running` would lie, and Stop/window-close would
    /// become no-ops against a turn that is very much still running.
    ///
    /// This proves `clear_turn_if_still_current` directly, at the unit this
    /// codebase can actually check without a real `AppHandle` (`tauri`'s
    /// `"test"` feature is not enabled here) -- `AppSink::finished` calls
    /// exactly this function with exactly `self.generation`, so proving the
    /// function is proving the fix.
    #[test]
    fn clear_turn_if_still_current_never_wipes_a_newer_turn() {
        let session = Session::default();
        // Generation 1 (turn A) occupied the slot, then generation 2 (turn
        // B) replaced it -- exactly what a `send()` landing while A's own
        // cancel/kill was still in flight would do.
        *session.turn.lock().unwrap() = Some((1, Box::new(AlwaysAliveRun)));
        *session.turn.lock().unwrap() = Some((2, Box::new(AlwaysAliveRun)));

        // A's own `finished()` callback fires late, believing it still owns
        // the slot under generation 1.
        clear_turn_if_still_current(&session, 1);

        let guard = session.turn.lock().unwrap();
        let (generation, _) = guard.as_ref().expect(
            "an old turn's late finished() call wiped a NEWER turn out of the slot -- is_running \
             would now lie and Stop/close would be a no-op against a turn that is still running",
        );
        assert_eq!(*generation, 2, "turn B must still be the one tracked, unchanged");
    }

    /// The other half of `clear_turn_if_still_current`: it must still
    /// actually clear the slot when it IS the current occupant -- proving
    /// only the "never wipes a newer one" half would leave a version that
    /// simply never clears anything passing too.
    #[test]
    fn clear_turn_if_still_current_clears_its_own_generation() {
        let session = Session::default();
        *session.turn.lock().unwrap() = Some((1, Box::new(AlwaysAliveRun)));
        clear_turn_if_still_current(&session, 1);
        assert!(session.turn.lock().unwrap().is_none(), "a turn clearing its OWN generation must succeed");
    }

    /// **RACE 2b: A FAILED CANCEL'S PUT-BACK MUST NEVER OVERWRITE A NEWER
    /// TURN EITHER — Cassandra, 2026-09-04, the other half of race 2.** The
    /// same window that lets a `send()` start turn B while turn A's cancel
    /// is in flight also applies when that cancel FAILS: `cancel_running_
    /// turn`'s own put-back must only restore A if the slot is still
    /// genuinely empty, never by overwriting whatever now occupies it.
    #[test]
    fn a_failed_cancel_never_overwrites_a_newer_turn_that_started_while_it_ran() {
        struct FailingRun;
        impl engine::RunningTurn for FailingRun {
            fn is_alive(&self) -> bool {
                true
            }
            fn cancel(&self) -> Result<(), String> {
                // Real time for the interleaving below to land inside it.
                std::thread::sleep(std::time::Duration::from_millis(80));
                Err("simulated kill failure".into())
            }
        }

        let session = std::sync::Arc::new(Session::default());
        *session.turn.lock().unwrap() = Some((1, Box::new(FailingRun)));

        let s = session.clone();
        let cancel_thread = std::thread::spawn(move || cancel_running_turn(&s));

        // While A's (slow, failing) cancel is still running, a new turn B
        // starts -- exactly what `send()` can do, since it reads the slot as
        // free the instant `take()` ran and has no reason to wait on
        // `cancel_gate` (starting a turn and cancelling one are different
        // operations).
        std::thread::sleep(std::time::Duration::from_millis(20));
        *session.turn.lock().unwrap() = Some((2, Box::new(AlwaysAliveRun)));

        let result = cancel_thread.join().unwrap();
        assert!(result.is_err(), "the simulated kill failure must still surface to the caller");

        let guard = session.turn.lock().unwrap();
        let (generation, _) =
            guard.as_ref().expect("A's failed-cancel put-back overwrote B's slot with nothing");
        assert_eq!(*generation, 2, "A's failed cancel must not have overwritten B's slot");
    }

    // -- "Clear conversation history" actually clears stored conversations. -

    fn scratch_conversations_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir()
            .join(format!("helloim-clear-conversations-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("could not make scratch conversations dir");
        dir
    }

    /// THE PROOF THE ITEM BRIEF ASKED FOR, LITERALLY: real transcript files on
    /// real disk, removed by the real function, checked by asking the
    /// filesystem afterwards rather than trusting the returned count alone.
    #[test]
    fn clear_conversation_files_removes_every_stored_transcript() {
        let dir = scratch_conversations_dir("removes-transcripts");
        for id in ["one", "two", "three"] {
            std::fs::write(dir.join(format!("{id}.json")), "{}").unwrap();
        }

        let removed = clear_conversation_files(&dir).expect("clearing should succeed");

        assert_eq!(removed, 3, "the count must match what was actually deleted");
        let left: Vec<_> = std::fs::read_dir(&dir).unwrap().flatten().collect();
        assert!(left.is_empty(), "a transcript survived the clear: {left:?}");
    }

    /// An empty conversations folder — nobody has talked to a native-engine
    /// brain yet — is a true zero, not an error. A person pressing this
    /// button on day one must not be shown a failure for having nothing to
    /// clear.
    #[test]
    fn clearing_an_empty_directory_removes_nothing_and_does_not_error() {
        let dir = scratch_conversations_dir("empty");
        assert_eq!(clear_conversation_files(&dir).expect("must not error"), 0);
    }

    /// THE ONE THING THIS BUTTON MUST NOT TOUCH, PROVEN RATHER THAN ASSERTED
    /// IN A COMMENT — a stray non-`.json` file living beside the transcripts
    /// (the shape a `.DS_Store` or an editor swap file would take) survives.
    /// `conversations_dir` is a folder this app alone writes to, so nothing
    /// else SHOULD end up in it — but "should never happen" is exactly the
    /// premise a filter like this one exists to not depend on.
    #[test]
    fn a_non_json_file_in_the_directory_is_left_alone() {
        let dir = scratch_conversations_dir("stray-file");
        std::fs::write(dir.join("real.json"), "{}").unwrap();
        std::fs::write(dir.join(".DS_Store"), "not ours").unwrap();

        let removed = clear_conversation_files(&dir).expect("clearing should succeed");

        assert_eq!(removed, 1, "only the .json transcript should be counted");
        assert!(!dir.join("real.json").exists());
        assert!(dir.join(".DS_Store").exists(), "a file we do not own must survive the clear");
    }

    // -- History: list_conversation_files / summarize. --------------------

    fn write_scratch_conversation(dir: &std::path::Path, id: &str, messages: &[(bool, &str)]) {
        use engine::native::store::{Conversation, Message, Role};
        let mut convo = Conversation::new(id.to_string(), "p1".into(), "m".into());
        for (is_user, text) in messages {
            convo.messages.push(Message {
                role: if *is_user { Role::User } else { Role::Assistant },
                text: text.to_string(),
                tool: None,
            });
        }
        convo.save(dir).expect("write scratch conversation");
    }

    /// The title comes from the PERSON's own first line, never the
    /// assistant's reply — proven able to fail: swapping `first_user` for
    /// `last` in `list_conversation_files` would still pass every other
    /// assertion here and silently title every row with the answer instead
    /// of the question.
    #[test]
    fn list_conversation_files_titles_from_the_users_own_first_message() {
        let dir = scratch_conversations_dir("history-title");
        write_scratch_conversation(
            &dir,
            "helloim-chat-aaaa",
            &[(true, "Draft a reply to Henderson"), (false, "Here you go.")],
        );

        let rows = list_conversation_files(&dir, None, std::time::SystemTime::now());

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].title, "Draft a reply to Henderson");
        assert_eq!(rows[0].message_count, 2);
    }

    /// Newest touched first — the same ordering SupportAssist's own history
    /// uses, and the one the front end trusts rather than re-sorting.
    #[test]
    fn list_conversation_files_sorts_newest_first() {
        let dir = scratch_conversations_dir("history-order");
        write_scratch_conversation(&dir, "helloim-chat-older", &[(true, "first one")]);
        write_scratch_conversation(&dir, "helloim-chat-newer", &[(true, "second one")]);
        let now = std::time::SystemTime::now();
        std::fs::File::open(dir.join("helloim-chat-older.json"))
            .unwrap()
            .set_modified(now - Duration::from_secs(3600))
            .unwrap();
        std::fs::File::open(dir.join("helloim-chat-newer.json")).unwrap().set_modified(now).unwrap();

        let rows = list_conversation_files(&dir, None, now);

        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].id, "helloim-chat-newer", "the more recently touched row must come first");
    }

    /// `days` genuinely excludes what it should — proven both ways, because a
    /// filter that always returns everything and one that always returns
    /// nothing both look "fine" if only one direction is checked.
    #[test]
    fn list_conversation_files_respects_the_day_cutoff() {
        let dir = scratch_conversations_dir("history-cutoff");
        write_scratch_conversation(&dir, "helloim-chat-old", &[(true, "nine days ago")]);
        write_scratch_conversation(&dir, "helloim-chat-recent", &[(true, "this morning")]);
        let now = std::time::SystemTime::now();
        std::fs::File::open(dir.join("helloim-chat-old.json"))
            .unwrap()
            .set_modified(now - Duration::from_secs(9 * 86_400))
            .unwrap();
        std::fs::File::open(dir.join("helloim-chat-recent.json")).unwrap().set_modified(now).unwrap();

        let last_7_days = list_conversation_files(&dir, Some(7), now);
        assert_eq!(last_7_days.len(), 1, "the nine-day-old row must be excluded");
        assert_eq!(last_7_days[0].id, "helloim-chat-recent");

        let everything = list_conversation_files(&dir, None, now);
        assert_eq!(everything.len(), 2, "with no cutoff both rows must still be there");
    }

    /// A file whose name is not one of ours (no `helloim-chat-` prefix, or one
    /// that fails `id_is_safe`) must never be read as a conversation — same
    /// boundary `Conversation::load` itself already enforces, checked again
    /// here because this is the function that walks the directory.
    #[test]
    fn list_conversation_files_ignores_a_file_that_is_not_one_of_ours() {
        let dir = scratch_conversations_dir("history-foreign");
        std::fs::write(dir.join("not-ours.json"), "{}").unwrap();
        write_scratch_conversation(&dir, "helloim-chat-real", &[(true, "hi")]);

        let rows = list_conversation_files(&dir, None, std::time::SystemTime::now());

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, "helloim-chat-real");
    }

    #[test]
    fn summarize_truncates_and_marks_that_it_did() {
        assert_eq!(summarize("short", 20), "short");
        assert_eq!(summarize(&"x".repeat(30), 10), format!("{}…", "x".repeat(10)));
    }

    /// Only the FIRST line is ever shown — a multi-line message must not spill
    /// a second sentence into what is meant to be a one-line summary.
    #[test]
    fn summarize_takes_only_the_first_line() {
        assert_eq!(summarize("first line\nsecond line", 60), "first line");
    }

    #[test]
    fn summarize_never_returns_an_empty_string() {
        assert_eq!(summarize("", 60), "(no text)");
        assert_eq!(summarize("\n\n", 60), "(no text)");
    }

    // -- Generic History capture: engines that do not persist their own -----
    //
    // Everything below is the regression for Mark, 2026-09-05, running the
    // shipped app: "history doesn't even work." He was on the built-in Claude
    // brain, and `ClaudeCodeEngine` never wrote a single byte into
    // `conversations_dir` -- only `engine::native`'s own internal
    // `Plan::store_dir` save did (proven correct on its own path by
    // `engine::native::tests::a_completed_turn_appears_in_the_history_list_
    // with_no_id_handed_in`, added earlier the same day). `claude_code.rs` has
    // no reference to `conversations_dir` or `store::Conversation` anywhere in
    // it, so before `AppSink::finished`'s generic capture existed, EVERY
    // Claude-brain turn -- the default a fresh install starts on -- produced
    // exactly zero rows here, no matter how many real conversations happened.
    // The tests below drive the same two pure functions `AppSink` itself now
    // calls (`assistant_text_of` in `event`, `capture_turn_into_history` in
    // `finished`) against event shapes modelled on `claude.exe`'s own real
    // `--output-format stream-json` output, and prove the missing half of the
    // loop: what those functions produce is exactly what `list_conversation_
    // files` -- the History tab's own call -- reads back.

    /// **THE SHAPE A REAL `claude.exe --output-format stream-json` ANSWER
    /// TAKES**, modelled from `engine::native`'s own `assistant_event`/
    /// `tool_use_event` (built to mimic it exactly -- see that file's header)
    /// and from `session_id_of`'s doc on the shared five-shape contract. A
    /// plain text answer, one content block, one call.
    #[test]
    fn assistant_text_of_reads_a_plain_claude_answer() {
        let ev = serde_json::json!({
            "type": "assistant",
            "message": { "role": "assistant", "content": [{ "type": "text", "text": "four" }] }
        });
        assert_eq!(assistant_text_of(&ev), Some("four".to_string()));
    }

    /// **THE SHAPE THAT MUST NOT BE CAPTURED AS "THE ANSWER."** A tool-call
    /// bubble carries no `text` block at all -- see `tool_use_event` in
    /// `engine::native::mod.rs` for the identical shape on the native side --
    /// and `AppSink`'s own `pending_answer` field doc is explicit that only
    /// the LAST text block counts, never a call in between.
    #[test]
    fn assistant_text_of_ignores_a_pure_tool_call() {
        let ev = serde_json::json!({
            "type": "assistant",
            "message": {
                "role": "assistant",
                "content": [{ "type": "tool_use", "id": "1", "name": "Read", "input": {} }]
            }
        });
        assert_eq!(assistant_text_of(&ev), None);
    }

    /// `system` and `result` lines -- the other two of the five shapes -- must
    /// never be read as an answer, and a message with no `content` array at
    /// all (malformed, or a shape this function has never seen) must not
    /// panic looking for one.
    #[test]
    fn assistant_text_of_is_none_for_non_assistant_lines_and_malformed_ones() {
        assert_eq!(assistant_text_of(&serde_json::json!({"type": "system", "session_id": "x"})), None);
        assert_eq!(assistant_text_of(&serde_json::json!({"type": "result", "is_error": false})), None);
        assert_eq!(assistant_text_of(&serde_json::json!({"type": "assistant"})), None);
        assert_eq!(assistant_text_of(&serde_json::json!("not an object")), None);
    }

    /// **THE REGRESSION, END TO END.** Simulates the exact sequence a real
    /// Claude-brain turn produces and that `AppSink` now reacts to: a
    /// `system` line naming the session (caught by `session_id_of`, exactly
    /// as `finished` reads it off `Session::session_id`), a tool-call bubble
    /// with no text (must not become "the answer"), then the real text
    /// answer (must become it, overwriting nothing useful because nothing
    /// useful preceded it). `capture_turn_into_history` is then called with
    /// exactly what `AppSink::finished` would have collected — no
    /// `AppHandle`, no live turn, because neither function needs one.
    ///
    /// **PROVEN ABLE TO FAIL BY WHAT USED TO BE TRUE, NOT BY A MUTATION —
    /// `claude_code.rs` contains no reference to `conversations_dir` or
    /// `store::Conversation` anywhere in it, checked directly.** Before
    /// `Engine::persists_own_history`/`AppSink`'s capture existed, nothing in
    /// this codebase would ever have called `capture_turn_into_history` for a
    /// Claude turn at all, so `list_conversation_files` on this exact `dir`
    /// would have returned empty regardless of how many real conversations
    /// happened — which is the literal shape of "history doesn't even work."
    #[test]
    fn a_completed_claude_engine_turn_appears_in_history() {
        let dir = scratch_conversations_dir("claude-engine-history");
        let session_id = "550e8400-e29b-41d4-a716-446655440000"; // a real claude.exe session id shape

        // What `AppSink::event` sees, in order, for one real Claude turn.
        let system = serde_json::json!({"type": "system", "session_id": session_id});
        let tool_call = serde_json::json!({
            "type": "assistant",
            "message": {"role": "assistant", "content": [{"type": "tool_use", "id": "1", "name": "Read", "input": {}}]}
        });
        let final_answer = serde_json::json!({
            "type": "assistant",
            "message": {"role": "assistant", "content": [{"type": "text", "text": "it says 42"}]}
        });

        // The exact bookkeeping `event` performs, and `pending_answer`'s own
        // doc explains why the tool-call bubble must not survive to here.
        let mut captured_id: Option<String> = None;
        let mut pending_answer = String::new();
        for ev in [&system, &tool_call, &final_answer] {
            if let Some(id) = session_id_of(ev) {
                captured_id = Some(id.to_string());
            }
            if let Some(text) = assistant_text_of(ev) {
                pending_answer = text;
            }
        }
        assert_eq!(captured_id.as_deref(), Some(session_id));
        assert_eq!(pending_answer, "it says 42", "the tool call must not overwrite the real answer, nor vice versa");

        // What `finished` does with what it collected.
        capture_turn_into_history(
            &dir,
            &captured_id.unwrap(),
            "read the file".to_string(),
            pending_answer,
            "claude".to_string(),
            String::new(),
        )
        .expect("a real turn's own text must always be safe to save");

        // THE ACTUAL PROOF: the History tab's own function, asked with no id
        // handed to it in advance, exactly as a person opening the tab does.
        let rows = list_conversation_files(&dir, None, std::time::SystemTime::now());
        assert_eq!(rows.len(), 1, "the Claude turn above must appear in History: {rows:?}");
        assert_eq!(rows[0].id, session_id);
        assert_eq!(rows[0].title, "read the file");
        assert_eq!(rows[0].snippet, "it says 42");
        assert_eq!(rows[0].message_count, 2);
    }

    /// **A SECOND TURN IN THE SAME CLAUDE CONVERSATION APPENDS, IT DOES NOT
    /// REPLACE.** `capture_turn_into_history` loads the existing file before
    /// minting a new one — proven able to fail: swapping the `unwrap_or_else`
    /// for an unconditional `Conversation::new` here would make the second
    /// call below overwrite the first exchange and this test would see one
    /// row with two messages, not four.
    #[test]
    fn a_second_claude_turn_in_the_same_conversation_is_appended_not_overwritten() {
        let dir = scratch_conversations_dir("claude-engine-history-append");
        let id = "550e8400-e29b-41d4-a716-446655440001";

        capture_turn_into_history(&dir, id, "first".into(), "one".into(), "claude".into(), String::new())
            .unwrap();
        capture_turn_into_history(&dir, id, "second".into(), "two".into(), "claude".into(), String::new())
            .unwrap();

        let rows = list_conversation_files(&dir, None, std::time::SystemTime::now());
        assert_eq!(rows.len(), 1, "still one conversation, not two files: {rows:?}");
        assert_eq!(rows[0].message_count, 4, "both exchanges must be present");
        assert_eq!(rows[0].title, "first", "the title stays the conversation's own opening line");
        assert_eq!(rows[0].snippet, "two", "the snippet is the most recent thing said");
    }

    /// A directory that cannot be listed at all — not merely empty, genuinely
    /// unreadable — is reported rather than silently claimed as a successful
    /// clear of nothing. Mirrors `clear_conversations`'s own doc: "report it
    /// rather than silently claiming success on a folder we could not see
    /// into."
    #[test]
    fn an_unreadable_directory_is_an_error_not_a_silent_zero() {
        let missing =
            std::env::temp_dir().join(format!("helloim-clear-conversations-missing-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&missing); // guarantee it does not exist
        assert!(clear_conversation_files(&missing).is_err());
    }

    /// Stop, pressed twice, or pressed after the turn ended on its own. Both
    /// really happen -- the Stop button and `CloseRequested` both call this --
    /// and neither may put a red error on screen for a button that did exactly
    /// what it was asked. An empty session must also be a clean no-op, which is
    /// the state every app starts in.
    #[cfg(unix)]
    #[test]
    fn cancel_running_turn_is_safe_on_an_idle_session_and_when_repeated() {
        let session = Session::default();
        assert!(cancel_running_turn(&session).is_ok(), "an idle session must cancel cleanly");
        assert!(!alive(&session));

        let child = Command::new("sleep").arg("30").spawn().expect("failed to spawn sleep");
        *session.turn.lock().unwrap() =
            Some((1, Box::new(engine::claude_code::ClaudeCodeRun::new(child))));
        assert!(cancel_running_turn(&session).is_ok());
        assert!(cancel_running_turn(&session).is_ok(), "a second Stop must not fail");
    }

    /// The session id is caught off a `system` line and off nothing else.
    ///
    /// **THE TWO WRONG ANSWERS ARE BOTH SILENT.** Missing it means every turn
    /// starts a fresh conversation and the product looks like it has no memory
    /// — the exact thing it is sold on. Taking one off the wrong line means
    /// `--resume` is handed an id Claude Code has never issued, which fails a
    /// whole turn for a reason nobody can see from the window. Neither shows up
    /// as an error anywhere, so it is checked here rather than trusted.
    #[test]
    fn the_session_id_comes_off_a_system_line_and_only_a_system_line() {
        use serde_json::json;
        assert_eq!(
            session_id_of(&json!({"type": "system", "session_id": "abc-123"})),
            Some("abc-123")
        );
        // A `result` line carries a session_id too and is NOT where this comes
        // from -- the pre-seam code keyed on `type == "system"` and this keeps
        // that exactly, because widening it is a change nobody asked for in a
        // refactor that is meant to change nothing.
        assert_eq!(session_id_of(&json!({"type": "result", "session_id": "xyz"})), None);
        assert_eq!(session_id_of(&json!({"type": "assistant"})), None);
        // A system line with no id, and one whose id is not a string, must
        // leave the remembered id alone rather than clearing it to nonsense.
        assert_eq!(session_id_of(&json!({"type": "system"})), None);
        assert_eq!(session_id_of(&json!({"type": "system", "session_id": 7})), None);
        assert_eq!(session_id_of(&json!("not an object")), None);
    }

    /// **THE SEAM IS THE ONLY WAY OUT OF `send`, AND THIS IS THE TEST THAT
    /// FAILS AGAINST THE PRE-SEAM CODE.** Before this refactor `send` built and
    /// spawned a `Command` inline; the point of stage one is that it no longer
    /// can, because a second engine will never produce a child process. A source
    /// scan is the honest check here -- the alternative is trusting that nobody
    /// adds a convenient `Command::new(claude_binary())` back into the send path
    /// the next time something needs one flag more.
    ///
    /// It is scoped to `send` alone, deliberately. `preflight`, `voice_demo`,
    /// `providers::test_binary`, `connectors` and `install` all still spawn the
    /// binary directly and are all correct to -- they are asking questions
    /// *about Claude Code*, not running a turn. Widening this to the whole crate
    /// would fail on all five and teach the next person to delete it.
    #[test]
    fn send_reaches_the_binary_only_through_the_engine() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/main.rs");
        let text = std::fs::read_to_string(&path).expect("main.rs is unreadable");
        let start = text.find("\nfn send(").expect("send() is not where this test thinks it is");
        // The next item at column 0 after `send` -- its closing brace is the
        // only `\n}` that can appear inside a function body at that indent.
        let body_len = text[start + 1..].find("\n}\n").expect("send() has no end");
        let body = &text[start..start + 1 + body_len];

        for (n, line) in body.lines().enumerate() {
            let code = line.split("//").next().unwrap_or("");
            assert!(
                !code.contains("Command::new") && !code.contains("claude_binary()"),
                "send() line {n} spawns the binary directly instead of going through \
                 the engine seam -- a second engine has no child process to spawn: {}",
                line.trim()
            );
        }
        // A scan that returned clean because it scanned nothing is the failure
        // this assertion exists to stop. `send` is ~120 lines.
        assert!(
            body.lines().count() > 50,
            "only scanned {} lines of send() -- the slice is wrong, not the code",
            body.lines().count()
        );
        assert!(
            body.contains("engine::for_provider"),
            "send() no longer selects an engine at all"
        );
    }

    /// **THE REGRESSION TEST FOR MARK'S 2026-09-03 REPORT, AT THE LAYER WHERE
    /// IT ACTUALLY LIVED.** `b2a2325` traced the front end exhaustively and
    /// found `ui/index.html`'s own `speak()` call site innocent -- one call,
    /// one text block, one listener. It never checked the OTHER thing that
    /// receives that listener's payload: `tauri.conf.json` declares a second
    /// window, `companion` (hidden, always-on-top, summoned by a hotkey), and
    /// Tauri creates and starts running a config-declared window's page at
    /// launch regardless of `visible` -- so `companion` loads this exact
    /// `index.html` and registers the identical `listen('claude:event', ...)`
    /// -> `speak()` path `main` does. `AppSink` used to call plain
    /// `self.app.emit(...)`, and Tauri's own docs are explicit that `emit` --
    /// called on ANY handle -- reaches every window in the app; only
    /// `emit_to(label, ...)` is scoped. One assistant turn, one broadcast, two
    /// listening webviews, two calls to `speak()` -- heard twice, and only
    /// ever SHOWN once because `companion` has no transcript UI of its own to
    /// display it in. Reproduced directly with the real `ui/index.html`, both
    /// windows, over CDP (2 before this fix, 1 after) -- this test is the
    /// half of that proof that lives in `cargo test`: it scans `AppSink`'s own
    /// `TurnSink` impl and refuses a plain, unscoped `.emit(` from ever
    /// coming back.
    #[test]
    fn app_sink_never_broadcasts_a_turn_event_to_every_window() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/main.rs");
        let text = std::fs::read_to_string(&path).expect("main.rs is unreadable");
        let start = text
            .find("impl engine::TurnSink for AppSink")
            .expect("AppSink's TurnSink impl is not where this test thinks it is");
        // Same column-0-brace slicing `send_reaches_the_binary_only_through_
        // the_engine` uses above -- the impl block's own closing `\n}\n`.
        let body_len = text[start..].find("\n}\n").expect("AppSink's TurnSink impl has no end");
        let body = &text[start..start + body_len];

        for (n, line) in body.lines().enumerate() {
            let code = line.split("//").next().unwrap_or("");
            // `.emit_to(` contains the substring `.emit(` nowhere -- checking
            // for a bare `.emit(` that is NOT part of `.emit_to(` is exactly
            // "unscoped emit crept back in."
            assert!(
                !(code.contains(".emit(") && !code.contains(".emit_to(")),
                "AppSink line {n} calls a plain, unscoped .emit(...) -- that \
                 broadcasts to every window Tauri has created, including the \
                 hidden 'companion' window, and doubles every spoken reply \
                 exactly as Mark reported on 2026-09-03: {}",
                line.trim()
            );
        }
        // A field, not just method bodies -- the fix depends on AppSink
        // actually carrying which window to target. Losing this field while
        // leaving `emit_to(&self.window_label, ...)` in place would fail to
        // compile, which is a weaker guarantee than this test failing with a
        // readable reason if someone reverts the struct and the calls in the
        // same edit.
        assert!(
            text.contains("window_label: String"),
            "AppSink no longer carries the window label needed to target a \
             single window -- the whole fix depends on this field existing"
        );
        assert!(
            body.lines().count() > 15,
            "only scanned {} lines of AppSink's TurnSink impl -- the slice is \
             wrong, not the code",
            body.lines().count()
        );
    }

    /// THE ACTUAL FAILURE, REPRODUCED, NOT ASSUMED AWAY -- Jarvis, 2026-08-29.
    /// A wedged binary is the whole reason `run_with_timeout` exists, so the
    /// test spawns a real child that genuinely never exits on its own and
    /// checks that the bound is actually enforced, rather than only checking
    /// the happy path and trusting the timeout logic by inspection.
    ///
    /// `#[cfg(unix)]`: `sleep` is not a Windows binary, and this box cannot run
    /// a Windows binary to test with regardless -- every Rust test in this
    /// crate only ever executes natively, on Linux, per the standing rule
    /// about what is verified where. `run_with_timeout` itself has no
    /// platform-specific code in it (plain `std::process`, `std::thread`), so
    /// proving the mechanism here is proving the mechanism; only the real
    /// Windows binary it will actually wrap (`claude.exe`) is Beck's to check.
    #[cfg(unix)]
    #[test]
    fn run_with_timeout_actually_kills_a_wedged_child() {
        let mut cmd = Command::new("sleep");
        cmd.arg("30"); // far longer than the timeout below; must not be waited out
        let started = Instant::now();
        let result = run_with_timeout(cmd, Duration::from_millis(200));
        let elapsed = started.elapsed();
        assert!(result.is_err(), "expected a timeout error, got {result:?}");
        assert_eq!(result.unwrap_err().kind(), std::io::ErrorKind::TimedOut);
        // Generous ceiling (a loaded CI box can be slow), but the whole point
        // is that this returns in a bounded time, not in the 30s the child
        // asked for -- so if this ever takes anywhere near that long, the
        // kill-and-return path is not doing its job.
        assert!(
            elapsed < Duration::from_secs(5),
            "run_with_timeout took {elapsed:?} to return against a 200ms bound \
             -- the deadline is not actually being enforced"
        );
    }

    /// The other half: a child that finishes well inside the deadline must
    /// still return its real output, not an error -- a timeout mechanism that
    /// is trigger-happy on ordinary, fast commands is its own outage.
    #[cfg(unix)]
    #[test]
    fn run_with_timeout_returns_real_output_when_the_child_is_fast() {
        let mut cmd = Command::new("echo");
        cmd.arg("hello");
        let out = run_with_timeout(cmd, Duration::from_secs(10)).expect("echo should not fail");
        assert!(out.status.success());
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "hello");
    }

    /// A OneDrive-redirected Documents is exactly the case this exists for:
    /// the literal join (`home/Documents`) is a real, existing folder on
    /// disk, and it is still the WRONG answer, because it is not the one
    /// OneDrive is syncing. The known-folder answer must win regardless of
    /// what the literal join looks like -- this cannot be exercised by
    /// calling the real Windows API from this box (this is `#[cfg(windows)]`
    /// and the test suite only ever runs natively on Linux, per the standing
    /// rule about what is verified where), so it stands in a fake answer for
    /// what `SHGetKnownFolderPath` would have returned.
    #[test]
    fn known_folder_answer_wins_even_over_an_existing_literal_join() {
        let home = std::path::PathBuf::from("/fake/home");
        let redirected = std::path::PathBuf::from(r"C:\Users\user\OneDrive\Documents");
        let chosen = choose_documents_parent(&home, Some(redirected.clone()), true);
        assert_eq!(chosen, redirected);
    }

    /// No known-folder answer (the Windows call failed, or this isn't
    /// Windows) and a literal `Documents` that exists: the old behaviour,
    /// unchanged. This is the path every non-Windows machine takes today.
    #[test]
    fn falls_back_to_the_literal_join_when_it_exists_and_there_is_no_known_answer() {
        let home = std::path::PathBuf::from("/fake/home");
        let chosen = choose_documents_parent(&home, None, true);
        assert_eq!(chosen, home.join("Documents"));
    }

    /// No known-folder answer and no literal `Documents` either: fall all the
    /// way back to home, same as before this change -- a system that has
    /// deliberately not got a Documents folder does not get one invented for
    /// it.
    #[test]
    fn falls_back_to_home_when_neither_answer_exists() {
        let home = std::path::PathBuf::from("/fake/home");
        let chosen = choose_documents_parent(&home, None, false);
        assert_eq!(chosen, home);
    }

    // -- The unrecognized-model diagnostic must never read as a failure. -----
    //
    // Pinned against the EXACT text Beck captured from the real claude.exe
    // 2.1.251, not a paraphrase — a filter that matches something close to
    // the real string but not the real string is worse than no filter,
    // because it looks fixed and is not.

    #[test]
    fn the_real_notice_line_is_recognised() {
        // b17's own regression: the marker and its JSON detail arrive as ONE
        // stderr write, space-separated, not two lines. Nine runs of the
        // real binary, byte-verified by Beck 2026-08-31 — this is the shape
        // that must match, and the old exact-equality predicate could not.
        assert!(is_unrecognized_model_notice(
            r#"[claude-code:unrecognized_model] {"model":"llama3.2:latest","query_source":"sdk"}"#
        ));
        // The older two-line shape (marker alone, detail on the next line)
        // must keep matching too — a different `claude` build may still use
        // it, and nothing here can confirm which one Mark's own copy is.
        assert!(is_unrecognized_model_notice("[claude-code:unrecognized_model]"));
        // Whitespace a BufReader line can legitimately carry (trailing \r on
        // Windows line endings, in particular) must not defeat the match.
        assert!(is_unrecognized_model_notice("  [claude-code:unrecognized_model]  \r"));
    }

    #[test]
    fn a_real_error_that_merely_mentions_the_words_is_not_the_notice() {
        // The match is a PREFIX, not a substring test — a genuine error that
        // happens to quote this diagnostic in the middle of its own text
        // must still surface, because the line does not START with it.
        assert!(!is_unrecognized_model_notice(
            "something else entirely [claude-code:unrecognized_model] happened"
        ));
        assert!(!is_unrecognized_model_notice("unrecognized_model"));
        assert!(!is_unrecognized_model_notice(""));
    }

    #[test]
    fn the_detail_line_is_recognised_and_nothing_else_shaped_like_it_is_eaten() {
        assert!(is_unrecognized_model_detail(
            r#"{"model":"llama3.2:latest","query_source":"sdk"}"#
        ));
        // A REAL error can also be JSON. This must not be treated as the
        // benign detail just because it starts with a brace.
        assert!(!is_unrecognized_model_detail(r#"{"error":"connection refused"}"#));
        assert!(!is_unrecognized_model_detail("Claude Code exited with code 1."));
        assert!(!is_unrecognized_model_detail(""));
    }
}
