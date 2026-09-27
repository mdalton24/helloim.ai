//! The engine that shipped: spawn `claude.exe`, forward its stream.
//!
//! **EVERY LINE BELOW IS A TRANSCRIPTION OF WHAT `main.rs::send` DID BEFORE THE
//! SEAM EXISTED (2026-08-31, commit `bdbe7a2`), NOT A REWRITE OF IT.** Argument
//! order, the exact spawn-failure sentence, the stderr filter, the reaper's poll
//! interval, and the exit code a cancelled run reports are all preserved
//! deliberately, because stage one of the neutral-engine work is required to
//! change nothing a customer can observe. Where a comment in here reads like it
//! is arguing with someone, it is: it was carried across with the code it
//! explains, and the failure it describes really happened.
//!
//! **THE ONE THING THAT IS GENUINELY DIFFERENT, and it is an improvement rather
//! than a change:** the child now lives in a slot that is *born holding it*,
//! instead of being placed into the app's session state a moment after the
//! reaper thread starts polling for it. The old comment on that line —
//! *"the other order is a race the reaper loses: it wakes first, finds no
//! child, and reports the run finished a millisecond after it began"* — was
//! describing a race avoided by careful ordering. It is now impossible by
//! construction. Nothing else about the observable behaviour moved.
//!
//! **WHAT THIS ENGINE WILL EVENTUALLY BE.** Not the default. Mark's decision on
//! 2026-08-31 is that a person choosing OpenAI or a local model should not have
//! to install anything from Anthropic; this engine survives that as the
//! *optional* path for somebody billing a Claude subscription, because
//! subscription OAuth is permitted only inside Claude Code itself and a native
//! engine calling `api.anthropic.com` must use a Console API key. Deleting this
//! file is the last stage of that work and is not this one.

use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::{Engine, Permission, RunningTurn, TurnRequest, TurnSink};

/// How often the reaper asks whether the child has exited.
///
/// Unchanged from the pre-seam value. It polls rather than blocking on `wait()`
/// because `wait()` would hold the child's lock for the whole run, and the Stop
/// button needs that same lock to kill it. **A Stop that hangs until the thing
/// it is stopping finishes is not a Stop button.**
const REAP_POLL: Duration = Duration::from_millis(150);

/// Claude Code's `--permission-mode` spelling for one of the product's three
/// choices.
///
/// **THIS IS THE VENDOR'S VOCABULARY AND IT LIVES HERE, WHICH IS THE POINT OF
/// THE SEAM.** `Permission` is the decision; these four strings are one
/// engine's way of expressing it. A native engine enforces the same three
/// choices at its own dispatch point and never learns these words.
fn permission_flag(p: Permission) -> &'static str {
    match p {
        Permission::AcceptEdits => "acceptEdits",
        Permission::Full => "bypassPermissions",
        Permission::Ask => "default",
    }
}

/// Every argument after the program name, in the order the pre-seam `send`
/// built them.
///
/// **PURE, AND SEPARATE FROM `build_command`, SO THE ARGUMENT VECTOR IS
/// TESTABLE AT ALL.** Before the seam this was a `Command` builder chain inside
/// a `#[tauri::command]` that needs a live Tauri `AppHandle`, which meant the
/// exact flags this product ships were checkable only by running the app on
/// Windows and watching what happened. They are now pinned by a test.
///
/// The order does not matter to `claude.exe`. It is preserved anyway, because
/// "identical" is a cheaper property to prove than "equivalent".
pub(crate) fn argv(req: &TurnRequest) -> Vec<String> {
    let mut args: Vec<String> = vec![
        "-p".into(),
        req.prompt.clone(),
        "--output-format".into(),
        "stream-json".into(),
        "--verbose".into(),
        "--permission-mode".into(),
        permission_flag(req.permission).into(),
        // APPEND, never --system-prompt. Replacing the default would throw away
        // everything Claude Code tells the model about its own tools, to win an
        // argument about tone.
        "--append-system-prompt".into(),
        req.system_prompt.clone(),
    ];
    if let Some(id) = &req.resume {
        args.push("--resume".into());
        args.push(id.clone());
    }
    if let Some(path) = &req.mcp_config {
        args.push("--mcp-config".into());
        args.push(path.to_string_lossy().into_owned());
    }
    // THE MEMORY TOOLS ARE PRE-APPROVED, and this is why memory works at all in
    // the default permission mode. Found by the slice-4 observed run
    // (2026-08-27): in headless -p mode nobody can answer a permission prompt,
    // so an unapproved MCP tool is denied — the model CALLED memory_save exactly
    // as the guidance asked, got permission_denied, and no fact landed. Slice
    // 3's own tests drove the server over raw pipes and could never see this.
    // Nothing else widens: connector tools keep their normal permission
    // treatment.
    if !req.allowed_tools.is_empty() {
        args.push("--allowedTools".into());
        args.push(req.allowed_tools.join(","));
    }
    args
}

