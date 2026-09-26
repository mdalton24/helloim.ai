//! The engine seam.
//!
//! **WHAT AN "ENGINE" IS HERE: the thing that takes one turn's worth of intent
//! and produces the five-shape event stream the window already knows how to
//! draw.** Nothing more. It is not the model, it is not the provider, and it is
//! not the brain the person picked — those are `providers.rs`. An engine is
//! *who runs the agent loop*.
//!
//! **WHY THIS EXISTS — Mark, 2026-08-31: *"we need to make each of the
//! providers stand alone without claude"*, and earlier the same day *"find a
//! way to do it without claude.. nothing should reside on claude. All providers
//! should be neutral."*** Today every provider — Claude, a local Ollama model,
//! and (when the gate opens) an OpenAI-compatible endpoint — funnels through a
//! single spawn of somebody else's binary. `providers::apply_env` only ever set
//! environment variables on that one child. So "pick a local brain" still meant
//! "download 214 MB from Anthropic and sign in to a terminal", which is the
//! customer objection Mark reached this from.
//!
//! **THERE ARE NOW TWO IMPLEMENTATIONS AND ONLY ONE OF THEM IS REACHABLE —
//! updated 2026-08-31, and the paragraph this replaces said there was exactly
//! one.** `claude_code` is today's `claude.exe` spawn, moved behind this trait
//! and behaving identically. `native` is the neutral one: our own turn loop,
//! one HTTP call, nothing from Anthropic anywhere in it. It is compiled, wired
//! and tested, and `for_provider` keeps it behind `NATIVE_ENABLED`, which is
//! **off**.
//!
//! **`native` GREW A SECOND WIRE — added 2026-09-02, Phase 1 of the same
//! room-approved plan `NATIVE_ENABLED`'s own comment records.** `native::ollama`
//! is Ollama's own shape; `native::openai` is OpenAI's Chat Completions shape,
//! which also covers Gemini through its own OpenAI-compat endpoint (one row
//! kind, `openai-compatible` — see `native::openai`'s module doc and
//! `brain_setup.rs`'s tiles). Both are the same `Engine`: no tools, one HTTP
//! call, nothing from Anthropic. They are gated SEPARATELY —
//! `NATIVE_OPENAI_ENABLED` is its own switch, currently shut, with its own
//! reasoning below — because a local row losing its tools and an
//! openai-compatible row losing its tools (and its already-shipped
//! `adapter.rs` path) are two different trades and neither should move because
//! somebody flipped the other one.
//!
//! **AND A THIRD — added 2026-09-03.** `native::anthropic` speaks Anthropic's
//! own `/v1/messages` shape (`tool_use`/`tool_result`), proven against a real
//! Anthropic-shaped endpoint the same way `native::openai` was — see that
//! module's own header. Gated on its own switch, `NATIVE_ANTHROPIC_ENABLED`,
//! for the same reason `NATIVE_OPENAI_ENABLED` is separate from
//! `NATIVE_ENABLED`: each row kind's trade is its own to make. **This one is
//! moot today rather than merely cautious** — `providers.rs`'s own
//! `ROUTABLE_KINDS` does not include `anthropic-compatible` at all (Mark's
//! 2026-08-28 ruling), so opening this gate moves no real row until that
//! separate decision is made; see `NATIVE_ANTHROPIC_ENABLED`'s own doc.
//!
//! **WHY IT IS OFF, and it is not caution:** the native loop has no tools —
//! no `Read`, `Write`, `Edit`, `Bash`, `Grep` or `Glob`, and no MCP. Opening the
//! gate today would silently take file editing away from everyone on a local
//! brain, and a capability that vanishes without a word is worse than one that
//! was never offered. The design, the costing and the staging are in
//! `~/Documents/Tessa/2026-08-31-neutral-engine-design.md`.
//!
//! **THE TEST OF THIS STAGE IS BORING ON PURPOSE: the app must do exactly what
//! it did before, provably.** A refactor that changes behaviour is a refactor
//! that hid a bug. Everything in `claude_code.rs` is a transcription of what
//! `main.rs::send` did on 2026-08-31, argument order included, and
//! `tests::the_argv_is_exactly_what_the_pre_seam_send_built` pins that argument
//! vector against the pre-seam source so a later edit cannot quietly drop
//! `--verbose` or reorder a flag.
//!
//! ## The boundary, in the four terms this house states every boundary in
//!
//! `Engine::start` is a trust boundary in the sense that matters here: it is
//! the last place a decision is made before a real OS process runs with the
//! user's folder and their permission mode.
//!
//! - **Who may call it.** `main.rs::send`, and nothing else. It is not
//!   `#[tauri::command]`, has no HTTP surface, and is `pub(crate)`.
//! - **What happens when someone who isn't calls it.** Not representable —
//!   there is no path to it from outside this binary. The gate that *is* real
//!   sits one step earlier and stays there: `folder_trust::gate` refuses an
//!   unreviewed folder, and `providers::apply_env` refuses a kind this build
//!   cannot route. Neither moved, and neither may be re-decided in here.
//! - **What happens on malformed input.** Every field of `TurnRequest` is
//!   already validated by `send` before it is built (non-empty prompt, a real
//!   directory, a chosen provider). An engine that cannot start returns `Err`
//!   with a sentence, and **nothing is spawned and nothing is billed** — the
//!   same contract `apply_env` already holds.
//! - **What an error leaks.** Spawn errors carry an OS message and the two
//!   paths that were searched. **Never a key, never the prompt, never the
//!   contents of the folder.** `providers.rs` owns the redaction for anything
//!   that touches a secret and none of it passes through here.

use std::path::PathBuf;
use std::sync::Arc;

pub(crate) mod claude_code;
pub(crate) mod native;

/// The window's three-way permission control, as a **decision** rather than as
/// a vendor's flag string.
///
/// This is the smallest thing that proves the seam is real. `--permission-mode
/// acceptEdits` is Claude Code's vocabulary; the choice underneath it is the
/// product's. Keeping the vendor spelling in `claude_code.rs` is what lets a
/// second engine enforce the same three choices at its own dispatch point
/// without inheriting somebody else's flag names.
///
/// **ANYTHING UNRECOGNISED FALLS BACK TO THE STRICTEST OPTION**, carried over
/// word for word from the `permission_mode` this replaces: a typo must never
/// silently widen what the agent is allowed to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Permission {
    /// Ask before anything that changes the machine.
    Ask,
    /// File edits go through; everything else still asks.
    AcceptEdits,
    /// Everything goes through.
    Full,
}

impl Permission {
    /// The word the window sends over `invoke`. Three are recognised; the
    /// window deliberately does not expose a fourth (see the note that used to
    /// sit on `permission_mode` in main.rs — a plan mode that researches and
    /// refuses to act reads as the app being broken).
    pub(crate) fn from_window(choice: &str) -> Self {
        match choice {
            "edits" => Permission::AcceptEdits,
            "full" => Permission::Full,
            _ => Permission::Ask,
        }
    }
}

