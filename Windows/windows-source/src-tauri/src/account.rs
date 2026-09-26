//! Account — signing in to helloim.ai from the app, and syncing settings with
//! it. Everything a signed-OUT install already does keeps working exactly as
//! it did before this file existed; this only ever ADDS a second, optional
//! path on top. See `desktop/WEBAPP-SYNC-SPEC.md` for the design this
//! implements and `FACTS.md` for what the app's account backend already is.
//!
//! **THE APP HAS TWO SEPARATE ACCOUNT SPACES, DELIBERATELY, AND THIS FILE IS
//! ONLY EVER THE SECOND ONE.** `nameos.ai` is the update/entitlement backend
//! `update.rs` and `providers.rs`'s migration path already talk to — see
//! `update.rs`'s own header for why it stayed `nameos.ai` through the
//! helloim.ai rename. `helloim.ai` is a DIFFERENT Worker (`helloim-api`, see
//! `helloim/worker.js`) with its own account space (`SESSION_AUD ==
//! "helloim"`), used here to hold "the options they see on the web" and
//! nothing else. Signing in here never presents a helloim.ai session to
//! nameos.ai or vice versa, and the two tokens are never stored under the
//! same keyring entry — see `KEYRING_SERVICE` below. The nameos.ai
//! update/account path (`update.rs`, `providers.rs`'s migration) is not
//! touched by this file at all.
//!
//! **HTTP CLIENT: `ureq`, NOT `reqwest`.** The brief this module was built
//! from named `reqwest`; this crate has never depended on it, and every
//! existing native HTTP call here — `update.rs`, `feedback.rs`,
//! `google_email.rs`, `ms_graph.rs`, `adapter.rs` — already uses `ureq`.
//! Adding a second HTTP client to reach one more helloim.ai route would be
//! new dependency surface for no benefit, so this follows the existing one.
//! One consequence that matters: `Cargo.toml` builds `ureq` with
//! `default-features = false, features = ["tls"]` — the `json` feature is
//! NOT compiled in, so `send_json`/`into_json` do not exist here. Bodies go
//! out with `send_string` over a `serde_json::Value` rendered by
//! `.to_string()`, and come back through `into_string()` then
//! `serde_json::from_str`, the same shape `feedback.rs`, `install.rs` and
//! `skills.rs` already use.
//!
//! **NO CSP CHANGE, FOR THE SAME REASON AS `feedback.rs`.** The webview's
//! `connect-src` (`tauri.conf.json`) allows `'self'` and `nameos.ai`, not
//! `helloim.ai`. A native Rust request is not subject to that policy at all,
//! so the fix was never to widen the CSP — a cost paid on every page load
//! for one feature — it is to make the request from here, the same choice
//! `feedback.rs` already made for the exact same host.
//!
//! **BLOCKING NETWORK CALLS RUN ON `spawn_blocking`, NOT A BARE SYNC
//! COMMAND.** `ureq` blocks the calling thread; `feedback.rs` and
//! `mic_hotkeys.rs` already established moving that work off Tauri's async
//! runtime rather than tying up whatever thread dispatches commands, and the
//! four commands here that touch the network (sign in, sign out, push, pull)
//! follow it. `account_status` touches neither the network nor anything
//! slow, so it stays a plain synchronous command, the same as
//! `profile::load_profile`.
//!
//! **THE SESSION TOKEN LIVES IN THE OS KEYRING, UNDER ITS OWN SERVICE NAME —
//! NEVER `connectors::KEYRING_SERVICE` ("NameOS").** That namespace already
//! holds real secrets keyed by a caller-chosen id ("ids are never reused" is
//! `connectors.rs`'s own invariant for it); reusing it here would put a
//! helloim.ai session in the same drawer as a person's OpenAI key and their
//! nameos.ai entitlement, one id collision away from meaning two things. A
//! dedicated service name costs nothing and removes the question entirely.
//!
//! **WHAT IS, AND IS NOT, APPLIED AUTOMATICALLY ON A PULL.** The profile
//! fields `profile.rs` already owns the anti-wipe discipline for — `about`,
//! `goal`, `memory`, `assistant_name`, `personality`, `wake_word` — go
//! straight into local storage through `profile::merge_and_write`, the exact
//! function `save_profile` itself uses. A field no device has ever synced
//! (the server encodes that as JSON `null`, which `RemoteProfile` below
//! deserializes straight to `None`) can never overwrite what is already on
//! this machine — the same bug `profile.rs`'s own `ProfilePatch` doc exists
//! to prevent, reached here from a second direction.
//!
//! Provider selection (`provider_id`/`provider_base_url`/`provider_model`)
//! and the two consequential per-provider switches (`allow_agency`,
//! `allow_shell`) are DELIBERATELY NOT applied the same way.
//! `providers::Provider` treats arming either of those as consequential
//! enough to need its own form, its own secret and its own round trip
//! (`save_provider`, `test_provider`) before the row goes green, and both
//! fields are pinned "default closed, forever" for any row the app has never
//! seen written (`a_legacy_row_with_no_allow_agency_field_defaults_closed`
//! and its `allow_shell` twin, in `providers.rs`). Auto-applying a pulled
//! `true` onto a matching local row from an unattended sync would be the
//! first place in this product that arms shell access without a person
//! looking at the checkbox. So this module reports the remote values in
//! `PullResult` and does nothing else with them; deciding whether and how to
//! offer applying them is a window decision, flagged in the dispatch report
//! rather than made here.
//!
//! **FIELD TIMESTAMPS ARE NOT SENT ON PUSH.** The last-writer-wins contract
//! (`helloim/worker.js`'s `handleProfile`) reads a client-supplied
//! `field_ts[field]` when present and falls back to the server's own clock
//! when it is absent or unparsable — `isoOrNull(clientTs[field]) || now`.
//! This crate has no date/time dependency, and hand-rolling an RFC3339 clock
//! for one optional field the server already covers gracefully is not worth
//! the new code. Omitting it costs precision only when two devices edit the
//! SAME field within moments of each other; the server's own clock is the
//! honest fallback that already exists for exactly that case.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::Duration;
use tauri::{AppHandle, Manager};

/// A DIFFERENT namespace from `connectors::KEYRING_SERVICE` — see the module
/// doc for why reusing "NameOS" would be the wrong call here.
const KEYRING_SERVICE: &str = "helloim.ai";
/// One helloim.ai account signed in per machine at a time, so a fixed id is
/// correct — the same "per-device" assumption `WEBAPP-SYNC-SPEC.md` already
/// makes for provider API keys.
const TOKEN_ACCOUNT: &str = "session";

const API_BASE: &str = "https://helloim.ai/api";
/// Generous relative to `feedback.rs`'s 10s: a sign-in does password hashing
/// server-side (PBKDF2 — see `helloim/worker.js`'s login header) and a sync
/// round-trips a KV read/write, both slower than that file's one-line POST.
const NET_TIMEOUT: Duration = Duration::from_secs(20);

/// `pub(crate)` so `memory_sync.rs` (the network half of shared-memory sync)
/// can talk to `helloim.ai` with the exact same client config — timeout
/// included — rather than a second `ureq::Agent` that could quietly drift
/// from this one.
pub(crate) fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new().timeout(NET_TIMEOUT).build()
}

// ---------------------------------------------------------------------------
// What the window is told.
// ---------------------------------------------------------------------------

#[derive(Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AccountStatus {
    pub signed_in: bool,
    pub email: Option<String>,
    pub display_name: Option<String>,
}

#[derive(Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PushResult {
    pub settings_version: u64,
    /// True when a 409 fired mid-push and this call resolved it itself (a
    /// re-pull, a merge, one retry) rather than the caller having to.
    pub resolved_conflict: bool,
}