/// The argument vector plus the working directory, the stdio wiring and the
/// environment.
///
/// **THE ENVIRONMENT ORDER IS LOAD-BEARING AND IS PRESERVED: the MCP servers'
/// variables go on FIRST, then `providers::apply_env`.** `apply_env` ends by
/// *removing* `ANTHROPIC_API_KEY` unconditionally on every non-default
/// provider — that is the CF-Connecting-IP lesson wearing an env var, and it
/// stops a user's real Anthropic credential being forwarded to a third party's
/// endpoint. Setting connector variables afterwards could put it back.
fn build_command(req: &TurnRequest) -> Result<Command, String> {
    let mut cmd = Command::new(crate::claude_binary());
    cmd.current_dir(&req.workdir)
        .args(argv(req))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if req.mcp_config.is_some() {
        for (k, v) in &req.mcp_env {
            cmd.env(k, v);
        }
    }
    // THE BRAIN IS SWAPPABLE HERE. For the built-in Claude row this touches
    // NOTHING — the product as it shipped. For a selected provider it sets the
    // documented routing variables and REMOVES any inherited
    // ANTHROPIC_API_KEY. The same `apply_env` runs in stage 2 of the provider
    // test, so what was tested is what launches.
    //
    // THE `?` IS DELIBERATE: if the chosen brain cannot be wired (a kind this
    // build does not route, or a translator that failed to start), the send
    // REFUSES with words. A silent fallback to the Anthropic cloud would bill
    // an account the user chose not to use.
    crate::providers::apply_env(&mut cmd, &req.provider)?;
    crate::hide_console(&mut cmd);
    Ok(cmd)
}

/// The child, and whether anyone has already reaped it.
///
/// `None` means the same thing it meant when this lived in `Session::child`:
/// the turn is over, either because it finished or because `cancel` took it.
/// Both `is_alive` and `cancel` read it, and both behave on `None` exactly as
/// the pre-seam code behaved on a cleared `Session::child`.
struct Slot {
    child: Option<Child>,
}

/// A live `claude.exe` turn.
pub(crate) struct ClaudeCodeRun {
    slot: Arc<Mutex<Slot>>,
}

impl ClaudeCodeRun {
    /// `pub(crate)` so the proof that cancelling really kills the OS process
    /// can be written against a real child without a Tauri app around it.
    pub(crate) fn new(child: Child) -> Self {
        ClaudeCodeRun { slot: Arc::new(Mutex::new(Slot { child: Some(child) })) }
    }
}

impl RunningTurn for ClaudeCodeRun {
    fn is_alive(&self) -> bool {
        let mut guard = self.slot.lock().unwrap();
        match guard.child.as_mut() {
            // try_wait returns Ok(None) while it is still alive.
            Some(child) => matches!(child.try_wait(), Ok(None)),
            None => false,
        }
    }

    /// CLOSING THE WINDOW LEFT NameOS.exe RUNNING -- Beck, b21/b20, 2026-08-31,
    /// on real hardware: every WebView2 child gone, the process itself still
    /// alive 187 seconds later. **Rust drops an unwaited `Child` without killing
    /// it** — that is documented `std::process` behaviour, not a bug — so
    /// "the field reads `None` again" and "the process is actually dead" are two
    /// different claims, and only checking the first is exactly how that bug
    /// shipped once already. `cancel_actually_terminates_the_process` below
    /// checks the second, from outside the `Child` object, against `/proc`.
    ///
    /// On a `kill()` error it returns `Err` and deliberately does NOT clear the
    /// slot, which is what the pre-seam `kill_running_child` did: a process we
    /// failed to signal is still out there, and forgetting the handle is the
    /// one response guaranteed to make it unkillable.
    fn cancel(&self) -> Result<(), String> {
        let mut guard = self.slot.lock().unwrap();
        if let Some(child) = guard.child.as_mut() {
            child.kill().map_err(|e| e.to_string())?;
            let _ = child.wait();
        }
        guard.child = None;
        Ok(())
    }
}