/// One turn's worth of intent, with every engine-agnostic decision already
/// made.
///
/// **WHAT IS IN HERE AND WHAT IS NOT IS THE WHOLE DESIGN.** Everything that
/// `send` computes about *this product* — the person's folder, their permission
/// choice, the voice and memory system prompt, which MCP servers are attached,
/// which tools are pre-approved — is in here and is engine-agnostic. Everything
/// about *how a particular engine is driven* — flag spellings, environment
/// variables, NDJSON on a pipe — is not, and lives in the implementation.
///
/// It is a plain struct with public fields rather than a builder: it is
/// constructed in exactly one place and consumed in exactly one place, and a
/// builder would only add a way for a field to be forgotten.
pub(crate) struct TurnRequest {
    /// What the person typed, already trimmed and already known non-empty.
    pub prompt: String,
    /// The folder they are working in, already known to exist and already
    /// through `folder_trust::gate`.
    pub workdir: PathBuf,
    pub permission: Permission,
    /// The voice card, the memory guidance and the session bridge, in that
    /// order — assembled by `send` because all three are the product's, not the
    /// engine's. An engine appends it to whatever it tells the model about its
    /// own tools; it never replaces that.
    pub system_prompt: String,
    /// The conversation to continue, if there is one. `None` starts fresh.
    pub resume: Option<String>,
    /// The generated MCP launch config, if it could be written. `None` means
    /// this turn runs without connectors and without memory — which is a
    /// degradation `send` has already decided to accept rather than refuse the
    /// person's words over.
    pub mcp_config: Option<PathBuf>,
    /// The secrets those MCP servers need, which are deliberately NOT in the
    /// config file. See `connectors::launch_config`.
    pub mcp_env: Vec<(String, String)>,
    /// Tools pre-approved for this turn — today, exactly this app's own two
    /// memory tools and nothing else. Empty means "whatever the engine's own
    /// default permission handling does", which is what shipped.
    pub allowed_tools: Vec<String>,
    /// Where this app keeps its own conversation transcripts.
    ///
    /// **IT IS THE APP'S DIRECTORY, NOT THE PERSON'S PROJECT FOLDER**, for the
    /// same reason `mcp-launch.json` is: it is ours, it is rewritten constantly,
    /// and generated files inside somebody's repository are litter they did not
    /// ask for and would eventually commit.
    ///
    /// `None` means the app could not work out where its own data lives.
    /// **That degrades the turn, it does not refuse it** — the person's words are
    /// worth more than our bookkeeping — and the engine says on screen that this
    /// conversation will not be remembered, because a model that silently forgets
    /// is the one failure this product exists to fix.
    ///
    /// `ClaudeCodeEngine` ignores it entirely: its transcripts are the vendor's,
    /// in the vendor's format, in the vendor's directory, reached by `--resume`.
    pub store_dir: Option<PathBuf>,
    /// The brain the person chose. An engine uses it to decide where the model
    /// call goes; `ClaudeCodeEngine` hands it straight to
    /// `providers::apply_env`.
    pub provider: crate::providers::Provider,
}

/// Where a turn's output goes.
///
/// **THIS IS THE FIVE-SHAPE CONTRACT, AND IT IS THE REASON THE FRONT END NEEDS
/// NO CHANGE.** `index.html` listens for exactly four Tauri events and parses
/// exactly five JSON shapes inside them; `main.rs` forwards lines untouched.
/// Any engine that can produce those events is drop-in. The method names here
/// say what a thing *means* rather than which pipe it came off, because a
/// native engine has no stdout and no stderr — but the events they map to keep
/// their existing names (`claude:event`, `claude:raw`, `claude:stderr`,
/// `claude:done`), because renaming them is a front-end change and this stage
/// makes none.
///
/// Implemented by `main.rs::AppSink` for real runs and by a recording sink in
/// the tests, which is how the stderr filter and the session-id capture became
/// testable at all — before this seam existed, both lived inside closures in a
/// `#[tauri::command]` and could only be exercised by running the app.
pub(crate) trait TurnSink: Send + Sync + 'static {
    /// One parsed line of the stream. The host is responsible for catching the
    /// session id off a `system` line, because that is a property of the
    /// contract rather than of any engine.
    fn event(&self, value: serde_json::Value);
    /// A line that was not JSON. Shown rather than eaten — a stream that
    /// silently drops what it cannot parse is a stream that hides the very
    /// failure worth seeing.
    fn raw(&self, line: String);
    /// A diagnostic that is not part of the conversation. The window paints
    /// these red, so an engine must not send it anything routine.
    fn failure(&self, line: String);
    /// The turn is over. Called exactly once, last, whatever happened — a run
    /// that ends without this leaves the window spinning forever.
    fn finished(&self, code: i32);

    /// **THE SAFE AGENCY LAYER'S PER-CALL GATE — added for `OpenUrl`,
    /// SAFE-AGENCY-SPEC.md room amendment 2, 2026-09-04.** Ask the person,
    /// right now, whether one specific action may proceed, and BLOCK the
    /// calling thread until they answer or a generous timeout passes. Every
    /// other method on this trait is fire-and-forget by design (see the
    /// five-shape contract above) — this is the one deliberate exception,
    /// because the room's ruling is that opening the person's own signed-in
    /// browser is not a one-time-opt-in action the way `LaunchApp`/
    /// `OpenSettingsPage` are.
    ///
    /// **THE DEFAULT IS `false` — DENY, NEVER ASK, THE SAME PHILOSOPHY
    /// `local-first-gate.py` AND `render-gate.py` ALREADY HOLD ON THIS BOX
    /// (see CLAUDE.md's own scars for both).** A sink with no real way to put
    /// this in front of a person — every sink except the real window's
    /// `AppSink` — must not silently wave an action through on its behalf;
    /// silence is refusal, not consent. `AppSink` is the only override, and it
    /// is the only implementation this method's contract is actually written
    /// for.
    fn confirm(&self, _prompt: &str) -> bool {
        false
    }
    fn confirm_cancellable(&self, prompt: &str, cancelled: &std::sync::atomic::AtomicBool) -> bool {
        !cancelled.load(std::sync::atomic::Ordering::SeqCst) && self.confirm(prompt)
    }

}

/// A turn that is currently running.
///
/// Deliberately tiny. The two things the app needs to know about a turn from
/// outside it are "is it still going" (the Send button refuses a second one)
/// and "stop it" (the Stop button, and closing the window). Everything else the
/// engine keeps to itself.
pub(crate) trait RunningTurn: Send + Sync {
    /// False once the turn has ended, for any reason, including cancellation.
    fn is_alive(&self) -> bool;
    /// Stop it now. **Must be idempotent and must succeed on a turn that has
    /// already finished** — `stop` is reachable from the Stop button and from
    /// closing the window, and both can arrive after the turn ended on its own.
    /// Returning `Err` there would put a red error on screen for a button that
    /// did exactly what it was asked.
    fn cancel(&self) -> Result<(), String>;
}

/// Starts turns.
pub(crate) trait Engine: Send + Sync {
    /// Which engine this is. Not shown to a customer.
    ///
    /// **Only a test calls it today, and that is the point of it** — the same
    /// standing as `providers::kind_is_routable`. "Which engine does this brain
    /// get" is a claim worth checking, and the alternative to a name is reading
    /// it off the concrete type, which stops being possible the moment there is
    /// more than one and they are both behind `Box<dyn Engine>`. It stops being
    /// dead the day a second engine exists and an error has to say which one
    /// refused.
    #[allow(dead_code)]
    fn label(&self) -> &'static str;

    /// Whether a stored conversation id belongs to this engine.
    ///
    /// **TWO ENGINES SHARE ONE `Session::session_id` SLOT AND THEIR IDS ARE NOT
    /// INTERCHANGEABLE.** `claude.exe --resume helloim-chat-…` fails outright,
    /// and a Claude Code session id arriving at the native engine would name one
    /// of our conversation files after somebody else's transcript. `send` asks
    /// this before passing `resume` in, so a foreign id starts a clean
    /// conversation instead of a broken one.
    ///
    /// **IT IS NOT COVERED BY `select_provider` CLEARING THE ID.** That handles
    /// somebody switching brains, which is the common path; it does not handle
    /// the active row being *edited* from one kind to another, which keeps the
    /// same id and changes the engine underneath it.
    ///
    /// Required rather than defaulted on purpose: a third engine has to state
    /// which ids are its own, and the answer is never "all of them".
    fn owns_session(&self, id: &str) -> bool;