#[derive(Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PullResult {
    pub settings_version: u64,
    /// Where the merged profile actually landed — empty when the working
    /// folder is not one the app may write into yet. Same honesty
    /// convention as `profile::save_profile`'s own return.
    pub claude_md_path: String,
    pub voice_id: Option<String>,
    pub default_skin_id: Option<String>,
    /// Reported, never applied — see the module doc's note on why provider
    /// selection and the two consequential switches below stop here.
    pub provider_id: Option<String>,
    pub provider_base_url: Option<String>,
    pub provider_model: Option<String>,
    pub allow_agency: Option<bool>,
    pub allow_shell: Option<bool>,
}

// ---------------------------------------------------------------------------
// What the window sends on a push. Same Option-per-field IPC shape as
// `profile::ProfilePatch`, and for the same reason: absent must mean
// "unchanged", never "clear this", and `deny_unknown_fields` turns a typo'd
// key into a loud refusal instead of a silent no-op.
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct SettingsPatch {
    pub assistant_name: Option<String>,
    pub personality: Option<String>,
    pub wake_word: Option<String>,
    pub voice_id: Option<String>,
    pub about: Option<String>,
    pub goal: Option<String>,
    pub memory: Option<String>,
    pub allow_agency: Option<bool>,
    pub allow_shell: Option<bool>,
    pub provider_id: Option<String>,
    pub provider_base_url: Option<String>,
    pub provider_model: Option<String>,
    pub default_skin_id: Option<String>,
}

impl SettingsPatch {
    fn is_empty(&self) -> bool {
        self.assistant_name.is_none()
            && self.personality.is_none()
            && self.wake_word.is_none()
            && self.voice_id.is_none()
            && self.about.is_none()
            && self.goal.is_none()
            && self.memory.is_none()
            && self.allow_agency.is_none()
            && self.allow_shell.is_none()
            && self.provider_id.is_none()
            && self.provider_base_url.is_none()
            && self.provider_model.is_none()
            && self.default_skin_id.is_none()
    }

    /// The wire shape `helloim/worker.js`'s `handleProfile` reads — plain
    /// snake_case keys, matching its `PROFILE_FIELDS`/`PROFILE_STR_FIELDS`/
    /// `PROFILE_BOOL_FIELDS` literally. THIS IS A DIFFERENT CASE CONVENTION
    /// from this struct's own IPC wire (the window talks `camelCase` to this
    /// binary; this binary talks `snake_case` to the server) — so this is
    /// built by hand rather than via `serde_json::to_value(self)`, which
    /// would carry the wrong case straight through. Only fields that are
    /// `Some` are included, which is what makes a partial patch a partial
    /// write on the far end too.
    fn to_wire(&self) -> serde_json::Value {
        let mut m = serde_json::Map::new();
        if let Some(v) = &self.assistant_name { m.insert("assistant_name".into(), v.clone().into()); }
        if let Some(v) = &self.personality { m.insert("personality".into(), v.clone().into()); }
        if let Some(v) = &self.wake_word { m.insert("wake_word".into(), v.clone().into()); }
        if let Some(v) = &self.voice_id { m.insert("voice_id".into(), v.clone().into()); }
        if let Some(v) = &self.about { m.insert("about".into(), v.clone().into()); }
        if let Some(v) = &self.goal { m.insert("goal".into(), v.clone().into()); }
        if let Some(v) = &self.memory { m.insert("memory".into(), v.clone().into()); }
        if let Some(v) = &self.provider_id { m.insert("provider_id".into(), v.clone().into()); }
        if let Some(v) = &self.provider_base_url { m.insert("provider_base_url".into(), v.clone().into()); }
        if let Some(v) = &self.provider_model { m.insert("provider_model".into(), v.clone().into()); }
        if let Some(v) = &self.default_skin_id { m.insert("default_skin_id".into(), v.clone().into()); }
        // Sent as real JSON booleans -- the server's own `asBool01` does a
        // truthiness check (`v ? 1 : 0`), so `true`/`false` land as 1/0
        // exactly as a 0/1 integer would, with no translation needed here.
        if let Some(v) = self.allow_agency { m.insert("allow_agency".into(), v.into()); }
        if let Some(v) = self.allow_shell { m.insert("allow_shell".into(), v.into()); }
        serde_json::Value::Object(m)
    }
}

// ---------------------------------------------------------------------------
// What the server sends back. Snake_case, matching its JSON keys literally —
// see `to_wire`'s own note on why that is the opposite convention from this
// file's IPC structs above.
// ---------------------------------------------------------------------------

/// A field absent from the JSON and a field present as `null` both land as
/// `None` here — serde's ordinary handling for an `Option<T>` field, no
/// `#[serde(default)]` required. That is exactly the shape the server
/// documents: `null` means "no device has ever synced this field", and
/// `to_profile_patch` below depends on that meaning "leave it alone" rather
/// than "clear it".
#[derive(Clone, Debug, Default, Deserialize)]
struct RemoteProfile {
    provider_id: Option<String>,
    provider_base_url: Option<String>,
    provider_model: Option<String>,
    default_skin_id: Option<String>,
    assistant_name: Option<String>,
    personality: Option<String>,
    wake_word: Option<String>,
    voice_id: Option<String>,
    about: Option<String>,
    goal: Option<String>,
    memory: Option<String>,
    /// `typeof src[k] === "number" ? src[k] : null` server-side — 0, 1 or
    /// `null`, never a bare JSON boolean.
    allow_agency: Option<i64>,
    allow_shell: Option<i64>,
}

impl RemoteProfile {
    /// The half of this record `profile.rs` already knows how to merge
    /// without wiping anything — see the module doc's note on why provider
    /// selection and the two consequential switches are not part of this.
    ///
    /// `voice` (the derived writing-style card) is never carried here at
    /// all. It is measured locally from things the person wrote on THIS
    /// machine; syncing it would mean either shipping the raw material that
    /// built it — the one thing `profile.rs`'s own header promises never
    /// leaves the machine — or shipping a stale copy that quietly drifts
    /// into exactly the thing that file calls out as the wrong design: "a
    /// summary written by a model".
    fn to_profile_patch(&self) -> crate::profile::ProfilePatch {
        crate::profile::ProfilePatch {
            about: self.about.clone(),
            goal: self.goal.clone(),
            memory: self.memory.clone(),
            assistant_name: self.assistant_name.clone(),
            personality: self.personality.clone(),
            wake_word: self.wake_word.clone(),
            voice: None,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
struct ProfileResponse {
    profile: RemoteProfile,
    settings_version: u64,
}

#[derive(Clone, Debug, Deserialize)]
struct LoginResponse {
    token: String,
    email: String,
    #[serde(rename = "displayName")]
    display_name: Option<String>,
}

/// The `license` object helloim.ai attaches to `/api/auth/me` and returns
/// whole from `/api/license` — see `helloim/API-auth.md`'s own table.
/// **`status: "none"` means `entitled: false` here — the OPPOSITE of
/// nameos.ai, where `none` means entitled.** That difference is the server's
/// own contract, not a bug in this struct: nameos.ai's `none` only ever
/// decided whether an UPDATE was offered, so it failed open; helloim.ai's
/// `none` can gate a paid feature one day, so it fails closed. Nothing in
/// this app may read `entitled` as a gate today regardless — see
/// `docs/plans/app-account-migration-to-helloim-2026-09-23.md` (4) — this
/// struct only carries the field so the badge can show it honestly.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LicenseInfo {
    pub status: String,
    pub plan: Option<String>,
    pub entitled: bool,
    pub expires_at: Option<String>,
}

/// `GET /api/auth/me` — the launch check. A strict superset of nameos.ai's
/// own shape (see `API-auth.md`); this struct reads only the fields the app
/// actually uses today. `email_verified`/`role`/`profile` are on the wire but
/// unread here — nothing in this release surfaces them, and adding fields
/// nobody reads is exactly the "premature" half of Simplicity First.
#[derive(Clone, Debug, Deserialize)]
struct MeResponse {
    email: String,
    #[serde(rename = "displayName")]
    display_name: Option<String>,
    license: LicenseInfo,
}

#[derive(Clone, Debug, Deserialize)]
struct LicenseResponse {
    license: LicenseInfo,
}

// ---------------------------------------------------------------------------
// The local, non-secret half of a sign-in — everything EXCEPT the token.
// Same split `providers.json`/`has_secret` already uses: metadata on disk,
// the actual credential in the OS keyring, never together.
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct AccountRecord {
    email: String,
    display_name: String,
    /// The last `settings_version` this machine has seen, so a push can send
    /// a `base_version` without needing a pull first. Best-effort cache, not
    /// a source of truth — the server's own count always wins a conflict.
    settings_version: u64,
    /// The `/api/persona` twin of `settings_version`, above — a SEPARATE
    /// counter, because `/api/persona` keeps its own `version` independent of
    /// `/api/profile`'s (see `helloim/API-auth.md`'s own note that the two
    /// are summed only for the legacy route's `settings_version` field, never
    /// merged into one number). `#[serde(default)]` on the container means an
    /// account.json written before this field existed loads as `0`, which is
    /// the correct "never synced persona from here" starting point, not a
    /// wipe of anything — same reasoning `Profile`'s own `#[serde(default)]`
    /// note gives for the read path.
    persona_version: u64,
}

fn account_path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_config_dir()
        .map_err(|e| format!("no config directory: {e}"))?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("could not create {dir:?}: {e}"))?;
    Ok(dir.join("account.json"))
}

