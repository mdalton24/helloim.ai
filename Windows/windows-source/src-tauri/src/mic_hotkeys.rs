// Global mic controls: a hard-mute kill switch and a push-to-talk hold,
// reachable even when helloim.ai does not have focus. Overnight hardening
// batch #2, ranked by the Mastermind room as the highest-value new work for
// a face+voice app; this is item 3 of that brief.
//
// WHY GLOBAL AND NOT JUST AN IN-WINDOW BUTTON: the in-window mic button
// (`ui/index.html`, `#mic`) already toggles wake-word listening, but it only
// works if the app has focus and is visible. "A real kill switch" and "a
// push-to-talk fallback for noisy rooms" both fail that test on purpose —
// you want to cut the mic, or hold to talk, from whatever window you are
// actually in. `tauri-plugin-global-shortcut` is the vendor's own answer to
// exactly this (see `companion.rs`'s header, and Cargo.toml), already wired
// into this app for the companion window's summon/dismiss chord — this
// module is the same mechanism, a second time, for a different purpose.
//
// THE MANAGER IS SHARED WITH `companion.rs`, AND THAT IS LOAD-BEARING. Both
// modules call `app.global_shortcut()`, which is one instance per app, not
// one per module. `companion::register()` used to call `unregister_all()`
// on every re-registration, which — before this module existed — was
// harmless because nothing else was registered. It is not harmless now; see
// the fix and its comment in `companion.rs`. Keep that constraint in mind if
// this module ever grows a "let the user pick their own chord" step of its
// own (mirroring companion's onboarding Step 3): re-registering a chord here
// must unregister only the ONE shortcut this module is replacing, by the
// same `CURRENT`-tracking pattern companion.rs uses, never `unregister_all()`.
//
// THAT STEP OF ITS OWN NOW EXISTS — 2026-09-05, `register_global_shortcut`
// below, called by `ui/index.html`'s onboarding Step 3 recorder for
// push-to-talk specifically (the companion's OWN chord is still
// `set_companion_hotkey` in `companion.rs`; this module never touches that
// command). See that function's own doc for the two things a re-registration
// here has to get right that `companion::register()` did not have to:
// leaving the OLD chord working if the new one is refused by the OS, and
// refusing a chord that would silently steal the mute or companion chord's
// OWN binding instead of failing loudly — see `validate_ptt_chord`'s doc for
// why the OS cannot be trusted to catch that second one on its own.
//
// WHAT THIS BOX CANNOT PROVE, same disclosure as companion.rs's own header:
// whether Windows actually hands these chords to us instead of eating them
// (or another app already owns one) is Windows runtime behaviour, checked
// here against the vendored `global-hotkey` 0.8.0 source (not recalled) —
// `src/platform_impl/windows/mod.rs`: `RegisterHotKey` fires exactly one
// `Pressed` on key-down, then a background thread polls `GetAsyncKeyState`
// every 50ms and fires exactly one `Released` when the key actually comes
// up. That poll-for-release shape is the crate's own documented answer to
// push-to-talk (see its comment citing
// github.com/tauri-apps/global-hotkey/issues/176) — confirmed by reading the
// source, not assumed from the plugin's higher-level docs. Whether it FEELS
// right on a real keyboard — release latency, a chord already owned by
// another app, an IME eating it — is Beck's to confirm on the real Windows
// box, same as companion's chord. The OS-level acceptance of
// `register_global_shortcut` itself is the same disclosure, restated on that
// function directly.
//
// THE RECORDER COULD NOT ACTUALLY HEAR A KEY UNTIL 2026-09-05, AND
// `suspend_global_shortcuts_for_recording`/`resume_global_shortcuts_after_
// recording` BELOW ARE THE FIX. Wren traced Mark's "hotkey listens but
// doesn't set the press" report to its root: `RegisterHotKey` delivers
// `WM_HOTKEY` to the registering process, never a keydown or keyup to any
// window — so onboarding Step 3's recorder, which listens for a raw key
// combo in the webview to learn a NEW chord, never sees a keystroke that
// collides with mute, push-to-talk or the companion chord, because the OS
// has already consumed it before the page's own listener gets a turn. The
// UI now suspends all three shortcuts before it starts listening and
// resumes them once it stops; see those two functions' own docs for the
// state machine and for why resume re-reads the live trackers rather than a
// value captured at suspend time.
//
// THAT FIX MOVED THE FREEZE RATHER THAN CLOSING IT — 2026-09-06, ROUND 2.
// Beck's real test (Win32 `SendInput`, a genuine OS-level keystroke — CDP
// cannot inject one, which is why only this test counts) showed the suspend
// half above works: `suspend_global_shortcuts_for_recording` takes all three
// chords down and stays stable. But EVERY exit from Listening — a commit,
// Escape, or Cancel — calls `resume_global_shortcuts_after_recording`, and
// THAT hung the app's main thread permanently. Esc alone was enough, with no
// new chord ever registered, so the cause is re-registering ANY of the
// three, not something specific to the commit path.
//
// ROOT CAUSE, READ FROM SOURCE, NOT ASSUMED. Every OS-touching call in
// `tauri-plugin-global-shortcut` 2.3.2 — `register`, `on_shortcut` AND
// `unregister` alike — goes through that crate's own `run_main_thread!`
// macro (its `lib.rs`), which posts the actual Win32 call onto the app's
// main thread and then BLOCKS the calling thread on an
// `mpsc::Receiver::recv()` until that task runs. Tauri's own documented
// behaviour (v2 IPC docs; github.com/orgs/tauri-apps/discussions/3561,
// checked 2026-09-06) is that a plain, non-`async` `#[tauri::command]` —
// which is what both `suspend_global_shortcuts_for_recording` and
// `resume_global_shortcuts_after_recording` were — runs DIRECTLY on that
// same main thread as part of dispatching the IPC call itself. A command
// already running on the main thread that then asks to run something else
// on the main thread and waits for it is asking the thread to unblock
// itself; nothing else is left to pump the queue it is stuck waiting on.
// This codebase already has the vendor-recommended way out of exactly this
// shape — `feedback.rs`'s `submit_feedback` is `async fn` and does its
// blocking work inside `tauri::async_runtime::spawn_blocking`, precisely so
// the command's own dispatch never occupies the main thread while the work
// runs. `register_global_shortcut`, `suspend_global_shortcuts_for_
// recording` and `resume_global_shortcuts_after_recording` below now follow
// that same shape; each keeps its old body verbatim in a `_blocking` sibling
// function and adds nothing else.
//
// WHY THIS HOLDS REGARDLESS OF THE EXACT REASON REGISTER HUNG AND UNREGISTER
// DID NOT. `tauri-runtime-wry`'s own dispatch (`send_user_message`, read
// from its source, not assumed) already has a same-thread fast path that
// runs a queued closure in place rather than truly waiting on the event
// loop — which is why a single naive reentrant call is not automatically
// fatal on its own, and why the precise Win32-level reason `register`
// wedges while `unregister` does not could not be pinned down further from
// this box: there is no Windows machine or debugger here to attach to the
// actual hang (see this module's own header above on what this box can and
// cannot prove about Windows runtime behaviour). What removes the hazard
// ENTIRELY, independent of that unresolved detail, is that an async
// command's own IPC dispatch never blocks the main thread waiting on the
// command's body at all (`respond_async`, in tauri's own `ipc/mod.rs`,
// spawns it) — so the main thread's event loop is always free to service
// whatever `run_main_thread!` posts to it, whichever thread the actual OS
// call ends up running on. Beck's `SendInput` re-test against a real build
// is what proves this closed, not this box — see the report this shipped
// with for the two behavioural gates.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