/// One JSON object per line on stdout, forwarded verbatim.
///
/// Parsing it in Rust would mean re-deriving Claude Code's schema in two places,
/// and it changes. **A non-JSON line is not a crash; show it rather than eat
/// it.**
fn forward_stdout(stdout: std::process::ChildStdout, sink: Arc<dyn TurnSink>) {
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<serde_json::Value>(&line) {
                Ok(value) => sink.event(value),
                Err(_) => sink.raw(line),
            }
        }
    });
}

/// stderr: surfaced, never swallowed. A silent failure here is the one that
/// makes the window look like it is thinking forever.
///
/// ONE NAMED EXCEPTION, ADDED 2026-08-31, CORRECTED THE SAME DAY: Claude Code's
/// own harmless `[claude-code:unrecognized_model]` diagnostic, usually carrying
/// its JSON detail on the SAME line, space-separated — see
/// `crate::is_unrecognized_model_notice`'s doc comment for how the first version
/// of this got that wrong and how Beck proved it against the real binary. It
/// fires on **every** local-brain turn, the run answers anyway, and forwarding
/// it painted a working local brain red on its very first message. Mark's first
/// message to a local brain came back as this diagnostic in a red bubble before
/// a word of the real answer arrived, and the only sane-looking response to what
/// LOOKS like a broken app — pressing Stop — killed the run that was about to
/// answer him correctly.
///
/// **THIS IS THE ONE AND ONLY LINE FILTERED OUT OF THIS STREAM.** Everything
/// else, real errors included, goes straight through.
///
/// It stays in this file rather than moving to the seam because it is
/// `claude.exe`'s own diagnostic about `claude.exe`'s own model registry. A
/// native engine will never emit it, and a filter that outlives the thing it
/// filters is a filter nobody can safely delete.
fn forward_stderr(stderr: std::process::ChildStderr, sink: Arc<dyn TurnSink>) {
    std::thread::spawn(move || {
        let mut swallow_detail = false;
        for line in BufReader::new(stderr).lines() {
            let Ok(line) = line else { break };
            if line.trim().is_empty() {
                continue;
            }
            if crate::is_unrecognized_model_notice(&line) {
                // Real claude.exe writes the marker and its detail as one line;
                // only arm the next-line swallow for the older two-line shape,
                // where this line was JUST the marker.
                swallow_detail = line.trim() == crate::UNRECOGNIZED_MODEL_MARKER;
                continue;
            }
            if swallow_detail {
                swallow_detail = false;
                if crate::is_unrecognized_model_detail(&line) {
                    continue;
                }
            }
            sink.failure(line);
        }
    });
}

/// Watches for the child to end and reports the exit code exactly once.
///
/// **A CANCELLED TURN REPORTS 0, AND THAT IS NOT AN ACCIDENT.** `cancel` takes
/// the child out of the slot under the same lock this loop reads it under, so
/// the next poll finds `None` and breaks with 0 — the pre-seam behaviour, where
/// `stop()` cleared `Session::child` and this same loop's `None` arm broke with
/// 0. It matters because the window prints a red *"Claude Code exited with code
/// N."* for any non-zero code, and pressing Stop is not an error. Killed-by-
/// signal statuses report `None` from `status.code()`, so a version that simply
/// waited on the child would turn every Stop press into an error message.
fn reap(slot: Arc<Mutex<Slot>>, sink: Arc<dyn TurnSink>) {
    std::thread::spawn(move || {
        let code = loop {
            {
                let mut guard = slot.lock().unwrap();
                match guard.child.as_mut() {
                    Some(child) => match child.try_wait() {
                        Ok(Some(status)) => break status.code().unwrap_or(-1),
                        Ok(None) => {}
                        Err(_) => break -1,
                    },
                    // cancel() took it and already reaped it.
                    None => break 0,
                }
            }
            std::thread::sleep(REAP_POLL);
        };
        // Cleared BEFORE `finished`, and with the lock released before that
        // call, in that order. The clear is what makes a later `cancel` a
        // no-op rather than a kill against an exited process; releasing the
        // lock first is what stops this thread holding the child's lock while
        // the host takes the session lock, which `stop` takes in the opposite
        // order.
        {
            slot.lock().unwrap().child = None;
        }
        crate::startup::note(&format!("turn done: engine=claude_code exit_code={code}"));
        sink.finished(code);
    });
}

/// Today's engine, behind the trait.
pub(crate) struct ClaudeCodeEngine;