fn load_record(app: &AppHandle) -> AccountRecord {
    account_path(app)
        .ok()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

fn save_record(app: &AppHandle, record: &AccountRecord) -> Result<(), String> {
    let path = account_path(app)?;
    let json = serde_json::to_string_pretty(record).map_err(|e| e.to_string())?;
    std::fs::write(&path, json).map_err(|e| format!("could not write {path:?}: {e}"))
}

/// Withdraws the local, non-secret half of a sign-in. Deleting rather than
/// zeroing, the same choice `profile::write_identity` makes for a cleared
/// name — a stale empty file left behind is a stale file somebody later has
/// to explain.
fn clear_record(app: &AppHandle) -> Result<(), String> {
    let path = account_path(app)?;
    if path.exists() {
        std::fs::remove_file(&path).map_err(|e| format!("could not clear {path:?}: {e}"))?;
    }
    Ok(())
}

fn bump_version(app: &AppHandle, version: u64) {
    let mut record = load_record(app);
    record.settings_version = version;
    let _ = save_record(app, &record);
}

/// The `/api/persona` twin of `bump_version` — see `AccountRecord::persona_version`'s
/// own doc for why this is a separate counter rather than sharing one.
fn bump_persona_version(app: &AppHandle, version: u64) {
    let mut record = load_record(app);
    record.persona_version = version;
    let _ = save_record(app, &record);
}

fn non_empty(s: String) -> Option<String> {
    let s = s.trim();
    if s.is_empty() { None } else { Some(s.to_string()) }
}

// ---------------------------------------------------------------------------
// The keyring. One entry, this service only.
// ---------------------------------------------------------------------------

fn token_entry() -> Result<keyring::Entry, String> {
    keyring::Entry::new(KEYRING_SERVICE, TOKEN_ACCOUNT).map_err(|e| format!("credential store: {e}"))
}

fn stored_token() -> Option<String> {
    token_entry().ok().and_then(|e| e.get_password().ok())
}

/// The token, or a refusal worded for the window to show directly.
///
/// Also cleans up: if the keyring has no token but `account.json` still
/// claims an email, the record is stale (cleared from outside the app, or a
/// run that crashed between the two writes) and is removed here rather than
/// left to answer `account_status` with a phantom sign-in later.
///
/// `pub(crate)` so `memory_sync.rs` shares this exact check rather than a
/// second copy that could disagree with it about what "signed in" means.
pub(crate) fn require_token(app: &AppHandle) -> Result<String, String> {
    match stored_token() {
        Some(t) => Ok(t),
        None => {
            let _ = clear_record(app);
            Err("Not signed in to helloim.ai. Sign in first, then this will sync.".to_string())
        }
    }
}

// ---------------------------------------------------------------------------
// Error wording. Both take the raw `ureq::Error` and turn it into a sentence
// this codebase's own rule says to show as-is: specific enough to act on,
// never a stack trace.
// ---------------------------------------------------------------------------

/// A failed sign-in attempt. Kept separate from `sync_failure` because a bad
/// password must never clear anything — there is no session yet to clear.
fn login_failure(e: ureq::Error) -> String {
    match e {
        ureq::Error::Status(401, _) => "That email or password was not recognized.".to_string(),
        ureq::Error::Status(429, _) => "Too many attempts. Wait a moment and try again.".to_string(),
        ureq::Error::Status(503, _) => "helloim.ai is temporarily unavailable. Try again shortly.".to_string(),
        ureq::Error::Status(code, _) => format!("helloim.ai rejected the sign-in ({code})."),
        ureq::Error::Transport(t) => {
            format!("Could not reach helloim.ai — check this computer's connection. ({t})")
        }
    }
}

/// The wording for a request that failed for a reason OTHER than a 401 —
/// split out of `sync_failure` (below) so `verify_blocking` can reuse the
/// exact same sentences for its own non-401 branch without also picking up
/// `sync_failure`'s side effect of clearing the stored session. Where the
/// launch gate handles a real 401 lives in `verify_blocking` itself, because
/// that call site needs to tell the caller "signed_out: true" in its Ok
/// return rather than only in an error string — see `VerifyResult`'s doc.
fn sync_error_message(e: ureq::Error) -> String {
    match e {
        ureq::Error::Status(429, _) => "helloim.ai is limiting requests. Wait a moment and try again.".to_string(),
        ureq::Error::Status(503, _) => "helloim.ai is temporarily unavailable. Try again shortly.".to_string(),
        ureq::Error::Status(code, _) => format!("helloim.ai rejected the request ({code})."),
        ureq::Error::Transport(t) => {
            format!("Could not reach helloim.ai — check this computer's connection. ({t})")
        }
    }
}

/// A failed sync call (push or pull). Handles 401 specially: there is no
/// refresh token in this design (one bearer token, re-entered by signing in
/// again — see `WEBAPP-SYNC-SPEC.md`), so a session helloim.ai no longer
/// honours can never succeed again on its own. Holding onto it would make
/// every future sync attempt fail the same way for a reason nothing in the
/// window explains, so this clears local sign-in state the moment the
/// server says the token is no good, and says so plainly rather than
/// leaving the app to look signed in while every sync silently fails.
///
/// `pub(crate)` so `memory_sync.rs`'s own calls to `helloim.ai` clear the
/// same session state on a 401 that every other sync path here already
/// does — a dead token is dead regardless of which route noticed first.
pub(crate) fn sync_failure(app: &AppHandle, e: ureq::Error) -> String {
    if let ureq::Error::Status(401, _) = &e {
        if let Ok(entry) = token_entry() {
            let _ = entry.delete_credential(); // best-effort; we are already reporting failure
        }
        let _ = clear_record(app);
        return "Your helloim.ai sign-in has expired. Sign in again to keep syncing.".to_string();
    }
    sync_error_message(e)
}

/// A failed signup, or a failed password-reset request. Both endpoints share
/// one status-code table (`helloim/worker.js`'s `handleSignup` and
/// `handleResetRequest` return the same codes for the same reasons: a bad
/// address, too many attempts, the mailer down) and neither can ever 401 —
/// there is no session yet on either call, so `sync_failure`'s 401 branch
/// does not apply here.
fn mail_request_failure(e: ureq::Error) -> String {
    match e {
        ureq::Error::Status(400, _) => "Enter a valid email address.".to_string(),
        ureq::Error::Status(429, _) => "Too many attempts. Wait a moment and try again.".to_string(),
        ureq::Error::Status(503, _) => "helloim.ai is temporarily unavailable. Try again shortly.".to_string(),
        ureq::Error::Status(code, _) => format!("helloim.ai rejected that ({code})."),
        ureq::Error::Transport(t) => {
            format!("Could not reach helloim.ai — check this computer's connection. ({t})")
        }
    }
}

// ---------------------------------------------------------------------------
// Sign in / sign out / status.
// ---------------------------------------------------------------------------

fn sign_in_blocking(app: AppHandle, email: String, password: String) -> Result<AccountStatus, String> {
    let email = email.trim().to_string();
    if email.is_empty() || password.is_empty() {
        // Local, so the person hears it instantly rather than after a round
        // trip — same reasoning as `feedback.rs`'s empty-message check.
        return Err("Enter your email and password.".to_string());
    }
    let payload = serde_json::json!({ "email": email, "password": password }).to_string();
    let resp = agent()
        .post(&format!("{API_BASE}/auth/login"))
        .set("Content-Type", "application/json")
        .send_string(&payload)
        .map_err(login_failure)?;
    let body = resp
        .into_string()
        .map_err(|e| format!("helloim.ai sent back something unreadable: {e}"))?;
    let parsed: LoginResponse = serde_json::from_str(&body)
        .map_err(|e| format!("helloim.ai sent back something this app could not parse: {e}"))?;

    token_entry()?
        .set_password(&parsed.token)
        .map_err(|e| format!("Signed in, but the session could not be stored on this machine: {e}"))?;

    let record = AccountRecord {
        email: parsed.email.clone(),
        display_name: parsed.display_name.clone().unwrap_or_default(),
        settings_version: 0,
        persona_version: 0,
    };
    save_record(&app, &record)?;

    Ok(AccountStatus {
        signed_in: true,
        email: Some(record.email),
        display_name: non_empty(record.display_name),
    })
}

#[tauri::command]
pub async fn account_sign_in(app: AppHandle, email: String, password: String) -> Result<AccountStatus, String> {
    tauri::async_runtime::spawn_blocking(move || sign_in_blocking(app, email, password))
        .await
        .map_err(|_| "The app could not start that request. Try again.".to_string())?
}

fn sign_out_blocking(app: AppHandle) -> Result<(), String> {
    // Best-effort server-side logout, using the token that is about to be
    // deleted. Its outcome is discarded on purpose: `handleLogout` is
    // idempotent by design (see `helloim/worker.js`), and signing out must
    // keep working with no network at all — a person offline is exactly the
    // person most likely to need this to just work.
    if let Some(token) = stored_token() {
        let _ = agent()
            .post(&format!("{API_BASE}/auth/logout"))
            .set("Authorization", &format!("Bearer {token}"))
            .call();
    }
    match token_entry()?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => {}
        Err(e) => return Err(format!("could not clear the stored session: {e}")),
    }
    clear_record(&app)
}

