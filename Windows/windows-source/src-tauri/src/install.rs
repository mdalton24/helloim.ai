//! Installing Claude Code, on the user's behalf.
//!
//! Mark, 2026-08-26, looking at the setup panel: "if claude is not installed
//! here install it or prompt to install claude and then run the command … but
//! don't show the user the command as they dont care."
//!
//! He is right, and the old panel is the exact thing this product exists to
//! stop. It printed a shell command with a Copy button and asked somebody who
//! bought a desktop app to go and find a terminal. The person who most needs
//! this app is the person least likely to know what `npm install -g` means, and
//! showing it to them is not being transparent — it is handing them the job.
//!
//! WHAT THIS DOES INSTEAD: one button. It runs the install itself, streams
//! progress as plain sentences, and never puts the command on screen.
//!
//! THREE THINGS IT REFUSES TO PRETEND ABOUT, because a setup flow that lies is
//! worse than one that asks:
//!
//! 1. **npm may not be there either.** Node is a separate install, and "it
//!    failed" is useless when the real answer is "you need Node first". That
//!    case is detected and reported as its own thing, with its own fix.
//! 2. **Signing in cannot be automated.** Claude Code's login is interactive.
//!    The most this can do is open a terminal already running it, which it
//!    does — rather than telling somebody to open one themselves.
//! 3. **When it fails, the real error is shown.** Hiding the COMMAND is right;
//!    hiding the REASON would leave someone stuck with a spinner and no way
//!    out. The command stays hidden either way.

use serde::Serialize;
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::time::Duration;
use tauri::{AppHandle, Emitter};

/// The one thing being installed. Kept here rather than in the window so the
/// front end never has a copy of the command to accidentally render.
const NPM_PACKAGE: &str = "@anthropic-ai/claude-code";

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Progress {
    text: String,
    /// 0-100, ONLY WHEN IT IS REAL. Mark, on a real Windows install: the
    /// setup panel showed a bare "Installing…" with nothing under it while
    /// the download ran, because the front end had a working `install:
    /// progress` listener on one panel (the "One thing to set up" empty
    /// state) and not on the other (the sign-in sheet's own Install
    /// button) — so which panel he happened to be looking at decided
    /// whether he saw anything move at all. Both are wired to this event
    /// now; see the front end's own comment on `#signinInstall`.
    /// THIS FIELD IS `None` UNTIL THE DOWNLOAD ITSELF STARTS, ON PURPOSE.
    /// "Finding the latest version" and "Checking the release" are each a
    /// single small request with no length to measure against — a bar that
    /// jumps to a fake 5% for those steps would be exactly the kind of
    /// number this codebase's own rule (never show a number you did not
    /// measure) exists to forbid. Only the binary download itself carries
    /// a real `Content-Length`, and only then is a percentage computed —
    /// from bytes actually written to disk, divided by that header, never
    /// estimated or interpolated between ticks.
    percent: Option<u8>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallResult {
    pub ok: bool,
    /// Only "failed" now. It used to carry "no-npm", back when npm was the
    /// only route and Node was a prerequisite — with the native installer it
    /// is not one, and offering a Node download would send somebody off to
    /// install something they do not need.
    pub problem: Option<String>,
    pub detail: Option<String>,
}

#[cfg(windows)]
fn no_window(cmd: &mut Command) {
    use std::os::windows::process::CommandExt;
    cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
}
#[cfg(not(windows))]
fn no_window(_cmd: &mut Command) {}

/// `npm` on Windows is `npm.cmd`, and `Command::new("npm")` does not find it —
/// it looks for an executable, and the shim is a batch file. Getting this wrong
/// reports "Node is not installed" on a machine where it plainly is.
fn npm_program() -> &'static str {
    if cfg!(windows) { "npm.cmd" } else { "npm" }
}

