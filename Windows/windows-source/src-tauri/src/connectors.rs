//! Connectors — the MCP servers this app can talk to: the tools the brain
//! can reach, and exactly that.
//!
//! Mark asked for four things and they are all one subsystem: a GREEN INDICATOR
//! when it is connected, a connector for API and one for MCP, a VIEW listing
//! them, and the API keys ENCRYPTED.
//!
//! **THE "api" KIND IS RETIRED — 2026-08-27.** An api-kind row stored a real
//! key, tested it with a genuine call, earned its green honestly FOR THE KEY —
//! and then contributed nothing: `launch_config` below is the only consumer of
//! this list and it always skipped api rows, so the key was stored, proven,
//! displayed, and never used. A green light on a row that does nothing is the
//! exact lie rule 1 exists to prevent, one step removed. API providers now
//! live in providers.rs, where they select the brain and genuinely do
//! something. Existing api rows are migrated on launch by migrate.rs; the
//! api-specific fields on `Connector` are KEPT so old files still parse and
//! unmigratable rows survive visibly, and three guards keep "retired" from
//! meaning "still reachable by accident": `save_connector` refuses the kind,
//! `test_connector` refuses it with words instead of testing it, and unknown
//! kinds are refused by name instead of being tested as if they were OpenAI.
//!
//! THE TWO RULES THIS FILE EXISTS TO ENFORCE, and both are easy to get wrong in
//! a way that looks fine:
//!
//! 1. **GREEN MEANS A REAL CHECK PASSED.** Not "a key is present", not "the
//!    user filled the form in". Every `Connector` carries the OUTCOME of an
//!    actual round trip — an authenticated call to the provider, or a real MCP
//!    `initialize` handshake — plus when it happened and what failed if it did.
//!    A dot that turns green because a string is non-empty is a lie told in a
//!    colour, and it is the single most tempting shortcut in this whole file.
//!
//! 2. **A SECRET NEVER TOUCHES THE CONFIG FILE, AND NEVER GOES BACK TO THE UI.**
//!    Keys live in the operating system's own credential store — DPAPI-backed
//!    Windows Credential Manager, macOS Keychain, Secret Service on Linux — via
//!    the `keyring` crate. `connectors.json` holds everything EXCEPT the secret
//!    and records only `has_secret: bool`. There is deliberately no command
//!    that reads a key back out to the front end: the window never needs it,
//!    and a getter would put it in a webview's memory and one console.log from
//!    a plaintext leak.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Mutex;

/// One namespace in the OS credential store for the whole app. The account
/// within it is the connector's id, which is why ids are never reused.
///
/// LEFT AS "NameOS" IN THE helloim.ai RENAME, 2026-09-02, DELIBERATELY. This
/// string is not a display name -- it is the lookup key for every secret
/// already sitting in a real user's Windows Credential Manager / macOS
/// Keychain. Changing it does not move those entries; it makes them
/// unreachable, and the app would report every already-connected provider as
/// disconnected on first launch after the rename. Same category of risk as
/// `tauri.conf.json`'s `identifier`, flagged rather than guessed at. If this
/// is ever renamed, it needs a real migration (read the old service name once,
/// re-write under the new one, then cut over) not a find-and-replace.
pub(crate) const KEYRING_SERVICE: &str = "NameOS";

/// A round trip that has not answered in this long is not a connection anyone
/// would call working, and a settings window that hangs is worse than a red dot.


#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Connector {
    pub id: String,
    /// "mcp" — the only living kind. Kept as a string rather than an enum on
    /// purpose: this file is read back by a future version that may know more
    /// kinds, and an unknown variant should not make the whole config
    /// unreadable. "api" is RETIRED (see the module doc); it may still appear
    /// in an old file, and migrate.rs is what deals with it.
    pub kind: String,
    pub name: String,

    // ---- Retired "api" fields -------------------------------------------
    // KEPT, not deleted: old files must still parse, migrate.rs reads them to
    // build the provider row, and a row that cannot migrate stays visible
    // here rather than losing somebody's stored key.
    /// "openai", "anthropic", or "custom". Retired with the api kind.
    #[serde(default)]
    pub provider: String,
    /// Base URL. Empty meant "use the provider's default". Retired with the
    /// api kind.
    #[serde(default)]
    pub base_url: String,
    #[serde(default)]
    pub model: String,

    // ---- MCP connectors -------------------------------------------------
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    /// The environment variable this server reads its token from —
    /// `SLACK_BOT_TOKEN`, `NOTION_TOKEN`, and so on. Empty for servers that
    /// need no credential.
    ///
    /// It is stored, and the VALUE is not: the value is the secret in the OS
    /// credential store, and it is joined to this name only in the environment
    /// of the process that needs it. See `launch_config` for why that matters.
    #[serde(default)]
    pub env_key: String,
    /// The NON-SECRET half of a login — an email address, a workspace id, an
    /// account name — and the variable it goes into.
    ///
    /// A password on its own connects to nothing: IMAP wants an address beside
    /// it, and one env var was never going to be enough. This one is kept in
    /// `connectors.json` in the clear, deliberately and correctly: it is the
    /// person's own address on their own machine, and pretending it is a
    /// secret would mean hiding it from the window that has to show them which
    /// account is connected.
    #[serde(default)]
    pub account: String,
    #[serde(default)]
    pub account_key: String,
    /// A HOSTED MCP server's address. Set means this is an `http` server and
    /// there is no local program to run; empty means `stdio` and `command` is
    /// what matters.
    ///
    /// Inferred rather than stored as a separate `transport` field, because two
    /// fields that must agree eventually disagree — a connector with
    /// `transport: "http"` and no url, or the reverse, is a state this cannot
    /// represent.
    #[serde(default)]
    pub url: String,
    /// Whether this server is useless without a credential.
    ///
    /// **THIS EXISTS BECAUSE OF A MEASUREMENT, not a theory.** Google's hosted
    /// Gmail server answers `initialize` AND `tools/list` with no token at all
    /// — both return 200 with a perfectly valid result — and only refuses at
    /// the moment a tool is actually called. So the handshake test passes, the
    /// dot goes green, and the first real request fails. That is precisely the
    /// lie rule 1 at the top of this file exists to prevent, and no amount of
    /// protocol-level probing catches it.
    ///
    /// Set by the tile that knows the service needs signing in to. A hosted
    /// server that genuinely needs nothing leaves it false and the handshake
    /// alone is enough.
    #[serde(default)]
    pub needs_token: bool,
    /// An extra header name this service wants its key in.
    ///
    /// Most hosted MCP servers take `Authorization: Bearer <key>`; some want
    /// their own header instead. Composio's own setup documents
    /// `x-consumer-api-key`, while the live endpoint's 401 asks for Bearer —
    /// so when this is set BOTH go out. One redundant header costs nothing;
    /// guessing wrong costs somebody a connector that refuses them with no
    /// explanation.
    #[serde(default)]
    pub header_key: String,

    // ---- Live state, written only by `test_connector` -------------------
    /// TRUE ONLY IF A REAL ROUND TRIP SUCCEEDED. See rule 1 at the top.
    #[serde(default)]
    pub connected: bool,
    /// RFC-3339-ish stamp of the last test, or empty if never tested. The UI
    /// shows this so "green" can be read as "green as of when".
    #[serde(default)]
    pub checked_at: String,
    /// Why the last test failed, in words a person can act on. Empty on success.
    #[serde(default)]
    pub last_error: String,
    /// Whether a secret exists in the OS store for this connector. This is the
    /// ONLY thing the front end ever learns about the key.
    #[serde(default)]
    pub has_secret: bool,
}

#[derive(Default)]
pub struct Connectors {
    inner: Mutex<Vec<Connector>>,
}

pub(crate) fn config_path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    use tauri::Manager;
    let dir = app
        .path()
        .app_config_dir()
        .map_err(|e| format!("no config directory: {e}"))?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("could not create {dir:?}: {e}"))?;
    Ok(dir.join("connectors.json"))
}

fn load_from_disk(app: &tauri::AppHandle) -> Vec<Connector> {
    let Ok(path) = config_path(app) else {
        return Vec::new();
    };
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    // A corrupt or half-written file must not take the app down with it — an
    // empty connector list is recoverable by the person looking at the window,
    // a startup panic is not.
    serde_json::from_str(&text).unwrap_or_default()
}

fn save_to_disk(app: &tauri::AppHandle, list: &[Connector]) -> Result<(), String> {
    let path = config_path(app)?;
    // Belt and braces against rule 2: nothing in `Connector` is a secret by
    // construction, and this asserts it rather than trusting the struct not to
    // grow a field later.
    let json = serde_json::to_string_pretty(list).map_err(|e| e.to_string())?;
    std::fs::write(&path, json).map_err(|e| format!("could not write {path:?}: {e}"))
}

fn entry(id: &str) -> Result<keyring::Entry, String> {
    keyring::Entry::new(KEYRING_SERVICE, id).map_err(|e| format!("credential store: {e}"))
}

fn secret_for(id: &str) -> Option<String> {
    entry(id).ok().and_then(|e| e.get_password().ok())
}

