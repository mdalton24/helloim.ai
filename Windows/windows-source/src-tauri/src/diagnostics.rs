//! Crash and hang reporting — Beck's v1.0.1 report (`~/Documents/Beck/rel101/
//! REPORT.md`) and Della's open list, both the same finding: "there's no
//! crash or hang report." Before this file, a panic anywhere in the app was
//! already caught and written down (`startup::install_panic_hook`, wired in
//! `main.rs` since before this file existed) — but there was no way for a
//! PERSON to ever see that, and nothing at all caught the other half of the
//! bug report's name: a hang, where the process is still running and nothing
//! panicked, but the window has stopped answering.
//!
//! **WHAT THIS ADDS, AND ONLY THIS:**
//!   1. A watchdog that notices the window has stopped answering IPC and
//!      writes ONE line to the SAME log the panic hook already writes to
//!      (`startup::log_file()`) — see `install_hang_watchdog` below for why a
//!      heartbeat from the front end is the honest way to detect this.
//!   2. `open_crash_log`, a Settings command that opens that log for the
//!      person to read.
//!
//! **WHAT THIS IS NOT: TELEMETRY.** Nothing here ever leaves the machine on
//! its own, nothing here runs on a schedule to phone anything, and there is
//! no server this app talks to about any of it. The log is a plain text file
//! next to the one panics already land in, redacted by `startup::note()`'s
//! own rules before a byte is written — it holds our own short sentences,
//! versions and platform error strings, never chat content and never a key
//! (see `note()`'s own doc for exactly what that promise covers and where its
//! honest edges are). The person decides whether to open it, and decides
//! separately whether to paste any of it into a Discord message — the
//! Settings button for that (ui/index.html, "Get help on Discord") reuses the
//! ALREADY-EXISTING, already-scoped `connectors::open_url` command, which
//! only ever opens `https://discord.gg/RXh5xrkqwN`, a literal in that file,
//! never anything this module constructs or sends anywhere first.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// Wall-clock seconds since the Unix epoch, at whole-second resolution —
/// plenty for "how long has it been since the last heartbeat", and it keeps
/// this file from needing its own clock helper: `startup::now_secs()` already
/// does exactly this and is already proven (its own `utc_stamp` tests).
fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// The last time the front end proved it was still alive.
static LAST_HEARTBEAT_SECS: AtomicU64 = AtomicU64::new(0);

/// Set once a hang has been written to the log, so the watchdog logs it
/// exactly once per hang rather than once per check for as long as it lasts.
static HANG_LOGGED: AtomicBool = AtomicBool::new(false);

/// How long without a heartbeat before this is called a hang rather than a
/// slow moment. Same value and the same reasoning as `startup::WATCHDOG_SECS`
/// (that module's own "window never appears" watchdog): long enough that a busy
/// render frame or a heavy tool call in flight does not get called a hang —
/// a false alarm here is worse than a late one, because it teaches whoever
/// reads the log to stop trusting it.
const HANG_THRESHOLD_SECS: u64 = 20;

/// How often the watchdog thread checks. A fifth of the threshold, so a real
/// hang is caught within one interval of crossing it rather than waiting a
/// full extra threshold-length past the moment it became true.
const WATCHDOG_INTERVAL_SECS: u64 = 5;

/// Called by the front end on a timer (see `ui/index.html`) — evidence the
/// main thread is still processing IPC at all, which is the one thing a
/// genuinely hung window cannot do. Every call just records "now"; the
/// watchdog thread below is what decides what silence since the last one
/// means.
#[tauri::command]
pub fn heartbeat() {
    let was_hung = HANG_LOGGED.swap(false, Ordering::SeqCst);
    LAST_HEARTBEAT_SECS.store(now_secs(), Ordering::SeqCst);
    if was_hung {
        crate::startup::note("recovered — the app is responding to input again");
    }
}