use crate::companion;

/// Only `main` runs the assistant pipeline at all — see `ui/index.html`'s
/// `COMPANION` constant and its header, which hard-disables
/// `armMicIfWanted()`/`submit()`/`speak()` on the companion surface. Scoping
/// the emit here is belt-and-braces on top of that JS-side gate, not a
/// substitute for it: `emit_to` is the exact fix `e78b4a1` shipped for the
/// double-voice bug (see `main.rs`'s `AppSink` and its own regression test)
/// and every new emit in this app follows that shape now, not just the ones
/// the old bug happened to touch.
const MAIN_LABEL: &str = "main";

/// "M" for mute — a real chord, not a symbol, so it reads back cleanly in a
/// tooltip or a settings row later. `Ctrl+Shift+M` shares no key with
/// companion's `Ctrl+Shift+Space` default or the push-to-talk default below.
const DEFAULT_MUTE_HOTKEY: &str = "Ctrl+Shift+M";

/// Push-to-talk, held. Backquote (the key above Tab, left of "1") sits under
/// a finger nobody is using to type a sentence, and it is not a function key
/// a screen-reader or a window manager is likely to already own.
const DEFAULT_PTT_HOTKEY: &str = "Ctrl+Shift+Backquote";

/// The chords currently holding each slot, so a future re-registration (a
/// settings UI, if one gets built) can let go of exactly its own shortcut —
/// see this module's own header and `companion.rs`'s `CURRENT` for why
/// `unregister_all()` must never be used here either.
static CURRENT_MUTE: Mutex<Option<Shortcut>> = Mutex::new(None);
static CURRENT_PTT: Mutex<Option<Shortcut>> = Mutex::new(None);

#[derive(Clone, Serialize)]
struct PttEvent {
    pressed: bool,
}

/// Registered once from `main()`'s `.setup()`, right after
/// `companion::register_default_hotkey`. Order between the two does not
/// matter for correctness (neither calls `unregister_all()` any more — see
/// both modules' headers) but keeping mic hotkeys declared right after the
/// hotkey that established the pattern is easier for the next person to find.
/// It DOES matter for `register_default_hotkeys` below, though: it reads
/// `companion::current_hotkey()` to check for a collision, so companion must
/// already be registered by the time this runs — which `main.rs`'s call
/// order already guarantees, and which is exactly why that order is called
/// out there rather than left to coincidence.
///
/// BEST-EFFORT AND NON-FATAL, same reasoning as every other optional
/// capability wired up in `setup()`: a mute or push-to-talk chord that failed
/// to register (already owned by another app, or by the OS) is a worse first
/// run than an app that starts anyway and simply has no working shortcut for
/// it yet. Each failure is printed, never swallowed — `startup.rs`'s own
/// header on silent launch failures applies here too.
pub(crate) fn register_default_hotkeys(app: &AppHandle) {
    if let Err(e) = register_mute(app, DEFAULT_MUTE_HOTKEY) {
        eprintln!("mute hotkey: {e}");
    }
    // PUSH-TO-TALK STARTS FROM WHATEVER WAS LAST SAVED — see
    // `register_global_shortcut`'s doc for where that file comes from and
    // why it exists at all. `saved_ptt_chord` already fails toward "nothing
    // saved" on a missing, unreadable or corrupt file, so the only new
    // failure mode here is a saved chord that no longer VALIDATES (hand-
    // edited, or made sense on a build with a different mute/companion
    // default) — `register_ptt` runs it through the exact same
    // `validate_ptt_chord` the live command uses, so a bad saved chord is
    // caught here too, not just when the person next opens settings.
    let chord = saved_ptt_chord(app).unwrap_or_else(|| DEFAULT_PTT_HOTKEY.to_string());
    if let Err(e) = register_ptt(app, &chord) {
        eprintln!("push-to-talk hotkey ({chord}): {e}");
        // THE SAVED CHORD DID NOT TAKE. Fall back to the compiled-in default
        // rather than leaving a cold start with NO working push-to-talk at
        // all until someone opens settings and records a new one — same
        // "never leave them with nothing" reasoning as
        // `register_global_shortcut`'s own doc, applied to the boot path
        // instead of the live one. Skipped if the saved chord WAS already
        // the default, so a genuinely broken default fails once and says so
        // once, rather than twice into the same wall.
        if chord != DEFAULT_PTT_HOTKEY {
            if let Err(e2) = register_ptt(app, DEFAULT_PTT_HOTKEY) {
                eprintln!("push-to-talk hotkey (default fallback): {e2}");
            }
        }
    }
}