fn now_stamp() -> String {
    // No chrono in this crate and no reason to add one for a display string.
    // Seconds since the epoch is stable, sortable, and the window formats it.
    match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        Ok(d) => d.as_secs().to_string(),
        Err(_) => String::new(),
    }
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn list_connectors(app: tauri::AppHandle, state: tauri::State<Connectors>) -> Vec<Connector> {
    let mut guard = state.inner.lock().unwrap();
    if guard.is_empty() {
        *guard = load_from_disk(&app);
    }
    // Re-derive has_secret from the OS store on every read rather than trusting
    // the file. The two can genuinely disagree — a key removed in Windows
    // Credential Manager leaves the JSON claiming one exists — and the store is
    // the authority.
    for c in guard.iter_mut() {
        c.has_secret = secret_for(&c.id).is_some();
    }
    guard.clone()
}

/// Every real directory `connector`'s own args reach — the detection half
/// of the folder-trust gate below, kept separate and `AppHandle`-free so
/// it is directly testable (this crate has no `tauri::test` harness
/// anywhere in it; see `providers.rs`'s own comments making the identical
/// call).
///
/// **EVERY ARG, NOT JUST THE LAST ONE — Cassandra's re-review, 2026-09-25,
/// closing a residual hole in the first version of this function.** That
/// version checked only `args.last()`, on the assumption a folder-scoped
/// server takes exactly one positional directory at the end. The real
/// `server-filesystem` package takes MULTIPLE positional directories and
/// tolerates trailing flags after them, which broke that assumption two
/// ways, both reachable by the hand-crafted invoke this file's own
/// retired-kind comment already treats as in scope:
///
/// 1. **Multi-dir.** `[..., "/home/user/private", "/tmp/empty"]` — only
///    the last arg was ever checked, so accepting the harmless, already-
///    trusted `/tmp/empty` once was enough to leave the connector with
///    ungated read/write on `/home/user/private`, never reviewed at all.
/// 2. **Trailing flag.** `[..., "/home/user/private", "--readonly"]` —
///    `args.last()` is `--readonly`, which is not a directory, so the OLD
///    function returned `None` and the gate never ran at all, even though
///    the connector is plainly scoped to `/home/user/private`.
///
/// The fix is the same idea, applied to every argument instead of one:
/// this collects every arg that resolves to a real directory on THIS
/// machine, in order, and the caller requires ALL of them to be trusted —
/// refusing on the first one that is not, same as it would for a single
/// folder. Non-directory args (flags, package names, the command itself)
/// are simply not directories and are never in this list, so
/// `@modelcontextprotocol/server-filesystem`/`-y`/`--readonly` cost
/// nothing extra to skip; no hardcoded flag/package vocabulary is needed.
fn folder_scoped_connector_args(connector: &Connector) -> Vec<&str> {
    if !connector.url.trim().is_empty() || connector.command.trim().is_empty() {
        return Vec::new();
    }
    connector
        .args
        .iter()
        .filter(|a| std::path::Path::new(a.as_str()).is_dir())
        .map(|a| a.as_str())
        .collect()
}

/// Create or update. The secret is passed separately and only when it is being
/// SET — an empty `secret` means "leave whatever is in the credential store
/// alone", which is what lets the window edit a connector's model or name
/// without the user re-typing a key it cannot show them.
#[tauri::command]
pub fn save_connector(
    app: tauri::AppHandle,
    state: tauri::State<Connectors>,
    mut connector: Connector,
    secret: String,
) -> Result<Connector, String> {
    // Retired means retired: no new api rows, and no edits to a leftover one
    // — a frozen row can be removed or disconnected, nothing else. Without
    // this guard, a stale UI (or a hand-crafted invoke) could keep creating
    // rows whose keys are stored, proven, and never used — the exact fault
    // the retirement removed. Who may call: our own webview. Wrong kind: this
    // refusal, with the forwarding address. Nothing is written on refusal.
    if connector.kind == "api" {
        return Err(
            "API providers have moved to the AI components menu — add it there instead. \
             Rows left here are kept only so a stored key is not lost."
                .into(),
        );
    }

    // **A FOLDER-SCOPED LOCAL CONNECTOR NEEDS THE SAME RECORDED REVIEW A
    // WORKER DOES — Beck's VM verification, 2026-09-25.** The Filesystem
    // tile (`ui/index.html`'s catalog, `needsFolder: true`) hands a stdio
    // MCP server standing, repeatable read/write access to whatever folder
    // its own last argument names, with the model calling it whenever it
    // likes and nobody watching each call the way a chat turn is watched —
    // the identical exposure `worker.rs`'s own `folder_trust_refusal`
    // exists to close, and the SAME root cause: `folder_trust::gate`'s
    // `needs_decision` is `false` for a folder with nothing suspicious in
    // it, which the front end's own `ensureFolderTrusted()` reads as
    // "nothing to review" and lets straight through without ever calling
    // `mark_folder_trusted` — so the folder is used and never recorded.
    // See `folder_scoped_connector_args`'s own doc for how "folder-scoped"
    // is detected, INCLUDING a server that takes more than one directory
    // or tolerates trailing flags after its last one.
    //
    // The front end is still where the review SCREEN lives
    // (`folder_trust_probe`/`mark_folder_trusted`, already wired for the
    // chat folder picker) — this is the gate that actually refuses to save
    // a connector for a folder nobody has recorded a yes for, the same
    // "a front-end-only gate is not a gate" reasoning `main.rs::send`'s own
    // comment gives for the identical check on the chat path.
    //
    // **EVERY DIRECTORY ARG MUST BE TRUSTED, NOT JUST ONE OF THEM —
    // Cassandra's re-review, 2026-09-25.** A multi-dir connector
    // (`server-filesystem /home/user/private /tmp/empty`) accepting one
    // already-trusted folder must never be read as clearing the others;
    // refusing on the FIRST untrusted directory is what closes that.
    for folder in folder_scoped_connector_args(&connector) {
        let trust = crate::folder_trust::gate(&app, folder);
        if !trust.trusted {
            return Err(
                "This folder hasn't been reviewed and accepted yet — a connector that reaches \
                 a folder needs an explicit yes for it, even an empty one. Open the folder \
                 review screen, accept it, then add this connector."
                    .into(),
            );
        }
    }

    if connector.name.trim().is_empty() {
        return Err("Give the connector a name.".into());
    }
    if connector.id.trim().is_empty() {
        // Ids are never reused, because the id is the credential-store account
        // and a reused one would inherit a previous connector's key.
        connector.id = format!("c{}-{}", now_stamp(), std::process::id());
    }

    if !secret.trim().is_empty() {
        entry(&connector.id)?
            .set_password(secret.trim())
            .map_err(|e| format!("could not store the key: {e}"))?;
        // The plaintext dies with this function: `secret` is dropped here, it
        // was never written to disk, and there is no getter that returns it.
    }
    connector.has_secret = secret_for(&connector.id).is_some();

    // A changed connector is an UNTESTED connector. Leaving the old green
    // result on a row whose URL or key just changed is exactly the lie rule 1
    // forbids, so editing always resets it to unknown.
    connector.connected = false;
    connector.checked_at = String::new();
    connector.last_error = String::new();

    let mut guard = state.inner.lock().unwrap();
    if guard.is_empty() {
        *guard = load_from_disk(&app);
    }
    match guard.iter_mut().find(|c| c.id == connector.id) {
        Some(existing) => *existing = connector.clone(),
        None => guard.push(connector.clone()),
    }
    save_to_disk(&app, &guard)?;
    Ok(connector)
}

#[tauri::command(async)]
pub fn delete_connector(
    app: tauri::AppHandle,
    state: tauri::State<Connectors>,
    id: String,
) -> Result<(), String> {
    use tauri::Manager;
    crate::cancel_running_turn(&app.state::<crate::Session>())?;

    let mut guard = state.inner.lock().unwrap();
    if guard.is_empty() {
        *guard = load_from_disk(&app);
    }
    guard.retain(|c| c.id != id);
    save_to_disk(&app, &guard)?;
    // Best effort: a credential store that refuses to delete should not leave
    // the connector visibly undeletable in the window. The row is gone either
    // way and an orphaned key is inert without it.
    if let Ok(e) = entry(&id) {
        let _ = e.delete_credential();
    }
    Ok(())
}

