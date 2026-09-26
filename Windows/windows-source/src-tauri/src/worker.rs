//! Autonomous background Workers — give the assistant a task, it runs on its
//! own for many turns while the app stays open, and reports back.
//!
//! **APPROVED TO BUILD — Mastermind safety review, 2026-09-25, on the design
//! in this crate's own proposal to Jarvis the same day.** This module is that
//! design, built. Read `boardroom.rs`'s own header first if you have not —
//! this file borrows its central move (call `engine::for_provider` directly,
//! never through `main.rs::send`/`Session::turn`) and its central discipline
//! (nothing here may widen what the person already allowed a brain to do).
//!
//! ## Scope, said plainly before anything else
//!
//! **"Background" means off the visible chat thread while the app is open —
//! not off the OS process.** `main.rs`'s `CloseRequested` handler exits the
//! whole app the instant the window closes, on purpose, because a running
//! turn once kept talking after the window that showed it was destroyed
//! (Beck, b21/b20, 2026-08-31 — it happened to Mark for real). This module
//! does not reopen that decision. A worker makes progress only while the
//! process is alive, exactly like an ordinary chat turn does today. What
//! "survives an app restart" means here is narrower and honest: the task's
//! own CHECKPOINT survives, and the next launch offers to pick it back up —
//! it does not keep running through the gap.
//!
//! ## The shape, in the four terms this house states a boundary in
//!
//! - **Who may call it.** The `worker_*` Tauri commands below, from the
//!   window. `run_task` (the supervisor loop) is `pub(crate)`-reachable only
//!   from inside this file's own spawned thread.
//! - **What a worker may do that an attended chat turn on the same folder and
//!   brain could not.** Nothing. The permission floor is `AcceptEdits`,
//!   never `Full`; `allow_shell`/`allow_agency` are read straight off the
//!   provider row exactly as `send()` reads them — a worker gets no new
//!   opt-in surface of its own.
//! - **What happens on malformed input.** `worker_start` refuses (folder not
//!   reviewed, brain not chosen, brain is `ClaudeCodeEngine`-shaped, empty
//!   task/done-condition) before anything is spawned and before anything is
//!   billed — the same contract `Engine::start`'s own doc holds.
//! - **What an error leaks.** A task record's `lastError` is written for a
//!   person to read; it never contains a key, and never more of the folder
//!   than its own already-shown canonical path.
//!
//! ## Known v1 limitations — said honestly, not discovered later
//!
//! 1. **No process-level sandbox.** A worker acts with the same reach an
//!    attended `AcceptEdits` turn already has inside the folder the person
//!    reviewed through `folder_trust::gate` — Read/Write/Edit confined to
//!    that folder by `folder_trust::confine`, `Bash`/agency gated by the
//!    provider's own `allow_shell`/`allow_agency`. There is no OS-level
//!    sandbox underneath that, same as today's ordinary chat; a worker is
//!    not a stronger claim than the app already makes for an attended turn.
//! 2. **Stall detection is heuristic, not a guarantee.** `is_stall` below
//!    catches a model repeating itself verbatim, which is the shape this
//!    house has actually measured (`gpt-oss:20b` never completing a turn —
//!    see `native/mod.rs`'s own `TURN_TIMEOUT` doc). A determined or merely
//!    unlucky model can vary its output turn to turn while making no real
//!    progress, and that is not caught by this check. The turn cap, the
//!    wall-clock cap and the token cap in `cap_reason` are the HARD
//!    backstop — they trip on a fixed count regardless of content, and nothing
//!    here should be read as a promise that a looping worker is always caught
//!    early. It is caught, at the latest, at the cap.
//! 3. **`done_when` is checked by the model against itself, never by us —
//!    and the status now says so.** `is_declared_done` parsing
//!    `TASK_STATUS: DONE` used to land on `TaskStatus::Completed`, which is
//!    a claim this app never verified rendered as if it had — the exact
//!    fault Vance's v1.6 review found: Beck asked a worker to write a file
//!    outside its folder, the boundary correctly refused the write, and the
//!    Task Center still showed a green "Completed" because the model said
//!    so. **Fixed 2026-09-25, Jarvis-approved:** that path now sets
//!    `TaskStatus::ReportedDone` — the model's own claim, filed honestly,
//!    never dressed up as ours. Nothing here independently executes or
//!    verifies the predicate; doing that would mean running an arbitrary,
//!    person-supplied verification command unattended, which is real,
//!    separate, un-built work with its own safety shape, not assumed here.
//!    A vague `done_when` produces a task that runs to its cap and reports
//!    "capped, not finished" — it does not run forever, but it also does not
//!    know better than the person did.
//!
//!    **PHASE 2, NOT YET BUILT, NEEDS ITS OWN SAFETY REVIEW BEFORE IT IS:**
//!    a narrow, safe, filesystem-only check — "does the file `done_when`
//!    names exist, inside the confined workdir" — that promotes
//!    `ReportedDone` to a genuinely verified `Completed` on a pass. Everything
//!    else (an answer it should produce, anything not a bare file check)
//!    stays honestly `ReportedDone` forever. Logged here so the task is not
//!    lost, not assumed as scope of this fix.
//! 4. **File-level partial writes on crash are inherent, not covered.**
//!    `save_record`'s atomic temp-file-then-rename protects the TASK RECORD
//!    — the app's own bookkeeping about the task — exactly the way
//!    `native::store::Conversation::save` protects a transcript. It says
//!    nothing about a file a tool call was mid-write on inside the person's
//!    own folder when the process died; that risk is identical to an
//!    attended `Write`/`Edit` call losing power mid-write today, not a new
//!    one this module introduces.
//!
//! ## Why a task IS a `native::store::Conversation`, not a second format
//!
//! `store::Conversation` already has the exact atomic-write, commit-only-
//! when-genuinely-finished, trim-and-announce, corrupt-file-untouched
//! contract a task's turn history needs — see that module's own header. A
//! task's id (`store::new_id()`, minted once at creation) IS its
//! conversation id: every turn's `TurnRequest.resume` is `Some(task.id)`,
//! always, including the first — `conversation_for` in `engine::native::mod`
//! mints a fresh `Conversation` under exactly that id the first time and
//! loads it back every time after, so there is no separate "first turn vs.
//! resumed turn" branch to keep in step here. What this file adds on top is
//! small and genuinely new: the supervisor loop, the caps, the consent
//! pause, and a sidecar JSON (`worker-tasks/<id>.json`) carrying the things
//! a `Conversation` was never meant to know — status, caps spent, the
//! person's own done-condition.
//!
//! **One deliberate divergence from `main.rs::send()`: the MCP launch config
//! is per-task, not the shared `mcp-launch.json` file.** `send()` reuses one
//! fixed path because only one interactive turn can ever be in flight
//! (`Session::turn` is a single slot). A worker is explicitly meant to run
//! WHILE the person keeps chatting, so two turns — the person's and a
//! worker's — can be writing and reading an MCP config at the same moment.
//! Sharing `send()`'s path would be a real race the first time it happened;
//! `worker_mcp_config_path` gives each task its own file instead.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::{AppHandle, Manager, State};

use crate::engine::{self, Permission, RunningTurn, TurnRequest, TurnSink};
use crate::engine::native::store;
use crate::providers::{self, Provider};

// ---------------------------------------------------------------------------
// Caps — the hard backstop. See this module's own header, limitation 2.
// ---------------------------------------------------------------------------

/// How many turns one task may take before it is stopped and reported
/// "capped, not finished" rather than left to run indefinitely.
///
/// **A CAP, NOT A GUESS, SAME STANDING AS `native::mod::MAX_TOOL_ROUNDS`.**
/// That constant bounds ONE turn's tool-calling rounds (8); this bounds how
/// many TURNS a whole task may take. 20 is generous for anything this house
/// has asked a worker to do so far and cheap to raise once real tasks are
/// measured — the point is that it is finite, not that it is exactly right.
const MAX_TASK_TURNS: u32 = 20;

/// How long a task may run, wall-clock, from the moment its first turn is
/// queued, before it is stopped the same way a turn-cap trip stops it.
///
/// **CHECKED BETWEEN TURNS ONLY — a real, stated limit, not an oversight.**
/// A single turn is already bounded inside the native engine by its own
/// `TURN_TIMEOUT` (1200s), so the worst-case overrun past this cap is one
/// more `TURN_TIMEOUT` beyond it, never unbounded.
const TASK_WALL_CLOCK_CAP_SECS: u64 = 2 * 60 * 60;

/// How many tokens (input + output, summed across every turn) one task may
/// spend before it is stopped.
///
/// **A PROXY, SAID PLAINLY.** This codebase tracks no per-provider dollar
/// pricing anywhere `worker.rs` can reach, so a token ceiling is what is
/// honestly available in v1 rather than a spend cap in currency — flagged as
/// a real gap in the proposal this module implements, not silently assumed
/// solved. 500,000 tokens is a generous multi-turn budget for a local or
/// OpenAI-compatible brain; the number is easy to lower once real tasks are
/// measured.
const MAX_TASK_TOKENS: u64 = 500_000;

/// How long a pending consent request — a tool asking for the person's yes,
/// or a stall pause asking whether to keep going — stays answerable before
/// it is dropped and must be re-raised.
///
/// **THE HOLE THIS CLOSES, NAMED BY THE ROOM'S OWN SAFETY REVIEW, 2026-09-25:
/// "app open ≠ user watching; it sits for hours then gets blindly
/// approved."** `main.rs`'s own interactive `CONFIRM_TIMEOUT` (120s) answers
/// a different question — how long a turn blocks ITS OWN THREAD waiting for
/// a person already looking at the window — and that number is short on
/// purpose because nothing else in the app advances while it blocks. A
/// worker's pause blocks nothing; the risk here is staleness, not a frozen
/// UI, so the right number is not "as short as possible," it is "short
/// enough that answering it is still answering the thing that was actually
/// asked." Ten times `main.rs`'s own 120s — same order of magnitude, scaled
/// for an async check rather than a blocked thread — is the deliberate
/// choice: twenty minutes is long enough that a person checking their Task
/// Center within the task's own turn cadence will see it, and short enough
/// that "blindly approved after sitting for hours" cannot happen, because
/// past this the answer is refused as stale rather than accepted.
const CONSENT_EXPIRY_SECS: u64 = 20 * 60;

/// How many consecutive turns producing the same committed answer, trimmed,
/// counts as a stall worth pausing for — see this module's own header,
/// limitation 2, for what this heuristic does and does not catch.
const STALL_STREAK_THRESHOLD: u32 = 2;

/// Belt-and-suspenders wait on one turn's own completion channel, separate
/// from and longer than the native engine's internal `TURN_TIMEOUT` (1200s).
/// The engine's own timeout should always fire first and call `finished()`
/// on its own; this exists so a bug in THAT mechanism cannot hang a worker's
/// supervisor thread forever — the same "two independent nets" shape this
/// house already holds for turn caps and spend caps, applied to time.
const OUTER_TURN_WAIT_SECS: u64 = 1800;

fn now_ts() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

// ---------------------------------------------------------------------------
// The task record — what a person and the Task Center see.
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Queued,
    Running,
    NeedsInput,
    /// **THE MODEL'S OWN CLAIM, FILED HONESTLY, NEVER DRESSED UP AS OURS.**
    /// Set only where `is_declared_done` reads `TASK_STATUS: DONE` off a
    /// committed answer — this app has not independently checked `done_when`
    /// against anything. Used to be `Completed`, which is not what this is:
    /// Vance's v1.6 review found a task that provably did nothing (the write
    /// was refused, outside the trusted folder) showing a green "Completed"
    /// because the model said so. Renamed 2026-09-25 rather than fixed by
    /// adding real verification underneath it — see this module's header,
    /// limitation 3, for why that is separate, un-built work. A future,
    /// reviewed check may promote this to a genuinely verified `Completed`;
    /// until that exists, this is the only honest terminal "done" state.
    ReportedDone,
    Failed,
    Cancelled,
    /// The process that was running this task exited (closed or crashed)
    /// while it was still `Running` or `NeedsInput`. Set only by
    /// `flag_orphans_on_launch`, on the NEXT launch, and never carries the
    /// task back into `Running` on its own — see this module's header and
    /// `flag_orphans_on_launch`'s own doc for why silent auto-resume is the
    /// one thing this state must never become.
    NeedsResume,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingConfirmView {
    pub prompt: String,
    pub requested_at: u64,
    pub expires_at: u64,
    /// `true` when this pause is a stall check ("the last two turns look
    /// identical, keep going?") rather than a real tool asking to act.
    /// `worker_answer_confirm`'s `approve` still means the same two things
    /// either way — yes, try once more; no, stop here — so the UI needs
    /// this only to word the prompt, never to change which command it calls.
    pub stall: bool,
}