#[tauri::command]
pub async fn account_sign_out(app: AppHandle) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || sign_out_blocking(app))
        .await
        .map_err(|_| "The app could not complete that request. Try again.".to_string())?
}

#[tauri::command]
pub fn account_status(app: AppHandle) -> AccountStatus {
    if stored_token().is_none() {
        // The keyring, never the disk, is the truth for whether a session
        // exists — same discipline as `providers::refresh` deriving
        // `has_secret` live rather than trusting what was last written. A
        // credential cleared from outside the app (Credential Manager,
        // Keychain Access) must not leave a phantom "signed in as ..."
        // behind.
        let _ = clear_record(&app);
        return AccountStatus::default();
    }
    let record = load_record(&app);
    AccountStatus {
        signed_in: true,
        email: non_empty(record.email),
        display_name: non_empty(record.display_name),
    }
}

// ---------------------------------------------------------------------------
// The launch gate. Added 2026-09-23 for the account migration off nameos.ai
// — see `docs/plans/app-account-migration-to-helloim-2026-09-23.md` (4) and
// `helloim/API-auth.md`. `account_status` above answers instantly from the
// keyring/disk and is right for "is there a session to even try"; these
// three ask helloim.ai a real question and are the ones the gate calls to
// find out whether that session still holds, and to create a new one
// email-first.
// ---------------------------------------------------------------------------

/// What `account_verify` tells the window. Two axes that must never be
/// collapsed into one (the exact bug `verifySession()` in `ui/index.html`
/// was written to avoid for the legacy nameos.ai path, and this is the same
/// rule reached from Rust instead of `fetch`):
///
/// - `Err(String)` from the command itself — the network failed, or
///   helloim.ai answered something other than 200/401 (5xx, a proxy page,
///   rate limiting). **This is not evidence the session is bad.** The stored
///   token is left exactly as it was; the window's job is to decide whether
///   this device has confirmed a helloim.ai session before and, if so, keep
///   going offline (see `CONFIRMED_KEY` in the module doc up top).
/// - `Ok(VerifyResult { signed_out: true, .. })` — a REAL 401. The only
///   answer that means the session is actually dead. The keyring and
///   `account.json` are already cleared by the time this returns, same as
///   `sync_failure`'s 401 branch does for push/pull.
/// - `Ok(VerifyResult { signed_out: false, .. })` — confirmed. `email` and
///   `display_name` are always `Some` on this branch; `license` is always
///   populated (server default `status: "none"` when nobody has ever set
///   one — see `LicenseInfo`'s own doc on why `none` means the FREE plan
///   here, not entitled).
#[derive(Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct VerifyResult {
    pub signed_out: bool,
    pub email: Option<String>,
    pub display_name: Option<String>,
    pub license: LicenseInfo,
}

fn verify_blocking(app: AppHandle) -> Result<VerifyResult, String> {
    let token = require_token(&app)?;
    let resp = agent()
        .get(&format!("{API_BASE}/auth/me"))
        .set("Authorization", &format!("Bearer {token}"))
        .call();
    match resp {
        Ok(r) => {
            let body = r
                .into_string()
                .map_err(|e| format!("helloim.ai sent back something unreadable: {e}"))?;
            let parsed: MeResponse = serde_json::from_str(&body)
                .map_err(|e| format!("helloim.ai sent back something this app could not parse: {e}"))?;
            // Keep the local, non-secret record in step with what the server
            // just confirmed -- the same fields `sign_in_blocking` writes on
            // a fresh login, refreshed here so a display name changed on
            // another device shows up after the next launch check too.
            let mut record = load_record(&app);
            record.email = parsed.email.clone();
            record.display_name = parsed.display_name.clone().unwrap_or_default();
            let _ = save_record(&app, &record);
            Ok(VerifyResult {
                signed_out: false,
                email: Some(parsed.email),
                display_name: non_empty(parsed.display_name.unwrap_or_default()),
                license: parsed.license,
            })
        }
        // THE ONE OUTCOME THAT IS REAL EVIDENCE. Cleared here, inline, rather
        // than by delegating to `sync_failure` -- that function reports the
        // 401 as an `Err(String)`, which is right for push/pull (there is
        // nothing else useful to return), but the gate needs to tell "the
        // session is dead" apart from "could not ask" in its OK value, not
        // only in an error string, so the two can never be confused by a
        // caller that only checks `is_err()`.
        Err(ureq::Error::Status(401, _)) => {
            if let Ok(entry) = token_entry() {
                let _ = entry.delete_credential(); // best-effort; we already have the answer
            }
            let _ = clear_record(&app);
            Ok(VerifyResult { signed_out: true, ..Default::default() })
        }
        Err(e) => Err(sync_error_message(e)),
    }
}

/// Ask helloim.ai whether the stored session still holds. See `VerifyResult`
/// for what each branch means and why "could not ask" and "signed out" are
/// never the same value.
#[tauri::command]
pub async fn account_verify(app: AppHandle) -> Result<VerifyResult, String> {
    tauri::async_runtime::spawn_blocking(move || verify_blocking(app))
        .await
        .map_err(|_| "The app could not start that request. Try again.".to_string())?
}