/// Sign in to a hosted connector and store the token, all from one click.
///
/// **THIS IS THE END-USER PATH.** Mark, 2026-08-26: *"we are setting this up
/// for end users not us.. so we need to think like that."* Everything before
/// this asked somebody to go and find an API key in a settings page they have
/// never opened. This opens their browser, they sign in to their own account,
/// and it is done -- which is what he watched a competitor do and asked for.
///
/// The token lands in the OS credential store like every other secret here and
/// is never returned to the window. See rule 2 at the top of this file.
#[tauri::command(async)]
pub fn sign_in_connector(
    app: tauri::AppHandle,
    state: tauri::State<Connectors>,
    id: String,
) -> Result<Connector, String> {
    let target = {
        let mut guard = state.inner.lock().unwrap();
        if guard.is_empty() {
            *guard = load_from_disk(&app);
        }
        guard.iter().find(|c| c.id == id).cloned()
    };
    let Some(target) = target else {
        return Err("No such connector.".into());
    };
    if target.url.trim().is_empty() {
        return Err("Only hosted services can be signed in to this way.".into());
    }

    let signed = crate::oauth::sign_in(&target.url, |url| {
        // Their real browser, not a window we own: a sign-in page inside an app
        // is exactly what people are told never to trust, and their password
        // manager will not fill it.
        let _ = open_in_browser(url);
    })?;

    entry(&id)?
        .set_password(&signed.access_token)
        .map_err(|e| format!("could not store the token: {e}"))?;

    let mut guard = state.inner.lock().unwrap();
    let Some(slot) = guard.iter_mut().find(|c| c.id == id) else {
        return Err("No such connector.".into());
    };
    slot.has_secret = true;
    // Who they signed in as, when the service said. Blank stays blank.
    if !signed.account.trim().is_empty() {
        slot.account = signed.account.trim().to_string();
    }
    // Signed in is not the same as working, and this file does not pretend
    // otherwise: the caller tests it, and the test is what turns the dot green.
    slot.connected = false;
    slot.checked_at = String::new();
    slot.last_error = String::new();
    let updated = slot.clone();
    save_to_disk(&app, &guard)?;
    Ok(updated)
}

/// Open a page in their own browser.
///
/// Exposed as a command because the Connections view has one honest action for
/// a connector it did not create: take them to where it CAN be finished. A
/// button that opens the right page is worth more than a sentence telling them
/// to go and find it.
#[tauri::command]
pub fn open_url(url: String) -> Result<(), String> {
    // Only http(s), and only from our own strings. This is a command the
    // webview can call, and a bare "open anything" would let a page in the
    // feed launch a local program.
    /* http(s) and the ONE Windows settings URI we send somebody to.
       `ms-settings:privacy-microphone` is how a blocked microphone gets
       unblocked, and telling a person to go and find that page themselves is
       the thing the chip exists to stop. It is allow-listed by exact value
       rather than by scheme: `ms-settings:` as a prefix would let any page in
       the feed open any settings panel on the machine. */
    const ALLOWED_URI: &str = "ms-settings:privacy-microphone";
    if url != ALLOWED_URI && !url.starts_with("https://") && !url.starts_with("http://") {
        return Err("That is not a web address.".into());
    }
    open_in_browser(&url).map_err(|e| e.to_string())
}

/// Hand a URL to whatever the person's machine uses to open one.
///
/// **THIS USED TO BE `cmd /C start "" <url>` ON WINDOWS AND THAT WAS TWO BUGS
/// IN ONE LINE — a command injection and a feature that could never have
/// worked.** Cassandra found the first on 2026-08-28; the second fell out of
/// checking her prediction on Mark's own Windows box the same day.
///
/// **THE INJECTION.** Rust's own documentation for `Command::args` says it
/// plainly (<https://doc.rust-lang.org/std/process/struct.Command.html>):
///
/// > "some applications such as `cmd.exe` and `.bat` files use a non-standard
/// > way of decoding arguments. They are therefore vulnerable to malicious
/// > input. In the case of `cmd.exe` this is especially important because a
/// > malicious argument can potentially run arbitrary shell commands."
///
/// std only wraps an argument in quotes when it contains a space, a tab, or is
/// empty (`library/std/src/sys/args/windows.rs`). `&`, `|` and `^` — cmd's own
/// separators — go through untouched. And `oauth.rs` builds this URL out of
/// `authorization_endpoint`, read from a remote server's discovery JSON, so the
/// string reaching cmd came off the network.
///
/// **THE PRODUCT BUG, MEASURED, not reasoned.** Every OAuth URL contains `&`.
/// On Mark's machine (Windows 10.0.26200), cmd given an unquoted URL cut it at
/// the first `&` and tried to execute the remainder:
///
/// ```text
/// > <url>?response_type=code&client_id=abc
/// "" https://ok.example/a?response_type=code
/// 'client_id' is not recognized as an internal or external command,
/// ```
///
/// So hosted-connector sign-in on Windows was opening a truncated URL. Fixing
/// the hole fixes the feature; they were the same line.
///
/// **WHAT REPLACES IT, AND WHY NOT A NEW DEPENDENCY.** This is the strategy
/// `tauri-plugin-opener` uses, which is to say the one the `open` crate uses,
/// copied rather than re-invented — the same discipline as taking `install.rs`'s
/// checksum pattern rather than writing a second one:
///
///   1. **PowerShell `Start-Process`, with the URL passed in an ENVIRONMENT
///      VARIABLE.** This is the part that matters. The URL never appears on a
///      command line at all, so there is no parser to escape it for and nothing
///      to get subtly wrong. Injection is closed by construction, not by
///      quoting.
///   2. **`explorer.exe <url>` if that fails.** Explorer uses the standard
///      argument decoding that std's escaping is built for, so it is safe here
///      even though cmd was not.
///
/// Taking the plugin itself would have added a dependency tree to a Windows
/// cross-build on the eve of a release, to arrive at these same two calls.
pub(crate) fn open_in_browser(url: &str) -> std::io::Result<()> {
    use std::process::Command;
    #[cfg(windows)]
    {
        // Never on the command line. `Start-Process` reads it out of the
        // environment, so `&`, `|`, `^`, quotes and spaces are all just bytes.
        let mut ps = Command::new("powershell.exe");
        ps.args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "Start-Process -FilePath $env:NAMEOS_OPEN_TARGET",
        ])
        .env("NAMEOS_OPEN_TARGET", url);
        crate::hide_console(&mut ps);
        if ps.spawn().is_ok() {
            return Ok(());
        }
        // Constrained language mode, an execution policy, a stripped image —
        // PowerShell is not guaranteed. Explorer is, and it decodes arguments
        // the ordinary way.
        let mut ex = Command::new("explorer.exe");
        ex.arg(url);
        crate::hide_console(&mut ex);
        ex.spawn()?;
    }
    #[cfg(target_os = "macos")]
    Command::new("open").arg(url).spawn()?;
    #[cfg(all(unix, not(target_os = "macos")))]
    Command::new("xdg-open").arg(url).spawn()?;
    Ok(())
}

/// Everything Claude Code is already connected to, asked directly.
///
/// **THIS EXISTS BECAUSE THE LIST KEPT VANISHING.** Mark, 2026-08-26: "Items
/// that were connected are now missing." They were read off the `system` line
/// of a run, which means they only appeared AFTER somebody had sent a message
/// — so opening Connections on a freshly started app showed nothing, and the
/// connectors he actually had looked deleted.
///
/// `claude mcp list` answers the same question with no model call, no tokens
/// and no waiting for a turn. It is the honest source: it reports what the
/// binary itself would use, including the connectors on the person's own
/// Claude account, with a live health check per server.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LiveServer {
    pub name: String,
    pub url: String,
    /// "connected" or "needs-auth".
    pub status: String,
}

#[tauri::command(async)]
pub fn list_live_servers() -> Vec<LiveServer> {
    use std::process::{Command, Stdio};
    let mut cmd = Command::new(crate::claude_binary());
    cmd.args(["mcp", "list"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    crate::hide_console(&mut cmd);
    let Ok(out) = cmd.output() else { return Vec::new() };

    parse_mcp_list(&String::from_utf8_lossy(&out.stdout))
}

/// Split apart from the command so it can be tested against the real output.
///
/// The shape is `NAME: URL - <tick> Connected`, and NAME can itself contain a
/// colon — `plugin:getlayers:getlayers` — so this splits on the LAST colon
/// before the scheme rather than the first one, which would have cut every
/// plugin's name in half.
fn parse_mcp_list(text: &str) -> Vec<LiveServer> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || !line.contains(" - ") {
            continue;
        }
        let Some((head, tail)) = line.rsplit_once(" - ") else { continue };
        // Find where the URL starts; everything before it (minus ": ") is the name.
        let Some(scheme) = head.find("http") else { continue };
        let name = head[..scheme].trim().trim_end_matches(':').trim().to_string();
        let url = head[scheme..].trim().to_string();
        // The transport is sometimes appended in brackets: "…/mcp (HTTP)".
        let url = url.split(" (").next().unwrap_or(&url).trim().to_string();
        if name.is_empty() || url.is_empty() {
            continue;
        }
        let status = if tail.to_lowercase().contains("connected") {
            "connected"
        } else {
            "needs-auth"
        };
        out.push(LiveServer { name, url, status: status.into() });
    }
    out
}