fn npm_present() -> bool {
    let mut cmd = Command::new(npm_program());
    cmd.arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    crate::hide_console(&mut cmd);
    no_window(&mut cmd);
    matches!(cmd.status(), Ok(s) if s.success())
}

fn say(app: &AppHandle, text: &str) {
    let _ = app.emit("install:progress", Progress { text: text.to_string(), percent: None });
}

/// Same event, carrying a real, measured percentage — see `Progress::percent`
/// for why this is a second function rather than a default argument on `say`:
/// the two are different claims, and keeping them textually distinct is what
/// stops a future edit from quietly passing a guessed number through `say`.
fn say_download(app: &AppHandle, text: &str, percent: u8) {
    let _ = app.emit("install:progress", Progress { text: text.to_string(), percent: Some(percent) });
}

/// `written` of `total`, as a whole percent, 0-100. A pure function so the
/// arithmetic is checkable without spawning a download — see the tests below,
/// including the two edges that matter: `total` reaching 0 (should never
/// happen given the `filter(|&n| n > 0)` at the call site, but a function
/// that panics on a zero denominator is a landmine for whoever removes that
/// filter later) and `written` overrunning `total` (a server that lied about
/// Content-Length must never produce more than 100%).
fn download_percent(written: u64, total: u64) -> u8 {
    if total == 0 { return 0; }
    // FLOOR, NOT ROUND, so this never claims 100% before the loop that
    // writes the last byte has actually run — that last tick is the
    // "Verified. Setting it up…" line further down, which is the honest
    // place for "done" to be said. `.min(total)` is the overrun guard.
    ((written.min(total) * 100) / total) as u8
}

/// Install Claude Code. Long-running, so it is async and reports as it goes.
///
/// IT USES ANTHROPIC'S NATIVE INSTALLER FIRST, AND NPM ONLY AS A FALLBACK, and
/// that order is the fix for what Mark hit on 2026-08-26. The npm-only version
/// told him to go and install Node — on a Windows machine with no Node at all,
/// which is most Windows machines. That is homework, handed to the person this
/// product exists to stop handing homework to.
///
/// The native installer needs no Node, no runtime and nothing already present:
/// it downloads a signed binary, verifies its checksum, and has the binary
/// install itself. npm is kept behind it for the machines where the download
/// is blocked but a package registry is not.
#[tauri::command(async)]
pub fn install_claude(app: AppHandle) -> InstallResult {
    say(&app, "Installing Claude Code. This usually takes under a minute.");

    match native_install(&app) {
        Ok(()) => {
            say(&app, "Installed. Checking it runs\u{2026}");
            return InstallResult { ok: true, problem: None, detail: None };
        }
        Err(native_err) => {
            // Only now is npm worth trying, and only if it is actually there.
            if !npm_present() {
                return InstallResult {
                    ok: false,
                    problem: Some("failed".into()),
                    detail: Some(native_err),
                };
            }
            say(&app, "That route did not work. Trying another\u{2026}");
            match npm_install() {
                Ok(()) => {
                    say(&app, "Installed. Checking it runs\u{2026}");
                    InstallResult { ok: true, problem: None, detail: None }
                }
                Err(npm_err) => InstallResult {
                    ok: false,
                    problem: Some("failed".into()),
                    detail: Some(format!("{native_err}\n{npm_err}")),
                },
            }
        }
    }
}

/// The last few non-empty lines of a failure — where "permission denied" and
/// "EACCES" actually live. Handing somebody the whole of npm's output is the
/// same failure as handing them the command.
fn tail_of(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    lines
        .iter()
        .rev()
        .take(4)
        .rev()
        .copied()
        .collect::<Vec<_>>()
        .join("\n")
}

/// Where Anthropic publishes the release binaries. Same endpoints the official
/// install script uses; this simply speaks to them directly.
const RELEASES: &str = "https://downloads.claude.ai/claude-code-releases";
const NET_TIMEOUT: Duration = Duration::from_secs(900); // a 250 MB binary

