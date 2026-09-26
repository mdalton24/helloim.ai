// The summonable companion — a second, always-on-top window, and the global
// hotkey that raises it. Spec: Iris, `~/Documents/Iris/helloim-facefirst-
// spec.md` §6 ("The new engineering — FLAG for Mason"). This module is that
// piece. Nothing like it existed before: the app was one opaque 1180×800
// workspace window with no way to reach it except the taskbar icon.
//
// WHY A SECOND WINDOW RATHER THAN RESHAPING `main`: "always-on-top, floats
// over your other work, summoned by a hotkey" is a property a single window
// cannot hold at two sizes at once — you cannot make the workspace both the
// console you work in AND a 380×430 floater sitting above everything else.
// `main` is untouched by this file; every function here only ever looks up
// or manipulates the window labelled `LABEL` ("companion" — see
// tauri.conf.json, which declares it statically, hidden, exactly like `main`
// is declared visible).
//
// TRANSPARENCY, ALWAYS-ON-TOP AND THE HOTKEY FIRING AT ALL ARE WINDOWS
// RUNTIME BEHAVIOUR THIS BOX CANNOT PROVE. This compiles here and the shapes
// match the vendor's own current docs (tauri 2.11.5 / tauri-plugin-global-
// shortcut 2.3.2, checked against docs.rs 2026-09-02 rather than recalled) —
// but "does it actually float over another program, does the OS actually
// hand this chord to us instead of eating it, does hide()/show() preserve
// position the way a real user expects" is Beck's to confirm on the real
// Windows box. See the report this shipped with for the exact ask.