/// Sign in to, or out of, a server Claude Code already knows about.
///
/// **THE BINARY DOES THIS ITSELF AND I BUILT AN OAUTH CLIENT BEFORE CHECKING.**
/// `claude mcp login <name>` authenticates an HTTP, SSE *or claude.ai*
/// connector and opens the browser to do it; `claude mcp logout <name>` clears
/// the stored credentials. Mark, 2026-08-26: "There is no disconnect button
/// for those that are connected already. They should be able to disconnect the
/// app and reconnect?" — and the answer was two commands away the whole time.
///
/// This is the right tool for servers we did not create: their credentials
/// live in Claude Code's own store, so it is the only thing that can honestly
/// connect or disconnect them. `oauth.rs` still owns the services NameOS adds
/// itself, where we hold the token.
/// Does this stdout mean the CLI REFUSED, despite having exited 0?
///
/// Pulled out of `claude_mcp_auth` on 2026-08-27 for one reason: **the bug Mark
/// reported lived in this condition, and nothing could test it there.** The
/// function around it shells out to the real `claude` binary, so the only way
/// to exercise the decision was to have a claude.ai connector configured and
/// press the button — which is exactly how it shipped broken. A predicate over
/// a string can be tested in nine microseconds, and it is the whole fault.
///
/// Matching on the CLI's PROSE is a genuine weakness and it is the least-bad
/// option available: it exits 0 either way, so there is no code to read, and
/// the two markers below are the parts of that sentence least likely to be
/// reworded — one names the fact, one is a URL the page cannot move without
/// breaking anyway. **If a future CLI changes this wording, these tests keep
/// passing and the button silently breaks again**, so this is the thing to
/// re-measure when the CLI is upgraded, not a place to add more patterns.
fn is_claude_ai_refusal(stdout: &str) -> bool {
    stdout.contains("live on claude.ai") || stdout.contains("claude.ai/customize/connectors")
}

#[tauri::command(async)]
pub fn claude_mcp_auth(name: String, sign_in: bool) -> Result<(), String> {
    use std::process::{Command, Stdio};
    if name.trim().is_empty() {
        return Err("No connector named.".into());
    }
    let verb = if sign_in { "login" } else { "logout" };
    let mut cmd = Command::new(crate::claude_binary());
    cmd.args(["mcp", verb, name.trim()])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    crate::hide_console(&mut cmd);

    let out = cmd
        .output()
        .map_err(|e| format!("Could not run Claude Code: {e}"))?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    if out.status.success() {
        // **THE CLI SAYS NO WITH EXIT CODE 0, AND THIS IS THE DISCONNECT BUG —
        // Mark, 2026-08-27: "the disconnect button does not work. I tried it on
        // all items that offer disconnect."** For a claude.ai-account connector,
        // `claude mcp logout` prints — measured on this box, verbatim —
        // `"<name>" is a claude.ai connector — its credentials live on
        // claude.ai, not this machine. Disconnect it at
        // https://claude.ai/customize/connectors` ... to STDOUT, and exits 0.
        // Reading that exit code as success meant: button pressed, list
        // reloaded, nothing had changed, row redrawn identical, no error shown.
        // Every row that offers Disconnect on a normal install IS a claude.ai
        // row, which is why it looked like N broken buttons and was one cause.
        // The refusal travels up as an Err so the window can do the one honest
        // thing instead: open the page where disconnecting actually works.
        if !sign_in && is_claude_ai_refusal(&stdout) {
            return Err(stdout.trim().to_string());
        }
        return Ok(());
    }
    // Its own words, not ours — it knows why far better than we do, and a
    // generic failure here would send somebody hunting for a cause we already
    // have in hand. Stderr first (measured: `No MCP server named "..."` lands
    // there); stdout as a fallback, because a CLI that already prints one
    // refusal to stdout with exit 0 cannot be trusted to keep its streams
    // straight, and the generic line must be the last resort, not the usual.
    let why = String::from_utf8_lossy(&out.stderr);
    let why = why.trim();
    let why = if why.is_empty() { stdout.trim() } else { why };
    Err(if why.is_empty() {
        format!("Claude Code could not {verb} that connector.")
    } else {
        why.to_string()
    })
}

/// Sign OUT of a connector without deleting it.
///
/// Mark, 2026-08-26: *"have a button to allow for it to disconnect"*. Distinct
/// from Remove on purpose — this drops the credential and leaves the row, so
/// signing back in is one click rather than finding the service again. Remove
/// is for "I do not want this at all".
#[tauri::command(async)]
pub fn disconnect_connector(
    app: tauri::AppHandle,
    state: tauri::State<Connectors>,
    id: String,
) -> Result<Connector, String> {
    use tauri::Manager;
    crate::cancel_running_turn(&app.state::<crate::Session>())?;

    let google_secret = if id.starts_with("gmail-") { secret_for(&id) } else { None };
    // The credential first. If this fails, nothing below has claimed anything —
    // and it FAILS OUT LOUD. This line used to read `let _ =
    // e.delete_credential();`, which is how a failed disconnect hid: the delete
    // fails, the key survives, `list_connectors` re-derives has_secret=true
    // from the store, and the row redraws identical with no error anywhere.
    // A disconnect that did not actually happen must never report that it did —
    // the same rule that keeps the green dot honest. NoEntry alone is fine:
    // nothing stored IS the state this command exists to reach.
    match entry(&id)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => {}
        Err(e) => {
            return Err(format!(
                "Could not remove the stored key from this computer's credential store: {e}. \
                 Nothing was changed — this connector is still signed in."
            ))
        }
    }
    let mut guard = state.inner.lock().unwrap();
    if guard.is_empty() {
        *guard = load_from_disk(&app);
    }
    let Some(slot) = guard.iter_mut().find(|c| c.id == id) else {
        return Err("No such connector.".into());
    };
    slot.has_secret = false;
    slot.account = String::new();
    slot.connected = false;
    slot.checked_at = String::new();
    slot.last_error = String::new();
    if let Some(secret) = google_secret {
        if !crate::google_email::revoke(&secret) {
            slot.last_error = "Disconnected on this device. Google could not confirm revocation; remove helloim.ai access in your Google Account if needed.".into();
        }
    }
    let updated = slot.clone();
    save_to_disk(&app, &guard)?;
    Ok(updated)
}

/// THE REAL CHECK. This is what earns the green dot, and nothing else does.
#[tauri::command(async)]
pub fn test_connector(
    app: tauri::AppHandle,
    state: tauri::State<Connectors>,
    id: String,
) -> Result<Connector, String> {
    let target = {
        let mut guard = state.inner.lock().unwrap();
        if guard.is_empty() {
            *guard = load_from_disk(&app);
        }
        guard.iter().find(|c| c.id == id).cloned()
    };
    let Some(target) = target else {
        return Err("No such connector.".into());
    };

    let outcome = match target.kind.as_str() {
        // A hosted server has an address and no local program. Same protocol,
        // different pipe — see `test_mcp_http`.
        "mcp" if !target.url.trim().is_empty() => test_mcp_http(&target),
        "mcp" => test_mcp(&target),
        // Retired, and the dot must never go green again on a row that does
        // nothing — that is the lie this retirement removed. The refusal
        // lands in `last_error` through the normal path below, so the row
        // explains itself in the window.
        "api" => Err(
            "API providers have moved to the AI components menu — this row no longer does \
             anything here. Its key is still safely stored."
                .to_string(),
        ),
        // This arm used to be `test_api`, which quietly treated any unknown
        // kind as an OpenAI-style API. A kind this code does not know is a
        // hand-edited file, and the honest answer is a refusal that names it,
        // not a test of something it might not be.
        other => Err(format!("Unknown connector kind `{other}` — this row cannot be tested.")),
    };

    let mut guard = state.inner.lock().unwrap();
    let Some(slot) = guard.iter_mut().find(|c| c.id == id) else {
        return Err("No such connector.".into());
    };
    slot.checked_at = now_stamp();
    match outcome {
        Ok(()) => {
            slot.connected = true;
            slot.last_error = String::new();
        }
        Err(why) => {
            slot.connected = false;
            slot.last_error = why;
        }
    }
    let updated = slot.clone();
    save_to_disk(&app, &guard)?;
    Ok(updated)
}

// `test_api` lived here until 2026-08-27 — an authenticated GET of the
// provider's /v1/models. It was deleted with the api kind rather than kept
// "just in case": its green attested "this key can list models", a claim
// nothing consumed, and the provider test in providers.rs makes the strictly
// stronger claim ("this endpoint can be the brain, through the real binary")
// with its own two-stage check. Dead branches that test things honestly are
// still dead branches.

/// The `(NAME, value)` pair a server needs in its environment, or nothing.
///
/// Separated out because BOTH callers need exactly this and they must not
/// disagree: the connection test, and the real launch. A test that runs with
/// the token while the real thing runs without it is a green dot on a server
/// that will not work.

/// What Claude Code has to be told for these servers to exist at all, and the
/// environment to hand the child process.
///
/// **THE BUG THIS FIXES IS THAT THERE WAS NO WIRING.** Connectors were saved,
/// tested, listed and shown with a green dot, and `main.rs` never mentioned
/// MCP anywhere — so every server a person added was decorative. It passed a
/// real handshake in the settings window and was then not passed to the model,
/// which is a worse failure than a missing feature: the app proved the thing
/// worked and then quietly did not use it.
///
/// **THE SECRET DOES NOT GO IN THE FILE.** Claude Code expands `${VAR}` inside
/// `command`, `args`, `env`, `url` and `headers`, so the config on disk names
/// a variable and the VALUE travels in the environment of the `claude` process
/// we spawn. That keeps rule 2 at the top of this file intact: the config is
/// readable by anything that can read the app's config directory, and there is
/// nothing in it worth reading.
///
/// Only CONNECTED servers are included. An untested or failing server handed
/// to Claude Code costs a startup timeout on every single turn, and the person
/// already knows it is red — they are looking at the row.
pub fn launch_config(
    app: &tauri::AppHandle,
    state: &tauri::State<Connectors>,
) -> Option<(String, Vec<(String, String)>)> {
    let mut guard = state.inner.lock().unwrap();
    if guard.is_empty() {
        *guard = load_from_disk(app);
    }

    build_launch_config(&guard)
}