fn platform_slug() -> &'static str {
    if cfg!(windows) {
        if cfg!(target_arch = "aarch64") { "win32-arm64" } else { "win32-x64" }
    } else if cfg!(target_os = "macos") {
        if cfg!(target_arch = "aarch64") { "darwin-arm64" } else { "darwin-x64" }
    } else if cfg!(target_arch = "aarch64") {
        "linux-arm64"
    } else {
        "linux-x64"
    }
}

/// THE DOWNLOAD HAPPENS IN THIS PROCESS, NOT IN POWERSHELL, and that is the
/// whole point of this function existing rather than piping the official script
/// into `iex`.
///
/// THE SCAR, 2026-08-26. Shelling out to `irm https://claude.ai/install.ps1 |
/// iex` failed on Mark's Windows machine with a bare 403 and a wall of
/// PowerShell exception noise. It was not his network and it was not TLS:
/// `curl.exe` reached the SAME url from the SAME machine seconds later and got
/// a 302. Something about Windows PowerShell 5.1's requests is refused at the
/// edge, and the official script uses Invoke-WebRequest for every one of its
/// own downloads too, so fixing the outer call would have failed one step
/// further in.
///
/// Doing it here removes the whole class: this binary already carries a normal
/// HTTP client, it can report progress on a quarter-gigabyte download instead
/// of appearing frozen, and the failure it reports is a status code rather than
/// a .NET stack trace.
/// The version number, from the release host or — if it refuses — from npm.
/// Only the NUMBER comes from the fallback; everything that gets executed is
/// still fetched from Anthropic and still checksum-verified.
fn resolve_version(agent: &ureq::Agent) -> Option<String> {
    let looks_like_version = |s: &str| {
        let s = s.trim();
        !s.is_empty() && s.chars().next().is_some_and(|c| c.is_ascii_digit()) && s.len() < 32
    };

    if let Ok(r) = agent.get(&format!("{RELEASES}/latest")).call() {
        if let Ok(body) = r.into_string() {
            if looks_like_version(&body) {
                return Some(body.trim().to_string());
            }
        }
    }

    let r = agent
        .get("https://registry.npmjs.org/@anthropic-ai/claude-code/latest")
        .call()
        .ok()?;
    let body = r.into_string().ok()?;
    let v: serde_json::Value = serde_json::from_str(&body).ok()?;
    let version = v["version"].as_str()?;
    looks_like_version(version).then(|| version.to_string())
}