    /// Whether this engine can actually give the model tools.
    ///
    /// **THE FAILURE IT EXISTS TO STOP IS `main.rs::send`'s OWN WARNING WEARING
    /// A DIFFERENT HAT: *"instructing tools that do not exist teaches the model
    /// to emit calls that go nowhere."*** `send` appends the memory guidance —
    /// prose telling the model to call `memory_search` and `memory_save` — and
    /// until this existed it did so whenever the MCP config had been written,
    /// which is a fact about a *file*, not about whether the engine driving the
    /// turn can reach it. The native engine has no MCP and no tools at all, so
    /// on that path every one of those instructions is a lie the model then
    /// tries to act on.
    fn supports_tools(&self) -> bool;

    /// Whether driving this engine means launching a program we did not write.
    ///
    /// **THIS IS WHAT MADE THE GATE MEAN ANYTHING, AND WITHOUT IT THE FLIP WAS
    /// COSMETIC.** `providers::test_provider` runs a second stage that spawns
    /// `claude.exe` — for *every* non-builtin row, local ones included. A row
    /// that fails the Test never turns green, `may_select` refuses a row that is
    /// not green, and `send` then answers `NO_BRAIN_YET`. So on 2026-08-31,
    /// with the engine gate open and no Claude Code installed, a person with
    /// Ollama running perfectly well could still not select their own machine as
    /// a brain: **the send path had been freed and the setup path still demanded
    /// the download the whole change exists to remove.**
    ///
    /// It is asked of the engine rather than computed from the kind on purpose.
    /// "Which rows need the binary" derived a second time, next to
    /// `for_provider`, is two encodings of one routing decision — the exact
    /// shape `providers::apply_env`'s header warns about.
    fn needs_vendor_binary(&self) -> bool;

    /// Does this engine already write its own conversation transcript
    /// somewhere `main.rs` can list it back from?
    ///
    /// **THE HISTORY TAB USED TO ANSWER THIS FOR ONE ENGINE ONLY, BY NEVER
    /// ASKING — Mark, 2026-09-05, running the shipped app: "history doesn't
    /// even work."** `engine::native` keeps its own store (`Plan::store_dir`,
    /// `store::Conversation::save`, inside its own `drive` loop) and always
    /// has. `ClaudeCodeEngine` never has — its transcripts belong to
    /// `claude.exe`'s own account-scoped storage, and `main.rs::
    /// conversations_dir`'s own doc used to call that "a real gap, said
    /// plainly rather than papered over." Plainly said is not the same as
    /// fixed: the built-in "claude" row is the brain every fresh install
    /// starts on, so for the common case the History tab was empty on every
    /// visit, forever, which is indistinguishable from broken to the person
    /// looking at it.
    ///
    /// **THE FIX IS GENERIC, ON PURPOSE, RATHER THAN "TEACH `ClaudeCodeEngine`
    /// TO WRITE `<id>.json` TOO.`main.rs::AppSink` ALREADY SEES EVERY TURN'S
    /// USER PROMPT AND FINAL ASSISTANT TEXT, ENGINE-AGNOSTIC** — it has to,
    /// to draw the window and speak the answer, via the identical five-shape
    /// event contract `engine::native`'s own header already documents as
    /// shared. So `AppSink` is where a SECOND engine's history gets captured,
    /// not a second copy of `store.rs`'s save logic pasted into
    /// `claude_code.rs`. This method is the only wire between the two: it
    /// tells `send` whether that generic capture is needed for the engine
    /// about to run, so a native turn — which already commits and trims and
    /// resumes its own store correctly, proven by `engine::native::tests`'s
    /// own suite — is never ALSO captured a second time from outside, which
    /// would silently discard everything the internal save already knows how
    /// to do (the trim budget, loading prior turns by resume id) and replace
    /// it with a single exchange.
    ///
    /// **Required, not defaulted — same reasoning `owns_session`'s own doc
    /// gives.** A third engine has to say which one it is; there is no safe
    /// guess for a brain nobody has answered this for yet.
    fn persists_own_history(&self) -> bool;

    /// Begin a turn. Returns as soon as the work is under way — output arrives
    /// on `sink`, never as a return value.
    ///
    /// **ON `Err`, NOTHING HAS BEEN STARTED AND NOTHING HAS BEEN BILLED**, and
    /// `sink` has not been called at all. That contract is what lets `send`
    /// return the error straight to the window as words instead of leaving a
    /// half-run to time out.
    fn start(
        &self,
        req: &TurnRequest,
        sink: Arc<dyn TurnSink>,
    ) -> Result<Box<dyn RunningTurn>, String>;
}

/// Is the neutral engine allowed to answer anything yet?
///
/// **A RECOMPILE, IN BOTH DIRECTIONS, IN THE STYLE OF `providers::OPENAI_ENABLED`
/// AND FOR THE SAME REASON: a switch a stray environment variable could flip is
/// not a switch.** This one decides whether a person's turn goes to somebody
/// else's binary or to our own loop, which is not a decision that should be
/// reachable from the environment of whatever launched the app.
///
/// **ON — OPENED 2026-08-31 ON MARK'S DECISION, AND WHAT IT COSTS IS WRITTEN
/// HERE RATHER THAN DISCOVERED IN THE FIELD.**
///
/// This was `false`, and the reason it was false was a real one: **the native
/// engine has no tools, so a local brain on this path cannot open, change, or
/// run anything.** No `Read`, `Write`, `Edit`, `Bash`, `Grep` or `Glob`, and no
/// MCP — which also means no connectors and no memory tools on a local turn.
///
/// **THE OBJECTION WAS ANSWERED, NOT OVERRULED.** Mark's words, verbatim:
/// *"we don't care what brain they connect it to. It's just a visualizer and a
/// voice. We don't care."* A brain that only talks is not a degraded NameOS
/// under that definition — it is NameOS. The product changed; the trade-off did
/// not, and it is stated in full above so that nobody has to re-derive it.
///
/// **WHAT A LOCAL-BRAIN USER GENUINELY LOSES**, said plainly because a
/// capability disappearing without a word is worse than one never offered: file
/// editing, shell commands, and connectors. What they gain is the whole point —
/// no 214 MB download from Anthropic, no terminal, no sign-in, and a machine
/// that answers with nothing of anyone else's running on it.
///
/// It stays a recompile in both directions, in the style of
/// `providers::OPENAI_ENABLED` and for the same reason: a switch a stray
/// environment variable could flip is not a switch. **Setting this back to
/// `false` is the one-line rollback**, and `with_the_gate_shut_nothing_moves`
/// keeps that path proven rather than hoped for.
const NATIVE_ENABLED: bool = true;