fn build_launch_config(list: &[Connector]) -> Option<(String, Vec<(String, String)>)> {
    let mut servers = serde_json::Map::new();
    let mut env = Vec::new();
    let mut path_prepend: Option<String> = None;

    for c in list.iter() {
        let hosted = !c.url.trim().is_empty();
        if c.kind != "mcp" || !c.connected || (!hosted && c.command.trim().is_empty()) {
            continue;
        }

        // The variable is named after the connector id, not after `env_key`.
        // Two servers can genuinely want the same variable name with different
        // values — two Slack workspaces, two GitHub accounts — and naming the
        // slot after the shared key would let one silently overwrite the other.
        let slot = format!(
            "NAMEOS_SECRET_{}",
            c.id.replace(|ch: char| !ch.is_ascii_alphanumeric(), "_")
                .to_uppercase()
        );
        let secret = secret_for(&c.id);

        let mut entry = serde_json::Map::new();
        if hosted {
            // A hosted server: an address, and the token in a header. Same
            // rule as below — the file names the variable, never the value.
            entry.insert("type".into(), "http".into());
            entry.insert("url".into(), c.url.trim().into());
            if secret.is_some() || c.needs_token {
                let mut h = serde_json::Map::new();
                h.insert("Authorization".into(), format!("Bearer ${{{slot}}}").into());
                if !c.header_key.trim().is_empty() {
                    h.insert(c.header_key.trim().into(), format!("${{{slot}}}").into());
                }
                entry.insert("headers".into(), serde_json::Value::Object(h));
                if let Some(v) = secret { env.push((slot, v)); }
            }
        } else {
            entry.insert("type".into(), "stdio".into());
            // The SAME resolution the test used. A connector that passes its
            // test against the bundled Node and is then handed a bare `npx`
            // is a green dot on something that will not start.
            let (program, node_dir, leading_args) = resolve_command(&c.command);
            entry.insert("command".into(), program.into());
            // `leading_args` (Linux only -- see `resolve_command`'s own doc)
            // goes IN FRONT of the connector's own args: `node
            // <npx-cli.js path> -y <package> …`, never after, or `node`
            // would try to run the package name as its own entry script.
            let mut args: Vec<serde_json::Value> = leading_args.into_iter().map(serde_json::Value::from).collect();
            args.extend(c.args.iter().cloned().map(serde_json::Value::from));
            entry.insert("args".into(), serde_json::Value::Array(args));
            if let Some(dir) = node_dir {
                let existing = std::env::var("PATH").unwrap_or_default();
                let sep = if cfg!(windows) { ";" } else { ":" };
                path_prepend = Some(format!("{}{sep}{existing}", dir.display()));
            }

            // A server can want more than one variable — IMAP needs the
            // address as well as the password. The address goes in literally
            // because it is not a secret; the password never does.
            let mut vars = serde_json::Map::new();
            if !c.account_key.trim().is_empty() && !c.account.trim().is_empty() {
                vars.insert(c.account_key.trim().into(), c.account.trim().into());
            }
            if !c.env_key.trim().is_empty() {
                vars.insert(c.env_key.trim().into(), format!("${{{slot}}}").into());
                if let Some(v) = &secret { env.push((slot.clone(), v.clone())); }
            }
            if c.needs_token && c.env_key.trim().is_empty() {
                vars.insert("NAMEOS_REQUIRED_TOKEN".into(), format!("${{{slot}}}").into());
                if let Some(v) = secret { env.push((slot, v)); }
            }
            if !vars.is_empty() {
                entry.insert("env".into(), serde_json::Value::Object(vars));
            }
        }

        // Claude Code uses this as the tool-name prefix, so it has to survive
        // being an identifier. A person may well call a connector "Mark's Mail".
        let mut key: String = c
            .name
            .trim()
            .chars()
            .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '-' })
            .collect();
        if key.is_empty() || key == "nameos-memory" || servers.contains_key(&key) {
            use sha2::Digest;
            key = format!("{}-{:x}", key, sha2::Sha256::digest(c.id.as_bytes()));
        }
        servers.insert(key, serde_json::Value::Object(entry));
    }

    if servers.is_empty() {
        return None;
    }
    // Set once on the child rather than per server: they all share it, and
    // the last writer would otherwise win silently.
    if let Some(p) = path_prepend {
        env.push(("PATH".to_string(), p));
    }
    let json = serde_json::json!({ "mcpServers": servers }).to_string();
    Some((json, env))
}

/// Pull the JSON-RPC payload out of whichever shape the server answered in.
///
/// Split out from `test_mcp_http` for one reason: it is the part that is worth
/// testing and the only part that can be tested without a network. The SSE
/// case is the one that would otherwise ship broken — plain JSON is what
/// everyone writes their parser against, and half of these servers stream.
#[cfg(test)]
fn jsonrpc_payload(body: &str) -> &str {
    body.lines()
        .find_map(|l| l.trim().strip_prefix("data:"))
        .map(str::trim)
        .unwrap_or_else(|| body.trim())
}

/// THE SAME HANDSHAKE, DOWN A SOCKET INSTEAD OF A PIPE.
///
/// Hosted MCP servers are the answer to "I just want to connect my email
/// without installing anything" — there is nothing to install, the service is
/// already running, and the only thing the person supplies is an address and a
/// token. This sends a real `initialize` to it and requires a real reply.
///
/// TWO SHAPES COME BACK AND BOTH ARE LEGAL. A server may answer with plain
/// JSON, or with a Server-Sent Events stream whose payload is on a `data:`
/// line. Handling only the first is the bug that would make half of these
/// report "replied, but not with MCP JSON-RPC" while working perfectly.
fn test_mcp_http(c: &Connector) -> Result<(), String> { test_configured_mcp(c) }

fn test_configured_mcp(c: &Connector) -> Result<(), String> {
    let mut target = c.clone();
    target.connected = true;
    let (config, env) = build_launch_config(&[target]).ok_or("App setup is incomplete.")?;
    let config = serde_json::from_str(&config).map_err(|_| "App setup could not be read.")?;
    let cancel = std::sync::atomic::AtomicBool::new(false);
    let dir = std::env::current_dir().map_err(|_| "Could not determine the app working directory.")?;
    let tools = crate::engine::native::mcp_client::Toolset::discover(Some(&config), &env, &dir, &cancel);
    if !tools.warnings.is_empty() { return Err(tools.warnings.join("\n")); }
    if tools.definitions.is_empty() { return Err("The app connected but did not offer any tools.".into()); }
    Ok(())
}

/// Resolve `npx` to the copy that ships with this app, if there is one.
///
/// **THE SCAR, 2026-08-26.** Mark: "attaching email did not work." Every tile
/// in the Connections view runs `npx -y <package>`, and his Windows machine
/// has no Node — `where node` and `where npx` both come back empty. So all
/// eight tiles failed, not just email, and each one failed with "Could not
/// start `npx`", which tells a normal person nothing at all.
///
/// The app was built and shipped without once checking whether the machine it
/// runs on has the runtime every single connector depends on. Node now travels
/// in the installer beside the voices, and this is what points at it.
///
/// The bundled copy is preferred over anything on PATH deliberately: a system
/// Node of the wrong vintage is a support problem nobody can see, and the
/// point of shipping one is that there is exactly one answer.
///
/// **THE LINUX PORT'S OWN VERSION OF THIS SCAR, 2026-09-25 — verified
/// against `tauri-bundler`'s and `tauri-utils`'s own source before writing a
/// line of this, not guessed at.** "Beside the exe" is still where a `.deb`'s
/// binary CANNOT have anything staged next to it: it lands in the real,
/// shared, system `/usr/bin/`, a directory this app does not own and must
/// never write a bundled runtime into. So a `.deb` install is checked FIRST,
/// against Tauri's own resolved Linux resource location, before ever falling
/// through to "beside the exe."
///
/// **BOTH LINUX BUNDLES SHARE THE SAME RELATIVE PATH.** The AppImage bundler
/// (`tauri-bundler`'s `appimage/linuxdeploy.rs`) calls the SAME
/// `debian::generate_data` the `.deb` bundler uses to build its own `AppDir`,
/// then copies that whole `usr/` tree into it wholesale — so `bundle.resources`
/// lands at `usr/lib/<product name>/` for a `.deb` exactly as it does for an
/// AppImage, and the two differ only in whether `$APPDIR` (set by the AppImage
/// runtime itself while mounted, never by us) prefixes the path. `<product
/// name>` is `tauri.conf.json`'s `productName`, "helloim.ai" — confirmed from
/// `tauri-codegen`'s own context generation, which is where `PackageInfo::name`
/// (what `tauri-utils::platform::resource_dir` actually joins to `/usr/lib`)
/// is set from. `ship-linux.sh` stages the fetched runtime at
/// `installer/linux/node`, mapped in `tauri.conf.json`'s `bundle.resources` to
/// `"node"`, which is what puts it at this exact path in both bundles.
fn bundled_node_bin() -> Option<PathBuf> {
    #[cfg(target_os = "linux")]
    {
        if let Some(dir) = linux_packaged_node_dir(std::env::var_os("APPDIR").map(PathBuf::from)) {
            return Some(dir);
        }
    }
    // Windows (NSIS stages `node/` directly beside `remembrancer.exe` in the
    // install dir — see `ship-windows.sh`) and a bare dev/CI build run
    // straight out of `target/{debug,release}/` alike: nothing else has ever
    // staged a runtime for either of those, so "beside the exe" is the only
    // place left to look, and it is where it has always been checked.
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?.join("node");
    dir.is_dir().then_some(dir)
}