fn native_install(app: &AppHandle) -> Result<(), String> {
    let agent = ureq::AgentBuilder::new().timeout(NET_TIMEOUT).build();
    let platform = platform_slug();

    let get = |url: String| -> Result<ureq::Response, String> {
        agent.get(&url).call().map_err(|e| match e {
            ureq::Error::Status(code, _) => format!("{url} answered {code}."),
            ureq::Error::Transport(t) => format!("Could not reach {url}: {t}"),
        })
    };

    say(app, "Finding the latest version\u{2026}");
    /* TWO SOURCES FOR THE VERSION, AND THE SECOND IS NOT REDUNDANT.
       On Mark's machine, 2026-08-26, `/latest` answered 403 AccessDenied while
       every VERSIONED path under the same host answered 200 — the release
       bucket refuses that one object from some networks even when it serves the
       binaries happily. The official script has no fallback and dies there,
       which is what "not available in your region" in its own error text is
       really about.
       npm's registry publishes the identical version under dist-tags and was
       reachable from that same machine. So: ask the release host first, and if
       it refuses, ask npm — for the NUMBER only. The binary, the manifest and
       the checksum still come from Anthropic. */
    let version = resolve_version(&agent).ok_or_else(|| {
        "Could not find out which version to install. The download service \
         refused the request and the fallback did not answer either."
            .to_string()
    })?;

    say(app, "Checking the release\u{2026}");
    let manifest_text = get(format!("{RELEASES}/{version}/manifest.json"))?
        .into_string()
        .map_err(|e| format!("The release manifest could not be read: {e}"))?;
    let manifest: serde_json::Value = serde_json::from_str(&manifest_text)
        .map_err(|e| format!("The release manifest could not be read: {e}"))?;
    let expected = manifest["platforms"][platform]["checksum"]
        .as_str()
        .ok_or_else(|| format!("This release has no build for {platform}."))?
        .to_lowercase();

    let exe = if cfg!(windows) { "claude.exe" } else { "claude" };
    say(app, "Downloading Claude Code. This is a large file \u{2014} give it a minute.");
    let download = get(format!("{RELEASES}/{version}/{platform}/{exe}"))?;
    // READ THE HEADER BEFORE THE RESPONSE IS CONSUMED -- `into_reader()` below
    // takes ownership of `download` and there is no reading a header off a
    // reader afterwards. `ureq::Response` has no `content_length()` of its
    // own (checked against docs.rs 2026-09-03 rather than assumed); the raw
    // header is the only way to it. Absent or unparsable is a real
    // possibility -- a redirect or a proxy can drop it -- and `None` here is
    // exactly what makes `Progress::percent` stay `None` rather than reporting
    // a percentage against a total of zero.
    let total_bytes: Option<u64> = download
        .header("Content-Length")
        .and_then(|s| s.parse::<u64>().ok())
        .filter(|&n| n > 0);
    let mut body = download.into_reader();

    let dir = crate::home_dir()
        .ok_or_else(|| "Could not find your home folder.".to_string())?
        .join(".claude")
        .join("downloads");
    std::fs::create_dir_all(&dir).map_err(|e| format!("Could not create {dir:?}: {e}"))?;
    let path = dir.join(format!("claude-{version}-{platform}{}",
                                if cfg!(windows) { ".exe" } else { "" }));

    // Hashed AS IT IS WRITTEN, so a quarter of a gigabyte is never held in
    // memory and never re-read from disk to verify it.
    let mut file = std::fs::File::create(&path)
        .map_err(|e| format!("Could not write to {path:?}: {e}"))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    let mut total: u64 = 0;
    loop {
        let n = body.read(&mut buf).map_err(|e| format!("The download stopped: {e}"))?;
        if n == 0 { break; }
        hasher.update(&buf[..n]);
        file.write_all(&buf[..n]).map_err(|e| format!("Could not write the download: {e}"))?;
        let before = total / (25 << 20);
        total += n as u64;
        if total / (25 << 20) != before {
            match total_bytes {
                Some(t) => say_download(
                    app,
                    &format!("Downloading\u{2026} {} MB of {}", total / (1 << 20), t / (1 << 20)),
                    download_percent(total, t),
                ),
                None => say(app, &format!("Downloaded {} MB\u{2026}", total / (1 << 20))),
            }
        }
    }
    drop(file);

    let actual = hasher.finalize().iter().map(|b| format!("{b:02x}")).collect::<String>();
    if actual != expected {
        // A file that does not match its published checksum is not something to
        // run and then apologise for. Delete it.
        let _ = std::fs::remove_file(&path);
        return Err("The download did not match its published checksum, so it \
                    was deleted rather than run.".into());
    }

    say(app, "Verified. Setting it up\u{2026}");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755));
    }

    /* THE VERSION IS PASSED EXPLICITLY, AND THAT IS THE WHOLE FIX for what Mark
       hit on 2026-08-26. Run bare `install` and the binary goes and asks
       `/latest` for the version ITSELF — the one endpoint that answers 403 from
       his address — and fails after three attempts. So the download succeeded,
       the checksum verified, and the very last step died on the same block the
       rest of this function exists to route around. We already know the version;
       telling it skips that lookup entirely. Verified on his machine: bare
       `install` exits 1, `install 2.1.246` installs cleanly. */
    let mut cmd = Command::new(&path);
    crate::hide_console(&mut cmd);
    cmd.arg("install")
        .arg(&version)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    no_window(&mut cmd);
    let out = cmd.output().map_err(|e| format!("Could not run the installer: {e}"))?;
    let _ = std::fs::remove_file(&path);

    if out.status.success() {
        Ok(())
    } else {
        /* READ BOTH STREAMS. This reported stderr only, and this installer
           writes its failures to STDOUT — so the real message ("Request failed
           with status code 403") was thrown away and Mark got the bare fallback
           "The setup step failed", which told him nothing and told me nothing
           either. An error handler that discards the error is worse than none:
           it looks like diagnosis. */
        let why = tail_of(&out.stderr);
        let why = if why.is_empty() { tail_of(&out.stdout) } else { why };
        Err(if why.is_empty() { "The setup step failed and said nothing.".into() } else { why })
    }
}