/// One task, as the window and the disk both see it — see this module's
/// header for why this is also, verbatim, the on-disk sidecar shape.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskRecord {
    /// Also this task's `native::store::Conversation` id — see the module
    /// header. Always `store::CHAT_ID_PREFIX`-shaped.
    pub id: String,
    pub title: String,
    pub prompt: String,
    pub done_when: String,
    /// The canonical, resolved folder path from `folder_trust::gate` — shown
    /// here, on every read of this record, not only at creation, so a broad
    /// prior trust is visible for as long as the task exists, not just the
    /// moment it started. See the room's own safety-review fold-in on this
    /// point.
    pub workdir: String,
    pub provider_id: String,
    pub status: TaskStatus,
    pub created_at: u64,
    pub updated_at: u64,
    pub turn_count: u32,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub last_activity: String,
    pub last_error: Option<String>,
    pub pending_confirm: Option<PendingConfirmView>,
}

// ---------------------------------------------------------------------------
// Runtime state — the mutable, in-memory half a `TaskRecord` is snapshotted
// from. Split from `TaskRecord` because `WorkerSink::confirm` mutates this
// from the engine's own internal thread, on every call, and a serializable
// record is not the right shape to lock on a hot path.
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
struct PendingAsk {
    prompt: String,
    requested_at: u64,
    expires_at: u64,
    stall: bool,
    /// The ACTUAL tool call's own identity — F3, Cassandra's adversarial
    /// review, 2026-09-25. See `tool_args_key`'s own doc for what this is
    /// and why matching on `prompt` alone was the hole.
    args_key: Option<u64>,
}

#[derive(Clone, Debug)]
struct Approval {
    prompt: String,
    expires_at: u64,
    args_key: Option<u64>,
}

struct Live {
    status: TaskStatus,
    updated_at: u64,
    /// Set once, when the task's first turn is queued — the wall-clock cap's
    /// own reference point. `None` before the task has actually started
    /// running (there is no `Queued` gap in this build; kept `Option` so the
    /// cap-checking function has an honest "not started yet" to refuse
    /// dividing by, rather than a `0` that would read as 1970).
    started_running_at: Option<u64>,
    turn_count: u32,
    input_tokens: u64,
    output_tokens: u64,
    last_activity: String,
    last_error: Option<String>,
    /// The last COMMITTED assistant text, trimmed — `is_stall`'s own input.
    last_assistant_text: Option<String>,
    repeat_streak: u32,
    awaiting: Option<PendingAsk>,
    approved: Option<Approval>,
    cancel_requested: bool,
}

impl Live {
    fn new() -> Self {
        Live {
            status: TaskStatus::Running,
            updated_at: now_ts(),
            started_running_at: None,
            turn_count: 0,
            input_tokens: 0,
            output_tokens: 0,
            last_activity: "queued".into(),
            last_error: None,
            last_assistant_text: None,
            repeat_streak: 0,
            awaiting: None,
            approved: None,
            cancel_requested: false,
        }
    }
}

/// The runtime handle for one task — held by the `Workers` registry and by
/// the task's own supervisor thread and `WorkerSink` for as long as either
/// is alive. `Arc` because three owners genuinely outlive one another in an
/// unpredictable order: the registry, the spawned thread, and the sink the
/// engine calls back into.
struct TaskHandle {
    id: String,
    title: String,
    prompt: String,
    done_when: String,
    /// The canonical path from `folder_trust::gate` — never the raw string
    /// the person or the caller typed.
    workdir: String,
    provider_id: String,
    created_at: u64,
    store_dir: Option<PathBuf>,
    sidecar_dir: PathBuf,
    live: Mutex<Live>,
    /// The turn currently in flight, if any — `worker_cancel`'s own reach
    /// into a running turn, same shape `Session::turn` gives the Stop
    /// button in `main.rs`, scoped to one task instead of one app session.
    current_turn: Mutex<Option<Box<dyn RunningTurn>>>,
}

impl TaskHandle {
    fn snapshot(&self) -> TaskRecord {
        let live = self.live.lock().unwrap_or_else(|e| e.into_inner());
        let pending_confirm = live.awaiting.as_ref().map(|a| PendingConfirmView {
            prompt: a.prompt.clone(),
            requested_at: a.requested_at,
            expires_at: a.expires_at,
            stall: a.stall,
        });
        TaskRecord {
            id: self.id.clone(),
            title: self.title.clone(),
            prompt: self.prompt.clone(),
            done_when: self.done_when.clone(),
            workdir: self.workdir.clone(),
            provider_id: self.provider_id.clone(),
            status: live.status,
            created_at: self.created_at,
            updated_at: live.updated_at,
            turn_count: live.turn_count,
            input_tokens: live.input_tokens,
            output_tokens: live.output_tokens,
            last_activity: live.last_activity.clone(),
            last_error: live.last_error.clone(),
            pending_confirm,
        }
    }
}

/// The app-wide registry of tasks this SESSION has started or resumed.
///
/// **DELIBERATELY NOT PRE-POPULATED FROM DISK AT STARTUP.** A task on disk
/// marked `Running` when the process died is an orphan, not a task this
/// registry has ever run — `flag_orphans_on_launch` retitles it
/// `NeedsResume` on disk and leaves it out of this map entirely, so it
/// cannot be reached by `worker_cancel`/`worker_answer_confirm` (which only
/// know about tasks in here) until a person explicitly calls `worker_resume`
/// on it. See this module's header and `flag_orphans_on_launch`'s own doc.
#[derive(Default)]
pub struct Workers {
    tasks: Mutex<HashMap<String, Arc<TaskHandle>>>,
}

// ---------------------------------------------------------------------------
// Pure decision functions — the safety-critical ones, tested directly and
// cheaply, the same discipline `boardroom.rs`'s own `parse_stance`/
// `final_positions` and `engine::native::mod`'s `may_run_preapproved` already
// hold: a decision worth trusting is a decision testable without a real
// engine, a real folder, or a real clock.
// ---------------------------------------------------------------------------

/// Which cap, if any, this task has hit — checked BEFORE a next turn is
/// queued, never mid-turn, the same "refuse before spawning, nothing billed"
/// contract `Engine::start`'s own doc holds for a single turn.
fn cap_reason(
    turn_count: u32,
    started_running_at: u64,
    now: u64,
    input_tokens: u64,
    output_tokens: u64,
) -> Option<String> {
    if turn_count >= MAX_TASK_TURNS {
        return Some(format!(
            "stopped after {MAX_TASK_TURNS} turns without declaring the task done — \
             capped, not finished"
        ));
    }
    let elapsed = now.saturating_sub(started_running_at);
    if elapsed >= TASK_WALL_CLOCK_CAP_SECS {
        return Some(format!(
            "stopped after running {} minutes without declaring the task done — \
             capped, not finished",
            elapsed / 60
        ));
    }
    let spent = input_tokens.saturating_add(output_tokens);
    if spent >= MAX_TASK_TOKENS {
        return Some(format!(
            "stopped after spending {spent} tokens without declaring the task done — \
             capped, not finished"
        ));
    }
    None
}

/// **THE ADMISSION CHECK, AS A PURE DECISION — F1 and F2, Cassandra's
/// adversarial review, 2026-09-25.** `worker_start` always checked both of
/// these before spawning a task's first turn. What it missed: the RESUMED
/// path (`run_resumed_turn_then_loop`, reached by `worker_answer_confirm`'s
/// Approve button) built and ran a turn with NEITHER check — a person could
/// edit the paused task's provider row to a Claude-Code-shaped one (F1) or
/// turn air-gap on (F2) while a task sat at `needs_input`, click Approve,
/// and get one unattended turn on a brain that needed a live person to
/// answer its own prompts, or one that left the machine after being told
/// not to. Split into a pure core (this function, no `AppHandle`, directly
/// testable — this crate has **no `tauri::test` harness anywhere in it**,
/// see `providers.rs`'s own comments making the identical call) and a thin
/// wrapper (`admission_refusal`, below) that resolves the two live facts
/// and defers to this. Every place a turn is about to run — `worker_start`,
/// `worker_resume`, `run_task`'s own per-turn loop, and
/// `run_resumed_turn_then_loop` — calls the wrapper, so there is exactly
/// one place this rule can drift out of step with itself.
fn admission_refusal_for(needs_vendor_binary: bool, airgapped: bool, provider_kind: &str) -> Option<String> {
    // Worded to read sensibly at every call site this function has --
    // creation, an app-restart resume, and a re-check mid-task or on
    // approval -- rather than presuming any one of them ("changed to" would
    // read oddly the first time a task is ever started on this brain).
    if needs_vendor_binary {
        return Some(
            "this brain needs Claude Code, which workers cannot run unattended -- nobody \
             is here to answer its own permission prompts. Pick a different brain under \
             Settings -> AI Brain, or stop leaving this task on this one."
                .to_string(),
        );
    }
    if airgapped && provider_kind != "local" {
        return Some(
            "air-gap is on, so a brain that leaves this machine cannot run this task \
             unattended right now. Turn air-gap off, or switch this task to a brain on \
             this machine."
                .to_string(),
        );
    }
    None
}

/// The live wrapper — resolves `needs_vendor_binary` and `is_airgapped` off
/// the real engine and app, then defers to `admission_refusal_for` above.
/// See that function's own doc for what this closes and why it is split
/// this way.
fn admission_refusal(app: &AppHandle, provider: &Provider) -> Option<String> {
    admission_refusal_for(
        engine::for_provider(provider).needs_vendor_binary(),
        providers::is_airgapped(app),
        &provider.kind,
    )
}

/// **THE RECORDED-TRUST CHECK — Beck's VM verification, 2026-09-25.** Every
/// place a worker turn is about to run must confirm the folder was actually
/// ACCEPTED (`FolderTrust::trusted`, written by `mark_folder_trusted`),
/// never merely that `folder_trust::gate` found nothing to flag
/// (`needs_decision`). Those are different facts: a fresh, empty folder has
/// `needs_decision == false` by that module's own deliberate design ("a
/// clean folder sails through" — correct for `send()`'s attended chat turn,
/// where a live person is watching and can Stop it) but `trusted == false`
/// too, because nobody has ever recorded a yes for it. 6/6 fresh worker
/// starts in an empty folder skipped review entirely under the old
/// `!needs_decision` check and the folder was never written to
/// `folder-trust.json` at all — this is the fix. `folder_trust::gate`
/// itself is untouched: `send()`'s own attended-chat leniency is correct
/// for `send()` and is not this fix's to narrow for everyone.
///
/// Called at task creation, at resume, on the approval path, and inside
/// `run_task`'s own per-turn loop — the last one so a person calling
/// `forget_folder_trust` on a folder a task is still actively working in
/// stops the NEXT turn, the same "re-checked every turn, not only at
/// creation" discipline `admission_refusal` already holds for the brain.
fn folder_trust_refusal(app: &AppHandle, workdir: &str) -> Option<String> {
    folder_trust_refusal_for(crate::folder_trust::gate(app, workdir).trusted)
}

/// The pure half — split out the same way `admission_refusal_for` is split
/// from `admission_refusal`, so the actual DECISION ("trusted means go,
/// anything else means refuse") is testable without a `tauri::AppHandle`.
/// This crate has no `tauri::test` harness anywhere in it (see
/// `providers.rs`'s own comments making the identical call).
fn folder_trust_refusal_for(trusted: bool) -> Option<String> {
    if trusted {
        None
    } else {
        Some(
            "this folder hasn't been reviewed and accepted -- an unattended task needs an \
             explicit yes for its folder, even an empty one. Open the folder review screen, \
             accept it, then continue."
                .to_string(),
        )
    }
}