/// The OS-touching half of `register_mute` — swaps `CURRENT_MUTE` and
/// tells the platform about it, given a `Shortcut` that is already known to
/// be valid. Split out on 2026-09-05, the same day and for the same reason
/// as `companion.rs`'s `attach()`: `resume_global_shortcuts_after_recording`
/// below needs to put the mute chord back from a `Shortcut` it already has
/// in `CURRENT_MUTE`, without re-parsing a string that was only ever going
/// to parse back into the exact value it started from.
///
/// PRESSED ONLY. A hard mute is a toggle, not a hold — see `attach_ptt`
/// below for the hold case, which genuinely needs both states. Reacting to
/// Released here too would flip the mute back off the instant the same
/// keystroke that turned it on finished, which is the identical "looks like
/// the hotkey does nothing" trap companion.rs's own Pressed-only comment
/// already names.
fn attach_mute(app: &AppHandle, shortcut: Shortcut) -> Result<(), String> {
    let gs = app.global_shortcut();
    if let Some(prev) = CURRENT_MUTE.lock().unwrap().take() {
        let _ = gs.unregister(prev);
    }
    gs.on_shortcut(shortcut, |app, _shortcut, event| {
        if event.state() == ShortcutState::Pressed {
            let _ = app.emit_to(MAIN_LABEL, "mic:mute-toggle", ());
        }
    })
    .map_err(|e| format!("could not register the mute hotkey: {e}"))?;
    *CURRENT_MUTE.lock().unwrap() = Some(shortcut);
    Ok(())
}

fn register_mute(app: &AppHandle, chord: &str) -> Result<(), String> {
    let shortcut: Shortcut = chord
        .parse()
        .map_err(|e| format!("'{chord}' is not a shortcut I understand: {e}"))?;
    attach_mute(app, shortcut)
}

/// The OS-touching half of `checked_register_ptt` — attaches the handler
/// for an ALREADY-VALIDATED `Shortcut`. Split out on 2026-09-05, same reason
/// as `attach_mute` above and `companion.rs`'s `attach()`: `reactivate_ptt`
/// below needs to re-attach whatever `CURRENT_PTT` already holds when a
/// recording session ends, and that chord was validated the last time it
/// was actually chosen — re-running `validate_ptt_chord` against it now
/// would only ever re-confirm what is already true, at the cost of reading
/// `CURRENT_MUTE` and `companion::current_hotkey()` a second time for
/// nothing. Does NOT touch `CURRENT_PTT`, same as `checked_register_ptt`
/// before it: callers decide when and whether to update it.
///
/// BOTH STATES, ON PURPOSE — the one place in this app that wants
/// `Released` at all. The front end opens its "you are being addressed, no
/// wake word needed" window on Pressed and gives any trailing speech a
/// moment to land on Released; see `pttStart()`/`pttEnd()` in
/// `ui/index.html`.
fn attach_ptt(app: &AppHandle, shortcut: Shortcut) -> Result<(), String> {
    app.global_shortcut()
        .on_shortcut(shortcut, |app, _shortcut, event| {
            let pressed = event.state() == ShortcutState::Pressed;
            let _ = app.emit_to(MAIN_LABEL, "mic:ptt", PttEvent { pressed });
        })
        .map_err(|e| format!("could not register '{shortcut}': {e}"))
}

/// Shared by the boot path (`register_ptt`) and the live re-registration
/// command (`register_global_shortcut`) so there is exactly one place that
/// decides what counts as a valid push-to-talk chord. Validates — see
/// `validate_ptt_chord`'s own doc for what that checks and why it cannot be
/// left to the OS — then hands off to `attach_ptt` for the actual
/// registration. Does NOT touch `CURRENT_PTT`; callers decide when and
/// whether to update it, because the boot path and the live command need
/// different answers to "what happens to the OLD binding if this fails"
/// (see `register_global_shortcut`'s doc).
fn checked_register_ptt(app: &AppHandle, chord: &str) -> Result<Shortcut, String> {
    let mute = *CURRENT_MUTE.lock().unwrap();
    let companion_chord = companion::current_hotkey();
    let shortcut = validate_ptt_chord(chord, mute, companion_chord)?;
    attach_ptt(app, shortcut)?;
    Ok(shortcut)
}

fn register_ptt(app: &AppHandle, chord: &str) -> Result<(), String> {
    let shortcut = checked_register_ptt(app, chord)?;
    // Registered first, unregistered second — see `register_global_shortcut`
    // for why that order matters for the live command. At boot `CURRENT_PTT`
    // is always `None`, so this branch never actually runs here; it is kept
    // in this shape anyway so `register_ptt` stays correct if anything other
    // than boot ever calls it directly, rather than being a special case
    // that only happens to be safe today.
    if let Some(prev) = CURRENT_PTT.lock().unwrap().take() {
        if prev != shortcut {
            let _ = app.global_shortcut().unregister(prev);
        }
    }
    *CURRENT_PTT.lock().unwrap() = Some(shortcut);
    Ok(())
}

/// Re-attaches whatever chord `CURRENT_PTT` already holds, without
/// re-validating it (see `attach_ptt`'s doc for why) and without changing
/// which chord that is. If `CURRENT_PTT` is `None` — push-to-talk failed to
/// register at startup, or nothing has registered one yet — this is a
/// no-op: there is nothing to put back, and that is a normal state, not a
/// failure. Used only by `resume_global_shortcuts_after_recording` below.
fn reactivate_ptt(app: &AppHandle) -> Result<(), String> {
    let current = *CURRENT_PTT.lock().unwrap();
    match current {
        Some(shortcut) => attach_ptt(app, shortcut),
        None => Ok(()),
    }
}