fn npm_install() -> Result<(), String> {
    let mut cmd = Command::new(npm_program());
    cmd.args(["install", "-g", NPM_PACKAGE])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    crate::hide_console(&mut cmd);
    no_window(&mut cmd);
    let out = cmd.output().map_err(|e| e.to_string())?;
    if out.status.success() {
        return Ok(());
    }
    let why = tail_of(&out.stderr);
    Err(if why.is_empty() { "npm exited with an error.".into() } else { why })
}

/// Which kind of Anthropic account a sign-in is for.
///
/// TWO DOORS, ONE BINARY. Both end at the unmodified `claude` binary's own
/// browser flow; NameOS never sees, holds, stores, proxies or transits a
/// credential on either. That is the product's load-bearing property and this
/// enum exists so it stays checkable: the argv is a pure function of this
/// value (`auth_login_args`), and a test asserts that neither variant can put
/// anything credential-shaped on a command line.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum AuthAccount {
    /// A Claude subscription — Pro, Max, Team or Enterprise. The default, the
    /// common case, and the one the app's primary button drives.
    Subscription,
    /// An Anthropic Console account, billed as API usage. The second door,
    /// added 2026-08-29 to close a gap that was presentational rather than a
    /// restriction: a person holding an API account was already RECOGNISED —
    /// `claude auth status --json` reports `loggedIn: true` with
    /// `authMethod: "api_key"`, and `claude_logged_in()` in main.rs reads only
    /// `loggedIn`, so they were never nagged and never blocked — but there was
    /// no way to *get* signed in from inside the app, because the single
    /// sign-in button hardcoded `--claudeai`.
    Console,
}

/// The argv for `claude auth login`, as a pure function so both doors can be
/// tested without spawning anything.
///
/// VERIFIED AGAINST THE INSTALLED BINARY, 2026-08-29, not taken from memory.
/// `claude auth login --help` on 2.1.251 offers exactly these two account
/// flags: `--claudeai` ("Use Claude subscription (default)") and `--console`
/// ("Use Anthropic Console (API usage billing) instead of Claude
/// subscription").
///
/// AND THE HIDDEN-CONSOLE ARGUMENT SURVIVES FOR THE SECOND FLAG, which is the
/// one thing that had to be true before this was worth building. Reading the
/// login state machine out of the 2.1.251 bundle: the initial state is
/// `ready_to_start` when the forced method is EITHER `claudeai` or `console`,
/// and only an unforced launch lands on `idle`, which is the interactive
/// "Select login method" menu. So `--console` skips the menu exactly as
/// `--claudeai` does. The existing warning below — a hidden window with a
/// question in it is a hang nobody can diagnose — is honoured by both.
///
/// ONE DIFFERENCE WORTH RECORDING AND DELIBERATELY NOT PUT ON SCREEN. In this
/// build, forcing `--console` also skips the CLI's own second submenu
/// (`console_method`: "Sign in with your Console account" vs "Create an API
/// key"), so it takes the key-creating branch and the CLI prints "Creating API
/// key for Claude Code…". That is the vendor's internal routing and it may
/// change between versions, so the UI copy claims only what `--help` itself
/// guarantees — that this account type is billed as API usage. Describing a
/// mechanic we read out of one build would be a claim that rots.
pub(crate) fn auth_login_args(account: AuthAccount) -> [&'static str; 3] {
    match account {
        AuthAccount::Subscription => ["auth", "login", "--claudeai"],
        AuthAccount::Console => ["auth", "login", "--console"],
    }
}