fn license_blocking(app: AppHandle) -> Result<LicenseInfo, String> {
    let token = require_token(&app)?;
    let resp = agent()
        .get(&format!("{API_BASE}/license"))
        .set("Authorization", &format!("Bearer {token}"))
        .call()
        .map_err(|e| sync_failure(&app, e))?;
    let body = resp
        .into_string()
        .map_err(|e| format!("helloim.ai sent back something unreadable: {e}"))?;
    let parsed: LicenseResponse = serde_json::from_str(&body)
        .map_err(|e| format!("helloim.ai sent back something this app could not parse: {e}"))?;
    Ok(parsed.license)
}

/// The plan badge's own read, kept separate from `account_verify` so the
/// badge can refresh on its own schedule without re-running the fuller
/// launch check (same division `iaLoadPlan()` already makes for nameos.ai in
/// `ui/index.html`, now mirrored here for helloim.ai). Same 401 handling as
/// push/pull: `sync_failure` clears the session, because a plan badge that
/// cannot read a licence for a dead session has nothing else to show anyway.
#[tauri::command]
pub async fn account_license(app: AppHandle) -> Result<LicenseInfo, String> {
    tauri::async_runtime::spawn_blocking(move || license_blocking(app))
        .await
        .map_err(|_| "The app could not start that request. Try again.".to_string())?
}

/// Email-first signup — `POST /api/auth/signup`. Creates no account and
/// returns no token; per `API-auth.md` the server's answer is identical
/// whether or not the address already has one, specifically so this call can
/// never be used to test who has an account. The window's job on `Ok(())` is
/// only ever "show Check your email" — never a token, never a reveal.
fn signup_blocking(email: String) -> Result<(), String> {
    let email = email.trim().to_string();
    if email.is_empty() {
        return Err("Enter your email.".to_string());
    }
    let payload = serde_json::json!({ "email": email }).to_string();
    agent()
        .post(&format!("{API_BASE}/auth/signup"))
        .set("Content-Type", "application/json")
        .send_string(&payload)
        .map_err(mail_request_failure)?;
    Ok(())
}

#[tauri::command]
pub async fn account_signup(email: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || signup_blocking(email))
        .await
        .map_err(|_| "The app could not start that request. Try again.".to_string())?
}

/// `POST /api/auth/reset/request` — same "identical answer either way" shape
/// as signup, and the same reason: this call must not be usable to test
/// which addresses have accounts. The page that actually sets a new password
/// is on helloim.ai, opened from the emailed link, not in this app.
fn reset_request_blocking(email: String) -> Result<(), String> {
    let email = email.trim().to_string();
    if email.is_empty() {
        return Err("Enter your email.".to_string());
    }
    let payload = serde_json::json!({ "email": email }).to_string();
    agent()
        .post(&format!("{API_BASE}/auth/reset/request"))
        .set("Content-Type", "application/json")
        .send_string(&payload)
        .map_err(mail_request_failure)?;
    Ok(())
}

#[tauri::command]
pub async fn account_reset_request(email: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || reset_request_blocking(email))
        .await
        .map_err(|_| "The app could not start that request. Try again.".to_string())?
}

// ---------------------------------------------------------------------------
// Pull.
// ---------------------------------------------------------------------------

fn fetch_profile(app: &AppHandle, token: &str) -> Result<(RemoteProfile, u64), String> {
    let resp = agent()
        .get(&format!("{API_BASE}/profile"))
        .set("Authorization", &format!("Bearer {token}"))
        .call()
        .map_err(|e| sync_failure(app, e))?;
    let body = resp
        .into_string()
        .map_err(|e| format!("helloim.ai sent back something unreadable: {e}"))?;
    let parsed: ProfileResponse = serde_json::from_str(&body)
        .map_err(|e| format!("helloim.ai sent back something this app could not parse: {e}"))?;
    Ok((parsed.profile, parsed.settings_version))
}

/// Writes the half of `remote` that `profile.rs` already knows how to merge
/// safely, through the exact function `save_profile` itself uses. Returns
/// the CLAUDE.md path it wrote — see `PullResult`'s own doc on why that may
/// be empty.
fn apply_remote_profile(app: &AppHandle, workdir: &str, remote: &RemoteProfile) -> Result<String, String> {
    crate::profile::merge_and_write(app, remote.to_profile_patch(), workdir)
}

fn pull_blocking(app: AppHandle, workdir: String) -> Result<PullResult, String> {
    let token = require_token(&app)?;
    let (remote, version) = fetch_profile(&app, &token)?;
    let claude_md_path = apply_remote_profile(&app, &workdir, &remote)?;
    bump_version(&app, version);
    Ok(PullResult {
        settings_version: version,
        claude_md_path,
        voice_id: remote.voice_id,
        default_skin_id: remote.default_skin_id,
        provider_id: remote.provider_id,
        provider_base_url: remote.provider_base_url,
        provider_model: remote.provider_model,
        allow_agency: remote.allow_agency.map(|v| v != 0),
        allow_shell: remote.allow_shell.map(|v| v != 0),
    })
}

#[tauri::command]
pub async fn pull_settings(app: AppHandle, workdir: String) -> Result<PullResult, String> {
    tauri::async_runtime::spawn_blocking(move || pull_blocking(app, workdir))
        .await
        .map_err(|_| "The app could not start that request. Try again.".to_string())?
}

// ---------------------------------------------------------------------------
// Push.
// ---------------------------------------------------------------------------

/// A failed push. Split from a plain `String` only so the 409 case can carry
/// what the server says is current, for the one retry `push_blocking` does.
enum PushErr {
    Stale(RemoteProfile, u64),
    Other(String),
}

fn push_once(
    app: &AppHandle,
    token: &str,
    patch: &SettingsPatch,
    base_version: u64,
) -> Result<(RemoteProfile, u64), PushErr> {
    let mut body = serde_json::json!({ "fields": patch.to_wire() });
    body["base_version"] = serde_json::json!(base_version);
    let payload = body.to_string();

    let res = agent()
        .post(&format!("{API_BASE}/profile"))
        .set("Authorization", &format!("Bearer {token}"))
        .set("Content-Type", "application/json")
        .send_string(&payload);

    match res {
        Ok(resp) => {
            let text = resp
                .into_string()
                .map_err(|e| PushErr::Other(format!("helloim.ai sent back something unreadable: {e}")))?;
            let parsed: ProfileResponse = serde_json::from_str(&text)
                .map_err(|e| PushErr::Other(format!("helloim.ai sent back something this app could not parse: {e}")))?;
            Ok((parsed.profile, parsed.settings_version))
        }
        Err(ureq::Error::Status(409, resp)) => {
            let text = resp
                .into_string()
                .map_err(|e| PushErr::Other(format!("a conflict response could not be read: {e}")))?;
            let parsed: ProfileResponse = serde_json::from_str(&text)
                .map_err(|e| PushErr::Other(format!("a conflict response could not be parsed: {e}")))?;
            Err(PushErr::Stale(parsed.profile, parsed.settings_version))
        }
        Err(e) => Err(PushErr::Other(sync_failure(app, e))),
    }
}

fn push_blocking(app: AppHandle, workdir: String, patch: SettingsPatch) -> Result<PushResult, String> {
    if patch.is_empty() {
        return Err("Nothing changed to sync.".to_string());
    }
    let token = require_token(&app)?;
    let base_version = load_record(&app).settings_version;

    match push_once(&app, &token, &patch, base_version) {
        Ok((_, version)) => {
            bump_version(&app, version);
            Ok(PushResult { settings_version: version, resolved_conflict: false })
        }
        Err(PushErr::Stale(remote, version)) => {
            // Somebody else changed the record first. Merge their version of
            // the fields this app already knows how to merge safely — the
            // same path a pull uses — then retry THIS push once against the
            // fresh version. Not a loop: a second conflict is reported
            // rather than retried again, the same "a loop that cannot close
            // should stop, not spin" reasoning any unattended retry here
            // has to follow.
            apply_remote_profile(&app, &workdir, &remote)?;
            bump_version(&app, version);
            match push_once(&app, &token, &patch, version) {
                Ok((_, v2)) => {
                    bump_version(&app, v2);
                    Ok(PushResult { settings_version: v2, resolved_conflict: true })
                }
                Err(PushErr::Stale(_, v3)) => Err(format!(
                    "Another device changed these settings while this was syncing. \
                     The latest was merged in, but a second change landed before this \
                     one could (now at version {v3}). Try again in a moment."
                )),
                Err(PushErr::Other(msg)) => Err(msg),
            }
        }
        Err(PushErr::Other(msg)) => Err(msg),
    }
}