/// Is the OpenAI-shaped native path allowed to answer anything yet?
///
/// **A SEPARATE SWITCH FROM `NATIVE_ENABLED`, DELIBERATELY — this is Mason's
/// call, made explicit rather than assumed, on the brief that asked for this
/// wire to be "gated the same compile-time-const + tri-state-test-override +
/// one-line-rollback way `NATIVE_ENABLED` gates local today."** Read literally
/// that could mean routing `openai-compatible` through the SAME const —
/// which, since `NATIVE_ENABLED` is already `true`, would move every existing
/// OpenAI and Gemini row onto this engine the moment this file compiled, with
/// no separate decision taken anywhere. That is a materially different trade
/// from the one `NATIVE_ENABLED` itself records: a `local` row moving loses
/// tools nobody was paying a third party for; an `openai-compatible` row
/// moving loses tools on a turn somebody's own key is billing, AND stops
/// going through `adapter.rs`, which is shipped, gated on `OPENAI_ENABLED`
/// (a THIRD switch, already on) and has been driving real Claude Code turns
/// since 2026-08-29. Collapsing three independent decisions onto one bit was
/// never what "the same way" was likely to mean, and "gated the same ... way"
/// reads more naturally as the same PATTERN — a compile-time const, a
/// tri-state test override, a one-line rollback — than as literally the same
/// variable. So: **the pattern is copied exactly; the switch is its own.**
/// This keeps `NATIVE_ENABLED` (local) and this gate (openai-compatible)
/// independently reversible, which they need to be — the room's own review
/// asked for Phase 1 "rigorously scoped to conversational parity" as its own
/// increment, and a shared switch would make it un-shippable without also
/// re-deciding local, which nobody asked to re-open.
///
/// **IT WAS OFF, on purpose, and here is what turning it on cost.**
/// `adapter.rs`'s own header says its translator "has never met a real OpenAI
/// endpoint" from THIS process — every one of its tests is synthetic; going
/// green there only ever proved the person's own machine could reach the
/// person's own key. This wire is the opposite: its request and response
/// shapes ARE checked against real endpoints (`native::openai`'s module doc
/// names the tests and what each proved), but only from Linux, against
/// `api.openai.com` and `openrouter.ai` directly — **never through this
/// app**, because helloim.ai ships Windows only and nobody had yet run a real
/// Windows build with this gate open against a live key end to end. Flipping
/// this on ships an unverified path to every OpenAI and Gemini row on the
/// belief that a wire proven correct on Linux behaves identically inside a
/// Windows Tauri binary — plausible, and exactly the kind of belief this
/// house does not ship on.
///
/// **THAT VERIFICATION PASS HAD NO WAY TO PRODUCE A GATE-ON BINARY —
/// found by Beck, fixed by Mason, 2026-09-02.** Before this, the only two
/// things that could make `native_openai_enabled()` return `true` were this
/// const itself and the `#[cfg(test)]` override below, and the override's own
/// doc says plainly there is no production path to it — it does not exist in
/// a release build. So the sole lever on a real, running, gate-ON Windows GUI
/// app WAS flipping this const, which is exactly the thing a verification
/// pass must not do to itself: it would be shipping the const's own default
/// changed, not testing a build that can be thrown away. `native_openai`
/// (see `Cargo.toml`'s `[features]`) is the missing lever — see
/// `native_openai_enabled` below for how it plugs in and the exact command
/// to run.
///
/// **ON — SET TO `true` BY MARK, 2026-09-02/03, COMMIT `753eb45`, AS A
/// STOPGAP AND NOT AS THE END OF THIS DECISION.** The room found the shipped
/// build routing every provider through Claude Code by default regardless —
/// the exact failure this whole gate exists to end — and the fastest way to
/// stop it reaching a customer was flipping this const directly rather than
/// waiting on the `native_openai` feature-build verification pass above. **It
/// is still true that nobody has proven this wire end to end inside a real
/// Windows Tauri binary against a live key**, so the Windows verification
/// pass this doc has always called for has not been retired by this flip —
/// it is Beck's, and it is still owed. Turning this back to `false` is the
/// rollback if that pass finds the wire wrong in the field, exactly as
/// before; the difference is only that today the const carries the risk
/// openly instead of a build flag nobody was passing.
const NATIVE_OPENAI_ENABLED: bool = true;

/// Is the Anthropic-shaped native path allowed to answer anything yet?
///
/// **A THIRD, INDEPENDENT SWITCH — added 2026-09-03, same pattern
/// `NATIVE_OPENAI_ENABLED`'s own doc records and the same reason: routing
/// `anthropic-compatible` through either of the other two gates would move a
/// row nobody decided to move the moment this file compiled.**
///
/// **OFF, and it is not caution — it is a decision this file cannot make.**
/// `native::anthropic::AnthropicWire` was proven end to end against a real
/// Anthropic-shaped endpoint (OpenRouter's own — see that module's header
/// for exactly what ran and what it does and does not establish), the same
/// standard `NATIVE_OPENAI_ENABLED` itself was held to before it opened. But
/// **`providers.rs`'s own `ROUTABLE_KINDS` does not include
/// `anthropic-compatible` at all** — Mark's 2026-08-28 ruling removed the
/// kind from the app entirely, and restoring it is a separate, unmade
/// decision that belongs to whoever owns that call, not to this engine
/// having a working wire. Opening this gate today would not put a single
/// real row anywhere different, because no row of this kind can exist —
/// `for_provider` would simply have a dead branch. It is left in place, gated
/// shut, for the day the kind comes back, so that day is a `providers.rs`
/// change and a flag flip rather than a fresh translation layer.
const NATIVE_ANTHROPIC_ENABLED: bool = false;

/// In tests only, the gate can be forced either way, so **both** sides of it
/// execute whichever way the const is currently set.
///
/// **IT IS TRI-STATE AND THAT IS NOT TIDINESS — corrected when the gate opened.**
/// It used to be an `AtomicBool` that could only force the gate ON, which was
/// correct while the const was `false` and became useless the moment it was
/// `true`: the shut path — which is the rollback — would have had no way to be
/// tested at all, and the test covering it would have had to be deleted. **A
/// one-directional override quietly deletes a test the day the default flips**,
/// which is precisely how a guard dies.
///
/// **There is no production path to this** — it does not exist in a release
/// build.
///
/// **ONE MODULE, THREE INDEPENDENT SWITCHES — grew a third, 2026-09-03, same
/// reasoning each time it grows.** `STATE` overrides `NATIVE_ENABLED`
/// (local); `OPENAI_STATE` overrides `NATIVE_OPENAI_ENABLED`
/// (openai-compatible); `ANTHROPIC_STATE` overrides `NATIVE_ANTHROPIC_ENABLED`
/// (anthropic-compatible). Sharing the module rather than the atomic keeps
/// the tri-state pattern in one place while letting a test force one gate
/// without disturbing the others — see `with_the_two_native_gates_open_and_
/// shut_independently_of_each_other` for the proof that STATE and
/// OPENAI_STATE do not interact, and
/// `the_anthropic_gate_is_independent_of_the_other_two` for the same proof
/// extended to this one.
#[cfg(test)]
mod gate_override {
    use std::sync::atomic::AtomicU8;
    pub(super) const UNSET: u8 = 0;
    pub(super) const FORCED_ON: u8 = 1;
    pub(super) const FORCED_OFF: u8 = 2;
    pub(super) static STATE: AtomicU8 = AtomicU8::new(UNSET);
    pub(super) static OPENAI_STATE: AtomicU8 = AtomicU8::new(UNSET);
    pub(super) static ANTHROPIC_STATE: AtomicU8 = AtomicU8::new(UNSET);
}

fn native_enabled() -> bool {
    #[cfg(test)]
    {
        use std::sync::atomic::Ordering::Relaxed;
        match gate_override::STATE.load(Relaxed) {
            gate_override::FORCED_ON => return true,
            gate_override::FORCED_OFF => return false,
            _ => {}
        }
    }
    NATIVE_ENABLED
}