/// The shape every push-to-talk chord must have, and the two ways a chord
/// that PARSES can still be wrong for this job.
///
/// **A BARE KEY PARSES FINE AND MUST STILL BE REJECTED.** The vendored
/// `global-hotkey` 0.8.0 grammar (`hotkey.rs::parse_hotkey`) refuses a chord
/// with NO trailing key at all — see this module's own
/// `modifier_only_chords_do_not_parse` test — but it happily accepts the
/// opposite shape: a lone key with no modifier parses as a valid `Shortcut`
/// on its own (`"A"`, `"F1"`, `"Escape"` all parse). A bare key as a
/// SYSTEM-WIDE push-to-talk chord would take that key over from every other
/// window on the machine, which `.parse()` has no way to know is wrong, so
/// it is checked here instead.
///
/// **A CHORD IDENTICAL TO THE MUTE OR COMPANION CHORD MUST BE REJECTED, AND
/// THE OS CANNOT BE TRUSTED TO CATCH IT.** Read from
/// `platform_impl/windows/mod.rs::register`, not assumed: every shortcut in
/// this app shares ONE window handle (`GlobalHotKeyManager::hwnd`), and the
/// Win32 id passed to `RegisterHotKey` is `HotKey::id()`, which
/// (`hotkey.rs::HotKey::new`) is a pure hash of `mods` and `key` alone —
/// nothing about WHICH purpose registered it. Windows returns
/// `ERROR_HOTKEY_ALREADY_REGISTERED` only when a DIFFERENT id on the SAME
/// hwnd already owns that mods+key combination; but two chords that are
/// textually identical hash to the IDENTICAL id, and MSDN's own documented
/// behaviour for "same hwnd, same id, registered again" is a silent REPLACE,
/// not a refusal. So choosing Ctrl+Shift+M as push-to-talk would not error —
/// it would silently steal the mute chord's OS registration, and the mute
/// key would keep "working" while quietly firing push-to-talk instead. This
/// has to be caught here, before either the OS or the shared HashMap in
/// `tauri-plugin-global-shortcut`'s own `GlobalShortcut::shortcuts` (also
/// keyed by that same id) ever sees the second registration.
///
/// **PURE ON PURPOSE.** `mute` and `companion` are passed in rather than read
/// from `CURRENT_MUTE`/`companion::current_hotkey()` here, so this function
/// can be unit-tested (see the tests below) without touching either global —
/// same reasoning the module header already gives for why `Shortcut::parse`
/// itself is safe to test on this box despite only mattering on Windows.
fn validate_ptt_chord(
    chord: &str,
    mute: Option<Shortcut>,
    companion_chord: Option<Shortcut>,
) -> Result<Shortcut, String> {
    let shortcut: Shortcut = chord
        .parse()
        .map_err(|e| format!("'{chord}' is not a shortcut I understand: {e}"))?;
    if shortcut.mods.is_empty() {
        return Err(
            "a push-to-talk shortcut needs at least one modifier (Ctrl, Alt, Shift or Win) \
             plus another key — a bare key would take that key over everywhere on your computer"
                .to_string(),
        );
    }
    if mute == Some(shortcut) {
        return Err(
            "that combination is already your mute shortcut — pick a different one for push-to-talk"
                .to_string(),
        );
    }
    if companion_chord == Some(shortcut) {
        return Err(
            "that combination already calls up the companion window — pick a different one for push-to-talk"
                .to_string(),
        );
    }
    Ok(shortcut)
}

/// **THE PUSH-TO-TALK CHORD'S OWN TINY FILE** — same shape as
/// `providers.rs`'s `AirgapFile`, and for the same reason stated there: this
/// is a property of the KEYBOARD, not of the profile or of any one provider,
/// so it does not belong nested inside either, and a hand-edited or
/// corrupted `profile.json` or `providers.json` can never silently move it
/// as a side effect of an edit about something else.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct PttHotkeyFile {
    #[serde(default)]
    chord: Option<String>,
}

fn ptt_hotkey_path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_config_dir()
        .map_err(|e| format!("no config directory: {e}"))?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("could not create {dir:?}: {e}"))?;
    Ok(dir.join("ptt-hotkey.json"))
}

/// Reads back whatever chord was last saved. **Fails toward "nothing saved",
/// never toward a panic or a poisoned default** — same reasoning as
/// `providers.rs::is_airgapped`'s own doc: a missing file, an unreadable one,
/// or one that will not parse as JSON are all indistinguishable from "nobody
/// has recorded one yet," and a fresh install and a corrupted file must land
/// in the same safe place (the compiled-in `DEFAULT_PTT_HOTKEY`, applied by
/// `register_default_hotkeys`).
fn saved_ptt_chord(app: &AppHandle) -> Option<String> {
    let path = ptt_hotkey_path(app).ok()?;
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str::<PttHotkeyFile>(&text)
        .ok()?
        .chord
        .filter(|c| !c.is_empty())
}

fn save_ptt_chord(app: &AppHandle, chord: &str) -> Result<(), String> {
    let path = ptt_hotkey_path(app)?;
    let body = serde_json::to_string(&PttHotkeyFile {
        chord: Some(chord.to_string()),
    })
    .map_err(|e| e.to_string())?;
    std::fs::write(&path, body).map_err(|e| format!("could not write {path:?}: {e}"))
}

/// **THE COMMAND `ui/index.html`'s onboarding Step 3 recorder calls** —
/// `invoke('register_global_shortcut', { chord })`, restated in that file's
/// own header as "does not exist yet" until this landed. The chord string it
/// sends is whatever `chordFromEvent()` builds there: zero or more of
/// `Ctrl`/`Alt`/`Shift`/`Meta` joined with `+`, then exactly one more token —
/// either a literal like `Space`/`Escape`/`ArrowUp`, a single letter/digit
/// (`A`, `1`), or (for Backquote and its punctuation neighbours) the literal
/// character itself, e.g. `` Ctrl+Shift+` ``. That is already the exact
/// grammar `Shortcut::from_str` accepts (`hotkey.rs::parse_key` matches the
/// bare punctuation characters and uppercases every token before comparing —
/// confirmed by reading that match arm, not assumed) — so no translation
/// layer is needed here, and none should be added on either side without
/// updating this doc.
///
/// **TWO THINGS THIS MUST GET RIGHT THAT `companion::set_companion_hotkey`
/// DID NOT HAVE TO, BOTH FROM THIS TASK'S BRIEF:**
///
/// **1. A refused re-registration must leave the OLD chord working.**
/// `companion::register()` unregisters the previous chord FIRST and only
/// then attempts the new one, which is fine for a chord nobody depends on
/// mid-session yet (companion's Step 3 is the same onboarding flow that is
/// choosing it). Push-to-talk is different: a person calling this command is
/// very possibly replacing the ONLY working push-to-talk they currently
/// have. So this registers the NEW chord first; the OLD one is only
/// unregistered after the new one has actually succeeded. If the OS refuses
/// the new one (already owned by another app), the old registration was
/// never touched and keeps working, and the caller gets a real error to
/// show instead of a silently dead shortcut.
///
/// **2. Re-submitting the SAME chord must not delete it.** If the new and
/// old chords are identical, `checked_register_ptt` re-registers the same
/// Win32 id (a harmless no-op replace — see `validate_ptt_chord`'s doc on
/// same-id-same-hwnd behaviour), and unregistering "the old one" afterwards
/// would then unregister that SAME id, turning a no-op into "push-to-talk
/// stopped working." Guarded below by comparing the two `Shortcut`s before
/// ever calling `unregister`.
///
/// **PERSISTED, NOT JUST HELD IN MEMORY** — `save_ptt_chord`, to
/// `ptt-hotkey.json` next to this app's other tiny per-concern settings
/// files (see `providers.rs::AirgapFile` for the established pattern this
/// follows). The front end's own `localStorage` copy (`HOTKEY_KEY` in
/// `ui/index.html`) is real but is a JS-side fact only Chromium's storage
/// can see; it is never re-sent to this command on a cold start (onboarding
/// only calls `register_global_shortcut` from the Record button, not from
/// page load), so without a Rust-side copy the OS binding would silently
/// reset to `DEFAULT_PTT_HOTKEY` on every relaunch even after someone had
/// successfully recorded a different one. The save is best-effort and
/// happens AFTER the OS registration already succeeded: a failed write here
/// means the next cold start falls back to whatever was saved before (or the
/// built-in default), not a reason to undo a working registration the
/// person is relying on for the rest of this session.
///
/// **OS ACCEPTANCE IS THE SAME UNVERIFIABLE-HERE CLAIM AS EVERYTHING ELSE IN
/// THIS MODULE** — see the module header. This box proves the chord parses,
/// passes the shape/collision checks above, and that the plugin call itself
/// returns `Ok`; whether Windows genuinely hands the chord to this process
/// end to end is Beck's to confirm on the real box.
///
/// **`async`, AND THE OS CALL NOW RUNS OFF THE MAIN THREAD — 2026-09-06.**
/// See this module's own header for why: this command's OWN registration
/// call (`checked_register_ptt` → `attach_ptt` → the plugin's `on_shortcut`)
/// goes through the exact same main-thread dispatch that hung
/// `resume_global_shortcuts_after_recording`, and it is why `ptt-hotkey.json`
/// was never written after a freeze — `save_ptt_chord` below was never wrong,
/// this function just never reached it. The body is otherwise byte-for-byte
/// what this command used to be, moved to `register_global_shortcut_blocking`
/// so it runs on a blocking-pool thread instead of whichever thread received
/// the IPC call.
#[tauri::command]
pub(crate) async fn register_global_shortcut(app: AppHandle, chord: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || register_global_shortcut_blocking(app, chord))
        .await
        .map_err(|_| "the app could not finish that registration — try again".to_string())?
}