#[tauri::command]
pub async fn push_settings(app: AppHandle, workdir: String, patch: SettingsPatch) -> Result<PushResult, String> {
    tauri::async_runtime::spawn_blocking(move || push_blocking(app, workdir, patch))
        .await
        .map_err(|_| "The app could not start that request. Try again.".to_string())?
}

// ---------------------------------------------------------------------------
// Shared persona sync — `GET`/`PUT /api/persona`, added 2026-09-23 so the
// desktop app and the ai.markdalton.com dashboard genuinely share one
// persona, per `helloim/API-auth.md`'s "Shared persona and memory" section.
//
// **A DIFFERENT ROUTE FROM `/api/profile`, DELIBERATELY, EVEN THOUGH THAT
// ROUTE NOW ALSO PROXIES THE SAME SEVEN FIELDS TO THE SAME SHARED STORE.**
// `/api/profile` exists for the provider/permission settings
// (`push_settings`/`pull_settings` above) and folds the persona fields in
// only for backward compatibility with the shape `SettingsPatch` already
// posts. `/api/persona` is the fields-only route: no provider/permission
// noise in the wire shape, its own independent `version` counter (see
// `AccountRecord::persona_version`), and the one place `field_ts`-per-field
// last-writer-wins is actually exposed to this app rather than handled
// silently inside `/api/profile`'s merge. Calling it directly is what lets
// "pull on sign-in and on focus, push on save" happen without dragging a
// provider-settings round trip along for a persona-only sync.
//
// **THE HARDENING FROM THE PROFILE MERGE PATH APPLIES HERE TOO, BECAUSE IT
// LIVES ONE LEVEL DOWN.** A pulled persona is applied through the exact same
// `profile::merge_and_write` / `ProfilePatch` `apply_remote_profile` already
// uses — fenced markers, the newline collapse on the three persona fields,
// and the folder-trust self-approval fix (`c46d6a17`) all run unconditionally
// inside that one function, regardless of which route the patch arrived
// through. This module never re-derives any of that.
// ---------------------------------------------------------------------------

const PERSONA_API: &str = "https://helloim.ai/api/persona";

/// The wire shape `GET`/`PUT /api/persona` reads and writes — snake_case,
/// matching `PROFILE_STR_FIELDS` in `helloim/worker.js` literally, the same
/// convention `RemoteProfile` above uses for the same reason. `null`/absent
/// both land as `None`, which is exactly the meaning `ProfilePatch::apply`
/// depends on: "no device has ever synced this field," never "clear it."
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
struct PersonaFields {
    assistant_name: Option<String>,
    personality: Option<String>,
    wake_word: Option<String>,
    /// The selected TTS voice id — NOT the derived "how they write" card
    /// (`Profile::voice`). Reported to the caller like `RemoteProfile`'s own
    /// `voice_id` and applied nowhere by this module; see this file's module
    /// doc on why provider/voice selection is a window decision.
    voice_id: Option<String>,
    about: Option<String>,
    goal: Option<String>,
    memory: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
struct PersonaResponse {
    persona: PersonaFields,
    version: u64,
}

impl PersonaFields {
    /// The half of a shared-persona read this app already knows how to
    /// merge without wiping anything — same split `RemoteProfile::to_profile_patch`
    /// makes, and for the identical reason: `voice` (the writing-style card)
    /// is never carried over this route either, so it is always `None` here.
    fn to_profile_patch(&self) -> crate::profile::ProfilePatch {
        crate::profile::ProfilePatch {
            about: self.about.clone(),
            goal: self.goal.clone(),
            memory: self.memory.clone(),
            assistant_name: self.assistant_name.clone(),
            personality: self.personality.clone(),
            wake_word: self.wake_word.clone(),
            voice: None,
        }
    }
}

/// What the window sends on a persona push. Same `Option`-per-field shape as
/// `SettingsPatch`/`ProfilePatch`, and `deny_unknown_fields` for the same
/// "a typo must be loud, not a silent no-op" reason.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonaPatch {
    pub assistant_name: Option<String>,
    pub personality: Option<String>,
    pub wake_word: Option<String>,
    pub voice_id: Option<String>,
    pub about: Option<String>,
    pub goal: Option<String>,
    pub memory: Option<String>,
}

impl PersonaPatch {
    fn is_empty(&self) -> bool {
        self.assistant_name.is_none()
            && self.personality.is_none()
            && self.wake_word.is_none()
            && self.voice_id.is_none()
            && self.about.is_none()
            && self.goal.is_none()
            && self.memory.is_none()
    }

    /// Built by hand, not via `serde_json::to_value`, for the same
    /// camelCase-in/snake_case-out reason `SettingsPatch::to_wire` is.
    fn to_wire(&self) -> serde_json::Value {
        let mut m = serde_json::Map::new();
        if let Some(v) = &self.assistant_name { m.insert("assistant_name".into(), v.clone().into()); }
        if let Some(v) = &self.personality { m.insert("personality".into(), v.clone().into()); }
        if let Some(v) = &self.wake_word { m.insert("wake_word".into(), v.clone().into()); }
        if let Some(v) = &self.voice_id { m.insert("voice_id".into(), v.clone().into()); }
        if let Some(v) = &self.about { m.insert("about".into(), v.clone().into()); }
        if let Some(v) = &self.goal { m.insert("goal".into(), v.clone().into()); }
        if let Some(v) = &self.memory { m.insert("memory".into(), v.clone().into()); }
        serde_json::Value::Object(m)
    }
}

#[derive(Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PersonaPullResult {
    pub version: u64,
    /// Same honesty convention as `PullResult::claude_md_path` — empty when
    /// the working folder cannot be written into yet.
    pub claude_md_path: String,
    pub voice_id: Option<String>,
}

#[derive(Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PersonaPushResult {
    pub version: u64,
    pub resolved_conflict: bool,
}

fn fetch_persona(app: &AppHandle, token: &str) -> Result<(PersonaFields, u64), String> {
    let resp = agent()
        .get(PERSONA_API)
        .set("Authorization", &format!("Bearer {token}"))
        .call()
        .map_err(|e| sync_failure(app, e))?;
    let body = resp
        .into_string()
        .map_err(|e| format!("helloim.ai sent back something unreadable: {e}"))?;
    let parsed: PersonaResponse = serde_json::from_str(&body)
        .map_err(|e| format!("helloim.ai sent back something this app could not parse: {e}"))?;
    Ok((parsed.persona, parsed.version))
}

fn pull_persona_blocking(app: AppHandle, workdir: String) -> Result<PersonaPullResult, String> {
    let token = require_token(&app)?;
    let (persona, version) = fetch_persona(&app, &token)?;
    let claude_md_path = crate::profile::merge_and_write(&app, persona.to_profile_patch(), &workdir)?;
    bump_persona_version(&app, version);
    Ok(PersonaPullResult { version, claude_md_path, voice_id: persona.voice_id })
}

/// Pull the shared persona and merge it in — through `merge_and_write`, so
/// the hardening in `c46d6a17` (fenced markers, newline collapse, the
/// folder-trust self-approval fix) applies exactly as it does to a typed
/// Save. Called from the window on sign-in and on window focus.
#[tauri::command]
pub async fn pull_persona(app: AppHandle, workdir: String) -> Result<PersonaPullResult, String> {
    tauri::async_runtime::spawn_blocking(move || pull_persona_blocking(app, workdir))
        .await
        .map_err(|_| "The app could not start that request. Try again.".to_string())?
}

enum PersonaPushErr {
    Stale(PersonaFields, u64),
    Other(String),
}