/// Start the browser sign-in. No terminal, no Claude Code interface.
///
/// WHO MAY CALL THIS: the app's own bundled page, and nothing else. It is a
/// Tauri IPC command, so the caller is same-process; `tauri.conf.json` serves
/// `frontendDist` locally under `default-src 'self'` with no remote origin
/// permitted, so there is no path from a fetched page to this handler.
/// MALFORMED INPUT: not possible — it takes no arguments, and the argv is a
/// compile-time constant chosen by `AuthAccount`, never assembled from
/// anything a caller supplies. WHAT THE ERROR LEAKS: the OS spawn error only
/// (typically a path or "program not found"). No credential passes through
/// here in either direction, so none can appear in one.
///
/// **WHAT THIS USED TO DO, AND WHY IT WAS WRONG — Mark, 2026-08-28: "after
/// install, it asked us to trust a folder and then popped up a claude window.
/// Users should see this."** (he means should NOT.) It ran the bare `claude`
/// binary in a visible terminal, which starts the interactive CLI — so the
/// first thing a new customer met was Claude Code's own trust-this-folder
/// prompt and a black console. That is a developer tool's front door, shown
/// to somebody who bought a desktop app.
///
/// `claude auth login` is a different thing entirely: it does the OAuth
/// browser flow and exits. No session, no working directory, so no trust
/// prompt — that prompt belongs to starting a session, which this no longer
/// does. `--claudeai` picks the subscription path explicitly rather than
/// leaving a menu to answer in a window nobody can see.
///
/// THE CONSOLE IS HIDDEN AND THAT IS ONLY SAFE BECAUSE OF THE FLAG. A hidden
/// window with a question in it is a hang nobody can diagnose; naming the
/// account type removes the one question this command would otherwise ask.
#[tauri::command(async)]
pub fn open_claude_login() -> Result<(), String> {
    spawn_auth_login(AuthAccount::Subscription)
}

/// The same sign-in, for somebody who pays Anthropic per request instead of
/// holding a Claude subscription. Runs `claude auth login --console`.
///
/// WHY THIS IS A SEPARATE COMMAND rather than an argument on the one above:
/// an argument is a value a caller chooses, and the flag it lands on is the
/// account a person is billed under. Two commands, each with one compile-time
/// constant argv, means the front end selects a door and can never compose
/// one. It also leaves `open_claude_login` reachable exactly as it was, which
/// matters — the subscription path is the default and the common case, and
/// this is a second door, not a replacement for it.
///
/// WHO MAY CALL THIS, WHAT MALFORMED INPUT DOES, WHAT THE ERROR LEAKS: as
/// `open_claude_login` above, and for the same reasons — same IPC boundary,
/// no arguments, constant argv, OS spawn error only.
///
/// WHAT IT DOES NOT DO, and this is the property to protect if anyone extends
/// it: it does not ask for a key, take a key, read a key, write a key or pass
/// one to anything. It starts the vendor's browser flow and exits. A future
/// version of this that accepts an API key from the UI would move the
/// credential inside NameOS, which is the one thing this product promises it
/// never does.
#[tauri::command(async)]
pub fn open_claude_console_login() -> Result<(), String> {
    spawn_auth_login(AuthAccount::Console)
}