fn register_global_shortcut_blocking(app: AppHandle, chord: String) -> Result<(), String> {
    let previous = *CURRENT_PTT.lock().unwrap();
    let shortcut = checked_register_ptt(&app, &chord).map_err(|e| {
        format!(
            "{e} — your current push-to-talk shortcut is still active",
        )
    })?;

    if let Some(prev) = previous {
        if prev != shortcut {
            let _ = app.global_shortcut().unregister(prev);
        }
    }
    *CURRENT_PTT.lock().unwrap() = Some(shortcut);

    if let Err(e) = save_ptt_chord(&app, &chord) {
        eprintln!(
            "push-to-talk hotkey registered but could not be saved for next launch: {e}"
        );
    }
    Ok(())
}

/// **THE SUSPEND/RESUME PAIR — 2026-09-05, item 3's missing half.** Wren
/// traced Mark's "hotkey listens but doesn't set the press" report to the
/// root: onboarding Step 3's recorder listens for a raw keydown in the
/// webview to learn a NEW chord, but `RegisterHotKey` does not deliver
/// keydown/keyup to any window at all — it delivers `WM_HOTKEY`, and only
/// to the process that registered it (see this module's own header on the
/// vendored crate's Windows implementation). So while any of the three
/// chords already registered here happen to match what the person is
/// pressing, the OS eats the keystroke before the webview's own listener
/// ever sees it, and the recorder looks dead. These two commands are what
/// the UI calls immediately before and after it listens, so for that one
/// window every chord in this app is out of the OS's hands and the raw
/// keys reach the page instead.
///
/// **`SUSPENDED` IS ONE FLAG FOR ALL THREE CHORDS, NOT THREE.** A recorder
/// either is or is not currently listening; there is no scenario where
/// mute, push-to-talk and the companion chord need to be suspended on
/// separate schedules; and the brief's own idempotency requirement (a
/// double-suspend or a double-resume must not corrupt anything) is far
/// easier to prove about one boolean than about three that could drift out
/// of step with each other.
///
/// **THE STATE MACHINE ITSELF LIVES IN `SuspendState`, BELOW, RATHER THAN AS
/// A BARE `bool` INLINE IN THESE TWO FUNCTIONS.** Pulling it out is what
/// makes "unit-test the suspend/resume state machine as far as possible
/// without a live OS" (this task's own brief) possible at all: `SuspendState`
/// needs no `Mutex`, no `AppHandle` and no `GlobalShortcut` manager to test,
/// only two states and the four transitions between them — see the tests
/// below. The two commands here are then a thin, deliberately un-clever
/// wrapper: ask the state machine whether there is anything to do, and if
/// there is, do the OS-touching part.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SuspendState {
    Live,
    Suspended,
}

impl SuspendState {
    /// Returns `true` the one time this actually moves the state from
    /// `Live` to `Suspended`, and `false` on a second, third, or Nth call
    /// while already suspended — a double-suspend must not walk this state
    /// machine any further, and it must not cause the caller to unregister
    /// shortcuts that a moment ago it already unregistered.
    fn enter_suspend(&mut self) -> bool {
        if *self == SuspendState::Suspended {
            false
        } else {
            *self = SuspendState::Suspended;
            true
        }
    }

    /// The mirror of `enter_suspend`: `true` only on the transition back to
    /// `Live`. A resume nobody suspended for — the recorder was never
    /// opened, or it already closed once — must be a safe no-op rather than
    /// an attempt to re-attach chords that were never taken down.
    fn enter_resume(&mut self) -> bool {
        if *self == SuspendState::Live {
            false
        } else {
            *self = SuspendState::Live;
            true
        }
    }
}

static SUSPEND_STATE: Mutex<SuspendState> = Mutex::new(SuspendState::Live);