fn push_persona_once(
    app: &AppHandle,
    token: &str,
    patch: &PersonaPatch,
    base_version: u64,
) -> Result<(PersonaFields, u64), PersonaPushErr> {
    let mut body = serde_json::json!({ "fields": patch.to_wire() });
    body["base_version"] = serde_json::json!(base_version);
    let payload = body.to_string();

    let res = agent()
        .post(PERSONA_API)
        .set("Authorization", &format!("Bearer {token}"))
        .set("Content-Type", "application/json")
        .send_string(&payload);

    match res {
        Ok(resp) => {
            let text = resp
                .into_string()
                .map_err(|e| PersonaPushErr::Other(format!("helloim.ai sent back something unreadable: {e}")))?;
            let parsed: PersonaResponse = serde_json::from_str(&text)
                .map_err(|e| PersonaPushErr::Other(format!("helloim.ai sent back something this app could not parse: {e}")))?;
            Ok((parsed.persona, parsed.version))
        }
        Err(ureq::Error::Status(409, resp)) => {
            let text = resp
                .into_string()
                .map_err(|e| PersonaPushErr::Other(format!("a conflict response could not be read: {e}")))?;
            let parsed: PersonaResponse = serde_json::from_str(&text)
                .map_err(|e| PersonaPushErr::Other(format!("a conflict response could not be parsed: {e}")))?;
            Err(PersonaPushErr::Stale(parsed.persona, parsed.version))
        }
        Err(e) => Err(PersonaPushErr::Other(sync_failure(app, e))),
    }
}

fn push_persona_blocking(app: AppHandle, workdir: String, patch: PersonaPatch) -> Result<PersonaPushResult, String> {
    if patch.is_empty() {
        return Err("Nothing changed to sync.".to_string());
    }
    let token = require_token(&app)?;
    let base_version = load_record(&app).persona_version;

    match push_persona_once(&app, &token, &patch, base_version) {
        Ok((_, version)) => {
            bump_persona_version(&app, version);
            Ok(PersonaPushResult { version, resolved_conflict: false })
        }
        Err(PersonaPushErr::Stale(remote, version)) => {
            // Same "merge, bump, retry once" shape as `push_blocking` above —
            // not a loop: a second conflict is reported rather than retried
            // again.
            crate::profile::merge_and_write(&app, remote.to_profile_patch(), &workdir)?;
            bump_persona_version(&app, version);
            match push_persona_once(&app, &token, &patch, version) {
                Ok((_, v2)) => {
                    bump_persona_version(&app, v2);
                    Ok(PersonaPushResult { version: v2, resolved_conflict: true })
                }
                Err(PersonaPushErr::Stale(_, v3)) => Err(format!(
                    "Another device changed the persona while this was syncing. \
                     The latest was merged in, but a second change landed before this \
                     one could (now at version {v3}). Try again in a moment."
                )),
                Err(PersonaPushErr::Other(msg)) => Err(msg),
            }
        }
        Err(PersonaPushErr::Other(msg)) => Err(msg),
    }
}

/// Push a persona change — called from the window right after a local Save
/// succeeds. Best-effort from the window's side (the local save is already
/// the source of truth; a failed push here is recoverable on the next pull
/// or save), but the push call itself follows the same conflict handling
/// `push_settings` uses.
#[tauri::command]
pub async fn push_persona(app: AppHandle, workdir: String, patch: PersonaPatch) -> Result<PersonaPushResult, String> {
    tauri::async_runtime::spawn_blocking(move || push_persona_blocking(app, workdir, patch))
        .await
        .map_err(|_| "The app could not start that request. Try again.".to_string())?
}