/// Start the background thread that turns heartbeat silence into a log line.
///
/// **GATED ON A PAGE HAVING LOADED AT LEAST ONCE
/// (`startup::window_has_loaded_once`).** Before the first page load there
/// are no heartbeats yet by construction — the front end has not run a line
/// of JavaScript — and that case already has an owner:
/// `startup::watch_for_a_window_that_never_appears`, which logs it with
/// words that say "never started", not "stopped responding". Checking here
/// keeps this watchdog from firing a second, differently-worded alarm about
/// the exact same 20 seconds of silence.
///
/// **A LONG GAP BETWEEN CHECKS IS TREATED AS THE MACHINE HAVING SLEPT, NOT
/// THE APP HAVING HUNG.** This app is meant to run for hours or days, so a
/// laptop suspending and resuming is an ordinary event this watchdog will see
/// often — and a live thread cannot itself "run slow"; it can only be
/// descheduled, which is exactly what a system suspend does to every thread
/// at once, this one included. If the watchdog's OWN sleep took far longer
/// than it asked for, that is the signature of a suspend, and reporting it as
/// an hours-long hang would be the identical false-alarm mistake
/// `WATCHDOG_SECS`'s own comment already warns against — so it resets
/// quietly instead of logging anything.
pub fn install_hang_watchdog() {
    LAST_HEARTBEAT_SECS.store(now_secs(), Ordering::SeqCst);
    std::thread::spawn(|| {
        let mut last_check = Instant::now();
        loop {
            std::thread::sleep(Duration::from_secs(WATCHDOG_INTERVAL_SECS));

            let slept_for = last_check.elapsed();
            last_check = Instant::now();
            if slept_for > Duration::from_secs(WATCHDOG_INTERVAL_SECS * 3) {
                // The machine was asleep, not the app. Start the clock over
                // rather than judging a gap that was never silence from the
                // app's own main thread.
                LAST_HEARTBEAT_SECS.store(now_secs(), Ordering::SeqCst);
                HANG_LOGGED.store(false, Ordering::SeqCst);
                continue;
            }

            if !crate::startup::window_has_loaded_once() {
                continue;
            }

            let elapsed = now_secs().saturating_sub(LAST_HEARTBEAT_SECS.load(Ordering::SeqCst));
            if elapsed >= HANG_THRESHOLD_SECS {
                if !HANG_LOGGED.swap(true, Ordering::SeqCst) {
                    crate::startup::note(&format!(
                        "POSSIBLE HANG — no response from the app for {elapsed}s. \
                         If the window is frozen or not responding, this is why."
                    ));
                }
            }
        }
    });
}

/// Opens the local startup/crash/hang log for the person to read and decide
/// whether to share — the Settings button `ui/index.html` wires to this.
///
/// **NEVER SENDS ANYTHING.** This is the read half of the privacy promise
/// this module's header makes: the log already exists on disk before anyone
/// ever presses this button — a panic or a hang writes to it whether or not
/// the log is ever opened — and this command's entire job is showing it to
/// the person who owns the machine it is on, exactly once, only when they
/// choose to. Reuses `connectors::open_in_browser`, the same
/// injection-hardened opener every other "hand the OS a path or URL" command
/// in this crate already uses (see that function's own header for why a
/// hand-rolled `cmd /C start` was never safe here), rather than a second
/// implementation of "open a local path" for this one case. It happens to
/// work for a local file exactly as it does for a URL — `Start-Process` on
/// Windows, `open`/`xdg-open` elsewhere all invoke the platform's normal
/// "open this with whatever handles it" behaviour regardless of which kind
/// of path they are given.
#[tauri::command]
pub fn open_crash_log() -> Result<String, String> {
    let Some(path) = crate::startup::log_file() else {
        return Err("Could not find where this app's log would be on this machine.".into());
    };
    if path.exists() {
        crate::connectors::open_in_browser(&path.display().to_string())
            .map_err(|e| e.to_string())?;
        return Ok("opened the log".into());
    }
    // Nothing has gone wrong yet. Opening the log's own folder instead of
    // returning an error keeps the button honest: this is where a log WOULD
    // appear, and it costs nothing to show that rather than a dead end.
    // create_dir_all rather than assuming it exists — `note()` only makes
    // this folder the first time it actually writes a line, and an app that
    // has never logged anything may not have one yet either.
    let dir = path.parent().unwrap_or(&path);
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    crate::connectors::open_in_browser(&dir.display().to_string()).map_err(|e| e.to_string())?;
    Ok("opened the log folder — nothing has been recorded yet".into())
}