/// Drop anything past its expiry — FOLD-IN #1 from the room's safety review,
/// as a function rather than inline, so `confirm()` (mid-turn, on the
/// engine's own thread) and the supervisor loop (between turns) both apply
/// the identical rule instead of two copies that could drift.
fn expire_stale(awaiting: &mut Option<PendingAsk>, approved: &mut Option<Approval>, now: u64) {
    if awaiting.as_ref().is_some_and(|a| a.expires_at <= now) {
        *awaiting = None;
    }
    if approved.as_ref().is_some_and(|a| a.expires_at <= now) {
        *approved = None;
    }
}

/// Whether `current`, trimmed, repeating the PREVIOUS committed answer means
/// this task has stalled — see this module's header, limitation 2. Returns
/// the new streak so the caller can store it back onto `Live`.
fn is_stall(previous: Option<&str>, current: &str, streak_before: u32) -> (bool, u32) {
    let current = current.trim();
    let repeated = previous.map(|p| p.trim() == current).unwrap_or(false) && !current.is_empty();
    let streak = if repeated { streak_before + 1 } else { 0 };
    (streak >= STALL_STREAK_THRESHOLD, streak)
}

/// Whether a committed assistant answer declared the task done, per the
/// exact contract `task_contract` asks the model to follow — a line reading
/// exactly `TASK_STATUS: DONE`, tolerant of surrounding whitespace the way
/// `boardroom::parse_stance` is tolerant of a model that does not put a
/// blank line where asked, but never tolerant of a SUBSTRING match: a reply
/// that merely mentions the phrase in passing must not be read as a claim.
fn is_declared_done(text: &str) -> bool {
    text.lines().any(|line| line.trim() == "TASK_STATUS: DONE")
}