// ---------------------------------------------------------------------------
// Tests. Everything network-shaped above is a thin, already-documented
// wrapper around calls this codebase already trusts (`ureq`, the exact
// status-code table `feedback.rs`/`google_email.rs`/`ms_graph.rs` use); what
// is genuinely this file's own logic — the wire shape, and the null-vs-value
// distinction the anti-wipe merge depends on — is what these pin.
// ---------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_keyring_service_is_not_the_nameos_one() {
        assert_ne!(
            KEYRING_SERVICE,
            crate::connectors::KEYRING_SERVICE,
            "a helloim.ai session must never share a keyring namespace with NameOS secrets"
        );
        assert_eq!(KEYRING_SERVICE, "helloim.ai");
    }

    #[test]
    fn to_wire_sends_only_fields_that_were_set() {
        let patch = SettingsPatch {
            assistant_name: Some("Bella".into()),
            allow_shell: Some(true),
            ..Default::default()
        };
        let wire = patch.to_wire();
        let obj = wire.as_object().expect("to_wire must produce a JSON object");
        assert_eq!(obj.len(), 2, "an unset field leaked onto the wire: {wire}");
        assert_eq!(obj["assistant_name"], "Bella");
        assert_eq!(obj["allow_shell"], true);
        assert!(!obj.contains_key("about"), "an unset field must never appear, not even as null");
    }

    #[test]
    fn to_wire_of_an_empty_patch_is_empty() {
        assert!(SettingsPatch::default().to_wire().as_object().unwrap().is_empty());
    }

    #[test]
    fn a_key_this_struct_does_not_know_is_refused() {
        // Same discipline as `profile::ProfilePatch` -- the window and this
        // struct ship in the same binary, so an unrecognised key is a typo,
        // not a future client, and must not be a silent no-op.
        //
        // THE OTHER TWO KEYS ARE camelCase, DELIBERATELY -- found failing on
        // a real `cargo test` run, 2026-09-23. This struct is
        // `#[serde(rename_all = "camelCase")]`; the payload originally read
        // `"assistant_name"`/`"provider_id"` (snake_case), which are THEMSELVES
        // unrecognised under that rename and got reported first (serde stops
        // at the first bad key it meets, in payload order) -- so the test was
        // never actually proving what its own name says, only that some key or
        // other gets refused. `assistantName`/`providerId` here are the two
        // real fields, correctly spelled, so `bogus` is the only thing left
        // for the refusal to be about.
        let e = serde_json::from_str::<SettingsPatch>(
            r#"{"assistantName": "x", "providerId": "y", "bogus": 1}"#,
        )
        .expect_err("an unknown key was accepted");
        assert!(e.to_string().contains("bogus"), "{e}");
    }

    #[test]
    fn a_field_the_server_has_never_seen_does_not_wipe_the_local_record() {
        // The server encodes "no device has ever synced this field" as JSON
        // null. `RemoteProfile` must turn that into `None`, not `Some("")` --
        // the exact distinction `ProfilePatch::apply` depends on to leave a
        // field alone instead of blanking it.
        let remote: RemoteProfile = serde_json::from_str(
            r#"{"about": "still here", "goal": null, "memory": null,
                "assistant_name": null, "personality": null, "wake_word": null,
                "provider_id": null, "provider_base_url": null, "provider_model": null,
                "default_skin_id": null, "voice_id": null,
                "allow_agency": null, "allow_shell": null}"#,
        )
        .expect("a real server response must parse");
        let patch = remote.to_profile_patch();
        assert_eq!(patch.about.as_deref(), Some("still here"));
        assert!(patch.goal.is_none(), "a null field arrived as a value that would wipe the local one");
        assert!(patch.memory.is_none());
        assert!(patch.voice.is_none(), "the voice card must never be pulled from the server");
    }

    #[test]
    fn a_field_missing_from_the_response_entirely_behaves_like_a_null_one() {
        // Belt and braces on the same guarantee as the test above, for a
        // server that omits a key rather than sending it as null -- serde's
        // ordinary `Option<T>` handling, not a `#[serde(default)]` this
        // struct deliberately does not carry.
        let remote: RemoteProfile = serde_json::from_str(r#"{"about": "kept"}"#).unwrap();
        assert_eq!(remote.about.as_deref(), Some("kept"));
        assert!(remote.assistant_name.is_none());
        assert!(remote.allow_shell.is_none());
    }

    #[test]
    fn allow_flags_convert_from_the_servers_0_1_shape() {
        let remote: RemoteProfile = serde_json::from_str(r#"{"allow_agency": 1, "allow_shell": 0}"#).unwrap();
        assert_eq!(remote.allow_agency.map(|v| v != 0), Some(true));
        assert_eq!(remote.allow_shell.map(|v| v != 0), Some(false));
    }

    #[test]
    fn non_empty_treats_whitespace_as_absent() {
        assert_eq!(non_empty("   ".into()), None);
        assert_eq!(non_empty("Bella".into()), Some("Bella".into()));
    }

    #[test]
    fn an_empty_patch_is_recognised_as_empty() {
        assert!(SettingsPatch::default().is_empty());
        assert!(!SettingsPatch { about: Some(String::new()), ..Default::default() }.is_empty());
    }

    // -----------------------------------------------------------------------
    // Shared persona sync (`/api/persona`) — the same three properties
    // `SettingsPatch`/`RemoteProfile` are already pinned for, proven again
    // here because this is a genuinely separate wire path, not a rename of
    // the existing one.
    // -----------------------------------------------------------------------

    #[test]
    fn persona_patch_to_wire_sends_only_fields_that_were_set() {
        let patch = PersonaPatch {
            assistant_name: Some("Bella".into()),
            voice_id: Some("af_heart".into()),
            ..Default::default()
        };
        let wire = patch.to_wire();
        let obj = wire.as_object().expect("to_wire must produce a JSON object");
        assert_eq!(obj.len(), 2, "an unset field leaked onto the wire: {wire}");
        assert_eq!(obj["assistant_name"], "Bella");
        assert_eq!(obj["voice_id"], "af_heart");
        assert!(!obj.contains_key("about"), "an unset field must never appear, not even as null");
    }

    #[test]
    fn persona_patch_to_wire_of_an_empty_patch_is_empty() {
        assert!(PersonaPatch::default().to_wire().as_object().unwrap().is_empty());
    }

    #[test]
    fn an_empty_persona_patch_is_recognised_as_empty() {
        assert!(PersonaPatch::default().is_empty());
        assert!(!PersonaPatch { about: Some(String::new()), ..Default::default() }.is_empty());
    }

    #[test]
    fn a_persona_key_this_struct_does_not_know_is_refused() {
        let e = serde_json::from_str::<PersonaPatch>(r#"{"assistantName": "x", "bogus": 1}"#)
            .expect_err("an unknown key was accepted");
        assert!(e.to_string().contains("bogus"), "{e}");
    }

    /// **THE SAME ANTI-WIPE GUARANTEE `RemoteProfile` ALREADY HAS, PROVEN FOR
    /// THIS SEPARATE STRUCT.** `GET /api/persona`'s own contract
    /// (`helloim/API-auth.md`) is "`field: string|null`" — a field no device
    /// has ever synced reads `null`, and that must become `None`, never
    /// `Some("")`, on the way into `ProfilePatch::apply`.
    #[test]
    fn a_persona_field_the_server_has_never_seen_does_not_wipe_the_local_record() {
        let remote: PersonaFields = serde_json::from_str(
            r#"{"about": "still here", "goal": null, "memory": null,
                "assistant_name": null, "personality": null, "wake_word": null,
                "voice_id": null}"#,
        )
        .expect("a real server response must parse");
        let patch = remote.to_profile_patch();
        assert_eq!(patch.about.as_deref(), Some("still here"));
        assert!(patch.goal.is_none(), "a null field arrived as a value that would wipe the local one");
        assert!(patch.memory.is_none());
        assert!(patch.voice.is_none(), "the voice card must never be pulled from the shared persona either");
    }

    #[test]
    fn a_persona_field_missing_from_the_response_entirely_behaves_like_a_null_one() {
        let remote: PersonaFields = serde_json::from_str(r#"{"about": "kept"}"#).unwrap();
        assert_eq!(remote.about.as_deref(), Some("kept"));
        assert!(remote.assistant_name.is_none());
        assert!(remote.voice_id.is_none());
    }

    /// The literal shape from `helloim/API-auth.md`'s own `GET /api/persona`
    /// section, so a server-side rename shows up here as a failing test.
    #[test]
    fn persona_response_parses_the_contracts_own_example_shape() {
        let body = r#"{
          "persona": { "assistant_name": "Bella", "personality": null,
                       "wake_word": null, "voice_id": "af_heart",
                       "about": "Runs a small firm.", "goal": null, "memory": null },
          "field_ts": { "assistant_name": "2026-09-23T00:00:00Z" },
          "version": 3
        }"#;
        let parsed: PersonaResponse = serde_json::from_str(body).expect("the contract's own shape must parse");
        assert_eq!(parsed.version, 3);
        assert_eq!(parsed.persona.assistant_name.as_deref(), Some("Bella"));
        assert_eq!(parsed.persona.voice_id.as_deref(), Some("af_heart"));
        assert!(parsed.persona.personality.is_none());
    }

    // -----------------------------------------------------------------------
    // The launch gate — `MeResponse`/`LicenseInfo` wire shapes, pinned
    // against the literal examples in `helloim/API-auth.md` so a server-side
    // rename shows up here as a failing test rather than as a badge that
    // quietly stops updating. Added with the account migration, 2026-09-23.
    // -----------------------------------------------------------------------

    #[test]
    fn me_response_parses_the_contracts_own_example() {
        // Copied verbatim from `helloim/API-auth.md`'s `GET /api/auth/me`
        // section, minus the fields this struct deliberately does not read
        // (emailVerified/role/profile) -- `MeResponse` must still parse with
        // them present, since it has no `deny_unknown_fields`.
        let body = r#"{
          "email": "person@example.com",
          "emailVerified": true,
          "displayName": null,
          "role": "user",
          "profile": {},
          "license": { "status": "none", "plan": null, "seats": 0, "expiresAt": null,
                       "entitled": false, "keySuffix": null }
        }"#;
        let parsed: MeResponse = serde_json::from_str(body).expect("the contract's own example must parse");
        assert_eq!(parsed.email, "person@example.com");
        assert!(parsed.display_name.is_none());
        assert_eq!(parsed.license.status, "none");
        assert!(!parsed.license.entitled);
    }

    #[test]
    fn helloim_status_none_means_free_not_entitled() {
        // THE SEMANTIC THAT IS BACKWARDS FROM nameos.ai, ON PURPOSE — see
        // `LicenseInfo`'s own doc. nameos.ai's `none` means `entitled: true`
        // (fails open, because it only ever decided whether an update was
        // offered). helloim.ai's `none` means `entitled: false` (fails
        // closed, because it can gate a paid feature). If this test ever
        // starts failing because someone "fixed" the mismatch to look like
        // nameos.ai's, THE BRIEF IS WRONG, not this test — see
        // `helloim/API-auth.md`'s table and Beck's own test list item 8.
        let lic: LicenseInfo = serde_json::from_str(
            r#"{"status":"none","plan":null,"entitled":false,"expiresAt":null}"#,
        )
        .unwrap();
        assert_eq!(lic.status, "none");
        assert!(!lic.entitled, "helloim.ai's none must read as NOT entitled");
    }

    #[test]
    fn license_response_unwraps_to_just_the_license() {
        let parsed: LicenseResponse = serde_json::from_str(
            r#"{"email":"a@b.com","license":{"status":"active","plan":"pro","entitled":true,"expiresAt":"2027-01-01"}}"#,
        )
        .unwrap();
        assert_eq!(parsed.license.status, "active");
        assert!(parsed.license.entitled);
        assert_eq!(parsed.license.plan.as_deref(), Some("pro"));
    }

    #[test]
    fn verify_result_defaults_to_not_signed_out() {
        // The `..Default::default()` used in `verify_blocking`'s 401 branch
        // sets every OTHER field — this pins that `signed_out` itself is
        // `false` by default, so a caller who forgets to set it explicitly
        // on the success path would fail loudly (a mismatched `signed_out`
        // is the one field in this struct that changes what the gate does).
        assert!(!VerifyResult::default().signed_out);
    }
}