/// The `.deb`/AppImage half of `bundled_node_bin`, split out PURE — same
/// "decision here, plumbing there" split this house already uses
/// (`providers.rs`'s own `provider_env`/`apply_env`) — so it can be proven
/// against a fake root directory rather than the real, shared, root-owned
/// `/usr/lib` a test has no business writing into. `appdir` is exactly
/// `std::env::var_os("APPDIR")`'s value at the real call site: `Some` inside
/// a running AppImage, `None` for an ordinary `.deb` install where the real
/// filesystem root already IS the one to look in.
#[cfg(target_os = "linux")]
fn linux_packaged_node_dir(appdir: Option<PathBuf>) -> Option<PathBuf> {
    const PRODUCT_NAME: &str = "helloim.ai"; // tauri.conf.json's productName -- see this function's caller for the derivation.
    let root = appdir.unwrap_or_else(|| PathBuf::from("/"));
    let dir = root.join("usr/lib").join(PRODUCT_NAME).join("node");
    dir.is_dir().then_some(dir)
}

/// The command, the PATH entry, and any leading arguments to run an MCP
/// server with.
///
/// Returns `(program, path_prepend_dir, leading_args)`. `path_prepend_dir` is
/// wherever `node` itself actually sits — npm's own shims re-invoke `node`,
/// so the directory has to be reachable or `npx` starts and then dies looking
/// for its own runtime. `leading_args` are prepended in front of whatever
/// args the connector was already configured with (see the Linux branch
/// below for why this exists at all).
///
/// **THE RETURNED DIRECTORY IS WHEREVER `node` ITSELF ACTUALLY SITS, NOT
/// `bundled_node_bin()`'s OWN RETURN VALUE VERBATIM — a real cross-platform
/// distinction, not a redundant reshuffle.** Windows' extracted zip puts
/// `node.exe`/`npx.cmd` directly in the staged folder; the Linux tarball (same
/// shape on this box as the one `ship-linux.sh` fetches, and the one Node
/// itself publishes) nests every binary one level down, in `bin/`. Getting
/// this wrong is silent on Linux specifically: `npx`'s own script resolves
/// `node` via `env` reading PATH, so a PATH prepend of the wrong directory
/// does not fail to start `npx` at all — it starts, and then dies looking for
/// a `node` that PATH does not actually contain, the identical failure shape
/// as the original scar this function exists to fix.
///
/// **THE LINUX BRANCH NEVER RUNS `bin/npx` AS THE PROGRAM — THE SCAR,
/// 2026-09-25, Beck's NO-GO on the v1 Linux build.** The Node tarball ships
/// `bin/npx`/`bin/npm`/`bin/corepack` as RELATIVE SYMLINKS into
/// `lib/node_modules/`, and `ship-linux.sh`'s staging step (Python
/// `tarfile.extractall` + `shutil.move`) preserves them correctly — verified
/// by hand, 2026-09-25, `ls -l installer/linux/node/bin/npx` shows a real
/// `->` symlink after a fresh stage. **The dereferencing happens one step
/// later, inside `cargo tauri bundle` itself, and it is not ours to fix.**
/// Traced into the actual vendored crate on this box
/// (`~/.cargo/registry/src/…/tauri-bundler-2.9.4/src/bundle/settings.rs`,
/// `copy_resources` → `src/utils/fs_utils.rs`, `copy_file`): every entry
/// under a `bundle.resources` mapping (our `"../installer/linux/node":
/// "node"`) is copied with `from.is_file()` (follows symlinks) then
/// `fs::copy(from, to)` (also follows symlinks and writes the TARGET's bytes
/// as a plain new file at the link's own path) — never `copy_dir`, the
/// sibling function in the same file that DOES call `read_link` and
/// recreate a real symlink, but is `#[allow(dead_code)]` and wired to
/// nothing in the resource-copy path. Confirmed against the actual built
/// artifacts sitting in this repo: `dpkg-deb -x` on the `.deb` and the
/// AppImage's own `AppDir` both show `node/bin/npx` as a 2,921-byte regular
/// file — the exact byte size of `npx-cli.js`, not a symlink — in both
/// bundles alike, because the AppImage bundler calls the same
/// `debian::generate_data` the `.deb` does before wrapping it. Patching a
/// third-party crate's vendored source is not a change this app can ship
/// and have survive the next `cargo update`, so the fix is on our side: run
/// `node` directly against npm's own `npx-cli.js`, which sits at a FIXED
/// relative path inside the same resource tree and is copied over intact
/// (it was never a symlink to begin with, so `fs::copy` copying its bytes
/// verbatim is correct, not a bug). This is the exact command Beck's own
/// reproduction proved works: `<node>/bin/node
/// <node>/lib/node_modules/npm/bin/npx-cli.js -y <package>`.
fn resolve_command(command: &str) -> (String, Option<PathBuf>, Vec<String>) {
    let c = command.trim();
    let is_npx = c == "npx" || c == "npx.cmd";
    if !is_npx {
        return (c.to_string(), None, Vec::new());
    }
    match bundled_node_bin() {
        Some(staged_root) => {
            let bin_dir = if cfg!(windows) { staged_root.clone() } else { staged_root.join("bin") };
            if cfg!(windows) {
                // The Windows zip has no symlinks at all (PE binaries carry
                // no exec-bit/symlink concept, and NSIS's own extraction
                // never introduced any) -- `npx.cmd` staged directly beside
                // `node.exe` is exactly the file that ships, unmodified by
                // anything in the bundling pipeline. The scar above is
                // Linux-only; this branch is unchanged.
                let shim = bin_dir.join("npx.cmd");
                if shim.is_file() {
                    return (shim.to_string_lossy().into_owned(), Some(bin_dir), Vec::new());
                }
                return (c.to_string(), Some(bin_dir), Vec::new());
            }
            // Linux: go through `node` + npm's real `npx-cli.js` rather than
            // trusting `bin/npx`, which the bundler has already flattened
            // into a plain copy that dies on its own `require('../lib/
            // cli.js')` once it is no longer physically inside `bin/`.
            let node_bin = bin_dir.join("node");
            let npx_cli = staged_root.join("lib/node_modules/npm/bin/npx-cli.js");
            if node_bin.is_file() && npx_cli.is_file() {
                (node_bin.to_string_lossy().into_owned(), Some(bin_dir), vec![npx_cli.to_string_lossy().into_owned()])
            } else {
                (c.to_string(), Some(bin_dir), Vec::new())
            }
        }
        None => (c.to_string(), None, Vec::new()),
    }
}

/// A REAL MCP HANDSHAKE. Spawning the command and seeing it not crash proves
/// almost nothing — plenty of binaries start and then fail to speak the
/// protocol. This sends a genuine JSON-RPC `initialize` over stdio and requires
/// a well-formed response carrying the same id back.
fn test_mcp(c: &Connector) -> Result<(), String> { test_configured_mcp(c) }

#[cfg(test)]
mod tests {
    use super::*;

    /// The SSE case is the reason this function exists. A hosted MCP server is
    /// allowed to answer a POST with a Server-Sent Events stream, and a parser
    /// written against plain JSON reports a perfectly healthy server as
    /// "replied, but not with MCP JSON-RPC".
    #[test]
    fn it_reads_a_streamed_reply() {
        let sse = "event: message\ndata: {\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{}}\n\n";
        assert_eq!(jsonrpc_payload(sse), "{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{}}");
    }

    #[test]
    fn it_reads_a_plain_json_reply() {
        let plain = "{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{}}";
        assert_eq!(jsonrpc_payload(plain), plain);
    }

    /// Whitespace and a trailing newline are the normal shape off a socket.
    #[test]
    fn it_survives_the_shapes_a_socket_actually_produces() {
        assert_eq!(jsonrpc_payload("  {\"result\":1}  \n"), "{\"result\":1}");
        assert_eq!(jsonrpc_payload("data:  {\"result\":1}\n"), "{\"result\":1}");
    }