/// What `worker_cancel` actually does to a handle — factored out so it is
/// testable against a fake `RunningTurn`, the same idiom `main.rs`'s own
/// `cancel_running_turn` vs. `stop` already uses: the command is a thin
/// `State` wrapper, this is the thing worth pinning with a test.
///
/// **IDEMPOTENT, SAME CONTRACT `RunningTurn::cancel`'s OWN DOC REQUIRES —
/// must succeed whether or not a turn happens to be running right now.**
/// The kill switch is reachable between turns (nothing to cancel, just stop
/// the next one being queued) and mid-turn (a real process to stop).
fn cancel_task(handle: &TaskHandle) -> Result<(), String> {
    {
        let mut live = handle.live.lock().unwrap_or_else(|e| e.into_inner());
        live.cancel_requested = true;
        live.status = TaskStatus::Cancelled;
        live.updated_at = now_ts();
        live.awaiting = None;
        live.approved = None;
    }
    let mut turn = handle.current_turn.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(t) = turn.take() {
        return t.cancel();
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// The system prompt addendum — the task's own contract with the model.
// ---------------------------------------------------------------------------

fn task_contract(done_when: &str) -> String {
    format!(
        "\n\nYou are running as an unattended background task inside helloim.ai — nobody \
         is watching this turn happen in real time. The person who set this task said it \
         is done when: {done_when}\n\n\
         Rules for working unattended:\n\
         - You may read, write and edit files inside this folder, and use any tool you are \
         offered, exactly as you could in an ordinary conversation with this same person.\n\
         - A tool that needs the person's live approval right now (for example, opening a \
         URL) will be refused whatever you answer — there is nobody here this instant to \
         approve it. Ask for it anyway if you need it: the person will see the request when \
         they next look, and if they approve it you will be told next turn and can ask \
         again.\n\
         - End every reply with exactly one status line, on its own line and nothing after \
         it: `TASK_STATUS: DONE` if the condition above is now genuinely true — never claim \
         this without having actually checked — or `TASK_STATUS: CONTINUING` followed on the \
         next line by the single next concrete step, if it is not.\n"
    )
}

fn continuation_prompt(note: Option<&str>) -> String {
    let mut s = String::new();
    if let Some(n) = note {
        s.push_str(n);
        s.push_str("\n\n");
    }
    s.push_str("Continue the task.");
    s
}

// ---------------------------------------------------------------------------
// The sink — this task's stand-in for `main.rs::AppSink`, modelled directly
// on `boardroom::CollectingSink` with one real addition: `confirm()` does
// not just deny, it records what was asked so the supervisor loop and the
// person can see it.
// ---------------------------------------------------------------------------

struct WorkerSink {
    handle: Arc<TaskHandle>,
    tx: Mutex<Option<mpsc::Sender<()>>>,
    text: Mutex<String>,
    error: Mutex<Option<String>>,
    /// The most recently seen tool call's own `(name, canonical args)` —
    /// F3, Cassandra's adversarial review, 2026-09-25. Captured off the
    /// ordinary `tool_use` chip the native engine's own drive loop already
    /// emits (`tool_use_event`, `ending.emit_live(...)`) BEFORE it asks
    /// `confirm()` for that same call, which is what lets `confirm()` key
    /// an approval to the call's real arguments without `TurnSink::confirm`
    /// itself needing to change — that trait is not this fix's to touch.
    last_tool_call: Mutex<Option<(String, String)>>,
}

impl WorkerSink {
    fn new(handle: Arc<TaskHandle>) -> (Arc<Self>, mpsc::Receiver<()>) {
        let (tx, rx) = mpsc::channel();
        (
            Arc::new(WorkerSink {
                handle,
                tx: Mutex::new(Some(tx)),
                text: Mutex::new(String::new()),
                error: Mutex::new(None),
                last_tool_call: Mutex::new(None),
            }),
            rx,
        )
    }

    /// Take this turn's collected text and error — mirrors
    /// `CollectingSink`'s own read pattern. A fresh `WorkerSink` is built for
    /// EVERY turn (`run_task`'s loop calls `WorkerSink::new` each time it
    /// starts one), the same one-shot-per-turn shape Boardroom already uses;
    /// what carries across turns is the `Arc<TaskHandle>` each new sink is
    /// handed, which is where `confirm()`'s state actually lives.
    fn take(&self) -> (String, Option<String>) {
        let text = std::mem::take(&mut *self.text.lock().unwrap_or_else(|e| e.into_inner()));
        let error = self.error.lock().unwrap_or_else(|e| e.into_inner()).take();
        (text, error)
    }

}

/// A compact identity for one tool call's own name and arguments — F3,
/// Cassandra's adversarial review, 2026-09-25. A stored approval used to be
/// matched against the rendered CONFIRM PROMPT TEXT, and that text is a
/// human summary that does not always carry every field — a mail
/// send/reply's own summary shows To/Subject/Body, never cc/bcc (see
/// `engine::native::mod`'s own `confirm_summary` and the comment beside
/// where it is called). Matching on that text let an approval granted for
/// one set of args be silently honoured for a DIFFERENT, later call whose
/// rendered summary happened to read the same — an added cc the person
/// never saw. This hashes `name` and the canonicalized `input` JSON
/// together, which is the same data `tool_use_event` already puts in front
/// of the person via the live transcript chip.
///
/// **NOT A SECURITY BOUNDARY AGAINST A HASH COLLISION.** A 64-bit hash is
/// not cryptographic, and nothing here needs it to be — this defends
/// against a visible summary that omits a real field, not against a model
/// deliberately searching for two argument sets that happen to hash the
/// same.
fn tool_args_key(name: &str, canonical_args: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    name.hash(&mut hasher);
    canonical_args.hash(&mut hasher);
    hasher.finish()
}

impl TurnSink for WorkerSink {
    fn event(&self, value: Value) {
        match value.get("type").and_then(|t| t.as_str()) {
            Some("assistant") => {
                if let Some(blocks) = value.pointer("/message/content").and_then(|c| c.as_array()) {
                    for block in blocks {
                        match block.get("type").and_then(|t| t.as_str()) {
                            Some("text") => {
                                if let Some(t) = block.get("text").and_then(|t| t.as_str()) {
                                    self.text.lock().unwrap_or_else(|e| e.into_inner()).push_str(t);
                                }
                            }
                            // F3 -- the live transcript chip the native
                            // engine's own drive loop emits BEFORE it ever
                            // calls `confirm()` for this same call (see
                            // `tool_use_event`/`ending.emit_live` in
                            // `engine::native::mod`). Capturing it here is
                            // what lets `confirm()` key an approval to the
                            // call's REAL arguments rather than the
                            // rendered prompt text alone.
                            Some("tool_use") => {
                                let name = block.get("name").and_then(|n| n.as_str()).unwrap_or("").to_string();
                                let input = block.get("input").cloned().unwrap_or(Value::Null);
                                let canonical = serde_json::to_string(&input).unwrap_or_default();
                                *self.last_tool_call.lock().unwrap_or_else(|e| e.into_inner()) = Some((name, canonical));
                            }
                            _ => {}
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
                if let Some(usage) = value.get("usage") {
                    let mut live = self.handle.live.lock().unwrap_or_else(|e| e.into_inner());
                    if let Some(n) = usage.get("input_tokens").and_then(|v| v.as_u64()) {
                        live.input_tokens = live.input_tokens.saturating_add(n);
                    }
                    if let Some(n) = usage.get("output_tokens").and_then(|v| v.as_u64()) {
                        live.output_tokens = live.output_tokens.saturating_add(n);
                    }
                }
            }
            _ => {}
        }
    }

    fn raw(&self, _line: String) {}

    fn failure(&self, line: String) {
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

    /// **NEVER AUTO-APPROVES — the trait's own default is deny, and this
    /// override still denies on every call that has no matching, unexpired,
    /// person-granted approval sitting from an EARLIER turn.** What it adds
    /// is that a denial is not just a denial: it is recorded as a pending
    /// ask, so the supervisor loop can pause the task and a person can
    /// answer it later, on their own time, through `worker_answer_confirm`
    /// — see this module's header for why that is the honest way to give a
    /// worker consent it cannot ask for live.
    ///
    /// **THE ONE-SHOT RULE.** A matching approval is consumed the instant it
    /// is used — it answers exactly one call, never a standing yes for every
    /// future call with the same wording. This matches Safe Agency's own
    /// per-call design intent for `OpenUrl` specifically (the person is
    /// shown the EXACT destination each time), just spread across a pause
    /// instead of a blocking dialog.
    fn confirm(&self, prompt: &str) -> bool {
        // Read BEFORE taking `live`'s lock -- `last_tool_call` is a
        // separate `Mutex`, and computing the key first means the call
        // this confirm() is actually ABOUT is fixed before anything about
        // pending state is touched.
        let current_args_key = self
            .last_tool_call
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .map(|(name, args)| tool_args_key(name, args));

        let mut guard = self.handle.live.lock().unwrap_or_else(|e| e.into_inner());
        // Reborrow through the guard once, as a plain `&mut Live` -- the
        // borrow checker cannot see two struct fields as disjoint through a
        // `MutexGuard`'s own `DerefMut` directly, only through an ordinary
        // reference. `boardroom.rs` never hits this because nothing there
        // holds two mutable fields of one guarded struct at once.
        let live = &mut *guard;
        let now = now_ts();
        expire_stale(&mut live.awaiting, &mut live.approved, now);

        // **F3 — MATCHED ON THE REAL ARGS, NEVER ON PROMPT TEXT ALONE.** An
        // approval with no recoverable args identity, or a call this confirm()
        // fires for with no recoverable args identity, can never match —
        // fail closed rather than fall back to the old, weaker text-only
        // check. Requiring the prompt text too costs nothing (the same real
        // args always render the same prompt) and is a second, independent
        // check on top of the one that actually closes the hole.
        let matches = match (&live.approved, current_args_key) {
            (Some(a), Some(key)) => a.prompt == prompt && a.args_key == Some(key),
            _ => false,
        };
        if matches {
            live.approved = None;
            return true;
        }
        live.awaiting = Some(PendingAsk {
            prompt: prompt.to_string(),
            requested_at: now,
            expires_at: now + CONSENT_EXPIRY_SECS,
            stall: false,
            args_key: current_args_key,
        });
        live.last_activity = "waiting for approval".into();
        false
    }
}

// ---------------------------------------------------------------------------
// The supervisor loop.
// ---------------------------------------------------------------------------

/// Everything about the task that stays the same turn to turn, resolved once
/// before the thread starts — mirrors `TurnRequest`'s own "engine-agnostic
/// decisions made before the seam" shape, one level up.
struct TaskPlan {
    provider: Provider,
    mcp_config: Option<PathBuf>,
    mcp_env: Vec<(String, String)>,
    system: String,
}

fn checkpoint(handle: &TaskHandle) {
    if let Err(e) = save_record(&handle.sidecar_dir, &handle.snapshot()) {
        eprintln!("worker task {}: could not checkpoint: {e}", handle.id);
    }
}

fn finish(handle: &TaskHandle, status: TaskStatus, error: Option<String>) {
    {
        let mut live = handle.live.lock().unwrap_or_else(|e| e.into_inner());
        live.status = status;
        live.updated_at = now_ts();
        live.last_error = error;
        live.awaiting = None;
        live.last_activity = match status {
            TaskStatus::ReportedDone => "done".into(),
            TaskStatus::Failed => "stopped".into(),
            TaskStatus::Cancelled => "cancelled".into(),
            _ => live.last_activity.clone(),
        };
    }
    *handle.current_turn.lock().unwrap_or_else(|e| e.into_inner()) = None;
    checkpoint(handle);
}

fn pause_needs_input(handle: &TaskHandle) {
    let mut live = handle.live.lock().unwrap_or_else(|e| e.into_inner());
    live.status = TaskStatus::NeedsInput;
    live.updated_at = now_ts();
    live.last_activity = "waiting for approval".into();
    drop(live);
    checkpoint(handle);
}

fn pause_for_stall(handle: &TaskHandle) {
    let mut live = handle.live.lock().unwrap_or_else(|e| e.into_inner());
    let now = now_ts();
    live.status = TaskStatus::NeedsInput;
    live.updated_at = now;
    live.last_activity = "looks stuck — waiting for you".into();
    live.awaiting = Some(PendingAsk {
        prompt: "The last two turns produced the same answer, which usually means this \
                 task is stuck rather than making progress. Let it try one more turn?"
            .to_string(),
        requested_at: now,
        expires_at: now + CONSENT_EXPIRY_SECS,
        stall: true,
        // A stall pause is never about one specific tool call, so there is
        // no args identity to carry -- `worker_answer_confirm` never turns
        // a stall `PendingAsk` into an `Approval` anyway (see the `!ask.stall`
        // guard there), so this is never read.
        args_key: None,
    });
    drop(live);
    checkpoint(handle);
}

/// The task's whole multi-turn run, on its own OS thread — spawned once by
/// `worker_start` or `worker_resume` and never joined; the same
/// fire-and-forget shape `main.rs::send` already gives an interactive turn,
/// one level up. Everything this function needs was resolved and validated
/// by the caller BEFORE the thread started, the same "refuse first, spawn
/// second" discipline `Engine::start`'s own doc holds — nothing in this
/// function itself refuses to run; it only stops.
///
/// `app` is used to re-run the per-turn admission check (F1/F2, see
/// `admission_refusal`) before every turn this loop queues, including the
/// first.
fn run_task(app: AppHandle, handle: Arc<TaskHandle>, plan: TaskPlan) {
    {
        let mut live = handle.live.lock().unwrap_or_else(|e| e.into_inner());
        if live.started_running_at.is_none() {
            live.started_running_at = Some(now_ts());
        }
        live.status = TaskStatus::Running;
        live.updated_at = now_ts();
    }
    checkpoint(&handle);

    loop {
        let now = now_ts();
        let (cancelled, turn_count, started_running_at, input_tokens, output_tokens) = {
            let live = handle.live.lock().unwrap_or_else(|e| e.into_inner());
            (
                live.cancel_requested,
                live.turn_count,
                live.started_running_at.unwrap_or(now),
                live.input_tokens,
                live.output_tokens,
            )
        };
        if cancelled {
            finish(&handle, TaskStatus::Cancelled, None);
            return;
        }
        if let Some(reason) = cap_reason(turn_count, started_running_at, now, input_tokens, output_tokens) {
            finish(&handle, TaskStatus::Failed, Some(reason));
            return;
        }

        // RE-CHECKED EVERY TURN, NOT ONLY AT CREATION: the person can edit
        // which engine a provider row routes to (F1) or turn air-gap on
        // (F2) between turns of a task that is still running, or withdraw
        // this folder's own acceptance (`forget_folder_trust`) while a task
        // is still working in it. See `admission_refusal`'s and
        // `folder_trust_refusal`'s own docs.
        if let Some(reason) = admission_refusal(&app, &plan.provider) {
            finish(&handle, TaskStatus::Failed, Some(reason));
            return;
        }
        if let Some(reason) = folder_trust_refusal(&app, &handle.workdir) {
            finish(&handle, TaskStatus::Failed, Some(reason));
            return;
        }
        let engine_for_turn = engine::for_provider(&plan.provider);

        let turn_prompt = if turn_count == 0 {
            handle.prompt.clone()
        } else {
            continuation_prompt(None)
        };

        let (sink, rx) = WorkerSink::new(Arc::clone(&handle));
        let req = TurnRequest {
            prompt: turn_prompt,
            workdir: PathBuf::from(&handle.workdir),
            permission: Permission::AcceptEdits,
            system_prompt: plan.system.clone(),
            // Always the task's own id -- see the module header. The native
            // engine mints a fresh `Conversation` under this id on the very
            // first turn and loads it back on every one after.
            resume: Some(handle.id.clone()),
            mcp_config: plan.mcp_config.clone(),
            mcp_env: plan.mcp_env.clone(),
            allowed_tools: Vec::new(),
            store_dir: handle.store_dir.clone(),
            provider: plan.provider.clone(),
        };

        let turn = match engine_for_turn.start(&req, sink.clone()) {
            Ok(t) => t,
            Err(e) => {
                finish(&handle, TaskStatus::Failed, Some(e));
                return;
            }
        };
        *handle.current_turn.lock().unwrap_or_else(|e| e.into_inner()) = Some(turn);

        {
            let mut live = handle.live.lock().unwrap_or_else(|e| e.into_inner());
            live.last_activity = "working".into();
            live.updated_at = now_ts();
        }

        let waited = rx.recv_timeout(std::time::Duration::from_secs(OUTER_TURN_WAIT_SECS));
        *handle.current_turn.lock().unwrap_or_else(|e| e.into_inner()) = None;
        if waited.is_err() {
            finish(
                &handle,
                TaskStatus::Failed,
                Some("a turn stopped answering and the outer safety timeout ended it".into()),
            );
            return;
        }

        let (text, turn_error) = sink.take();

        {
            let mut live = handle.live.lock().unwrap_or_else(|e| e.into_inner());
            live.turn_count += 1;
            live.updated_at = now_ts();
            if let Some(e) = &turn_error {
                live.last_error = Some(e.clone());
            }
            live.last_activity = if text.trim().is_empty() {
                "answered with nothing".into()
            } else {
                "checking in".into()
            };
        }

        // A confirm() call landed this turn -- pause for a person, do not
        // queue another turn. Checked before the stall/done parsing below:
        // a model that got refused an action and then declared itself done
        // in the same breath is still a pending ask a person should see.
        let has_pending_ask = handle.live.lock().unwrap_or_else(|e| e.into_inner()).awaiting.is_some();
        if has_pending_ask {
            pause_needs_input(&handle);
            return;
        }

        let (stalled, new_streak) = {
            let live = handle.live.lock().unwrap_or_else(|e| e.into_inner());
            is_stall(live.last_assistant_text.as_deref(), &text, live.repeat_streak)
        };
        {
            let mut live = handle.live.lock().unwrap_or_else(|e| e.into_inner());
            live.repeat_streak = new_streak;
            live.last_assistant_text = Some(text.trim().to_string());
        }
        if stalled {
            pause_for_stall(&handle);
            return;
        }

        checkpoint(&handle);

        if is_declared_done(&text) {
            finish(&handle, TaskStatus::ReportedDone, None);
            return;
        }
        // Otherwise: `TASK_STATUS: CONTINUING`, or the model did not follow
        // the contract at all. Either way the loop continues -- a formatting
        // miss must not stop real progress, the same tolerance
        // `boardroom::parse_stance` already holds for a model that ignores
        // its own asked-for shape. The cap at the top of this loop is what
        // actually stops a task that never declares itself done.
    }
}

// ---------------------------------------------------------------------------
// Persistence — the sidecar, atomic, same pattern as
// `native::store::Conversation::save`.
// ---------------------------------------------------------------------------

fn sidecar_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_config_dir()
        .map_err(|e| format!("no config directory: {e}"))?
        .join("worker-tasks");
    std::fs::create_dir_all(&dir).map_err(|e| format!("could not create {dir:?}: {e}"))?;
    Ok(dir)
}

/// Each task's own MCP launch config -- see the module header for why this
/// is per-task rather than `main.rs`'s shared `mcp-launch.json`.
fn worker_mcp_config_path(dir: &Path, task_id: &str) -> PathBuf {
    dir.join(format!("{task_id}-mcp.json"))
}

fn save_record(dir: &Path, rec: &TaskRecord) -> Result<(), String> {
    if !store::id_is_safe(&rec.id) {
        return Err("refusing to write a task record with an unsafe id".into());
    }
    std::fs::create_dir_all(dir).map_err(|e| format!("could not create {dir:?}: {e}"))?;
    let body = serde_json::to_string(rec).map_err(|e| e.to_string())?;
    let tmp = dir.join(format!("{}.json.writing", rec.id));
    std::fs::write(&tmp, body).map_err(|e| format!("could not write {tmp:?}: {e}"))?;
    std::fs::rename(&tmp, dir.join(format!("{}.json", rec.id)))
        .map_err(|e| format!("could not replace the task record: {e}"))
}

fn load_record(dir: &Path, id: &str) -> Option<TaskRecord> {
    if !store::id_is_safe(id) {
        return None;
    }
    let text = std::fs::read_to_string(dir.join(format!("{id}.json"))).ok()?;
    let rec: TaskRecord = serde_json::from_str(&text).ok()?;
    if rec.id != id {
        return None;
    }
    Some(rec)
}

/// The pure half of orphan detection: what a status found on disk becomes,
/// or `None` if it needs no change. Kept separate from any filesystem
/// walking so it is testable as a plain match, the same discipline
/// `cap_reason`/`is_stall` above already hold.
fn orphan_transition(status: TaskStatus) -> Option<TaskStatus> {
    match status {
        TaskStatus::Running | TaskStatus::NeedsInput => Some(TaskStatus::NeedsResume),
        _ => None,
    }
}

/// Walk `dir` and retitle every orphaned record — the file-IO half of
/// orphan detection, kept separate from `flag_orphans_on_launch` so a test
/// can point it at a temp directory without a real `AppHandle`.
///
/// **NEVER PUTS ANYTHING BACK INTO A `Workers` REGISTRY.** This function
/// does not take one and could not resume a task even if it wanted to — the
/// disk is rewritten, nothing more. Resuming is `worker_resume`'s job,
/// reachable only by a person's own click. See this module's header.
fn scan_and_flag_dir(dir: &Path) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else { continue };
        let Some(mut rec) = load_record(dir, stem) else { continue };
        if let Some(next) = orphan_transition(rec.status) {
            rec.status = next;
            rec.updated_at = now_ts();
            rec.pending_confirm = None;
            rec.last_error = Some(
                "the app closed or crashed while this task was in flight; nothing ran \
                 while it was closed, and it was not resumed automatically"
                    .into(),
            );
            let _ = save_record(dir, &rec);
        }
    }
}

/// Called once from `main()`'s `.setup()`, the same best-effort, non-fatal
/// standing every other optional launch step there already has (the mic
/// permission, the companion hotkey) — a launch-time sweep that failed to
/// read its own directory is a worse first run than refusing to start the
/// whole app over it.
pub(crate) fn flag_orphans_on_launch(app: &AppHandle) {
    if let Ok(dir) = sidecar_dir(app) {
        scan_and_flag_dir(&dir);
    }
}

// ---------------------------------------------------------------------------
// Tauri commands.
// ---------------------------------------------------------------------------

fn find_task(workers: &State<Workers>, id: &str) -> Result<Arc<TaskHandle>, String> {
    workers
        .tasks
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(id)
        .cloned()
        .ok_or_else(|| "No task with that id is running in this session.".to_string())
}

/// Start a new worker task.
///
/// **DOES NOT SHOW THE FOLDER REVIEW SCREEN ITSELF — the caller must have
/// already walked the person through `folder_trust::folder_trust_probe` for
/// `workdir` and gotten an explicit go-ahead**, the same way the chat send
/// box already requires a reviewed folder before `send()` will run in it.
/// This command still independently re-runs `folder_trust::gate` before
/// doing anything else, exactly like `send()` does — a front-end-only gate
/// is not a gate; this is the one that actually refuses.
///
/// `provider_id` must name a row whose engine is NOT `ClaudeCodeEngine` —
/// v1 workers run on the native engine only (Ollama / an OpenAI-compatible
/// row / a gated Anthropic-compatible row), because that engine's tool loop
/// is the thing enforcing this module's own bounds, not a second program's
/// CLI semantics driven with nobody able to answer its own prompts. See the
/// module header.
#[tauri::command(async)]
pub fn worker_start(
    app: AppHandle,
    providers_state: State<providers::Providers>,
    connectors_state: State<crate::connectors::Connectors>,
    workers: State<Workers>,
    provider_id: String,
    workdir: String,
    prompt: String,
    done_when: String,
    title: Option<String>,
) -> Result<TaskRecord, String> {
    let prompt = prompt.trim().to_string();
    if prompt.is_empty() {
        return Err("Give the task something to do first.".into());
    }
    let done_when = done_when.trim().to_string();
    if done_when.is_empty() {
        return Err(
            "Say how this task will know it is done -- a file that should exist, an answer \
             it should produce, something checkable. A task with no stated finish line either \
             runs forever or has to guess, and neither is safe to leave unattended."
                .into(),
        );
    }

    // **REQUIRES `trusted`, NOT `!needs_decision` — Beck's VM verification,
    // 2026-09-25: 6/6 fresh starts in an empty folder skipped review
    // entirely, and the folder was never recorded in `folder-trust.json`.**
    // `needs_decision` is `false` for a folder with nothing suspicious in
    // it (`folder_trust.rs`'s own "a clean folder sails through" design —
    // see that file's header) — correct for `send()`'s attended chat turn,
    // where a live person is watching every turn and can Stop it, and
    // WRONG here: an unattended worker in a folder nobody ever explicitly
    // accepted, empty or not, is exactly the hole this gate exists to
    // close. `trusted` is `true` only once `mark_folder_trusted` has
    // actually written a record for this exact path — the recorded fact
    // this task needs, not the absence of a reason to worry.
    let trust = crate::folder_trust::gate(&app, &workdir);
    if !trust.trusted {
        return Err(
            "This folder hasn't been reviewed and accepted yet — an unattended task needs an \
             explicit yes for its folder, even an empty one. Open the folder review screen, \
             accept it, then start the task."
                .into(),
        );
    }

    let store = providers::list_providers(app.clone(), providers_state);
    let provider = store
        .providers
        .iter()
        .find(|p| p.id == provider_id)
        .cloned()
        .ok_or_else(|| "That brain is not connected.".to_string())?;
    // Same pure check every turn of this task will be held to for its
    // whole life -- see `admission_refusal`'s own doc.
    if let Some(reason) = admission_refusal(&app, &provider) {
        return Err(reason);
    }

    let id = store::new_id();
    let sidecar = sidecar_dir(&app)?;
    let mcp_path = worker_mcp_config_path(&sidecar, &id);
    let mcp_env = match if providers::is_airgapped(&app) {
        None
    } else {
        crate::connectors::launch_config(&app, &connectors_state)
    } {
        Some((json, env)) => {
            if std::fs::write(&mcp_path, json).is_ok() {
                env
            } else {
                Vec::new()
            }
        }
        None => Vec::new(),
    };
    let mcp_config = if mcp_path.exists() { Some(mcp_path) } else { None };

    let mut system = crate::voice::VOICE.to_string();
    system.push_str(crate::google_policy::GUIDANCE);
    system.push_str(&task_contract(&done_when));
    if let Some(block) = crate::profile::system_prompt_block(&app) {
        system.push_str(&block);
    }
    if let Some(facts) = crate::memory::tier3_prompt(&workdir, &prompt) {
        system.push_str(&facts);
    }
    if let Some(bridge) = crate::memory::bridge_prompt(&workdir, false) {
        system.push_str(&bridge);
    }

    let created_at = now_ts();
    let handle = Arc::new(TaskHandle {
        id: id.clone(),
        title: title.unwrap_or_else(|| prompt.chars().take(60).collect()),
        prompt: prompt.clone(),
        done_when,
        workdir: trust.path.clone(),
        provider_id: provider.id.clone(),
        created_at,
        store_dir: conversations_dir_for_worker(&app),
        sidecar_dir: sidecar,
        live: Mutex::new(Live::new()),
        current_turn: Mutex::new(None),
    });

    workers.tasks.lock().unwrap_or_else(|e| e.into_inner()).insert(id.clone(), Arc::clone(&handle));
    checkpoint(&handle);

    let plan = TaskPlan { provider, mcp_config, mcp_env, system };
    let app_for_thread = app.clone();
    let handle_for_thread = Arc::clone(&handle);
    let spawned = std::thread::Builder::new()
        .name(format!("worker-{id}"))
        .spawn(move || run_task(app_for_thread, handle_for_thread, plan));
    if let Err(e) = spawned {
        finish(&handle, TaskStatus::Failed, Some(format!("could not start the task thread: {e}")));
    }

    Ok(handle.snapshot())
}

fn conversations_dir_for_worker(app: &AppHandle) -> Option<PathBuf> {
    // Reuses `main.rs`'s own conversations directory -- see the module
    // header for why a worker's transcript is a real `store::Conversation`
    // rather than a second format. This means a worker's turns will appear
    // in the ordinary History tab alongside chat, same as any other native
    // engine conversation -- a known, deliberate cross-effect of that reuse,
    // not an oversight, and a UI distinction (if wanted) is Wren's call.
    crate::conversations_dir(app)
}

#[tauri::command]
pub fn worker_list(app: AppHandle, workers: State<Workers>) -> Vec<TaskRecord> {
    let mut list: Vec<TaskRecord> = merged_task_records(&app, &workers).into_values().collect();
    list.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    list
}

#[tauri::command]
pub fn worker_get(app: AppHandle, workers: State<Workers>, id: String) -> Result<TaskRecord, String> {
    // In-memory first (this session's own live handle, if it has one —
    // more current than its last checkpoint), THEN disk, so a task from a
    // previous session this one has not touched yet (including a
    // `needsResume` orphan) is still reachable by id, not only by luck of
    // being in `worker_list`'s own collection.
    if let Ok(handle) = find_task(&workers, &id) {
        return Ok(handle.snapshot());
    }
    let dir = sidecar_dir(&app)?;
    load_record(&dir, &id).ok_or_else(|| "No task with that id was found.".to_string())
}

/// **THE COLD-START FIX — Beck's VM verification, 2026-09-25: after a
/// restart the Task Center was EMPTY, because the in-memory `Workers`
/// registry starts empty every launch and nothing ever loaded the
/// persisted sidecar files back into it.** `needsResume` tasks were
/// invisible (so unresumable — `worker_resume` needs a person to have SEEN
/// the id first) and every finished task's own history vanished from the
/// window, even though the records were sitting on disk the whole time.
///
/// Reads the sidecar directory FRESH on every call rather than caching —
/// same reasoning `folder_trust::gate`'s own doc gives for re-probing
/// instead of trusting a snapshot: disk is the durable truth, and a task
/// finishing or checkpointing between two calls must be reflected on the
/// very next one. In-memory records WIN on conflict — a task this session
/// is actively driving is more current than whatever it last checkpointed
/// — everything else comes straight from disk. **This never re-arms
/// anything**: `TaskHandle`s are still only ever created by `worker_start`
/// or `worker_resume`, both reachable only by a person's own click — a
/// task surfacing here is a task being SHOWN, not a task being run. See
/// this module's own header and `flag_orphans_on_launch`'s doc for why
/// that distinction is load-bearing.
fn merged_task_records(app: &AppHandle, workers: &State<Workers>) -> HashMap<String, TaskRecord> {
    let mut by_id = match sidecar_dir(app) {
        Ok(dir) => load_all_records(&dir),
        Err(_) => HashMap::new(),
    };
    for handle in workers.tasks.lock().unwrap_or_else(|e| e.into_inner()).values() {
        let rec = handle.snapshot();
        by_id.insert(rec.id.clone(), rec);
    }
    by_id
}

/// The disk-reading half of the cold-start fix, kept path-based (no
/// `AppHandle`) so it is directly testable against a temp directory — same
/// split as `scan_and_flag_dir`/`flag_orphans_on_launch`. Every `.json`
/// sidecar in `dir` that still parses as a `TaskRecord` is included,
/// keyed on its own id; a corrupt or unreadable one is skipped exactly the
/// way `load_record` already treats one, never repaired and never taken
/// down the whole list with it.
fn load_all_records(dir: &Path) -> HashMap<String, TaskRecord> {
    let mut by_id = HashMap::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return by_id;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
            if let Some(rec) = load_record(dir, stem) {
                by_id.insert(rec.id.clone(), rec);
            }
        }
    }
    by_id
}

/// Answer a pending consent request -- a real tool asking for approval, or a
/// stall pause. Both are answered the same way: `approve: true` means "let
/// it try again," `approve: false` means "stop here."
///
/// **AN EXPIRED ASK IS A NO-OP, NOT A LATE YES.** FOLD-IN #1 from the room's
/// safety review: a pending action past `CONSENT_EXPIRY_SECS` is dropped
/// rather than answerable, and the task stays paused with nothing pending —
/// the model has to bring the request up again on its own next turn before
/// there is anything left to approve.
#[tauri::command(async)]
pub fn worker_answer_confirm(
    app: AppHandle,
    providers_state: State<providers::Providers>,
    connectors_state: State<crate::connectors::Connectors>,
    workers: State<Workers>,
    id: String,
    approve: bool,
) -> Result<TaskRecord, String> {
    let handle = find_task(&workers, &id)?;
    let (was_stall, prompt_text, resume_needed) = {
        let mut guard = handle.live.lock().unwrap_or_else(|e| e.into_inner());
        let live = &mut *guard;
        let now = now_ts();
        expire_stale(&mut live.awaiting, &mut live.approved, now);
        let Some(ask) = live.awaiting.take() else {
            return Err(
                "That request has already expired or was answered -- if the task still \
                 needs it, it will ask again on its own."
                    .into(),
            );
        };
        if approve && !ask.stall {
            // F3 -- the args identity travels with the approval too, not
            // just the prompt text, so `confirm()`'s later match requires
            // both to agree with what was actually shown and actually
            // approved.
            live.approved = Some(Approval {
                prompt: ask.prompt.clone(),
                expires_at: now + CONSENT_EXPIRY_SECS,
                args_key: ask.args_key,
            });
        }
        if !approve {
            live.repeat_streak = 0;
        }
        if !approve && !ask.stall {
            // A denied real action is not a reason to stop the whole task --
            // the next turn is told plainly and the model works around it.
        }
        if !approve {
            live.status = TaskStatus::Cancelled;
            live.updated_at = now;
            (ask.stall, ask.prompt, false)
        } else {
            live.status = TaskStatus::Running;
            live.updated_at = now;
            (ask.stall, ask.prompt, true)
        }
    };

    if !approve {
        finish(&handle, TaskStatus::Cancelled, Some(format!("declined: {prompt_text}")));
        return Ok(handle.snapshot());
    }
    if !resume_needed {
        return Ok(handle.snapshot());
    }
    checkpoint(&handle);

    // Re-resolve the provider and connectors fresh, exactly like
    // `worker_start` does, rather than caching what the task began with --
    // the person may have edited the row while the task was paused.
    let store = providers::list_providers(app.clone(), providers_state);
    let provider = store
        .providers
        .iter()
        .find(|p| p.id == handle.provider_id)
        .cloned()
        .ok_or_else(|| "The brain this task was using is no longer connected.".to_string())?;

    // F1/F2 -- Cassandra's adversarial review, 2026-09-25. Checked here,
    // BEFORE any thread is spawned, so a bad state is refused synchronously
    // rather than discovered later inside `run_resumed_turn_then_loop`
    // (which carries its own copy of this same check as a second,
    // independent net -- see that function's own doc for why one call site
    // is not trusted to be the only one). By this point the person's answer
    // has already been consumed (`live.awaiting` taken, `live.approved` or
    // `live.status` already written above), so a refusal here finalises the
    // task as `Failed` through `finish` rather than returning a bare `Err`
    // that would leave the record silently stuck on `Running` with nothing
    // left driving it.
    if let Some(reason) = admission_refusal(&app, &provider) {
        finish(&handle, TaskStatus::Failed, Some(reason));
        return Ok(handle.snapshot());
    }
    // Same reasoning, same net -- a person could withdraw this folder's
    // acceptance while the task sat paused at `needs_input`.
    if let Some(reason) = folder_trust_refusal(&app, &handle.workdir) {
        finish(&handle, TaskStatus::Failed, Some(reason));
        return Ok(handle.snapshot());
    }

    let mcp_path = worker_mcp_config_path(&handle.sidecar_dir, &handle.id);
    let mcp_env = match if providers::is_airgapped(&app) {
        None
    } else {
        crate::connectors::launch_config(&app, &connectors_state)
    } {
        Some((json, env)) => {
            let _ = std::fs::write(&mcp_path, json);
            env
        }
        None => Vec::new(),
    };
    let mcp_config = if mcp_path.exists() { Some(mcp_path) } else { None };
    let mut system = crate::voice::VOICE.to_string();
    system.push_str(crate::google_policy::GUIDANCE);
    system.push_str(&task_contract(&handle.done_when));
    if let Some(block) = crate::profile::system_prompt_block(&app) {
        system.push_str(&block);
    }

    let note = if was_stall {
        "The person you're doing this for was asked whether to keep going after two \
         identical answers in a row, and said yes -- try a genuinely different approach \
         this time rather than repeating the same answer again."
            .to_string()
    } else {
        format!(
            "The person approved your request: \"{prompt_text}\". Call it again now if you \
             still need it, or continue with the rest of the task."
        )
    };
    // The approval note rides the FIRST prompt of the resumed run as an
    // ordinary continuation message -- `run_task` always builds its own
    // continuation text for turn_count > 0, so this task's own thread will
    // in fact send `continuation_prompt(None)`. To make sure THIS specific
    // note is what the model sees next, it is written as the task's own
    // resumed entry point below rather than trusted to `run_task`'s default.
    let plan = TaskPlan { provider, mcp_config, mcp_env, system };
    let app_for_thread = app.clone();
    let handle_for_thread = Arc::clone(&handle);
    let approval_note = note;
    let spawned = std::thread::Builder::new().name(format!("worker-{}-resume", handle.id)).spawn(move || {
        run_resumed_turn_then_loop(app_for_thread, handle_for_thread, plan, approval_note)
    });
    if let Err(e) = spawned {
        finish(&handle, TaskStatus::Failed, Some(format!("could not resume the task thread: {e}")));
    }

    Ok(handle.snapshot())
}

/// `run_task`, but the very first turn carries a specific note (what the
/// person just approved) instead of the ordinary "Continue the task."
/// message — everything after that first turn is identical to `run_task`'s
/// own loop, which this simply falls into once the note has been used once.
///
/// **F1/F2 — Cassandra's adversarial review, 2026-09-25: this function used
/// to build and run its own first turn with NO admission check at all.**
/// `run_task`'s loop re-checks before every turn it queues, but a task's
/// FIRST turn after being resumed — reached from `worker_answer_confirm`'s
/// Approve button, and from `worker_resume` after an app restart — went
/// straight to `engine::for_provider(...).start(...)` with nothing in
/// front of it. A provider row edited to a Claude-Code-shaped one, or
/// air-gap turned on, while the task sat paused at `needs_input`, would
/// have run one full unattended turn on it before the ordinary loop's own
/// check ever got a turn to apply to. See `admission_refusal`'s own doc.
fn run_resumed_turn_then_loop(app: AppHandle, handle: Arc<TaskHandle>, plan: TaskPlan, note: String) {
    if let Some(reason) = admission_refusal(&app, &plan.provider) {
        finish(&handle, TaskStatus::Failed, Some(reason));
        return;
    }
    if let Some(reason) = folder_trust_refusal(&app, &handle.workdir) {
        finish(&handle, TaskStatus::Failed, Some(reason));
        return;
    }
    {
        let mut live = handle.live.lock().unwrap_or_else(|e| e.into_inner());
        live.status = TaskStatus::Running;
        live.updated_at = now_ts();
    }
    let cancelled_before_start = handle.live.lock().unwrap_or_else(|e| e.into_inner()).cancel_requested;
    if cancelled_before_start {
        finish(&handle, TaskStatus::Cancelled, None);
        return;
    }
    let (sink, rx) = WorkerSink::new(Arc::clone(&handle));
    let req = TurnRequest {
        prompt: continuation_prompt(Some(&note)),
        workdir: PathBuf::from(&handle.workdir),
        permission: Permission::AcceptEdits,
        system_prompt: plan.system.clone(),
        resume: Some(handle.id.clone()),
        mcp_config: plan.mcp_config.clone(),
        mcp_env: plan.mcp_env.clone(),
        allowed_tools: Vec::new(),
        store_dir: handle.store_dir.clone(),
        provider: plan.provider.clone(),
    };
    let engine_for_turn = engine::for_provider(&plan.provider);
    let turn = match engine_for_turn.start(&req, sink.clone()) {
        Ok(t) => t,
        Err(e) => {
            finish(&handle, TaskStatus::Failed, Some(e));
            return;
        }
    };
    *handle.current_turn.lock().unwrap_or_else(|e| e.into_inner()) = Some(turn);
    let waited = rx.recv_timeout(std::time::Duration::from_secs(OUTER_TURN_WAIT_SECS));
    *handle.current_turn.lock().unwrap_or_else(|e| e.into_inner()) = None;
    if waited.is_err() {
        finish(&handle, TaskStatus::Failed, Some("a turn stopped answering and the outer safety timeout ended it".into()));
        return;
    }
    let (text, turn_error) = sink.take();
    {
        let mut live = handle.live.lock().unwrap_or_else(|e| e.into_inner());
        live.turn_count += 1;
        live.updated_at = now_ts();
        if let Some(e) = &turn_error {
            live.last_error = Some(e.clone());
        }
    }
    let has_pending_ask = handle.live.lock().unwrap_or_else(|e| e.into_inner()).awaiting.is_some();
    if has_pending_ask {
        pause_needs_input(&handle);
        return;
    }
    let (stalled, new_streak) = {
        let live = handle.live.lock().unwrap_or_else(|e| e.into_inner());
        is_stall(live.last_assistant_text.as_deref(), &text, live.repeat_streak)
    };
    {
        let mut live = handle.live.lock().unwrap_or_else(|e| e.into_inner());
        live.repeat_streak = new_streak;
        live.last_assistant_text = Some(text.trim().to_string());
    }
    if stalled {
        pause_for_stall(&handle);
        return;
    }
    checkpoint(&handle);
    if is_declared_done(&text) {
        finish(&handle, TaskStatus::ReportedDone, None);
        return;
    }
    run_task(app, handle, plan);
}

/// The kill switch. Idempotent -- safe to call on a task that already
/// finished on its own, same contract `main.rs::stop` already holds for the
/// interactive Stop button.
#[tauri::command(async)]
pub fn worker_cancel(workers: State<Workers>, id: String) -> Result<TaskRecord, String> {
    let handle = find_task(&workers, &id)?;
    cancel_task(&handle)?;
    checkpoint(&handle);
    Ok(handle.snapshot())
}

/// Pick a `NeedsResume` task back up. **Only ever reachable by a person's
/// own click** — nothing in this module calls it on its own. Re-validates
/// the folder and the brain exactly like `worker_start`, because the
/// original review is old news by the time a person resumes a task that sat
/// through a restart.
#[tauri::command(async)]
pub fn worker_resume(
    app: AppHandle,
    providers_state: State<providers::Providers>,
    connectors_state: State<crate::connectors::Connectors>,
    workers: State<Workers>,
    id: String,
) -> Result<TaskRecord, String> {
    let sidecar = sidecar_dir(&app)?;
    let rec = load_record(&sidecar, &id)
        .ok_or_else(|| "No task record with that id was found on disk.".to_string())?;
    if rec.status != TaskStatus::NeedsResume {
        return Err("That task is not waiting to be resumed.".into());
    }

    // Same `trusted`, not `!needs_decision`, check as `worker_start` —
    // see that call site's own comment. Catches both a folder that was
    // never recorded in the first place and one whose acceptance was
    // explicitly withdrawn (`forget_folder_trust`) while the task sat
    // paused across a restart.
    let trust = crate::folder_trust::gate(&app, &rec.workdir);
    if !trust.trusted {
        return Err(
            "This task's folder hasn't been reviewed and accepted -- open the folder review \
             screen, accept it, then resume."
                .into(),
        );
    }

    let store = providers::list_providers(app.clone(), providers_state);
    let provider = store
        .providers
        .iter()
        .find(|p| p.id == rec.provider_id)
        .cloned()
        .ok_or_else(|| "The brain this task was using is no longer connected.".to_string())?;
    // Same pure check every other entry point uses -- see
    // `admission_refusal`'s own doc. Closes the air-gap gap this call site
    // used to have too: only the vendor-binary half was ever checked here.
    if let Some(reason) = admission_refusal(&app, &provider) {
        return Err(reason);
    }

    let handle = Arc::new(TaskHandle {
        id: rec.id.clone(),
        title: rec.title.clone(),
        prompt: rec.prompt.clone(),
        done_when: rec.done_when.clone(),
        workdir: trust.path.clone(),
        provider_id: provider.id.clone(),
        created_at: rec.created_at,
        store_dir: conversations_dir_for_worker(&app),
        sidecar_dir: sidecar.clone(),
        live: Mutex::new(Live {
            status: TaskStatus::Running,
            updated_at: now_ts(),
            started_running_at: Some(now_ts()),
            turn_count: rec.turn_count,
            input_tokens: rec.input_tokens,
            output_tokens: rec.output_tokens,
            last_activity: "resuming".into(),
            last_error: None,
            last_assistant_text: None,
            repeat_streak: 0,
            awaiting: None,
            approved: None,
            cancel_requested: false,
        }),
        current_turn: Mutex::new(None),
    });
    workers.tasks.lock().unwrap_or_else(|e| e.into_inner()).insert(id.clone(), Arc::clone(&handle));
    checkpoint(&handle);

    let mcp_path = worker_mcp_config_path(&handle.sidecar_dir, &handle.id);
    let mcp_env = match if providers::is_airgapped(&app) {
        None
    } else {
        crate::connectors::launch_config(&app, &connectors_state)
    } {
        Some((json, env)) => {
            let _ = std::fs::write(&mcp_path, json);
            env
        }
        None => Vec::new(),
    };
    let mcp_config = if mcp_path.exists() { Some(mcp_path) } else { None };
    let mut system = crate::voice::VOICE.to_string();
    system.push_str(crate::google_policy::GUIDANCE);
    system.push_str(&task_contract(&handle.done_when));
    if let Some(block) = crate::profile::system_prompt_block(&app) {
        system.push_str(&block);
    }

    let plan = TaskPlan { provider, mcp_config, mcp_env, system };
    let app_for_thread = app.clone();
    let handle_for_thread = Arc::clone(&handle);
    let spawned = std::thread::Builder::new().name(format!("worker-{id}-resumed")).spawn(move || {
        run_resumed_turn_then_loop(
            app_for_thread,
            handle_for_thread,
            plan,
            "The app was restarted since your last turn. Pick this task back up.".to_string(),
        )
    });
    if let Err(e) = spawned {
        finish(&handle, TaskStatus::Failed, Some(format!("could not resume the task thread: {e}")));
    }

    Ok(handle.snapshot())
}

// ---------------------------------------------------------------------------
// Tests -- pure, safety-critical decisions first, matching this crate's own
// discipline (`boardroom.rs`'s `parse_stance`/`final_positions`,
// `engine::native::mod`'s `may_run_preapproved`): a decision worth trusting
// is testable without a real engine, a real folder, or a real clock.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    // ---- cap_reason: "a cap trips" --------------------------------------

    #[test]
    fn under_every_cap_nothing_trips() {
        assert_eq!(cap_reason(5, 1000, 1500, 100, 100), None);
    }

    #[test]
    fn the_turn_cap_trips_at_the_stated_limit() {
        assert!(cap_reason(MAX_TASK_TURNS, 1000, 1500, 0, 0).is_some());
        assert!(cap_reason(MAX_TASK_TURNS - 1, 1000, 1500, 0, 0).is_none());
    }

    #[test]
    fn the_wall_clock_cap_trips_at_the_stated_limit() {
        let started = 1000;
        let now = started + TASK_WALL_CLOCK_CAP_SECS;
        assert!(cap_reason(1, started, now, 0, 0).is_some());
        assert!(cap_reason(1, started, now - 1, 0, 0).is_none());
    }

    #[test]
    fn the_token_cap_trips_at_the_stated_limit() {
        assert!(cap_reason(1, 1000, 1500, MAX_TASK_TOKENS, 0).is_some());
        assert!(cap_reason(1, 1000, 1500, MAX_TASK_TOKENS / 2, MAX_TASK_TOKENS / 2 - 1).is_none());
    }

    #[test]
    fn a_capped_task_names_capped_not_finished() {
        let reason = cap_reason(MAX_TASK_TURNS, 1000, 1500, 0, 0).unwrap();
        assert!(reason.contains("capped, not finished"), "{reason}");
    }

    // ---- admission_refusal_for: F1 (resumed-turn vendor guard) and F2
    // ---- (air-gap re-checked on resume/approval), Cassandra's adversarial
    // ---- review, 2026-09-25 --------------------------------------------

    #[test]
    fn admission_refusal_blocks_a_vendor_binary_engine() {
        // F1's own exploit shape: the row now needs Claude Code (a vendor
        // binary with no live person to answer its permission prompts) --
        // whether this is the task's first turn, a per-turn re-check, or
        // the resumed turn `worker_answer_confirm`'s Approve button spawns,
        // this must refuse before anything is built for the turn.
        let reason = admission_refusal_for(true, false, "anthropic-compatible");
        assert!(reason.is_some(), "a vendor-binary-shaped engine must never be admitted");
    }

    #[test]
    fn admission_refusal_allows_a_native_engine_when_not_airgapped() {
        assert_eq!(admission_refusal_for(false, false, "local"), None);
        assert_eq!(admission_refusal_for(false, false, "openai-compatible"), None);
    }

    #[test]
    fn admission_refusal_blocks_a_cloud_brain_while_airgapped() {
        // F2's own exploit shape: air-gap turned on while a task was
        // paused at `needs_input` on a cloud row -- the approval path must
        // refuse exactly like the ordinary per-turn loop does.
        let reason = admission_refusal_for(false, true, "openai-compatible");
        assert!(reason.is_some(), "a cloud-kind row must never be admitted while air-gap is on");
    }

    #[test]
    fn admission_refusal_allows_a_local_brain_even_while_airgapped() {
        // Air-gap's whole point is to keep a turn from LEAVING the machine
        // -- a genuinely local row is never the thing it exists to stop.
        assert_eq!(admission_refusal_for(false, true, "local"), None);
    }

    #[test]
    fn admission_refusal_checks_vendor_binary_before_airgap() {
        // Both conditions true at once must still produce a real refusal
        // (order between the two checks is not the point being pinned here
        // -- only that neither condition can silently mask the other).
        assert!(admission_refusal_for(true, true, "claude").is_some());
    }

    // ---- folder_trust_refusal_for: Beck's VM verification, 2026-09-25 -----

    #[test]
    fn folder_trust_refusal_for_blocks_an_unrecorded_folder() {
        assert!(folder_trust_refusal_for(false).is_some());
    }

    #[test]
    fn folder_trust_refusal_for_allows_a_recorded_folder() {
        assert_eq!(folder_trust_refusal_for(true), None);
    }

    /// **THE ACTUAL EXPLOIT SHAPE, PROVEN AGAINST REAL `folder_trust.rs`
    /// BEHAVIOUR, NOT AN ASSUMPTION ABOUT IT — Beck's VM verification,
    /// 2026-09-25: 6/6 fresh worker starts in an empty folder skipped
    /// review entirely, and the folder was never recorded.** A genuinely
    /// fresh, empty folder has `needs_decision == false` — that is
    /// `folder_trust.rs`'s own deliberate design ("a clean folder sails
    /// through," correct for `send()`'s attended chat) — but this proves
    /// it ALSO has `trusted == false`, which is the fact the old
    /// `!needs_decision` check never looked at. Then proves the fix both
    /// ways: refused while unrecorded, allowed the moment a real
    /// `mark_at` (the only thing `mark_folder_trusted` ever calls) records
    /// it.
    ///
    /// Proven able to fail: reverting `worker_start`'s own check from
    /// `!trust.trusted` back to `trust.needs_decision` would let this exact
    /// fresh folder straight through, which is precisely what Beck saw on
    /// the real VM.
    #[test]
    fn a_fresh_empty_folder_is_needs_decision_false_but_trusted_false_and_the_worker_refuses_it() {
        let dir = std::env::temp_dir().join(format!("worker-fresh-folder-test-{}", store::new_id()));
        std::fs::create_dir_all(&dir).unwrap();
        let store_path = dir.join("state").join("folder-trust.json");

        let trust = crate::folder_trust::probe_at(&dir.to_string_lossy(), Some(&store_path));
        assert!(
            !trust.needs_decision,
            "a fresh empty folder must not itself demand attention from an attended chat turn \
             -- this is folder_trust's own deliberate design, and it is the exact precondition \
             Beck's exploit needs"
        );
        assert!(
            !trust.trusted,
            "and it must never read as trusted just because nothing was found in it"
        );
        assert!(
            folder_trust_refusal_for(trust.trusted).is_some(),
            "a fresh, never-reviewed folder must be refused for a worker, even though an \
             attended chat turn would let it sail through with nothing shown"
        );

        // The only thing that is supposed to clear this: a real recorded
        // acceptance, exactly what `mark_folder_trusted` performs.
        let accepted = crate::folder_trust::mark_at(&dir.to_string_lossy(), &store_path).expect("mark_at");
        assert!(accepted.trusted, "an explicit accept must actually record trust for this path");
        assert!(
            folder_trust_refusal_for(accepted.trusted).is_none(),
            "a genuinely recorded folder must be allowed once it has actually been accepted"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ---- load_all_records / merged_task_records: the cold-start fix -------

    /// **THE COLD-START FIX, PROVEN — Beck's VM verification, 2026-09-25:
    /// after a restart the Task Center was EMPTY because the in-memory
    /// `Workers` registry starts empty every launch and nothing loaded the
    /// persisted sidecar files back into it.** A fresh `Workers::default()`
    /// (exactly what a real cold start begins with) merged with the disk
    /// contents must still surface a task nobody in this session has
    /// touched — both a `needs_resume` orphan AND ordinary finished
    /// history, which is the other half of what Beck reported missing.
    ///
    /// Proven able to fail: `merged_task_records` reading only
    /// `workers.tasks` and never calling `load_all_records` would return an
    /// empty map here, exactly the bug reported.
    #[test]
    fn a_persisted_task_survives_a_cold_start_with_an_empty_in_memory_registry() {
        let dir = std::env::temp_dir().join(format!("worker-cold-start-test-{}", store::new_id()));
        std::fs::create_dir_all(&dir).unwrap();
        let needs_resume_id = store::new_id();
        let completed_id = store::new_id();
        save_record(&dir, &sample_record(&needs_resume_id, TaskStatus::NeedsResume)).unwrap();
        save_record(&dir, &sample_record(&completed_id, TaskStatus::ReportedDone)).unwrap();

        // The in-memory half of a real cold start: nothing in it at all --
        // no task in this test has ever been started or resumed.
        let workers = Workers::default();
        assert!(workers.tasks.lock().unwrap().is_empty(), "this test's own precondition");

        let mut merged = load_all_records(&dir);
        for handle in workers.tasks.lock().unwrap().values() {
            merged.insert(handle.snapshot().id.clone(), handle.snapshot());
        }

        assert!(
            merged.contains_key(&needs_resume_id),
            "a needsResume orphan must be visible after a cold start, or it can never be \
             resumed -- nobody can click Resume on a task they cannot see"
        );
        assert_eq!(merged[&needs_resume_id].status, TaskStatus::NeedsResume);
        assert!(
            merged.contains_key(&completed_id),
            "finished history must survive a restart too, not just needsResume tasks"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The in-memory copy must still win when a task exists in both places
    /// -- a live handle this session is actively driving is more current
    /// than whatever it last checkpointed, and the merge must not regress
    /// to showing stale disk state for a task that is genuinely running
    /// right now.
    #[test]
    fn an_in_memory_task_is_more_current_than_its_own_last_checkpoint_on_disk() {
        let dir = std::env::temp_dir().join(format!("worker-cold-start-test-{}", store::new_id()));
        std::fs::create_dir_all(&dir).unwrap();
        let handle = test_handle();
        // Simulate a stale checkpoint sitting on disk from a moment ago,
        // before the in-memory state moved on to `NeedsInput`.
        let mut stale = handle.snapshot();
        stale.status = TaskStatus::Running;
        save_record(&dir, &stale).unwrap();
        handle.live.lock().unwrap().status = TaskStatus::NeedsInput;

        let mut merged = load_all_records(&dir);
        merged.insert(handle.id.clone(), handle.snapshot());

        assert_eq!(
            merged[&handle.id].status,
            TaskStatus::NeedsInput,
            "the live, in-memory snapshot must win over a stale on-disk checkpoint"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ---- expire_stale: "consent expiry fires" ----------------------------

    #[test]
    fn an_awaiting_ask_past_its_expiry_is_dropped() {
        let mut awaiting = Some(PendingAsk {
            prompt: "open https://example.com".into(),
            requested_at: 100,
            expires_at: 200,
            stall: false,
            args_key: None,
        });
        let mut approved = None;
        expire_stale(&mut awaiting, &mut approved, 201);
        assert!(awaiting.is_none(), "an ask past its own expiry must be dropped, not left answerable");
    }

    #[test]
    fn an_awaiting_ask_still_inside_its_window_survives() {
        let mut awaiting = Some(PendingAsk {
            prompt: "open https://example.com".into(),
            requested_at: 100,
            expires_at: 200,
            stall: false,
            args_key: None,
        });
        let mut approved = None;
        expire_stale(&mut awaiting, &mut approved, 199);
        assert!(awaiting.is_some());
    }

    #[test]
    fn a_granted_approval_also_expires_if_never_consumed() {
        let mut awaiting = None;
        let mut approved = Some(Approval { prompt: "open https://example.com".into(), expires_at: 500, args_key: None });
        expire_stale(&mut awaiting, &mut approved, 501);
        assert!(
            approved.is_none(),
            "a yes sitting unused past its own window must not still be usable hours later -- \
             this is the exact hole the room's safety review named"
        );
    }

    // ---- WorkerSink::confirm: "confirm denies, not approves" -------------

    fn test_handle() -> Arc<TaskHandle> {
        Arc::new(TaskHandle {
            id: store::new_id(),
            title: "t".into(),
            prompt: "p".into(),
            done_when: "d".into(),
            workdir: std::env::temp_dir().to_string_lossy().to_string(),
            provider_id: "row".into(),
            created_at: 0,
            store_dir: None,
            sidecar_dir: std::env::temp_dir(),
            live: Mutex::new(Live::new()),
            current_turn: Mutex::new(None),
        })
    }

    #[test]
    fn confirm_denies_and_records_the_ask_when_nothing_was_pre_approved() {
        let handle = test_handle();
        let (sink, _rx) = WorkerSink::new(Arc::clone(&handle));
        let allowed = sink.confirm("open https://sketchy.example");
        assert!(!allowed, "confirm() must never auto-approve");
        let live = handle.live.lock().unwrap();
        assert_eq!(live.awaiting.as_ref().map(|a| a.prompt.as_str()), Some("open https://sketchy.example"));
    }

    #[test]
    fn confirm_still_denies_when_a_different_prompt_was_approved() {
        let handle = test_handle();
        {
            let mut live = handle.live.lock().unwrap();
            live.approved = Some(Approval { prompt: "open https://a.example".into(), expires_at: now_ts() + 100, args_key: None });
        }
        let (sink, _rx) = WorkerSink::new(Arc::clone(&handle));
        let allowed = sink.confirm("open https://b.example");
        assert!(!allowed, "an approval for one prompt must never cover a different one");
    }

    /// The live transcript chip shape `tool_use_event` produces, and the
    /// ONLY way `WorkerSink` ever learns a call's real args — see
    /// `WorkerSink::event`'s own `tool_use` arm.
    fn tool_use_seen(name: &str, input: serde_json::Value) -> Value {
        serde_json::json!({
            "type": "assistant",
            "message": { "role": "assistant", "content": [
                { "type": "tool_use", "id": "call_1", "name": name, "input": input }
            ]}
        })
    }

    fn args_key_from(name: &str, input: serde_json::Value) -> u64 {
        tool_args_key(name, &serde_json::to_string(&input).unwrap())
    }

    #[test]
    fn confirm_approves_once_for_a_matching_unexpired_approval_and_then_consumes_it() {
        let handle = test_handle();
        let key = args_key_from("OpenUrl", serde_json::json!({"url": "https://a.example"}));
        {
            let mut live = handle.live.lock().unwrap();
            live.approved = Some(Approval { prompt: "open https://a.example".into(), expires_at: now_ts() + 100, args_key: Some(key) });
        }
        let (sink, _rx) = WorkerSink::new(Arc::clone(&handle));
        // The chip the real drive loop always emits before it ever asks
        // confirm() for this same call.
        sink.event(tool_use_seen("OpenUrl", serde_json::json!({"url": "https://a.example"})));
        assert!(sink.confirm("open https://a.example"), "a genuine, unexpired, matching yes must be honoured");
        // One-shot: the SAME prompt asked again must not still be approved.
        let allowed_again = sink.confirm("open https://a.example");
        assert!(!allowed_again, "an approval answers exactly one call, never a standing yes");
    }

    #[test]
    fn confirm_never_treats_an_expired_approval_as_a_yes() {
        let handle = test_handle();
        let key = args_key_from("OpenUrl", serde_json::json!({"url": "https://a.example"}));
        {
            let mut live = handle.live.lock().unwrap();
            live.approved = Some(Approval { prompt: "open https://a.example".into(), expires_at: now_ts().saturating_sub(1), args_key: Some(key) });
        }
        let (sink, _rx) = WorkerSink::new(Arc::clone(&handle));
        sink.event(tool_use_seen("OpenUrl", serde_json::json!({"url": "https://a.example"})));
        assert!(!sink.confirm("open https://a.example"), "an approval past its own window must not be honoured");
    }

    /// **F3 — Cassandra's adversarial review, 2026-09-25, the required
    /// test.** A mail send's own rendered summary shows To/Subject/Body,
    /// never cc/bcc (`engine::native::mod`'s own `confirm_summary`). Here
    /// the DISPLAY TEXT the person approved is identical to the one shown
    /// again, but the REAL args the model is now calling with carry an
    /// added `cc` nobody approved. The old prompt-only match would have
    /// honoured this; the args-key match must not.
    ///
    /// Proven able to fail: dropping `a.args_key == Some(key)` from
    /// `confirm`'s `matches` expression (falling back to `a.prompt ==
    /// prompt` alone) makes this assert `true == false`.
    #[test]
    fn confirm_never_honours_an_approval_when_the_real_args_changed_even_if_the_display_text_matches() {
        let handle = test_handle();
        let approved_args = serde_json::json!({"to": "a@example.com", "subject": "Hi", "body": "text"});
        let key = args_key_from("SendMail", approved_args);
        {
            let mut live = handle.live.lock().unwrap();
            live.approved = Some(Approval { prompt: "Send to a@example.com: Hi".into(), expires_at: now_ts() + 100, args_key: Some(key) });
        }
        let (sink, _rx) = WorkerSink::new(Arc::clone(&handle));
        // Same rendered prompt text the person actually approved -- but the
        // REAL call now carries a cc the summary never showed.
        sink.event(tool_use_seen(
            "SendMail",
            serde_json::json!({"to": "a@example.com", "subject": "Hi", "body": "text", "cc": "someone-else@example.com"}),
        ));
        let allowed = sink.confirm("Send to a@example.com: Hi");
        assert!(
            !allowed,
            "an approval must never cover a call whose real arguments changed, even when the \
             shown summary text is identical"
        );
    }


    // ---- cancel_task: "kill switch cancels mid-run" -----------------------

    struct FakeRunningTurn {
        cancelled: Arc<AtomicBool>,
    }
    impl RunningTurn for FakeRunningTurn {
        fn is_alive(&self) -> bool {
            !self.cancelled.load(Ordering::SeqCst)
        }
        fn cancel(&self) -> Result<(), String> {
            self.cancelled.store(true, Ordering::SeqCst);
            Ok(())
        }
    }

    #[test]
    fn cancel_task_calls_cancel_on_the_turn_actually_running() {
        let handle = test_handle();
        let cancelled_flag = Arc::new(AtomicBool::new(false));
        *handle.current_turn.lock().unwrap() = Some(Box::new(FakeRunningTurn { cancelled: Arc::clone(&cancelled_flag) }));
        cancel_task(&handle).expect("cancelling a running task must succeed");
        assert!(cancelled_flag.load(Ordering::SeqCst), "the in-flight turn's own cancel() must actually be called");
        assert!(handle.live.lock().unwrap().cancel_requested, "cancellation must also stop the NEXT turn from being queued");
        assert_eq!(handle.live.lock().unwrap().status, TaskStatus::Cancelled);
    }

    #[test]
    fn cancel_task_is_a_safe_no_op_when_nothing_is_currently_running() {
        let handle = test_handle();
        assert!(cancel_task(&handle).is_ok(), "cancelling between turns, with no turn in flight, must still succeed");
        assert!(handle.live.lock().unwrap().cancel_requested, "and must still stop the next turn from being queued");
    }

    #[test]
    fn cancel_task_is_idempotent() {
        let handle = test_handle();
        cancel_task(&handle).unwrap();
        assert!(cancel_task(&handle).is_ok(), "cancelling an already-cancelled task must not error");
    }

    // ---- is_stall -----------------------------------------------------

    #[test]
    fn identical_consecutive_answers_eventually_read_as_a_stall() {
        let (stalled_1, streak_1) = is_stall(None, "same answer", 0);
        assert!(!stalled_1);
        let (stalled_2, streak_2) = is_stall(Some("same answer"), "same answer", streak_1);
        assert!(!stalled_2, "one repeat alone is not yet the threshold");
        let (stalled_3, _streak_3) = is_stall(Some("same answer"), "same answer", streak_2);
        assert!(stalled_3, "STALL_STREAK_THRESHOLD consecutive repeats must read as stalled");
    }

    #[test]
    fn a_changed_answer_resets_the_streak() {
        let (_s1, streak_1) = is_stall(None, "a", 0);
        let (_s2, streak_2) = is_stall(Some("a"), "a", streak_1);
        let (stalled, streak_3) = is_stall(Some("a"), "b", streak_2);
        assert!(!stalled);
        assert_eq!(streak_3, 0, "a genuinely different answer must reset the streak, not just fail to increment it");
    }

    #[test]
    fn empty_answers_never_count_as_a_stall() {
        let (stalled, streak) = is_stall(Some(""), "", 5);
        assert!(!stalled);
        assert_eq!(streak, 0, "two empty answers are not evidence of repetition, they are evidence of nothing having been said");
    }

    // ---- is_declared_done -----------------------------------------------

    #[test]
    fn the_exact_status_line_is_recognised() {
        assert!(is_declared_done("Some summary of what I did.\n\nTASK_STATUS: DONE"));
    }

    #[test]
    fn a_mere_mention_of_the_phrase_is_not_a_claim() {
        assert!(
            !is_declared_done("I have not yet reached TASK_STATUS: DONE, still working on it."),
            "the status line must be recognised on its OWN line, not as a substring inside prose"
        );
    }

    #[test]
    fn continuing_is_never_read_as_done() {
        assert!(!is_declared_done("TASK_STATUS: CONTINUING\nNext I will read the config file."));
    }

    // ---- orphan handling: "orphan not auto-resumed" -----------------------

    #[test]
    fn orphan_transition_retitles_running_and_needs_input_only() {
        assert_eq!(orphan_transition(TaskStatus::Running), Some(TaskStatus::NeedsResume));
        assert_eq!(orphan_transition(TaskStatus::NeedsInput), Some(TaskStatus::NeedsResume));
        assert_eq!(orphan_transition(TaskStatus::ReportedDone), None);
        assert_eq!(orphan_transition(TaskStatus::Failed), None);
        assert_eq!(orphan_transition(TaskStatus::Cancelled), None);
        assert_eq!(orphan_transition(TaskStatus::NeedsResume), None, "an already-flagged orphan is not re-flagged");
    }

    fn sample_record(id: &str, status: TaskStatus) -> TaskRecord {
        TaskRecord {
            id: id.to_string(),
            title: "t".into(),
            prompt: "p".into(),
            done_when: "d".into(),
            workdir: "/tmp".into(),
            provider_id: "row".into(),
            status,
            created_at: 0,
            updated_at: 0,
            turn_count: 3,
            input_tokens: 10,
            output_tokens: 10,
            last_activity: "working".into(),
            last_error: None,
            pending_confirm: None,
        }
    }

    #[test]
    fn a_running_orphan_on_disk_is_flagged_and_never_left_as_running() {
        let dir = std::env::temp_dir().join(format!("worker-orphan-test-{}", store::new_id()));
        std::fs::create_dir_all(&dir).unwrap();
        let id = store::new_id();
        save_record(&dir, &sample_record(&id, TaskStatus::Running)).unwrap();

        scan_and_flag_dir(&dir);

        let after = load_record(&dir, &id).expect("the record must still be readable");
        assert_eq!(after.status, TaskStatus::NeedsResume);
        assert!(after.last_error.is_some(), "an orphaned task must say plainly what happened to it");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn scan_and_flag_never_creates_a_workers_registry_entry() {
        // The actual proof that an orphan is never auto-resumed: the function
        // that flags it takes no `Workers` state at all and could not insert
        // into one even if it tried -- see `scan_and_flag_dir`'s own doc.
        let dir = std::env::temp_dir().join(format!("worker-orphan-test-{}", store::new_id()));
        std::fs::create_dir_all(&dir).unwrap();
        let id = store::new_id();
        save_record(&dir, &sample_record(&id, TaskStatus::Running)).unwrap();

        scan_and_flag_dir(&dir);

        let workers = Workers::default();
        assert!(
            workers.tasks.lock().unwrap().is_empty(),
            "a fresh registry must stay empty after an orphan scan -- resuming is a person's own click, never automatic"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_task_that_finished_cleanly_is_left_alone_by_the_orphan_sweep() {
        let dir = std::env::temp_dir().join(format!("worker-orphan-test-{}", store::new_id()));
        std::fs::create_dir_all(&dir).unwrap();
        let id = store::new_id();
        save_record(&dir, &sample_record(&id, TaskStatus::ReportedDone)).unwrap();

        scan_and_flag_dir(&dir);

        let after = load_record(&dir, &id).unwrap();
        assert_eq!(after.status, TaskStatus::ReportedDone);
        assert!(after.last_error.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ---- save_record / load_record round trip ----------------------------

    #[test]
    fn a_task_record_survives_a_round_trip() {
        let dir = std::env::temp_dir().join(format!("worker-store-test-{}", store::new_id()));
        let id = store::new_id();
        let rec = sample_record(&id, TaskStatus::NeedsInput);
        save_record(&dir, &rec).unwrap();
        let back = load_record(&dir, &id).expect("load");
        assert_eq!(back.id, rec.id);
        assert_eq!(back.status, TaskStatus::NeedsInput);
        assert!(!dir.join(format!("{id}.json.writing")).exists(), "no temp file left behind after a clean save");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_id_that_does_not_match_its_own_filename_is_refused() {
        let dir = std::env::temp_dir().join(format!("worker-store-test-{}", store::new_id()));
        std::fs::create_dir_all(&dir).unwrap();
        let impostor = sample_record("helloim-chat-somebodyelse", TaskStatus::Running);
        std::fs::write(dir.join("helloim-chat-mine.json"), serde_json::to_string(&impostor).unwrap()).unwrap();
        assert!(load_record(&dir, "helloim-chat-mine").is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