/// **`invoke('suspend_global_shortcuts_for_recording')`** — called by the
/// onboarding Step 3 recorder right before it starts listening for a raw
/// key combo. Unregisters all three of this app's live shortcuts (mute,
/// push-to-talk, and the companion window's own chord) from the OS, using
/// whatever each one's own tracker (`CURRENT_MUTE`, `CURRENT_PTT`,
/// `companion::current_hotkey()`) says is currently bound — the same
/// trackers `register_mute`/`register_ptt`/`companion::register` already
/// keep up to date, so there is no second source of truth to fall out of
/// sync with them.
///
/// **DELIBERATELY DOES NOT CLEAR THOSE TRACKERS.** Only the OS-side
/// registration comes down; `CURRENT_MUTE`, `CURRENT_PTT` and companion's
/// `CURRENT` keep holding the chord that SHOULD be active once recording
/// ends. That is what lets `resume_global_shortcuts_after_recording` below
/// re-read them directly instead of being handed a snapshot this function
/// took — see that function's own doc for why a cached snapshot was
/// rejected as the design.
///
/// **A SECOND CALL WHILE ALREADY SUSPENDED IS A NO-OP**, guarded by
/// `SuspendState::enter_suspend`. Without the guard, calling this twice in a
/// row would call `gs.unregister` on shortcuts the first call already took
/// down — harmless to the OS (unregistering an already-unregistered id just
/// fails, and the failure is ignored the same way every other unregister in
/// this codebase ignores it), but pointless OS traffic the guard makes
/// unnecessary to reason about.
///
/// Unregister failures here are swallowed, not surfaced — same convention
/// as every other `gs.unregister(prev)` call in this file and in
/// `companion.rs`: a shortcut that fails to unregister because it was
/// already gone is not a failure at all. So this always returns `Ok(())`
/// today — the `Result<(), String>` signature exists anyway to mirror
/// `resume_global_shortcuts_after_recording` (the shape `ui/index.html`'s
/// own FLAG-FOR-MASON comment specifies for both), so a future caller can
/// treat the pair identically without checking which one is allowed to
/// fail.
///
/// **`async` — 2026-09-06, same reasoning as `register_global_shortcut`
/// above.** Beck's test found THIS half already stable even as a plain
/// `fn`, but making only the sibling `resume_*` command async while this one
/// stayed synchronous would leave two commands that are meant to be called
/// as a pair on genuinely different threading models — see this module's
/// header on why that asymmetry is not worth carrying forward once the real
/// fix exists. The body is unchanged, moved verbatim to
/// `suspend_global_shortcuts_for_recording_blocking`.
#[tauri::command]
pub(crate) async fn suspend_global_shortcuts_for_recording(app: AppHandle) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        suspend_global_shortcuts_for_recording_blocking(app)
    })
    .await
    .map_err(|_| "the app could not finish that — try again".to_string())?
}

fn suspend_global_shortcuts_for_recording_blocking(app: AppHandle) -> Result<(), String> {
    if !SUSPEND_STATE.lock().unwrap().enter_suspend() {
        return Ok(());
    }
    let gs = app.global_shortcut();
    if let Some(shortcut) = *CURRENT_MUTE.lock().unwrap() {
        let _ = gs.unregister(shortcut);
    }
    if let Some(shortcut) = *CURRENT_PTT.lock().unwrap() {
        let _ = gs.unregister(shortcut);
    }
    if let Some(shortcut) = companion::current_hotkey() {
        let _ = gs.unregister(shortcut);
    }
    Ok(())
}

/// **`invoke('resume_global_shortcuts_after_recording')`** — called when the
/// recorder stops listening, whether the person saved a new chord or backed
/// out. Puts back whatever should currently be active for each of the three
/// chords.
///
/// **RE-READS THE LIVE TRACKERS RATHER THAN USING A VALUE SAVED AT SUSPEND
/// TIME — this task's own brief, and it is what lets suspend and resume be
/// two independent calls with no handshake between them.** If this
/// function instead took a snapshot argument produced by the suspend call,
/// the two would have to agree on that value across whatever the UI does in
/// between, and a UI bug that called them out of order, or dropped one,
/// would resurrect a stale chord instead of the current one. Reading
/// `CURRENT_MUTE`/`CURRENT_PTT`/`companion::current_hotkey()` fresh, right
/// now, means resume always restores whatever is genuinely supposed to be
/// active at the moment it runs — including a chord `register_global_shortcut`
/// happened to change while shortcuts were suspended, if the UI ever ends up
/// calling things in that order.
///
/// **EACH OF THE THREE IS ATTEMPTED INDEPENDENTLY, AND ONE FAILING DOES NOT
/// STOP THE OTHERS** — this task's own brief: never leave the person with NO
/// shortcuts because the OS refused to hand back one of the three. Companion
/// is re-attached first only because `checked_register_ptt`'s own collision
/// check reads `companion::current_hotkey()` live (see `attach_ptt`'s doc);
/// resuming it before push-to-talk means that check, if push-to-talk ever
/// routes back through `checked_register_ptt` here in the future, sees the
/// real, already-restored companion chord rather than a stale gap. Failures
/// are collected and returned as one combined `Err` so the UI can decide
/// whether to tell the person something did not come back — this is
/// deliberately NOT the silent `eprintln!`-and-carry-on pattern
/// `register_default_hotkeys` uses at startup, because that pattern exists
/// for a cold start nobody is watching, and this runs while someone is
/// actively sitting in the recorder UI waiting for their shortcuts to work
/// again.
///
/// **A SECOND CALL WHILE ALREADY LIVE — OR A STRAY CALL WITH NOTHING EVER
/// SUSPENDED — IS A SAFE NO-OP**, guarded by `SuspendState::enter_resume`.
/// And per-chord, if a given tracker is `None` (that chord was never
/// registered in the first place — a startup failure, most likely),
/// `companion::reactivate` and `reactivate_ptt` are already no-ops for that
/// case, and the mute arm here mirrors them with the same `if let Some`.
///
/// **THIS WAS THE HALF THAT ACTUALLY DEADLOCKED — 2026-09-06, and `async`
/// below is the fix.** See this module's own header for the full read of
/// `tauri-plugin-global-shortcut`'s `run_main_thread!` macro and why a
/// synchronous command already running on the main thread cannot also make
/// the main thread wait on itself. The body is unchanged — still re-reads
/// the live trackers, still attempts companion, mute and push-to-talk
/// independently, still collects failures rather than stopping at the
/// first one — moved verbatim to `resume_global_shortcuts_after_recording_
/// blocking`, which now runs on a blocking-pool thread instead of whatever
/// thread received the IPC call.
#[tauri::command]
pub(crate) async fn resume_global_shortcuts_after_recording(
    app: AppHandle,
) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        resume_global_shortcuts_after_recording_blocking(app)
    })
    .await
    .map_err(|_| "the app could not finish that — try again".to_string())?
}