use std::sync::Mutex;
use tauri::{AppHandle, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

/// The window label. `tauri.conf.json` declares a window with this exact
/// label; every function below fails cleanly (never panics) if that ever
/// stops being true, because a typo in the config and a typo here are two
/// different files agreeing by coincidence, not by the compiler.
pub(crate) const LABEL: &str = "companion";

/// The chord that shows/hides the companion until a person picks their own
/// in onboarding Step 3 (spec §4). Iris's own reasoning for this exact
/// string, kept here rather than re-derived: `Ctrl+Space` collides with IMEs
/// and Spotlight-likes, `Alt+Space` opens Windows' own system menu, so
/// `Ctrl+Shift+Space` is the one she named as the safe default. "Safe" does
/// not mean "proven free on every machine" — nothing short of trying is —
/// which is exactly why `set_companion_hotkey` below exists and reports a
/// real failure instead of pretending success.
const DEFAULT_HOTKEY: &str = "Ctrl+Shift+Space";

fn window(app: &AppHandle) -> Result<tauri::WebviewWindow, String> {
    app.get_webview_window(LABEL)
        .ok_or_else(|| format!("no window named '{LABEL}' — check tauri.conf.json"))
}

/// Show it and take focus. Shared by the hotkey handler and `show_companion`
/// so there is exactly one place that decides what "summoned" means.
pub(crate) fn show(app: &AppHandle) -> Result<(), String> {
    let w = window(app)?;
    w.show().map_err(|e| e.to_string())?;
    w.set_focus().map_err(|e| e.to_string())
}

pub(crate) fn hide(app: &AppHandle) -> Result<(), String> {
    window(app)?.hide().map_err(|e| e.to_string())
}

/// Show if hidden, hide if shown. This is what the hotkey actually calls.
///
/// `is_visible()` FAILING IS TREATED AS "NOT VISIBLE", NOT PROPAGATED — a
/// platform call that could not answer is exactly the moment to fall back to
/// the safer of the two outcomes. Showing a window that happened to already
/// be visible just re-focuses it, which is harmless; hiding one that was
/// actually hidden because a status query lied would mean the chord looks
/// dead. A person pressing a hotkey and getting nothing back is the worse
/// failure of the two.
pub(crate) fn toggle(app: &AppHandle) -> Result<(), String> {
    let w = window(app)?;
    if w.is_visible().unwrap_or(false) {
        w.hide().map_err(|e| e.to_string())
    } else {
        w.show().map_err(|e| e.to_string())?;
        w.set_focus().map_err(|e| e.to_string())
    }
}

#[tauri::command]
pub(crate) fn show_companion(app: AppHandle) -> Result<(), String> {
    show(&app)
}

#[tauri::command]
pub(crate) fn hide_companion(app: AppHandle) -> Result<(), String> {
    hide(&app)
}

#[tauri::command]
pub(crate) fn toggle_companion(app: AppHandle) -> Result<(), String> {
    toggle(&app)
}

/// The chord currently holding the companion's slot, so a re-registration
/// can let go of exactly that one. See `register()`'s own header for why
/// this exists and what it replaced.
static CURRENT: Mutex<Option<Shortcut>> = Mutex::new(None);

/// Register `chord` as the summon/dismiss hotkey, replacing whatever was
/// registered before.
///
/// THIS USED TO CALL `unregister_all()` FIRST, ALWAYS, ON THE REASONING THAT
/// "there is at most one companion hotkey in this app, ever, and the plugin
/// registers nothing else." That second clause is what changed: `mic_hotkeys.rs`
/// now holds its own two shortcuts (mute, push-to-talk) on this exact same
/// `GlobalShortcut` manager — `app.global_shortcut()` is one shared instance,
/// not one per module — and `unregister_all()` clears every shortcut on it,
/// whoever registered them. Left as it was, the day someone changes the
/// companion chord in onboarding Step 3, the mute hotkey and push-to-talk
/// would both silently stop working, with nothing in the mic UI to explain
/// why — exactly the kind of quiet regression this file's own comments exist
/// to stop the NEXT person from re-introducing.
///
/// So this now unregisters only the ONE shortcut this module itself last
/// registered, tracked in `CURRENT`. Still safe from a cold start (`CURRENT`
/// is `None`, nothing to unregister) and still fully replaces the old chord
/// on a change — it is just scoped to this module's own shortcut instead of
/// the whole manager.
///
/// **SPLIT INTO PARSING AND ATTACHING ON 2026-09-05** — see `attach()`
/// immediately below for the half that actually touches the OS and why it
/// needed to exist on its own.
fn register(app: &AppHandle, chord: &str) -> Result<(), String> {
    let shortcut: Shortcut = chord
        .parse()
        .map_err(|e| format!("'{chord}' is not a shortcut I understand: {e}"))?;
    attach(app, shortcut)
}

/// The OS-touching half of `register()` — swaps `CURRENT` and tells the
/// platform about it. Pulled out of `register()` on 2026-09-05 so
/// `reactivate()` below (added the same day for `mic_hotkeys.rs`'s
/// suspend/resume pair — see that module's header) can re-issue the
/// identical registration from a `Shortcut` it already has (read via
/// `current_hotkey()`), rather than going through `Shortcut`'s `Display`
/// output and back through `.parse()` a second time just to reach this
/// code. `register()` keeps the string parsing; this keeps everything that
/// happens after — the same "exactly one place decides what this means"
/// shape as `show()`/`toggle()` above, just for the OS registration instead
/// of the window.
///
/// PRESSED ONLY. A chord fires both Pressed and Released; toggling on both
/// would show the window and then hide it again on the same keystroke,
/// which looks exactly like the hotkey doing nothing.
fn attach(app: &AppHandle, shortcut: Shortcut) -> Result<(), String> {
    let gs = app.global_shortcut();
    if let Some(prev) = CURRENT.lock().unwrap().take() {
        let _ = gs.unregister(prev);
    }
    gs.on_shortcut(shortcut, |app, _shortcut, event| {
        if event.state() == ShortcutState::Pressed {
            if let Err(e) = toggle(app) {
                eprintln!("companion hotkey fired but the window did not respond: {e}");
            }
        }
    })
    .map_err(|e| format!("could not register the companion hotkey: {e}"))?;
    *CURRENT.lock().unwrap() = Some(shortcut);
    Ok(())
}

/// Read-only peek at whichever chord companion currently holds — added
/// 2026-09-05 so `mic_hotkeys.rs`'s push-to-talk registration can refuse to
/// collide with it. See that module's `validate_ptt_chord` for the full
/// reasoning; the short version: two different purposes bound to the
/// identical mods+key pair hash to the identical `RegisterHotKey` id
/// (`hotkey.rs::HotKey::new` hashes only mods+key, never which module asked
/// for it), and on the ONE window handle every shortcut in this app shares,
/// Windows treats a second registration of an id already held on that same
/// handle as a silent REPLACE rather than a refusal — confirmed by reading
/// `platform_impl/windows/mod.rs::register`, not assumed. So the OS cannot
/// be trusted to catch a companion/push-to-talk collision on its own, which
/// means whoever checks for it needs the chord companion is ACTUALLY bound
/// to right now — not `DEFAULT_HOTKEY`, which goes stale the moment
/// `set_companion_hotkey` runs.
///
/// `Shortcut` (`global_hotkey::hotkey::HotKey`) derives `Copy`, so this reads
/// out a value rather than handing back a reference into `CURRENT` — no
/// caller outside this module can ever observe or hold the lock.
pub(crate) fn current_hotkey() -> Option<Shortcut> {
    *CURRENT.lock().unwrap()
}

/// Re-issues the OS registration for whatever chord `CURRENT` already
/// holds, without changing which chord that is. Added 2026-09-05 for
/// `mic_hotkeys.rs`'s suspend/resume pair: a hotkey recorder needs every
/// shortcut in the app unregistered for a moment so the OS stops eating the
/// raw keydown/up the recorder itself needs to see, and afterwards each one
/// has to come back exactly as it was — `mic_hotkeys.rs` handles its own
/// mute and push-to-talk chords the same way (`attach_mute`/`reactivate_ptt`
/// in that module); this is companion's side of the same contract.
///
/// **RE-READS `CURRENT` ITSELF RATHER THAN TAKING A CHORD ARGUMENT, ON
/// PURPOSE.** The caller could pass back whatever it read from
/// `current_hotkey()` before suspending, but that would be exactly the
/// "cached snapshot" `mic_hotkeys.rs`'s own resume contract says not to use
/// — see that module's `resume_global_shortcuts_after_recording`. Reading
/// `CURRENT` fresh here means the two sides never have to agree on a value
/// in advance, which is what lets suspend and resume be two independent
/// calls with no handshake between them.
///
/// If `CURRENT` is `None` — companion's hotkey failed to register at
/// startup, or nothing has registered one yet — this is a no-op. There is
/// nothing to put back, and that is a normal state, not a failure.
pub(crate) fn reactivate(app: &AppHandle) -> Result<(), String> {
    match current_hotkey() {
        Some(shortcut) => attach(app, shortcut),
        None => Ok(()),
    }
}

/// Called once from `main()`'s `.setup()`.
///
/// BEST-EFFORT AND NON-FATAL, the same as every other optional capability
/// wired up in that block (the mic permission, the no-window watchdog): a
/// hotkey that failed to register is a worse first run than a companion
/// nobody can summon by keyboard yet, not a reason to refuse to start the
/// whole app. The failure is printed rather than swallowed — see
/// `startup.rs`'s own header on why silence about a failed launch step is
/// the thing this codebase keeps having to re-learn not to do.
pub(crate) fn register_default_hotkey(app: &AppHandle) {
    if let Err(e) = register(app, DEFAULT_HOTKEY) {
        eprintln!("companion hotkey: {e}");
    }
}

/// The onboarding Step 3 contract (spec §4): "must be re-registrable when
/// the user changes it, and must report failure if the chord is already
/// owned by the OS or another app." This command is that contract, built now
/// so the UI that calls it — recording a chord and showing the Step-3 error
/// state — is the only thing left to do.
#[tauri::command]
pub(crate) fn set_companion_hotkey(app: AppHandle, chord: String) -> Result<(), String> {
    register(&app, &chord)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The one thing about `register()` that is checkable without a real
    /// window manager: garbage in gets a real `Err`, not a panic and not a
    /// silent `Ok`. `Shortcut::parse` runs the same on every platform this
    /// crate builds for (it is pure string parsing, no OS call), so this is
    /// safe to run here even though the app it feeds only ships to Windows.
    #[test]
    fn a_chord_it_cannot_parse_is_a_reported_error_not_a_panic() {
        let parsed: Result<Shortcut, _> = "not a real shortcut".parse();
        assert!(parsed.is_err());
    }

    #[test]
    fn the_documented_default_hotkey_actually_parses() {
        let parsed: Result<Shortcut, _> = DEFAULT_HOTKEY.parse();
        assert!(
            parsed.is_ok(),
            "DEFAULT_HOTKEY ({DEFAULT_HOTKEY:?}) must itself be a valid chord — \
             a bad literal here would fail silently at every cold start, \
             caught only by someone noticing the hotkey never works"
        );
    }

    /// This module's regression, source-scanned rather than exercised —
    /// same shape as `app_sink_never_broadcasts_a_turn_event_to_every_window`
    /// in `main.rs`, and for the same reason: `global_hotkey::GlobalHotKeyManager`
    /// needs a real OS message loop `cargo test` does not have, so the OS-level
    /// behaviour is Beck's to prove on the real box (see the module header).
    /// What IS checkable here, cheaply and on every commit, is that the fix
    /// stays the fix: nothing in this file may call `unregister_all()` on
    /// the shared manager, because that call clears every shortcut on it —
    /// including `mic_hotkeys.rs`'s mute and push-to-talk chords, registered
    /// on the same `GlobalShortcut` instance — not just this module's own.
    ///
    /// **WIDENED FROM `register()`'S BODY TO THE WHOLE FILE (MINUS THIS
    /// TEST MODULE), 2026-09-05.** The original version of this test only
    /// scanned inside `register()`, which was fine while that was the only
    /// function that ever touched the manager. It was split into
    /// `register()` (parsing) and `attach()` (the actual OS call) the same
    /// day `reactivate()` was added for suspend/resume — a second function
    /// now does the touching, and a test still pointed at `register()`'s
    /// body alone would have gone on passing forever no matter what
    /// `attach()` or `reactivate()` did, which is worse than no test: it
    /// would have read as covered.
    ///
    /// Scanning stops at `mod tests {` on purpose. Everything above that
    /// line is production code, where the real regression lives; everything
    /// from there down is this module's own test prose describing the
    /// forbidden call, and a test that scanned its own source text would be
    /// tripped by describing what it checks for — the same self-reference
    /// problem a spell-checker has with the word "misspelled".
    #[test]
    fn no_function_in_this_file_calls_unregister_all() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/companion.rs");
        let text = std::fs::read_to_string(&path).expect("companion.rs is unreadable");
        let production_code = text
            .split("mod tests {")
            .next()
            .expect("split always yields at least one piece");
        let forbidden = [".", "unregister_all", "("].concat();
        assert!(
            !production_code.contains(&forbidden),
            "companion.rs calls .unregister_all() outside its test module — \
             that wipes mic_hotkeys.rs's mute and push-to-talk shortcuts too, \
             since all three share one GlobalShortcut manager"
        );
    }
}