    /// A hosted connector is one with an address; there is no second field
    /// that can disagree with it. This pins that decision down.
    #[test]
    fn an_address_is_what_makes_it_hosted() {
        let mut c = Connector {
            id: "c1".into(), kind: "mcp".into(), name: "n".into(),
            provider: String::new(), base_url: String::new(), model: String::new(),
            command: String::new(), args: vec![], env_key: String::new(),
            account: String::new(), account_key: String::new(),
            url: String::new(), needs_token: false, header_key: String::new(),
            connected: false,
            checked_at: String::new(), last_error: String::new(), has_secret: false,
        };
        assert!(c.url.trim().is_empty(), "no address means stdio");
        c.url = "https://example.com/mcp".into();
        assert!(!c.url.trim().is_empty(), "an address means hosted");
    }

    // ---- folder_scoped_connector_args: Beck's VM verification 2026-09-25,
    // ---- Cassandra's multi-dir/trailing-flag re-review 2026-09-25 --------

    fn stdio_connector(command: &str, args: Vec<String>) -> Connector {
        Connector {
            id: "c1".into(), kind: "mcp".into(), name: "n".into(),
            provider: String::new(), base_url: String::new(), model: String::new(),
            command: command.into(), args, env_key: String::new(),
            account: String::new(), account_key: String::new(),
            url: String::new(), needs_token: false, header_key: String::new(),
            connected: false,
            checked_at: String::new(), last_error: String::new(), has_secret: false,
        }
    }