fn resume_global_shortcuts_after_recording_blocking(app: AppHandle) -> Result<(), String> {
    if !SUSPEND_STATE.lock().unwrap().enter_resume() {
        return Ok(());
    }

    let mut failures: Vec<String> = Vec::new();

    if let Err(e) = companion::reactivate(&app) {
        failures.push(format!("companion window hotkey: {e}"));
    }
    if let Some(shortcut) = *CURRENT_MUTE.lock().unwrap() {
        if let Err(e) = attach_mute(&app, shortcut) {
            failures.push(format!("mute hotkey: {e}"));
        }
    }
    if let Err(e) = reactivate_ptt(&app) {
        failures.push(format!("push-to-talk hotkey: {e}"));
    }

    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures.join("; "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Same limit as companion.rs's own tests: `Shortcut::parse` is pure
    /// string parsing with no OS call, so it is safe to run here even though
    /// the chord only fires for real on the Windows box this ships to.
    #[test]
    fn both_default_hotkeys_actually_parse() {
        assert!(
            DEFAULT_MUTE_HOTKEY.parse::<Shortcut>().is_ok(),
            "DEFAULT_MUTE_HOTKEY ({DEFAULT_MUTE_HOTKEY:?}) must itself be valid — \
             a bad literal fails silently at every cold start"
        );
        assert!(
            DEFAULT_PTT_HOTKEY.parse::<Shortcut>().is_ok(),
            "DEFAULT_PTT_HOTKEY ({DEFAULT_PTT_HOTKEY:?}) must itself be valid — \
             a bad literal fails silently at every cold start"
        );
    }

    /// MARK ASKED FOR "HOLD CTRL" AND THEN "CTRL+SHIFT IS FINE" — 2026-09-05.
    /// Both were investigated against the vendored `global-hotkey` 0.8.0
    /// source this module's header already points at, not the crate's docs:
    /// `hotkey.rs::parse_hotkey` splits on `+`, folds every recognised
    /// modifier token into `mods`, and requires exactly one *remaining*
    /// token to resolve through `parse_key` into a `Code` — there is no
    /// path through that function that returns `Ok` with `key` still
    /// `None`. A chord made of nothing but modifier words is therefore not
    /// a parse edge case, it is a shape the grammar has no slot for at all,
    /// and it fails identically whether it is one modifier or two. This
    /// test is that source-reading turned into something that actually
    /// runs, so the next person who tries "just drop the trailing key" gets
    /// a failing assert instead of a cold start where the hotkey silently
    /// never registers. See `platform_impl/windows/mod.rs::register` for
    /// why it can't be patched around at the Win32 layer either:
    /// `RegisterHotKey` is called with a `vk_code` this crate insists on
    /// having, there is no modifier-only branch to fall into.
    #[test]
    fn modifier_only_chords_do_not_parse() {
        assert!(
            "Ctrl".parse::<Shortcut>().is_err(),
            "a bare modifier must never silently succeed here — if this \
             starts passing, the crate changed and DEFAULT_PTT_HOTKEY's \
             trailing key may no longer be load-bearing"
        );
        assert!(
            "Ctrl+Shift".parse::<Shortcut>().is_err(),
            "two modifiers and no key is the same wall as one modifier — \
             see this test's own doc comment before ever dropping Backquote"
        );
    }

    /// The collision this module would otherwise ship with silently:
    /// `RegisterHotKey` on Windows refuses a chord already bound to another
    /// id in the SAME process (see this module's header on the shared
    /// manager), so if any two of these three defaults ever matched, one of
    /// the three would fail to register at every single cold start and the
    /// only symptom would be a hotkey that "just doesn't do anything." Three
    /// literal string comparisons here are cheaper than that discovery.
    #[test]
    fn the_three_default_chords_in_this_app_are_all_different() {
        // companion.rs's DEFAULT_HOTKEY is private to that module — restated
        // here as a literal rather than imported, which is also why this
        // test is the one place that would need updating if either default
        // ever changes.
        const COMPANION_DEFAULT: &str = "Ctrl+Shift+Space";
        assert_ne!(DEFAULT_MUTE_HOTKEY, DEFAULT_PTT_HOTKEY);
        assert_ne!(DEFAULT_MUTE_HOTKEY, COMPANION_DEFAULT);
        assert_ne!(DEFAULT_PTT_HOTKEY, COMPANION_DEFAULT);
    }

    /// `validate_ptt_chord`'s four required outcomes, per this task's own
    /// brief: modifier-only rejected, bare key rejected, a valid chord
    /// accepted, and a colliding chord rejected. Pure — no `AppHandle`, no
    /// plugin, no OS call — because `validate_ptt_chord` takes its `mute`
    /// and `companion_chord` inputs as plain arguments rather than reading
    /// the module's own globals; see that function's doc for why.
    #[test]
    fn validate_rejects_modifier_only() {
        let err = validate_ptt_chord("Ctrl+Shift", None, None).unwrap_err();
        assert!(
            err.contains("not a shortcut I understand"),
            "a modifier-only string fails to PARSE at all — this must surface \
             as that error, not the separate bare-key shape error below: got {err}"
        );
    }

    #[test]
    fn validate_rejects_a_bare_key_with_no_modifier() {
        // "F1" and "A" both PARSE fine on their own (see this module's own
        // header on `validate_ptt_chord`) — the rejection here has to come
        // from the modifier check, not from `.parse()` failing.
        let err = validate_ptt_chord("F1", None, None).unwrap_err();
        assert!(
            err.contains("at least one modifier"),
            "a bare key parses but must still be refused as a system-wide \
             push-to-talk chord: got {err}"
        );
    }

    #[test]
    fn validate_accepts_a_real_chord_with_no_collision() {
        let shortcut = validate_ptt_chord("Ctrl+Shift+Space", None, None)
            .expect("a real modifier+key chord with nothing to collide against must be accepted");
        assert_eq!(shortcut, "Ctrl+Shift+Space".parse::<Shortcut>().unwrap());
    }

    #[test]
    fn validate_rejects_a_collision_with_the_current_mute_chord() {
        let mute: Shortcut = DEFAULT_MUTE_HOTKEY.parse().unwrap();
        let err = validate_ptt_chord(DEFAULT_MUTE_HOTKEY, Some(mute), None).unwrap_err();
        assert!(
            err.contains("mute shortcut"),
            "picking the live mute chord as push-to-talk must be refused by \
             name, not by a generic OS error that may never actually fire — \
             see validate_ptt_chord's doc on why the OS cannot be trusted \
             to catch this on its own: got {err}"
        );
    }

    #[test]
    fn validate_rejects_a_collision_with_the_current_companion_chord() {
        let companion_chord: Shortcut = "Ctrl+Shift+Space".parse().unwrap();
        let err =
            validate_ptt_chord("Ctrl+Shift+Space", None, Some(companion_chord)).unwrap_err();
        assert!(
            err.contains("companion window"),
            "picking the live companion chord as push-to-talk must be \
             refused by name for the same reason as the mute case above: \
             got {err}"
        );
    }

    /// **A REAL MISMATCH BETWEEN THIS CRATE AND `ui/index.html`'s RECORDER,
    /// FOUND WHILE WRITING THIS TEST, NOT ASSUMED.** The recorder's
    /// `liveModsFromEvent()` sends the literal word `Meta` for a held Windows
    /// key (`e.metaKey`). The vendored `global-hotkey` 0.8.0 grammar
    /// (`hotkey.rs::parse_hotkey`'s modifier match arms) recognises
    /// `OPTION`/`ALT`, `CONTROL`/`CTRL`, `COMMAND`/`CMD`/`SUPER` and `SHIFT`
    /// — there is no `META` arm at all, so it falls into the `_ =>
    /// parse_key(token)` branch and fails as an unrecognised KEY, not a
    /// recognised modifier. So `"Meta+Shift+A"` — exactly what the recorder
    /// would send for Win+Shift+A — fails to parse here. It fails LOUDLY
    /// (a real `Err`, surfaced to the person as "not a shortcut I
    /// understand"), which is the safe direction for a mismatch to fail in,
    /// but the Windows-key modifier is otherwise unusable for push-to-talk
    /// until `ui/index.html`'s recorder sends `Super`, `Cmd` or `Command`
    /// instead of `Meta` for that one key — a UI change, so out of this
    /// module's scope; see this task's own report for the exact wording
    /// coordinated back.
    #[test]
    fn the_recorders_meta_token_does_not_match_this_crates_grammar() {
        let err = validate_ptt_chord("Meta+Shift+A", None, None).unwrap_err();
        assert!(
            err.contains("not a shortcut I understand"),
            "if this starts passing, the crate gained a META arm and the              note above (and the report this shipped with) is stale: {err}"
        );
    }

    /// A collision check keyed on the display STRING rather than the parsed
    /// `Shortcut` would miss this: `Ctrl+Shift+M` and `Shift+Ctrl+M` are
    /// different text and the identical binding (`Modifiers` is an unordered
    /// bitflag set — see `keyboard-types` 0.7.0's `Modifiers` derive). This
    /// pins that `validate_ptt_chord` compares the PARSED chord, not the
    /// string the person happened to type.
    #[test]
    fn validate_rejects_a_collision_regardless_of_modifier_order() {
        let mute: Shortcut = DEFAULT_MUTE_HOTKEY.parse().unwrap(); // "Ctrl+Shift+M"
        let err = validate_ptt_chord("Shift+Ctrl+M", Some(mute), None).unwrap_err();
        assert!(err.contains("mute shortcut"), "got {err}");
    }

    /// **`SuspendState`'s four transitions — the state machine behind the
    /// suspend/resume pair, tested on its own with no `Mutex`, no
    /// `AppHandle` and no OS call anywhere near it.** This is the "as far as
    /// possible without a live OS" half of this task's verification brief;
    /// whether `gs.unregister`/`gs.on_shortcut` actually do anything on a
    /// real Windows box is the same unverifiable-here claim as the rest of
    /// this module (see the header) and is Beck's to confirm.
    #[test]
    fn suspend_then_resume_is_the_ordinary_round_trip() {
        let mut state = SuspendState::Live;
        assert!(
            state.enter_suspend(),
            "the first suspend from Live must report that it actually moved something"
        );
        assert_eq!(state, SuspendState::Suspended);
        assert!(
            state.enter_resume(),
            "the first resume from Suspended must report that it actually moved something"
        );
        assert_eq!(state, SuspendState::Live);
    }

    #[test]
    fn a_second_suspend_before_any_resume_is_a_reported_no_op() {
        let mut state = SuspendState::Live;
        assert!(state.enter_suspend());
        assert!(
            !state.enter_suspend(),
            "a double-suspend must not report a fresh transition -- that is              the signal suspend_global_shortcuts_for_recording uses to skip              re-unregistering shortcuts that are already down"
        );
        // And it must not have moved the state anywhere stranger than
        // Suspended in the process.
        assert_eq!(state, SuspendState::Suspended);
    }

    #[test]
    fn a_resume_with_nothing_suspended_is_a_reported_no_op() {
        let mut state = SuspendState::Live;
        assert!(
            !state.enter_resume(),
            "a stray resume with nothing suspended must not report a              transition -- resume_global_shortcuts_after_recording relies on              this to skip re-attaching chords nobody took down"
        );
        assert_eq!(
            state,
            SuspendState::Live,
            "a no-op resume must leave the state exactly where it found it"
        );
    }

    #[test]
    fn a_second_resume_after_the_first_already_returned_is_a_reported_no_op() {
        let mut state = SuspendState::Live;
        assert!(state.enter_suspend());
        assert!(state.enter_resume());
        assert!(
            !state.enter_resume(),
            "a double-resume must not report a second transition -- the              OS-touching command guards on exactly this so it never              double-registers a chord that already came back"
        );
        assert_eq!(state, SuspendState::Live);
    }

    #[test]
    fn repeated_suspend_calls_never_leave_the_machine_anywhere_but_suspended_or_live() {
        // A cheap fuzz over interleavings, since the real risk with a
        // hand-rolled two-state machine is a transition table typo that
        // only shows up after a specific, unlikely sequence of calls.
        let mut state = SuspendState::Live;
        let mut suspend_transitions = 0;
        let mut resume_transitions = 0;
        // suspend, suspend, resume, resume, suspend, resume -- deliberately
        // includes two doubled calls in a row in both directions.
        for call in [true, true, false, false, true, false] {
            if call {
                if state.enter_suspend() {
                    suspend_transitions += 1;
                }
            } else if state.enter_resume() {
                resume_transitions += 1;
            }
            // The machine only ever has two states -- this is really just
            // pinning that `enter_suspend`/`enter_resume` never panic and
            // never produce a third value, which `PartialEq` on the enum's
            // two variants already makes exhaustive.
            assert!(state == SuspendState::Live || state == SuspendState::Suspended);
        }
        assert_eq!(suspend_transitions, 2, "one real suspend per genuine Live->Suspended move");
        assert_eq!(resume_transitions, 2, "one real resume per genuine Suspended->Live move");
        assert_eq!(state, SuspendState::Live);
    }
}