/// Three ways this can come back `true`, checked in this order, and only the
/// first two are test-only or opt-in — the third is what ships:
///
/// 1. The `#[cfg(test)]` override (`gate_override::OPENAI_STATE`) — a unit
///    test forcing one side of the gate, no production path, unchanged by
///    the addition below.
/// 2. **The `native-openai` cargo feature — the real build-time lever, added
///    2026-09-02 so a verification pass never has to touch the const.**
///    `cfg!(feature = "native-openai")` is resolved at COMPILE time: with the
///    feature off (the default — it is not in any `default = [...]` list,
///    and there is no such list), this collapses to a no-op and the function
///    behaves exactly as it did before. With it on, the gate opens in that
///    one binary without moving `NATIVE_OPENAI_ENABLED` and without needing
///    `#[cfg(test)]` at all, so the artifact is an ordinary release build —
///    installable, uninstallable, launchable twice — not a test harness
///    wearing a GUI. **The build Beck runs for a gate-ON Windows binary:**
///    ```text
///    cargo xwin build --release --target x86_64-pc-windows-msvc \
///      --features native-openai
///    ```
///    That is `ship-windows.sh`'s own `cargo xwin build` line with one flag
///    added — everything else about the build (the DirectML staging, the
///    installer packaging) is unchanged, because this only ever touches
///    which engine one provider kind runs on.
/// 3. `NATIVE_OPENAI_ENABLED`, the shipping default — `true` (opened after native OpenAI's Windows GO; one-line revert to `false` if ever needed) — for every
///    build that names neither of the above.
fn native_openai_enabled() -> bool {
    #[cfg(test)]
    {
        use std::sync::atomic::Ordering::Relaxed;
        match gate_override::OPENAI_STATE.load(Relaxed) {
            gate_override::FORCED_ON => return true,
            gate_override::FORCED_OFF => return false,
            _ => {}
        }
    }
    cfg!(feature = "native-openai") || NATIVE_OPENAI_ENABLED
}

/// `native_openai_enabled`'s own twin, for `NATIVE_ANTHROPIC_ENABLED`. Same
/// three-way check, same `native-anthropic` cargo feature as the real
/// build-time lever — kept for whenever `providers.rs` restores the kind,
/// so that day needs no new gating mechanism, only a flag flip.
fn native_anthropic_enabled() -> bool {
    #[cfg(test)]
    {
        use std::sync::atomic::Ordering::Relaxed;
        match gate_override::ANTHROPIC_STATE.load(Relaxed) {
            gate_override::FORCED_ON => return true,
            gate_override::FORCED_OFF => return false,
            _ => {}
        }
    }
    cfg!(feature = "native-anthropic") || NATIVE_ANTHROPIC_ENABLED
}