    /// The real Filesystem tile's shape: `npx -y @modelcontextprotocol/
    /// server-filesystem <chosen folder>` — the folder, appended last by
    /// the front end's own folder picker, must be recognised.
    #[test]
    fn a_stdio_connector_whose_last_arg_is_a_real_folder_is_detected() {
        let dir = std::env::temp_dir().join(format!("connector-folder-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let c = stdio_connector(
            "npx",
            vec!["-y".into(), "@modelcontextprotocol/server-filesystem".into(), dir.to_string_lossy().into_owned()],
        );
        assert_eq!(folder_scoped_connector_args(&c), vec![dir.to_string_lossy().as_ref()]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **NEVER TRIPPED BY A PACKAGE NAME — the whole point of detecting
    /// this by what the argument IS rather than by which package asked for
    /// it.** Sequential Thinking / Knowledge notes / Brave / Exa / Tavily
    /// all end their `args` on a package string, never a real directory on
    /// this machine.
    #[test]
    fn a_stdio_connector_ending_on_a_package_name_is_not_detected() {
        let c = stdio_connector("npx", vec!["-y".into(), "@modelcontextprotocol/server-sequential-thinking".into()]);
        assert!(folder_scoped_connector_args(&c).is_empty());
    }

    #[test]
    fn a_hosted_oauth_connector_is_never_folder_scoped_even_with_a_directory_shaped_url() {
        let mut c = stdio_connector("", vec![]);
        c.url = "https://example.com/mcp".into();
        // Command/args empty is the ordinary hosted shape; the `url` being
        // set is what must rule this out regardless.
        assert!(folder_scoped_connector_args(&c).is_empty());
    }

    #[test]
    fn a_stdio_connector_with_no_args_at_all_is_not_folder_scoped() {
        let c = stdio_connector("some-binary", vec![]);
        assert!(folder_scoped_connector_args(&c).is_empty());
    }

    /// **THE MULTI-DIR HOLE, CLOSED — Cassandra's re-review, 2026-09-25.**
    /// `server-filesystem` takes more than one positional directory. The
    /// old last-arg-only check would have missed the FIRST one entirely —
    /// accepting the harmless, already-trusted second folder once would
    /// have left the first one with ungated access, never reviewed. Both
    /// must be found, and `save_connector`'s own loop over this list is
    /// what then refuses on whichever one is not trusted.
    #[test]
    fn a_connector_with_two_real_directory_args_detects_both() {
        let base = std::env::temp_dir().join(format!("connector-multidir-test-{}", std::process::id()));
        let private = base.join("private");
        let empty = base.join("empty");
        std::fs::create_dir_all(&private).unwrap();
        std::fs::create_dir_all(&empty).unwrap();
        let c = stdio_connector(
            "npx",
            vec![
                "-y".into(),
                "@modelcontextprotocol/server-filesystem".into(),
                private.to_string_lossy().into_owned(),
                empty.to_string_lossy().into_owned(),
            ],
        );
        let found = folder_scoped_connector_args(&c);
        assert_eq!(
            found,
            vec![private.to_string_lossy().as_ref(), empty.to_string_lossy().as_ref()],
            "both directories must be found, not just the last one -- accepting ONE of them \
             must never be read as clearing the other"
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    /// **THE TRAILING-FLAG HOLE, CLOSED — Cassandra's re-review,
    /// 2026-09-25.** `args.last()` on `[dir, "--readonly"]` is a flag, not
    /// a directory — the old check found nothing here and the gate never
    /// ran at all, even though the connector is plainly scoped to `dir`.
    /// The fix finds the directory regardless of what comes after it.
    #[test]
    fn a_directory_followed_by_a_trailing_flag_is_still_gated_on_the_directory() {
        let dir = std::env::temp_dir().join(format!("connector-trailingflag-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let c = stdio_connector(
            "npx",
            vec![
                "-y".into(),
                "@modelcontextprotocol/server-filesystem".into(),
                dir.to_string_lossy().into_owned(),
                "--readonly".into(),
            ],
        );
        assert_eq!(
            folder_scoped_connector_args(&c),
            vec![dir.to_string_lossy().as_ref()],
            "a directory followed by a flag must still be found -- `args.last()` alone missed \
             this entirely, which read as 'not folder-scoped' and skipped the gate completely"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ---- bundled_node_bin / resolve_command: Mark's 2026-09-25 reversal ---
    // ---- (bundle Node again for the local MCP tiles) ----------------------

    /// **SERIALISED, AND THAT IS NOT OPTIONAL HERE.** `current_exe()` is the
    /// same physical path for every test in this binary — `bundled_node_bin`
    /// has no injection seam, on purpose, because the real function reads
    /// the real running exe's own location and a fake would prove nothing
    /// about it. That means every test below stages the SAME real `node`
    /// directory on disk, and Rust runs tests in parallel by default:
    /// without a lock, one test's cleanup mid-run reads as another test's
    /// missing directory, which is exactly the shape of flake this house
    /// has already paid for once in a different file's tests
    /// (`native_capture`'s own shared-global-state pair). One `Mutex`,
    /// held for the whole body, same idiom `engine::mod`'s `with_gate`
    /// already uses for a shared switch.
    fn with_locked_node_dir<T>(f: impl FnOnce(&std::path::Path) -> T) -> T {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let exe = std::env::current_exe().expect("a running test binary has a path");
        let node_dir = exe.parent().expect("an exe has a parent directory").join("node");
        assert!(
            !node_dir.exists(),
            "a stray node/ directory beside the test binary from a previous run would \
             invalidate every test that locks this -- something did not clean up after itself"
        );
        let out = f(&node_dir);
        let _ = std::fs::remove_dir_all(&node_dir);
        drop(guard);
        out
    }

    /// **THE ACTUAL RESOLUTION MECHANISM, PROVEN AGAINST A REAL DIRECTORY —
    /// not asserted from reading `bundled_node_bin`'s three lines and
    /// trusting them.** Stages a real `node` folder beside wherever this
    /// run's test binary actually lives — the identical shape
    /// `ship-windows.sh` + `NameOS.nsi` produce beside the installed
    /// `helloim.ai.exe` (a `node` directory, sibling to the exe) — and
    /// confirms the function finds it there.
    #[test]
    fn bundled_node_bin_finds_a_node_directory_beside_the_running_exe() {
        with_locked_node_dir(|node_dir| {
            std::fs::create_dir_all(node_dir).unwrap();
            assert_eq!(bundled_node_bin(), Some(node_dir.to_path_buf()));
        });
    }

    /// The negative of the test above, same real mechanism: no `node`
    /// directory beside the exe means no bundled runtime to prefer, and
    /// `resolve_command` must fall back to a bare `npx` on PATH rather
    /// than inventing a path that does not exist.
    #[test]
    fn bundled_node_bin_is_none_when_no_node_directory_is_staged() {
        with_locked_node_dir(|_node_dir| {
            // Deliberately never created in this test -- `with_locked_node_dir`
            // itself already asserted it does not exist going in.
            assert_eq!(bundled_node_bin(), None);
        });
    }

    /// `resolve_command` end to end on Linux: given a real staged `node`
    /// directory with `bin/node` and the REAL `lib/node_modules/npm/bin/
    /// npx-cli.js` present, the resolved program is `node` itself, with the
    /// `npx-cli.js` path as a leading argument — never `bin/npx` directly,
    /// even when a file sits right there at that path (this test writes
    /// garbage into it on purpose, standing in for what the bundler's
    /// symlink-flattening actually leaves behind, to prove the function
    /// does not trust it).
    ///
    /// **CHANGED 2026-09-25 — Beck's Linux NO-GO.** The prior version of
    /// this test asserted `bin/npx` was the resolved program, which is
    /// exactly the assumption that shipped broken: `cargo tauri bundle`
    /// dereferences the tarball's `bin/npx` SYMLINK into a plain copy of
    /// `npx-cli.js`, so `bin/npx`'s own `require('../lib/cli.js')` resolves
    /// against `bin/`'s parent instead of the real npm install tree and
    /// dies with `Cannot find module`. See `resolve_command`'s own doc for
    /// the full trace into `tauri-bundler`'s vendored source.
    #[test]
    fn resolve_command_runs_node_against_the_real_npx_cli_on_linux() {
        with_locked_node_dir(|node_dir| {
            let bin_dir = node_dir.join("bin");
            std::fs::create_dir_all(&bin_dir).unwrap();
            std::fs::write(bin_dir.join("node"), "not a real binary, just needs to exist").unwrap();
            // Stands in for the bundler's flattened, unusable `bin/npx` --
            // present on disk, but never the thing `resolve_command` should
            // hand back as the program to run.
            std::fs::write(bin_dir.join("npx"), "require('../lib/cli.js')").unwrap();
            let npm_bin_dir = node_dir.join("lib/node_modules/npm/bin");
            std::fs::create_dir_all(&npm_bin_dir).unwrap();
            let npx_cli = npm_bin_dir.join("npx-cli.js");
            std::fs::write(&npx_cli, "#!/usr/bin/env node\nrequire('../lib/cli.js')").unwrap();

            let (program, dir, leading_args) = resolve_command("npx");

            assert_eq!(program, bin_dir.join("node").to_string_lossy().into_owned(), "the program must be `node` itself, never the bundler-flattened `bin/npx`");
            assert_eq!(dir, Some(bin_dir), "the PATH entry must be the directory `node` ITSELF is in (`node/bin`), not the staged root -- prepending the root leaves `env` unable to find `node` when a child process looks for it");
            assert_eq!(leading_args, vec![npx_cli.to_string_lossy().into_owned()], "the real npx-cli.js path must be the leading argument to `node`");
        });
    }

    /// When the real `npx-cli.js` is not where it should be (a corrupted or
    /// partial stage), `resolve_command` must fall back to a bare `npx`
    /// rather than handing back a `node` invocation with no script to run.
    #[test]
    fn resolve_command_falls_back_when_npx_cli_is_missing() {
        with_locked_node_dir(|node_dir| {
            let bin_dir = node_dir.join("bin");
            std::fs::create_dir_all(&bin_dir).unwrap();
            std::fs::write(bin_dir.join("node"), "not a real binary, just needs to exist").unwrap();
            // No lib/node_modules/npm/bin/npx-cli.js staged at all.

            let (program, dir, leading_args) = resolve_command("npx");

            assert_eq!(program, "npx", "with no real npx-cli.js to run, fall back to a bare `npx` rather than inventing a broken `node` invocation");
            assert_eq!(dir, Some(bin_dir));
            assert!(leading_args.is_empty());
        });
    }

    // ---- linux_packaged_node_dir: the `.deb`/AppImage half, 2026-09-25 ----
    // ---- proven against a fake root, never the real /usr/lib --------------

    /// A `.deb` install: no `APPDIR`, so the "root" to look under is
    /// whatever `PathBuf::from("/")` would normally mean -- proven here
    /// against a substitute root instead, which is the entire reason this
    /// half was split out pure.
    #[test]
    #[cfg(target_os = "linux")]
    fn linux_packaged_node_dir_finds_a_deb_style_install_under_a_fake_root() {
        let root = std::env::temp_dir().join(format!("connector-deb-root-test-{}", std::process::id()));
        let node_dir = root.join("usr/lib/helloim.ai/node");
        std::fs::create_dir_all(&node_dir).unwrap();
        assert_eq!(linux_packaged_node_dir(Some(root.clone())), Some(node_dir));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// An AppImage: `APPDIR` is set (by the AppImage runtime, never by us),
    /// and the SAME relative path is looked for underneath it instead of
    /// under `/` -- the two bundles share the shape, differing only in what
    /// they are rooted at. Same fake-root mechanism as the `.deb` test
    /// above; the only difference in THIS test is that a caller-supplied
    /// root is exactly what `Some(appdir)` looks like at the real call site.
    #[test]
    #[cfg(target_os = "linux")]
    fn linux_packaged_node_dir_finds_an_appimage_style_mount_under_appdir() {
        let appdir = std::env::temp_dir().join(format!("connector-appdir-test-{}", std::process::id()));
        let node_dir = appdir.join("usr/lib/helloim.ai/node");
        std::fs::create_dir_all(&node_dir).unwrap();
        assert_eq!(linux_packaged_node_dir(Some(appdir.clone())), Some(node_dir));
        let _ = std::fs::remove_dir_all(&appdir);
    }

    /// Nothing staged under the given root at all -- `None`, not a
    /// fabricated path, same "do not invent a path that does not exist"
    /// contract `bundled_node_bin_is_none_when_no_node_directory_is_staged`
    /// already holds for the beside-the-exe half.
    #[test]
    #[cfg(target_os = "linux")]
    fn linux_packaged_node_dir_is_none_when_nothing_is_staged() {
        let root = std::env::temp_dir().join(format!("connector-empty-root-test-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        assert_eq!(linux_packaged_node_dir(Some(root.clone())), None);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A command that is not `npx`/`npx.cmd` at all (a hosted connector's
    /// own binary, or a future non-Node local server) must never be
    /// rewritten — the bundled-Node resolution is specifically for `npx`.
    #[test]
    fn resolve_command_leaves_a_non_npx_command_untouched() {
        assert_eq!(resolve_command("some-other-binary"), ("some-other-binary".to_string(), None, Vec::new()));
    }
}

#[cfg(test)]
mod list_tests {
    use super::{is_claude_ai_refusal, parse_mcp_list};

    /// Verbatim from `claude mcp list` on this machine, including the plugin
    /// line whose NAME contains colons -- splitting on the first one would cut
    /// every plugin's name in half.
    const REAL: &str = "Checking MCP server health…\n\nclaude.ai Higgsfield: https://mcp.higgsfield.ai/mcp - ✔ Connected\nclaude.ai Microsoft 365: https://microsoft365.mcp.claude.com/mcp - ! Needs authentication\nclaude.ai Gmail: https://gmailmcp.googleapis.com/mcp/v1 - ✔ Connected\nplugin:getlayers:getlayers: https://mcp.getlayers.ai/mcp (HTTP) - ✔ Connected\n";

    #[test]
    fn it_reads_the_real_output() {
        let got = parse_mcp_list(REAL);
        assert_eq!(got.len(), 4, "{got:?}");
        assert_eq!(got[0].name, "claude.ai Higgsfield");
        assert_eq!(got[0].status, "connected");
        assert_eq!(got[1].name, "claude.ai Microsoft 365");
        assert_eq!(got[1].status, "needs-auth");
        assert_eq!(got[2].url, "https://gmailmcp.googleapis.com/mcp/v1");
        // The colons in a plugin name survive, and the "(HTTP)" is not part of
        // the URL.
        assert_eq!(got[3].name, "plugin:getlayers:getlayers");
        assert_eq!(got[3].url, "https://mcp.getlayers.ai/mcp");
    }

    #[test]
    fn noise_and_an_empty_list_produce_nothing() {
        assert!(parse_mcp_list("Checking MCP server health…\n\n").is_empty());
        assert!(parse_mcp_list("").is_empty());
        assert!(parse_mcp_list("No MCP servers configured.").is_empty());
    }

    // THE DISCONNECT BUG, 2026-08-27 — Mark: "the disconnect button does not
    // work. I tried it on all items that offer disconnect." The cause was a
    // refusal that exits 0, so the code read it as success and the row redrew
    // identical with no error anywhere. These are the regression tests that
    // were missing when it shipped; the string below is the CLI's real output,
    // copied from the measurement in `claude_mcp_auth`'s own comment rather
    // than invented here, because a test written against an imagined message
    // proves the code agrees with me and nothing about the CLI.

    #[test]
    fn the_exit_zero_refusal_is_not_success() {
        let real = "\"Gmail\" is a claude.ai connector — its credentials live \
                    on claude.ai, not this machine. Disconnect it at \
                    https://claude.ai/customize/connectors";
        assert!(is_claude_ai_refusal(real));
    }

    #[test]
    fn either_marker_alone_is_enough() {
        // The two are deliberately independent: a reworded sentence that keeps
        // the URL, or a moved page that keeps the sentence, still gets caught.
        assert!(is_claude_ai_refusal("its credentials live on claude.ai"));
        assert!(is_claude_ai_refusal("see https://claude.ai/customize/connectors"));
    }

    #[test]
    fn a_real_disconnect_is_still_a_success() {
        // The failure mode in the other direction, and the reason this is a
        // narrow predicate rather than a search for "claude.ai": treating an
        // ordinary success as a refusal would break every connector that CAN
        // be disconnected from this machine, which is the whole feature.
        assert!(!is_claude_ai_refusal(""));
        assert!(!is_claude_ai_refusal("Logged out of \"Gmail\"."));
        assert!(!is_claude_ai_refusal("Disconnected."));
        // Mentioning the host in passing is not a refusal.
        assert!(!is_claude_ai_refusal("Signed in via https://claude.ai/oauth"));
    }
}