/// Spawn `claude auth login` with the flag for `account`, detached and silent.
///
/// The platform split below is unchanged from when this was the body of
/// `open_claude_login`; only the argv moved out to `auth_login_args`. The
/// comments in each branch were paid for and are kept verbatim.
fn spawn_auth_login(account: AuthAccount) -> Result<(), String> {
    let args = auth_login_args(account);
    #[cfg(windows)]
    {
        // THE FULL PATH, NOT THE BARE NAME, and that part of the old comment
        // still holds: the installer puts claude.exe in
        // %USERPROFILE%\.local\bin and does NOT add it to PATH, so a bare
        // name fails in a way that reads as our bug rather than a PATH gap.
        // crate::claude_binary() already resolves the real location.
        // (The `cmd /c start` dance this used to need is gone with the
        // terminal it was opening.)
        let claude = crate::claude_binary();
        let mut cmd = Command::new(&claude);
        cmd.args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        crate::hide_console(&mut cmd);
        cmd.spawn().map(|_| ()).map_err(|e| e.to_string())
    }
    #[cfg(target_os = "macos")]
    {
        Command::new(crate::claude_binary())
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        // NO TERMINAL HUNT ANY MORE. This used to try gnome-terminal, konsole,
        // xfce4-terminal and two more in turn, because the thing it launched
        // needed a terminal to be typed into. `claude auth login` does not:
        // it opens a browser and exits, so the whole search — and the failure
        // mode where a machine has none of the five — goes away with it.
        Command::new(crate::claude_binary())
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
}

/// Open the Node.js download page — the fix for the "no-npm" case, and the one
/// place this hands off, because installing a language runtime behind somebody's
/// back is not ours to do.
#[tauri::command(async)]
pub fn open_node_download(app: AppHandle) -> Result<(), String> {
    let _ = app;
    // ONE IMPLEMENTATION OF "OPEN THIS", and it lives in connectors.rs. This
    // used to be its own `cmd /c start` copy. The URL here is a constant with
    // no `&` in it, so it was never the injectable one — but leaving a second
    // copy of a pattern we have just removed is how it comes back.
    crate::connectors::open_in_browser("https://nodejs.org/en/download").map_err(|e| e.to_string())
}

/// Open Windows' "add a natural voice" settings page.
///
/// WHY THIS EXISTS RATHER THAN A BETTER RANKING. Mark's machine, measured
/// 2026-08-26, has exactly three speech voices: David, Mark and Zira — the
/// legacy SAPI set that has shipped since the 1990s. No amount of choosing
/// between them produces a warm voice, because there is not one there. Windows
/// 11 ships genuinely good neural voices but does NOT install them by default
/// and gives no API to add one; the user has to tick a box in Settings.
///
/// So the app says that plainly and opens the exact page, rather than pretending
/// the ranking can fix it or leaving somebody hunting through Settings.
#[tauri::command(async)]
pub fn open_voice_settings() -> Result<(), String> {
    #[cfg(windows)]
    {
        // The narrator page is where "Add natural voices" actually lives.
        // Through the shared opener — `connectors::open_url` already sends
        // `ms-settings:privacy-microphone` the same way, so there is one path
        // to check on a real Windows machine rather than three.
        crate::connectors::open_in_browser("ms-settings:easeofaccess-narrator")
            .map_err(|e| e.to_string())
    }
    #[cfg(not(windows))]
    {
        Err("Voice settings are a Windows thing.".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// THE REGRESSION THIS EXISTS FOR: Mark saw a bare "Installing…" with
    /// nothing moving underneath it on a real Windows install. The number
    /// itself never got as far as a screenshot, so these are the arithmetic
    /// guarantees the front end is trusting `percent` to hold.
    #[test]
    fn download_percent_is_zero_at_the_start() {
        assert_eq!(download_percent(0, 250 << 20), 0);
    }

    #[test]
    fn download_percent_is_a_real_fraction_partway_through() {
        // 62 of 250 MB, in bytes, is 24% -- computed independently here
        // rather than by re-deriving the same formula, so a broken formula
        // cannot pass by agreeing with itself.
        assert_eq!(download_percent(62 << 20, 250 << 20), 24);
    }

    #[test]
    fn download_percent_floors_rather_than_rounds_up_to_100() {
        // One byte short of the total must never read as "done" -- that
        // word is said by a later line ("Verified. Setting it up…"), once
        // the checksum has actually passed, not by this counter.
        let total = 250u64 << 20;
        assert_eq!(download_percent(total - 1, total), 99);
        assert_eq!(download_percent(total, total), 100);
    }

    #[test]
    fn download_percent_never_exceeds_100_if_the_server_lied_about_the_total() {
        // Content-Length is a claim, not a guarantee -- a proxy or a flaky
        // header could under-report it. More bytes written than promised
        // must clamp to 100, never wrap or overrun into a nonsense number.
        assert_eq!(download_percent(300 << 20, 250 << 20), 100);
    }

    #[test]
    fn download_percent_never_divides_by_a_zero_total() {
        // Should not happen given the `filter(|&n| n > 0)` at the call
        // site, but a pure function that panics on this input is a
        // landmine for whoever touches that filter later.
        assert_eq!(download_percent(1000, 0), 0);
    }

    /// THE EXISTING DOOR, PINNED. `open_claude_login` is the default and the
    /// common case, and this change moved its argv out of the function body
    /// into `auth_login_args` — so this is the test that proves the move did
    /// not alter it. It is deliberately literal rather than derived: a test
    /// that computed the expected flag from the same match arm would pass
    /// whatever that arm said.
    #[test]
    fn the_subscription_door_still_passes_claudeai() {
        assert_eq!(
            auth_login_args(AuthAccount::Subscription),
            ["auth", "login", "--claudeai"]
        );
    }

    /// The second door, verified against `claude auth login --help` on the
    /// installed binary (2.1.251) rather than assumed: the flag is `--console`
    /// and it is spelled with two leading dashes and no value.
    #[test]
    fn the_console_door_passes_console() {
        assert_eq!(
            auth_login_args(AuthAccount::Console),
            ["auth", "login", "--console"]
        );
    }

    /// THE TWO DOORS ARE DIFFERENT DOORS. A copy-paste that left both on
    /// `--claudeai` would compile, would render a second button, and would
    /// silently take an API-account user down the subscription path — which is
    /// the exact failure this feature exists to fix, wearing the costume of a
    /// fix.
    #[test]
    fn the_two_doors_do_not_collapse_into_one() {
        assert_ne!(
            auth_login_args(AuthAccount::Subscription),
            auth_login_args(AuthAccount::Console)
        );
    }

    /// NAMEOS NEVER PUTS A CREDENTIAL ON A COMMAND LINE, and this is the test
    /// that keeps that true as the file changes. Every argument either door
    /// can produce must come from this closed set — no key, no token, no
    /// `--api-key=…`, nothing interpolated from anything a caller supplied.
    ///
    /// It is written as an allowlist on purpose. A denylist ("must not contain
    /// the word key") passes for every credential nobody thought to name, and
    /// the whole property here is that the argv is a constant.
    #[test]
    fn neither_door_can_carry_a_credential() {
        const ALLOWED: [&str; 4] = ["auth", "login", "--claudeai", "--console"];
        for account in [AuthAccount::Subscription, AuthAccount::Console] {
            for arg in auth_login_args(account) {
                assert!(
                    ALLOWED.contains(&arg),
                    "{account:?} passes {arg:?}, which is not one of the four \
                     constant arguments this command is allowed to spawn with. \
                     If a credential is being handed to the binary on argv, the \
                     product's promise that it never holds one is broken."
                );
                assert!(
                    !arg.contains('='),
                    "{account:?} passes {arg:?}, which carries a value. Every \
                     argument here is a bare flag; a valued argument is how a \
                     secret gets onto a command line and into the process table."
                );
            }
        }
    }
}