/// Which engine runs a given brain.
///
/// **ONE SELECTION POINT, AND THIS IS IT.** The alternative is the choice
/// arriving as an `if` somewhere inside `send`, which is exactly the shape
/// `providers::apply_env`'s own header warns about: a decision spread across
/// call sites is a decision that comes back through whichever one somebody
/// forgets.
///
/// With both gates shut every brain gets the engine that shipped, which is
/// the promise made when the seam was built: no existing install's provider
/// row changes meaning. With `NATIVE_ENABLED` open, a **local** row moves.
/// With `NATIVE_OPENAI_ENABLED` open too, an **openai-compatible** row moves
/// as well — OpenAI and Gemini, per `brain_setup.rs`'s own tiles; see that
/// gate's doc for why it defaults to shut independently of the other one. A
/// Claude subscription NEVER moves: it is billed through OAuth that is
/// permitted inside Claude Code and nowhere else, so that row keeps its
/// engine whatever either gate does.
pub(crate) fn for_provider(p: &crate::providers::Provider) -> Box<dyn Engine> {
    if native_enabled() && p.kind == "local" {
        return Box::new(native::NativeEngine::ollama());
    }
    if native_openai_enabled() && p.kind == "openai-compatible" {
        return Box::new(native::NativeEngine::openai_compatible());
    }
    if native_anthropic_enabled() && p.kind == "anthropic-compatible" {
        return Box::new(native::NativeEngine::anthropic_compatible());
    }
    Box::new(claude_code::ClaudeCodeEngine)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The three words the window sends, and the one rule that matters about
    /// the fourth: **anything unrecognised is the strictest option.** Carried
    /// from `permission_mode`, whose own comment said a typo must never
    /// silently widen what the agent may do. The mapping to Claude Code's flag
    /// spellings is checked separately, in `claude_code`.
    #[test]
    fn an_unknown_permission_word_is_the_strictest_one() {
        assert_eq!(Permission::from_window("edits"), Permission::AcceptEdits);
        assert_eq!(Permission::from_window("full"), Permission::Full);
        assert_eq!(Permission::from_window("ask"), Permission::Ask);
        // The ones that matter: a typo, an empty string, and a word from a
        // future version of the window that this build has never heard of.
        assert_eq!(Permission::from_window("edit"), Permission::Ask);
        assert_eq!(Permission::from_window(""), Permission::Ask);
        assert_eq!(Permission::from_window("bypassPermissions"), Permission::Ask);
        assert_eq!(Permission::from_window("FULL"), Permission::Ask);
    }

    fn a_row(kind: &str) -> crate::providers::Provider {
        crate::providers::Provider {
            id: "row".into(),
            kind: kind.into(),
            name: "row".into(),
            base_url: String::new(),
            model: String::new(),
            builtin: kind == "claude",
            disconnected: false,
            connected: false,
            checked_at: String::new(),
            last_error: String::new(),
            has_secret: false,
            caveat: String::new(),
            migration_note: String::new(),
            allow_shell: false,
            allow_agency: false,
        }
    }

    /// Flip the gate for one test, in the house pattern (`providers.rs`'s
    /// `with_openai_enabled`). **Serialised, because the tests run in parallel
    /// and a shared switch is a race** — and restored even if the body panics,
    /// or one failure would cascade into every test that reads the gate
    /// afterwards. **Every test that reads the gate takes this lock, including
    /// the one that asserts it is shut**, which is the part the pattern it is
    /// copied from does not do.
    fn with_gate<T>(open: bool, f: impl FnOnce() -> T) -> T {
        use std::sync::atomic::Ordering::SeqCst;
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        gate_override::STATE.store(
            if open { gate_override::FORCED_ON } else { gate_override::FORCED_OFF },
            SeqCst,
        );
        let out = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
        gate_override::STATE.store(gate_override::UNSET, SeqCst);
        drop(guard);
        match out {
            Ok(v) => v,
            Err(p) => std::panic::resume_unwind(p),
        }
    }

    /// **WITH THE GATE OPEN, EXACTLY ONE KIND MOVES.** This is the half of the
    /// switch that would otherwise never execute — a gated engine nobody has
    /// ever selected is unfinished code with a switch beside it, not a gated
    /// engine.
    ///
    /// The Claude row staying put is the load-bearing assertion: a subscription
    /// is billed through OAuth that is permitted inside Claude Code and nowhere
    /// else, so routing it to a native engine would break billing for everybody
    /// on the built-in row.
    ///
    /// Proven able to fail: dropping the `p.kind == "local"` condition from
    /// `for_provider` sends the claude row to the native engine here.
    #[test]
    fn with_the_gate_open_only_a_local_row_changes_engine() {
        // `with_openai_gate(false, ...)` and `with_anthropic_gate(false,
        // ...)` pin the OTHER TWO gates shut explicitly rather than trusting
        // them to default there — since `native-openai` and
        // `native-anthropic` (cargo features) can each make their own
        // default `true` for the whole test binary, a test about the LOCAL
        // gate must not silently depend on what either other one happens to
        // be.
        with_openai_gate(false, || {
            with_anthropic_gate(false, || {
                with_gate(true, || {
                    assert_eq!(for_provider(&a_row("local")).label(), "native-ollama");
                    for kind in ["claude", "openai-compatible", "anthropic-compatible", "wat"] {
                        assert_eq!(
                            for_provider(&a_row(kind)).label(),
                            "claude-code",
                            "the {kind} row must not be moved by the local-brain gate"
                        );
                    }
                });
            });
        });
    }

    /// **NEITHER ENGINE WILL ADOPT THE OTHER'S CONVERSATION ID**, and this is
    /// the pair asserted together — the pin is that no id is owned by both, and
    /// no id claimed by neither is passed along by `send`.
    #[test]
    fn the_two_engines_own_disjoint_session_ids() {
        let claude = for_provider(&a_row("claude"));
        let ours = with_gate(true, || for_provider(&a_row("local")));

        let native_id = native::store::new_id();
        let claude_id = "6f1b0a2c-9d3e-4f55-8a1b-2c3d4e5f6071";

        assert!(ours.owns_session(&native_id));
        assert!(!claude.owns_session(&native_id), "claude.exe --resume would fail on this id");
        assert!(claude.owns_session(claude_id));
        assert!(!ours.owns_session(claude_id), "a foreign id must not name one of our files");
    }

    /// The tool question, which is what `send` uses to decide whether to tell
    /// the model about tools it may not have.
    #[test]
    fn only_the_engine_with_tools_says_it_has_them() {
        assert!(for_provider(&a_row("claude")).supports_tools());
        assert!(!native::NativeEngine::ollama().supports_tools());
    }

    /// **WHAT THIS BUILD ACTUALLY SHIPS, ASSERTED AGAINST THE CONST ITSELF AND
    /// NOT AGAINST A FORCED OVERRIDE.** It takes no override at all, so it reads
    /// `NATIVE_ENABLED` exactly as a customer's build does.
    ///
    /// **IT REPLACES `with_the_gate_shut_every_kind_gets_the_engine_that_shipped`,
    /// WHICH WAS WRITTEN TO FAIL THE DAY THE CONST FLIPPED — and then the const
    /// flipped.** That test did its job: it made opening the gate a decision
    /// somebody had to take deliberately, in this file, rather than discover in
    /// the field. **It was rewritten to assert the new truth rather than deleted,
    /// because a guard that gets removed the moment it becomes inconvenient was
    /// never a guard.** The shut path it used to cover is still covered, one test
    /// down, through the override's new off position.
    ///
    /// So this now fails if somebody flips the const **back** without meaning
    /// to — the tripwire still exists, pointing the other way.
    #[test]
    fn this_build_sends_a_local_brain_to_the_native_engine() {
        assert!(
            NATIVE_ENABLED,
            "the gate was opened on 2026-08-31 on Mark's decision; closing it is a \
             product decision and belongs here, deliberately"
        );
        assert_eq!(for_provider(&a_row("local")).label(), "native-ollama");
        assert_eq!(for_provider(&a_row("claude")).label(), "claude-code");
    }

    /// **THE ROLLBACK, PROVEN RATHER THAN HOPED FOR.** Setting `NATIVE_ENABLED`
    /// back to `false` is the one-line remedy if the native engine turns out
    /// wrong in the field, and a rollback nobody has ever executed is not a
    /// rollback. This is the shut path, still exercised.
    ///
    /// The kinds below are the ones `providers.rs` can route, plus the retired
    /// spellings a `providers.json` written by an older build can still carry —
    /// a row this build refuses must still not be handed a different engine on
    /// the way to being refused.
    #[test]
    fn with_the_gate_shut_nothing_moves() {
        // Same reasoning as `with_the_gate_open_only_a_local_row_changes_
        // engine`'s own comment: this test is about the LOCAL gate, so the
        // OTHER TWO are pinned shut explicitly rather than trusted to
        // default there under every possible feature combination.
        with_openai_gate(false, || {
            with_anthropic_gate(false, || {
                with_gate(false, || {
                    for kind in ["claude", "local", "openai-compatible", "anthropic-compatible", "wat"] {
                        assert_eq!(
                            for_provider(&a_row(kind)).label(),
                            "claude-code",
                            "with the gate shut the {kind} row must run on the engine that shipped"
                        );
                    }
                });
            });
        });
    }

    /// **THE FLIP IS COSMETIC WITHOUT THIS.** `providers::test_provider` asks
    /// the engine whether a vendor binary is involved before it goes looking for
    /// one; if the native engine ever answered `true` here, a local row would
    /// again fail its Test on a machine with no Claude Code, never turn green,
    /// never be selectable, and every send would answer `NO_BRAIN_YET` — with
    /// the send path working perfectly the whole time.
    #[test]
    fn only_the_vendor_engine_admits_to_needing_a_vendor_binary() {
        assert!(for_provider(&a_row("claude")).needs_vendor_binary());
        assert!(!native::NativeEngine::ollama().needs_vendor_binary());
        // And through the real selector, the way `test_provider` asks it.
        assert!(!for_provider(&a_row("local")).needs_vendor_binary());
    }

    // -- The OpenAI-shaped native gate — its own switch, its own tests, same
    // -- pattern as the local one above and deliberately not sharing a lock
    // -- with it (see `with_the_two_native_gates_open_and_shut_independently_
    // -- of_each_other`, which is the test that would catch it if they did).

    /// `with_gate`'s twin for `OPENAI_STATE`. A SEPARATE `Mutex`, not the one
    /// `with_gate` uses — sharing a lock across two independent switches would
    /// serialise tests that have no reason to wait on each other, and would
    /// hide the one bug that actually matters here: the two gates reading each
    /// other's state by accident.
    fn with_openai_gate<T>(open: bool, f: impl FnOnce() -> T) -> T {
        use std::sync::atomic::Ordering::SeqCst;
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        gate_override::OPENAI_STATE.store(
            if open { gate_override::FORCED_ON } else { gate_override::FORCED_OFF },
            SeqCst,
        );
        let out = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
        gate_override::OPENAI_STATE.store(gate_override::UNSET, SeqCst);
        drop(guard);
        match out {
            Ok(v) => v,
            Err(p) => std::panic::resume_unwind(p),
        }
    }

    /// **WITH THE OPENAI GATE OPEN, EXACTLY ONE KIND MOVES**, mirroring
    /// `with_the_gate_open_only_a_local_row_changes_engine` for the other
    /// switch. `local` is FORCED SHUT here (nested inside `with_gate(false,
    /// …)`) rather than left to read the live `NATIVE_ENABLED` const —
    /// without that, this test's result would depend on the OTHER gate's
    /// current default, which is not what it exists to prove and would make
    /// it pass or fail for the wrong reason the day that default changes.
    /// With local forced shut, `local` staying on `claude-code` while
    /// `openai-compatible` alone moves is the assertion that would catch the
    /// two gates being merged into one `if` by mistake. `claude` staying put
    /// is the same billing argument as the local gate's own test: an OAuth
    /// subscription never moves for either switch.
    ///
    /// Proven able to fail: dropping the `p.kind == "openai-compatible"`
    /// condition from `for_provider`'s second `if` sends every kind to the
    /// native OpenAI engine here.
    #[test]
    fn with_the_openai_gate_open_only_an_openai_compatible_row_changes_engine() {
        // `with_anthropic_gate(false, ...)` pins the THIRD gate shut
        // explicitly for the same reason `with_gate(false, ...)` already
        // pins the local one — `native-anthropic` (cargo feature) can make
        // its own default `true` for the whole test binary, and this test
        // is about the OPENAI gate.
        with_gate(false, || {
            with_anthropic_gate(false, || {
                with_openai_gate(true, || {
                    assert_eq!(for_provider(&a_row("openai-compatible")).label(), "native-openai");
                    for kind in ["claude", "local", "anthropic-compatible", "wat"] {
                        assert_eq!(
                            for_provider(&a_row(kind)).label(),
                            "claude-code",
                            "the {kind} row must not be moved by the openai-native gate"
                        );
                    }
                });
            });
        });
    }

    /// **WHAT THIS BUILD ACTUALLY SHIPS FOR THE OPENAI-SHAPED ROW, ASSERTED
    /// AGAINST THE CONST ITSELF AND NOT AGAINST A FORCED OVERRIDE** — same
    /// shape as `this_build_sends_a_local_brain_to_the_native_engine` above,
    /// for the same reason.
    ///
    /// **IT REPLACES `this_build_sends_an_openai_compatible_row_to_claude_
    /// code_because_the_gate_is_shut`, WHICH WAS WRITTEN TO FAIL THE DAY THE
    /// CONST FLIPPED — and then the const flipped, 2026-09-02/03, `753eb45`,
    /// exactly like `NATIVE_ENABLED` did on 2026-08-31.** That test did its
    /// job: it made opening this gate a decision somebody had to take
    /// deliberately, in this file, rather than discover in the field. It is
    /// rewritten to assert the new truth rather than deleted, for the same
    /// reason the local one was: a guard removed the moment it becomes
    /// inconvenient was never a guard. The shut path is still covered, one
    /// test down, through the override's off position.
    ///
    /// So this now fails if somebody flips the const **back** without
    /// meaning to — the tripwire still exists, pointing the other way.
    ///
    /// **`#[cfg(not(feature = "native-openai"))]` — this test's whole premise
    /// is what happens with the FEATURE not forcing anything**, so the answer
    /// it proves is the const's own. Its exact complement, `the_native_openai_
    /// feature_still_opens_the_gate_now_the_const_already_does` below, is
    /// `#[cfg(feature = "native-openai")]` — between the two, every build
    /// this crate can produce runs exactly one of them, never both and never
    /// neither. Both now expect the SAME outcome, because the const being
    /// `true` already makes the feature a no-op rather than a lever — the
    /// split stays because the feature remains the rollback's own backstop:
    /// if the const is ever reverted to `false` in the field, a build with
    /// this feature on still ships with the gate open while that is sorted
    /// out, and a build without it does not.
    #[cfg(not(feature = "native-openai"))]
    #[test]
    fn this_build_sends_an_openai_compatible_row_to_the_native_engine() {
        assert!(
            NATIVE_OPENAI_ENABLED,
            "the gate was opened as a stopgap on 2026-09-02/03 on Mark's decision after the \
             shipped build was found routing every provider through Claude Code regardless; \
             closing it is a product decision and belongs here, deliberately"
        );
        assert_eq!(for_provider(&a_row("openai-compatible")).label(), "native-openai");
    }

    /// Same forced-local-shut reasoning as the test above: `local`'s own
    /// routing is not this gate's to answer for, so it is pinned to a known
    /// state rather than left to read whatever `NATIVE_ENABLED` happens to be
    /// today.
    #[test]
    fn with_the_openai_gate_shut_nothing_moves() {
        with_gate(false, || {
            with_anthropic_gate(false, || {
                with_openai_gate(false, || {
                    for kind in
                        ["claude", "local", "openai-compatible", "anthropic-compatible", "wat"]
                    {
                        assert_eq!(
                            for_provider(&a_row(kind)).label(),
                            "claude-code",
                            "with the openai-native gate shut the {kind} row must run on the \
                             engine that shipped"
                        );
                    }
                });
            });
        });
    }

    /// **THE TEST THE WHOLE SEPARATE-SWITCH DESIGN IS FOR.** Both gates are
    /// forced at once, in the two combinations `with_the_openai_gate_open_
    /// only_an_openai_compatible_row_changes_engine` alone cannot reach:
    /// local open while openai stays shut (today's actual shipped
    /// combination), and both open together. If the two switches were ever
    /// merged onto one `AtomicU8`, or `for_provider`'s two `if`s were ever
    /// collapsed into one condition, one of these four assertions would move.
    #[test]
    fn the_two_native_gates_open_and_shut_independently_of_each_other() {
        with_gate(true, || {
            with_openai_gate(false, || {
                assert_eq!(for_provider(&a_row("local")).label(), "native-ollama");
                assert_eq!(for_provider(&a_row("openai-compatible")).label(), "claude-code");
            });
        });
        with_gate(false, || {
            with_openai_gate(true, || {
                assert_eq!(for_provider(&a_row("local")).label(), "claude-code");
                assert_eq!(for_provider(&a_row("openai-compatible")).label(), "native-openai");
            });
        });
        with_gate(true, || {
            with_openai_gate(true, || {
                assert_eq!(for_provider(&a_row("local")).label(), "native-ollama");
                assert_eq!(for_provider(&a_row("openai-compatible")).label(), "native-openai");
            });
        });
    }

    /// **THE FEATURE LEVER ITSELF, PROVEN — added 2026-09-02, updated
    /// 2026-09-03 when the const it used to be independent of stopped being
    /// `false`.** Only compiled when the feature is actually on, so a plain
    /// `cargo test` never runs this and every test above it is untouched by
    /// its existence. It used to prove the feature opens the gate "on its
    /// own... without moving `NATIVE_OPENAI_ENABLED`, which stays asserted
    /// `false`" — that was true the day this was written and stopped being
    /// true the day Mark flipped the const itself. **What it proves now:
    /// with the feature on, the gate answers open for the same reason the
    /// const already gives it — the feature is no longer doing independent
    /// work, and this test says so rather than asserting a `false` that
    /// would fail the moment somebody actually built with this feature.**
    /// The feature stays compiled and stays tested because it is still the
    /// rollback's own backstop — see the doc on the test above.
    #[cfg(feature = "native-openai")]
    #[test]
    fn the_native_openai_feature_still_opens_the_gate_now_the_const_already_does() {
        assert!(
            NATIVE_OPENAI_ENABLED,
            "the const opened on 2026-09-02/03; this feature build must not depend on it \
             ever having been false"
        );
        assert!(native_openai_enabled(), "open, whether from the const or the feature");
        assert_eq!(for_provider(&a_row("openai-compatible")).label(), "native-openai");
    }

    /// The tool and vendor-binary questions, for the second backend — same
    /// contract `only_the_engine_with_tools_says_it_has_them` and
    /// `only_the_vendor_engine_admits_to_needing_a_vendor_binary` already
    /// pin for the first one. Neither answer is allowed to depend on which
    /// backend it is: both are properties of `NativeEngine` as a whole (see
    /// that struct's own doc comment for why).
    #[test]
    fn the_openai_native_engine_answers_the_same_as_the_ollama_one() {
        assert!(!native::NativeEngine::openai_compatible().supports_tools());
        assert!(!native::NativeEngine::openai_compatible().needs_vendor_binary());
        assert_eq!(
            native::NativeEngine::openai_compatible().supports_tools(),
            native::NativeEngine::ollama().supports_tools(),
        );
    }

    /// **THE THREE NATIVE BACKENDS SHARE ONE ID NAMESPACE, ON PURPOSE —
    /// extended to the third one, 2026-09-03, same reasoning each time
    /// another backend is added.** Unlike `the_two_engines_own_disjoint_
    /// session_ids` — which pins that Claude Code and `native` must NEVER
    /// adopt each other's ids — this pins the opposite for the backends
    /// INSIDE `native`: all mint ids through the same `store::new_id`, all
    /// store conversations in the same `store_dir`, and a person switching
    /// their row between providers mid-conversation should not lose the
    /// thread over an id no engine recognises as its own.
    #[test]
    fn the_three_native_backends_share_one_session_id_namespace() {
        let id = native::store::new_id();
        assert!(native::NativeEngine::ollama().owns_session(&id));
        assert!(native::NativeEngine::openai_compatible().owns_session(&id));
        assert!(native::NativeEngine::anthropic_compatible().owns_session(&id));
    }

    // -- The Anthropic-shaped native gate — its own switch, its own tests,
    // -- same pattern as the OpenAI one above and deliberately not sharing a
    // -- lock with either of the other two (see
    // -- `the_anthropic_gate_is_independent_of_the_other_two`, the test that
    // -- would catch it if it did).

    /// `with_gate`'s and `with_openai_gate`'s twin for `ANTHROPIC_STATE`. A
    /// separate `Mutex` for the same reason those two do not share one:
    /// three independent switches serialised on one lock would hide the
    /// exact bug this test file exists to catch, two gates reading each
    /// other's state by accident.
    fn with_anthropic_gate<T>(open: bool, f: impl FnOnce() -> T) -> T {
        use std::sync::atomic::Ordering::SeqCst;
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        gate_override::ANTHROPIC_STATE.store(
            if open { gate_override::FORCED_ON } else { gate_override::FORCED_OFF },
            SeqCst,
        );
        let out = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
        gate_override::ANTHROPIC_STATE.store(gate_override::UNSET, SeqCst);
        drop(guard);
        match out {
            Ok(v) => v,
            Err(p) => std::panic::resume_unwind(p),
        }
    }

    /// **WITH THE ANTHROPIC GATE OPEN, EXACTLY ONE KIND MOVES**, mirroring
    /// `with_the_openai_gate_open_only_an_openai_compatible_row_changes_
    /// engine`. Both other gates are forced shut explicitly rather than left
    /// to read their live defaults, for the same reason those tests give:
    /// this test's result must not depend on what either OTHER gate
    /// currently defaults to.
    ///
    /// Proven able to fail: dropping the `p.kind == "anthropic-compatible"`
    /// condition from `for_provider`'s third `if` sends every kind to the
    /// native Anthropic engine here.
    #[test]
    fn with_the_anthropic_gate_open_only_an_anthropic_compatible_row_changes_engine() {
        with_gate(false, || {
            with_openai_gate(false, || {
                with_anthropic_gate(true, || {
                    assert_eq!(
                        for_provider(&a_row("anthropic-compatible")).label(),
                        "native-anthropic"
                    );
                    for kind in ["claude", "local", "openai-compatible", "wat"] {
                        assert_eq!(
                            for_provider(&a_row(kind)).label(),
                            "claude-code",
                            "the {kind} row must not be moved by the anthropic-native gate"
                        );
                    }
                });
            });
        });
    }

    /// **THE ROLLBACK FOR THIS GATE, PROVEN THE SAME WAY THE OTHER TWO ARE.**
    /// `NATIVE_ANTHROPIC_ENABLED` ships `false`; this is what a customer's
    /// build actually does today, asserted against the const itself rather
    /// than a forced override. Same `#[cfg(not(feature = ...))]` /
    /// `#[cfg(feature = ...)]` split as the OpenAI gate's own pair, for the
    /// identical reason: every build this crate can produce runs exactly one
    /// of the two, never both and never neither.
    #[cfg(not(feature = "native-anthropic"))]
    #[test]
    fn this_build_sends_an_anthropic_compatible_row_to_claude_code_because_the_gate_is_shut() {
        assert!(
            !NATIVE_ANTHROPIC_ENABLED,
            "the anthropic-native gate ships shut -- providers.rs cannot currently route this \
             kind at all, see this const's own doc for why"
        );
        assert_eq!(for_provider(&a_row("anthropic-compatible")).label(), "claude-code");
    }

    /// Same forced-shut reasoning as the test above: the OTHER two gates'
    /// routing is not this gate's to answer for, so both are pinned to a
    /// known state rather than left to read whatever they happen to default
    /// to today.
    #[test]
    fn with_the_anthropic_gate_shut_nothing_moves() {
        with_gate(false, || {
            with_openai_gate(false, || {
                with_anthropic_gate(false, || {
                    for kind in
                        ["claude", "local", "openai-compatible", "anthropic-compatible", "wat"]
                    {
                        assert_eq!(
                            for_provider(&a_row(kind)).label(),
                            "claude-code",
                            "with the anthropic-native gate shut the {kind} row must run on \
                             the engine that shipped"
                        );
                    }
                });
            });
        });
    }

    /// **THE TEST THE THREE-SWITCH DESIGN IS FOR, EXTENDED TO THE THIRD
    /// GATE.** All eight combinations of the three gates are forced in turn;
    /// if any two switches were ever merged onto one `AtomicU8`, or any two
    /// of `for_provider`'s three `if`s were ever collapsed into one
    /// condition, one of these assertions would move.
    #[test]
    fn the_anthropic_gate_is_independent_of_the_other_two() {
        for local_open in [false, true] {
            for openai_open in [false, true] {
                for anthropic_open in [false, true] {
                    with_gate(local_open, || {
                        with_openai_gate(openai_open, || {
                            with_anthropic_gate(anthropic_open, || {
                                assert_eq!(
                                    for_provider(&a_row("local")).label(),
                                    if local_open { "native-ollama" } else { "claude-code" },
                                    "local={local_open} openai={openai_open} anthropic={anthropic_open}"
                                );
                                assert_eq!(
                                    for_provider(&a_row("openai-compatible")).label(),
                                    if openai_open { "native-openai" } else { "claude-code" },
                                    "local={local_open} openai={openai_open} anthropic={anthropic_open}"
                                );
                                assert_eq!(
                                    for_provider(&a_row("anthropic-compatible")).label(),
                                    if anthropic_open { "native-anthropic" } else { "claude-code" },
                                    "local={local_open} openai={openai_open} anthropic={anthropic_open}"
                                );
                                // The billed row never moves for any combination.
                                assert_eq!(for_provider(&a_row("claude")).label(), "claude-code");
                            });
                        });
                    });
                }
            }
        }
    }

    /// **THE FEATURE LEVER ITSELF, PROVEN — mirrors `the_native_openai_
    /// feature_opens_the_gate_without_touching_the_const`.** Only compiled
    /// when the feature is actually on, so a plain `cargo test` never runs
    /// this. What it proves: the `native-anthropic` feature opens the gate
    /// on its own, with no `#[cfg(test)]` override in play, and without
    /// moving `NATIVE_ANTHROPIC_ENABLED`, which stays asserted `false` in
    /// the same breath.
    #[cfg(feature = "native-anthropic")]
    #[test]
    fn the_native_anthropic_feature_opens_the_gate_without_touching_the_const() {
        assert!(!NATIVE_ANTHROPIC_ENABLED, "the shipping const itself must stay false");
        assert!(native_anthropic_enabled(), "the cargo feature alone must open the gate");
        assert_eq!(for_provider(&a_row("anthropic-compatible")).label(), "native-anthropic");
    }

    /// The tool and vendor-binary questions, for the third backend — same
    /// contract already pinned for the first two.
    #[test]
    fn the_anthropic_native_engine_answers_the_same_as_the_others() {
        assert!(!native::NativeEngine::anthropic_compatible().supports_tools());
        assert!(!native::NativeEngine::anthropic_compatible().needs_vendor_binary());
        assert_eq!(
            native::NativeEngine::anthropic_compatible().supports_tools(),
            native::NativeEngine::ollama().supports_tools(),
        );
    }
}