impl Engine for ClaudeCodeEngine {
    fn label(&self) -> &'static str {
        "claude-code"
    }

    /// **EVERY ID EXCEPT THE NATIVE ENGINE'S.** Claude Code mints its own and
    /// this engine cannot tell one of those from any other string, so the honest
    /// answer is "anything that is not somebody else's" rather than a pattern
    /// match on a format the vendor is free to change. What it must refuse is
    /// exact and known: `claude.exe --resume helloim-chat-…` fails outright, and
    /// the visible symptom is a turn that dies before the model is ever called.
    fn owns_session(&self, id: &str) -> bool {
        !id.starts_with(super::native::store::CHAT_ID_PREFIX)
    }

    /// Yes — this is the whole reason the native engine stays behind a gate.
    fn supports_tools(&self) -> bool {
        true
    }

    /// Yes. This engine **is** a spawn of somebody else's program, which is what
    /// the Test button's stage 2 needs to know before it goes looking for one.
    fn needs_vendor_binary(&self) -> bool {
        true
    }

    /// **NO — AND THIS WAS THE WHOLE OF "HISTORY DOESN'T EVEN WORK."** This
    /// engine's transcripts are `claude.exe`'s own, in its own account-scoped
    /// storage, and nothing in this file ever writes into `conversations_dir`.
    /// Answering `false` here is what turns on `main.rs::AppSink`'s generic
    /// capture for this engine — see `Engine::persists_own_history`'s own doc
    /// for why that capture lives there instead of a second store bolted onto
    /// this file, and reaching into `claude.exe`'s own directory is exactly the
    /// scope creep `main.rs::clear_conversations`'s doc already refused, for
    /// the identical reason, one screen over.
    fn persists_own_history(&self) -> bool {
        false
    }

    fn start(
        &self,
        req: &TurnRequest,
        sink: Arc<dyn TurnSink>,
    ) -> Result<Box<dyn RunningTurn>, String> {
        let mut cmd = build_command(req)?;
        let mut child = cmd.spawn().map_err(|e| {
            format!("Could not start Claude Code ({e}). Checked PATH and ~/.local/bin.")
        })?;
        // TURN START -- same diagnostic line the native engine writes in
        // `engine::native::mod::drive_with_timeout`, so a future hang is
        // traceable through `startup.log` regardless of which engine was
        // running. `req.provider.id` (e.g. "claude"), never the prompt.
        crate::startup::note(&format!("turn start: engine=claude_code provider={}", req.provider.id));

        let stdout = child.stdout.take().ok_or("No stdout on the child process.")?;
        let stderr = child.stderr.take().ok_or("No stderr on the child process.")?;

        // The slot is born holding the child, so the reaper cannot start before
        // there is something for it to find. See the module header.
        let run = ClaudeCodeRun::new(child);
        let slot = Arc::clone(&run.slot);

        forward_stdout(stdout, Arc::clone(&sink));
        forward_stderr(stderr, Arc::clone(&sink));
        reap(slot, sink);

        Ok(Box::new(run))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::Provider;
    // Only the `#[cfg(unix)]` stream helpers below use these; on Windows the
    // whole recorder is compiled out (see the note above them).
    #[cfg(unix)]
    use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};

    // -- helpers ------------------------------------------------------------

    /// The built-in Claude row — the one `apply_env` treats as "touch nothing".
    /// Constructed field by field on purpose: a new field on `Provider` should
    /// break this build and make somebody decide what a turn request does with
    /// it, rather than inheriting a default nobody chose.
    fn a_provider() -> Provider {
        Provider {
            id: "claude".into(),
            kind: "claude".into(),
            name: "Claude".into(),
            base_url: String::new(),
            model: String::new(),
            builtin: true,
            disconnected: false,
            connected: true,
            checked_at: String::new(),
            last_error: String::new(),
            has_secret: false,
            caveat: String::new(),
            migration_note: String::new(),
            allow_shell: false,
            allow_agency: false,
        }
    }

    fn a_request() -> TurnRequest {
        TurnRequest {
            prompt: "hello".into(),
            workdir: std::path::PathBuf::from("/tmp"),
            permission: Permission::AcceptEdits,
            system_prompt: "SYSTEM".into(),
            resume: None,
            mcp_config: None,
            mcp_env: Vec::new(),
            allowed_tools: Vec::new(),
            // This engine ignores it: its transcripts are the vendor's, reached
            // by `--resume`. Set to `None` here so the pin below fails if a
            // future change ever starts putting it on the command line.
            store_dir: None,
            provider: a_provider(),
        }
    }

    // -- THE STREAM HELPERS BELOW ARE `#[cfg(unix)]` AND SO IS EVERY TEST THAT
    // -- USES THEM, WHICH IS A LIMIT WORTH READING RATHER THAN A TIDINESS
    // -- CHOICE.
    //
    // They drive a real child over real pipes, and the child is `sh -c`. There
    // is no `sh` on Windows, and this box cannot run a Windows binary to check
    // a `cmd /c` variant against -- so writing one would be shipping a test
    // nobody here has ever executed, which is the failure this house keeps
    // paying for. `forward_stdout`, `forward_stderr` and `reap` contain no
    // platform-specific code (plain `std::process`, `std::io`, `std::thread`),
    // so proving the mechanism here proves the mechanism. **What is NOT proven
    // by them is the mechanism running on Windows**, and that is Beck's.

    /// What the window would have been told.
    #[cfg(unix)]
    #[derive(Debug, Clone, PartialEq)]
    enum Seen {
        Event(serde_json::Value),
        Raw(String),
        Failure(String),
        Finished(i32),
    }

    #[cfg(unix)]
    struct Recorder {
        tx: Mutex<Sender<Seen>>,
    }

    #[cfg(unix)]
    impl Recorder {
        fn new() -> (Arc<Recorder>, Receiver<Seen>) {
            let (tx, rx) = channel();
            (Arc::new(Recorder { tx: Mutex::new(tx) }), rx)
        }
        fn send(&self, s: Seen) {
            let _ = self.tx.lock().unwrap().send(s);
        }
    }

    #[cfg(unix)]
    impl TurnSink for Recorder {
        fn event(&self, value: serde_json::Value) {
            self.send(Seen::Event(value));
        }
        fn raw(&self, line: String) {
            self.send(Seen::Raw(line));
        }
        fn failure(&self, line: String) {
            self.send(Seen::Failure(line));
        }
        fn finished(&self, code: i32) {
            self.send(Seen::Finished(code));
        }
    }

    /// Collect everything the sink saw, waiting for the channel to DISCONNECT
    /// rather than stopping at `Finished`.
    ///
    /// **THAT DISTINCTION IS THE TEST BEING HONEST ABOUT A REAL RACE.** stdout,
    /// stderr and the reaper are three independent threads, so `finished` can
    /// legitimately arrive before the last line off a pipe. That is true of the
    /// shipped product too (see the finding in the write-up) and this refactor
    /// deliberately does not change it — so the tests must not assert an
    /// ordering the code has never actually promised. Waiting for every sender
    /// to drop is the only way to know the whole turn's output is in hand.
    #[cfg(unix)]
    fn drain(rx: &Receiver<Seen>) -> Vec<Seen> {
        let mut out = Vec::new();
        loop {
            match rx.recv_timeout(Duration::from_secs(10)) {
                Ok(s) => out.push(s),
                Err(RecvTimeoutError::Disconnected) => return out,
                Err(RecvTimeoutError::Timeout) => {
                    panic!("the turn never finished; saw only {out:?}")
                }
            }
        }
    }

    #[cfg(unix)]
    fn failures(seen: &[Seen]) -> Vec<String> {
        seen.iter()
            .filter_map(|s| match s {
                Seen::Failure(t) => Some(t.clone()),
                _ => None,
            })
            .collect()
    }

    #[cfg(unix)]
    fn events(seen: &[Seen]) -> Vec<serde_json::Value> {
        seen.iter()
            .filter_map(|s| match s {
                Seen::Event(v) => Some(v.clone()),
                _ => None,
            })
            .collect()
    }

    #[cfg(unix)]
    fn raws(seen: &[Seen]) -> Vec<String> {
        seen.iter()
            .filter_map(|s| match s {
                Seen::Raw(t) => Some(t.clone()),
                _ => None,
            })
            .collect()
    }

    /// Exactly one `finished`, and its code. More than one would leave the
    /// window's state machine reading a second run's ending.
    #[cfg(unix)]
    fn finished(seen: &[Seen]) -> i32 {
        let codes: Vec<i32> = seen
            .iter()
            .filter_map(|s| match s {
                Seen::Finished(c) => Some(*c),
                _ => None,
            })
            .collect();
        assert_eq!(codes.len(), 1, "expected exactly one finish, got {codes:?}");
        codes[0]
    }

    // -- the argument vector ------------------------------------------------

    /// **THE PIN. This is the whole safety argument for stage one.**
    ///
    /// Every string and every position below was read off `main.rs::send` as it
    /// stood at commit `bdbe7a2`, before the seam existed. If a later change
    /// drops `--verbose`, reorders `--append-system-prompt`, spells
    /// `--allowedTools` differently, or stops passing `--resume`, this fails.
    ///
    /// **It has been proven able to fail, by mutation rather than by
    /// assertion** — removing `"--verbose"` from `argv`, swapping the `--resume`
    /// and `--mcp-config` blocks, and joining `allowed_tools` with `", "`
    /// instead of `","` each make it fail. A test that has only ever passed
    /// proves nothing about the code it names.
    #[test]
    fn the_argv_is_exactly_what_the_pre_seam_send_built() {
        let mut req = a_request();
        req.prompt = "do the thing".into();
        req.system_prompt = "voice+memory+bridge".into();
        req.resume = Some("sess-123".into());
        req.mcp_config = Some(std::path::PathBuf::from("/cfg/mcp-launch.json"));
        req.allowed_tools = vec![
            "mcp__nameos-memory__memory_search".into(),
            "mcp__nameos-memory__memory_save".into(),
        ];

        assert_eq!(
            argv(&req),
            vec![
                "-p",
                "do the thing",
                "--output-format",
                "stream-json",
                "--verbose",
                "--permission-mode",
                "acceptEdits",
                "--append-system-prompt",
                "voice+memory+bridge",
                "--resume",
                "sess-123",
                "--mcp-config",
                "/cfg/mcp-launch.json",
                "--allowedTools",
                "mcp__nameos-memory__memory_search,mcp__nameos-memory__memory_save",
            ]
        );
    }

    /// The other half of the pin: a first turn, no connectors, no memory. The
    /// three optional blocks must be **absent**, not present-and-empty — an
    /// empty `--resume` would send Claude Code hunting for a session id that
    /// does not exist, and an empty `--allowedTools` is a different instruction
    /// from no flag at all.
    #[test]
    fn the_optional_flags_are_absent_rather_than_empty_on_a_first_turn() {
        assert_eq!(
            argv(&a_request()),
            vec![
                "-p",
                "hello",
                "--output-format",
                "stream-json",
                "--verbose",
                "--permission-mode",
                "acceptEdits",
                "--append-system-prompt",
                "SYSTEM",
            ]
        );
    }

    /// The vendor spellings, pinned. These are the exact three strings the
    /// pre-seam `permission_mode` produced, and getting one wrong either widens
    /// what the agent may do or breaks the flag outright.
    #[test]
    fn the_permission_flag_spellings_are_claude_codes_own() {
        assert_eq!(permission_flag(Permission::AcceptEdits), "acceptEdits");
        assert_eq!(permission_flag(Permission::Full), "bypassPermissions");
        assert_eq!(permission_flag(Permission::Ask), "default");
        // And the whole chain, from the word the window sends: an unknown mode
        // must reach the strictest flag, never the loosest.
        for word in ["", "edit", "FULL", "plan", "bypassPermissions", "acceptEdits"] {
            assert_eq!(
                permission_flag(Permission::from_window(word)),
                "default",
                "the window word {word:?} must not widen the permission mode"
            );
        }
    }

    // -- the stream, driven through a real child ----------------------------

    /// **THE STDERR FILTER, EXERCISED AS A LOOP FOR THE FIRST TIME.** Before the
    /// seam, `is_unrecognized_model_notice` had unit tests and the loop that
    /// uses it had none — it lived inside a closure in a `#[tauri::command]`
    /// and could only be reached by running the app on Windows. b17 shipped a
    /// version of that loop whose predicate never matched a real line, every
    /// local-brain turn painted red, and nothing in `cargo test` could see it.
    ///
    /// This drives a real child that writes the real byte sequence Beck
    /// captured from `claude.exe` 2.1.251 — marker, one space, JSON detail, one
    /// line — plus a genuine error, and asserts that exactly one of them
    /// reaches the window.
    #[cfg(unix)]
    #[test]
    fn the_benign_model_notice_is_swallowed_and_a_real_error_is_not() {
        let script = format!(
            "printf '%s\\n' '{marker} {{\"model\":\"llama3.2:latest\",\"query_source\":\"sdk\"}}' \
             'connection refused' >&2",
            marker = crate::UNRECOGNIZED_MODEL_MARKER
        );
        let seen = run_script(&script);
        assert_eq!(
            failures(&seen),
            vec!["connection refused".to_string()],
            "the benign notice must not reach the window, and the real error must"
        );
        assert_eq!(finished(&seen), 0);
    }

    /// The older two-line shape: marker alone, JSON detail beneath it. A
    /// different `claude` build may still split it that way and nothing here can
    /// confirm which one a customer has, so both shapes stay handled.
    ///
    /// The third line proves the swallow is armed for **one** line only — a real
    /// JSON error immediately after a detail line must still surface.
    #[cfg(unix)]
    #[test]
    fn the_two_line_shape_swallows_exactly_one_detail_line() {
        let script = format!(
            "printf '%s\\n' '{marker}' '{{\"model\":\"llama3.2:latest\"}}' \
             '{{\"error\":\"connection refused\"}}' >&2",
            marker = crate::UNRECOGNIZED_MODEL_MARKER
        );
        let seen = run_script(&script);
        assert_eq!(
            failures(&seen),
            vec![r#"{"error":"connection refused"}"#.to_string()]
        );
    }

    /// **THE DISARM IS LOAD-BEARING, AND IT IS THE ONLY THING BETWEEN A REAL
    /// MODEL ERROR AND SILENCE.** `swallow_detail` is set back to `false` the
    /// moment it is used; a version that left it armed eats every later line
    /// that merely *looks* like the benign detail — and a genuine failure about
    /// a model looks exactly like that, because the predicate's whole test is
    /// "a JSON object naming a model".
    ///
    /// **THIS TEST EXISTS BECAUSE A MUTATION SURVIVED.** Deleting the
    /// `swallow_detail = false` line passed the entire suite: every other case
    /// happened to follow the detail with something that was not detail-shaped,
    /// so the permanently-armed version looked correct. A filter that silently
    /// eats errors is the same class of bug as b17 — the local brain painted
    /// red — pointed the other way, and it is worse, because nothing appears on
    /// screen at all.
    #[cfg(unix)]
    #[test]
    fn the_swallow_is_spent_after_one_line_and_a_real_model_error_still_surfaces() {
        let script = format!(
            "printf '%s\\n' '{marker}' '{{\"model\":\"llama3.2:latest\",\"query_source\":\"sdk\"}}' \
             '{{\"model\":\"llama3.2:latest\",\"error\":\"context length exceeded\"}}' >&2",
            marker = crate::UNRECOGNIZED_MODEL_MARKER
        );
        let seen = run_script(&script);
        assert_eq!(
            failures(&seen),
            vec![r#"{"model":"llama3.2:latest","error":"context length exceeded"}"#.to_string()],
            "the swallow must be spent after one line -- the second detail-shaped line \
             here is a real failure and the person must see it"
        );
    }

    /// A blank line after the marker must NOT consume the swallow, and must not
    /// be forwarded either. This is the case the pre-seam loop got right by
    /// accident of ordering — the emptiness check sits above the filter — and
    /// it is worth pinning, because moving those two checks past each other
    /// would let a real error be eaten.
    #[cfg(unix)]
    #[test]
    fn a_blank_line_neither_arms_nor_disarms_the_swallow() {
        let script = format!(
            "printf '%s\\n' '{marker}' '' '{{\"model\":\"x\"}}' 'real problem' >&2",
            marker = crate::UNRECOGNIZED_MODEL_MARKER
        );
        let seen = run_script(&script);
        assert_eq!(failures(&seen), vec!["real problem".to_string()]);
    }

    /// The two stdout cases in the five-shape contract: a parseable line becomes
    /// an event, an unparseable one is shown rather than eaten, and a blank line
    /// is neither.
    #[cfg(unix)]
    #[test]
    fn json_lines_become_events_and_junk_is_shown_not_eaten() {
        let script = r#"printf '%s\n' '{"type":"system","session_id":"abc"}' '' 'not json at all'"#;
        let seen = run_script(script);
        assert_eq!(
            events(&seen),
            vec![serde_json::json!({"type": "system", "session_id": "abc"})]
        );
        assert_eq!(raws(&seen), vec!["not json at all".to_string()]);
        assert_eq!(finished(&seen), 0);
    }

    /// A non-zero exit reaches the window as itself. The window prints
    /// *"Claude Code exited with code N."* for anything but 0, so an engine that
    /// flattened this would hide a crash.
    #[cfg(unix)]
    #[test]
    fn a_failing_child_reports_its_real_exit_code() {
        assert_eq!(finished(&run_script("exit 3")), 3);
    }

    /// **CANCELLING REPORTS 0, NOT -1.** The pre-seam code got this by having
    /// `stop()` clear `Session::child` before the reaper could observe the
    /// signal status; this gets it by `cancel` clearing the slot under the same
    /// lock the reaper polls. On Unix a killed child's `status.code()` is
    /// `None`, so a version that simply waited on the child reports -1 — and the
    /// window paints a red error for a button the person pressed on purpose.
    ///
    /// Proven able to fail: leaving the child in the slot inside `cancel` turns
    /// this into a -1.
    #[cfg(unix)]
    #[test]
    fn cancelling_a_turn_reports_a_clean_exit_rather_than_a_kill() {
        let (sink, rx) = Recorder::new();
        let child = Command::new("sleep")
            .arg("30")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn sleep");
        let run = ClaudeCodeRun::new(child);
        reap(Arc::clone(&run.slot), sink);

        assert!(run.is_alive(), "a sleeping child must read as alive");
        run.cancel().expect("cancel must succeed");
        assert!(!run.is_alive(), "a cancelled turn must not read as alive");

        assert_eq!(finished(&drain(&rx)), 0);
    }

    /// CLOSING THE WINDOW LEFT NameOS.exe RUNNING -- Beck, b21/b20, 2026-08-31:
    /// every WebView2 child gone, the process itself alive 187 seconds later.
    /// This is that proof, carried across the refactor intact and pointed at the
    /// code that now owns the `Child`.
    ///
    /// It checks the claim from **outside** the `Child` object entirely:
    /// `/proc/<pid>` existing is the kernel's own answer, not Rust's. A version
    /// that merely dropped the handle — forgetting the pid without ever
    /// signalling it — would satisfy "the slot reads empty" and fail this.
    ///
    /// `#[cfg(unix)]` for the same reason the pre-seam version was: `sleep` is
    /// not a Windows binary and this box cannot run one regardless. `cancel` has
    /// no platform-specific code in it, so proving the mechanism here is proving
    /// the mechanism.
    #[cfg(unix)]
    #[test]
    fn cancel_actually_terminates_the_process() {
        let child = Command::new("sleep").arg("30").spawn().expect("spawn sleep");
        let pid = child.id();
        let run = ClaudeCodeRun::new(child);

        run.cancel().expect("cancel should succeed");

        assert!(
            !std::path::Path::new(&format!("/proc/{pid}")).exists(),
            "the child process (pid {pid}) is still alive after cancel -- it was \
             forgotten, not killed, which is the exact shape of the window-close bug"
        );
    }

    /// **CANCEL MUST BE SAFE TO CALL TWICE, AND SAFE AFTER THE TURN ENDED ON ITS
    /// OWN.** Both really happen: the Stop button and `CloseRequested` both call
    /// it, and either can arrive after the run finished by itself.
    /// `Child::kill` on an already-reaped process is an error, so a version that
    /// skipped the empty-slot check would put a red error on screen for a button
    /// that did exactly what it was asked.
    #[cfg(unix)]
    #[test]
    fn cancel_is_idempotent_and_harmless_after_the_turn_is_over() {
        let child = Command::new("true").spawn().expect("spawn true");
        let run = ClaudeCodeRun::new(child);
        assert!(run.cancel().is_ok());
        assert!(run.cancel().is_ok(), "a second cancel must not fail");
        assert!(!run.is_alive());
    }

    /// The one benign race the seam introduces, asserted rather than argued: if
    /// the child exits before `send` has stored the handle, the handle it stores
    /// is already spent. It must behave exactly like no handle at all —
    /// `is_alive` false, `cancel` a successful no-op — because that is what the
    /// pre-seam `Session::child = None` behaved like.
    #[cfg(unix)]
    #[test]
    fn a_handle_stored_after_its_run_already_finished_behaves_like_no_handle() {
        let (sink, rx) = Recorder::new();
        let child = Command::new("true")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn true");
        let run = ClaudeCodeRun::new(child);
        reap(Arc::clone(&run.slot), sink);
        assert_eq!(finished(&drain(&rx)), 0);

        assert!(!run.is_alive());
        assert!(run.cancel().is_ok());
    }

    /// Spawn `sh -c <script>` through the same forwarding code the engine uses,
    /// and return everything the sink saw.
    ///
    /// It deliberately does NOT go through `Engine::start` — that spawns
    /// `claude.exe`, which is not on this box and would not be evidence if it
    /// were. What is under test is the stream handling, and this drives the real
    /// functions with a real child over real pipes.
    #[cfg(unix)]
    fn run_script(script: &str) -> Vec<Seen> {
        let (sink, rx) = Recorder::new();
        let mut child = Command::new("sh")
            .arg("-c")
            .arg(script)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn sh");
        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();
        let run = ClaudeCodeRun::new(child);
        forward_stdout(stdout, Arc::clone(&sink) as Arc<dyn TurnSink>);
        forward_stderr(stderr, Arc::clone(&sink) as Arc<dyn TurnSink>);
        reap(Arc::clone(&run.slot), sink);
        drain(&rx)
    }
}
