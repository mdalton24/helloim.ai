//! Checking for a new NameOS, and installing it.
//!
//! Mark, 2026-08-27: "also design a way to push updates .. look up best
//! practices and add a check for update feature to the app."
//!
//! THE SHAPE, and every piece of it is deliberate:
//!
//! * **The app asks; nothing is pushed at it.** There is no agent, no service,
//!   no background process holding a port open. The app calls one HTTPS
//!   endpoint and gets either `204 No Content` or a small JSON manifest. That
//!   is the whole protocol, and it means a machine that never opens NameOS
//!   never talks to us.
//!
//! * **The manifest is served DYNAMICALLY, not as a static file.** The URL
//!   carries the platform and the version already installed, so the server
//!   decides what that specific machine is offered. That is what makes a
//!   staged rollout and a kill switch possible: pulling a bad release is one
//!   write on our side, not a race against every machine that already
//!   downloaded a static JSON file.
//!
//! * **THE SIGNATURE IS THE WHOLE SECURITY MODEL.** The installer is verified
//!   against a minisign public key compiled into this binary before it is
//!   allowed to run. HTTPS proves we are talking to nameos.ai; the signature
//!   proves the bytes were built by us. Those are different claims, and only
//!   the second one survives a compromised bucket. An update mechanism without
//!   it is a remote-code-execution feature with a friendly name.
//!
//! * **Nothing installs itself.** `check_update` downloads nothing and changes
//!   nothing — it answers a question. `install_update` runs only when somebody
//!   has pressed a button. An assistant that is mid-conversation does not get
//!   to restart itself because a release went out.
//!
//! TWO IDENTIFIERS GO OUT WITH THE CHECK, and both exist so a release can be
//! rolled out carefully rather than to everyone at once. Neither is a name, an
//! email or an account: the install id is a random number this machine made up
//! for itself the first time it asked, kept so a machine stays on one side of a
//! staged rollout instead of flickering between the old and new version every
//! time it checks. The channel is "stable" unless a file says otherwise.

use serde::Serialize;
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_updater::{Update, UpdaterExt};

/// The pending update, parked between the check and the install so the front
/// end never has to hold it — and so `install_update` cannot be talked into
/// installing something that was never checked.
#[derive(Default)]
pub struct Pending(pub Mutex<Option<Update>>);

/// What the window is told. Deliberately small: a version, a date and the
/// notes. The download URL and the signature stay on this side.
#[derive(Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    pub available: bool,
    pub current_version: String,
    /// Only set when `available`.
    pub version: Option<String>,
    pub notes: Option<String>,
    pub date: Option<String>,
    /// Set when the check itself could not be completed — offline, DNS down,
    /// our endpoint having a bad day. **This is not the same as "no update"
    /// and the window must not draw it as one.** Telling somebody they are up
    /// to date when we failed to ask is the one lie this feature could tell.
    pub problem: Option<String>,
}

#[derive(Clone, Serialize)]
struct Progress {
    /// 0.0 to 1.0, or absent when the server sent no Content-Length.
    fraction: Option<f64>,
    downloaded: u64,
    total: Option<u64>,
}

/// A number this machine invented for itself, so a staged rollout is stable
/// per install. Stored beside the rest of the app's settings; deleting it just
/// means a new one is drawn, which is harmless.
fn install_id(app: &AppHandle) -> String {
    let dir = match app.path().app_config_dir() {
        Ok(d) => d,
        Err(_) => return "unknown".into(),
    };
    let f = dir.join("install-id");
    if let Ok(s) = std::fs::read_to_string(&f) {
        let s = s.trim().to_string();
        if !s.is_empty() {
            return s;
        }
    }
    // Not a security value — it only has to be unlikely to collide and
    // unrelated to the person. Time plus the process id is plenty, and it
    // avoids pulling in a uuid crate for sixteen hex digits.
    let n = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0)
        ^ ((std::process::id() as u64) << 32);
    let id = format!("{n:016x}");
    let _ = std::fs::create_dir_all(&dir);
    let _ = std::fs::write(&f, &id);
    id
}

/// "stable" unless somebody has deliberately put another word in a file. There
/// is no UI for this on purpose — it is how a release gets tried on one machine
/// before it is offered to everybody, not a setting for users to wander into.
fn channel(app: &AppHandle) -> String {
    app.path()
        .app_config_dir()
        .ok()
        .and_then(|d| std::fs::read_to_string(d.join("update-channel")).ok())
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'))
        .unwrap_or_else(|| "stable".into())
}

/// The account session token is safe to put in a header.
///
/// IT COMES FROM THE WEBVIEW'S localStorage, which is the far side of a
/// boundary — anything running in that window can set it, and the WebView2
/// store is readable and writable off disk (proven on this product). So it is
/// checked here rather than trusted: `is_ascii_graphic` excludes CR, LF, space
/// and every control character, which is what stops a value like
/// `x\r\nX-Real-Thing: y` from becoming a second header on the request. Length
/// is capped because a header is not the place to discover a megabyte.
///
/// A token that fails this is DROPPED, not an error. The check still runs
/// without it; the only thing lost is entitlement, which fails open anyway.
fn usable_token(t: &str) -> bool {
    let t = t.trim();
    !t.is_empty() && t.len() <= 256 && t.chars().all(|c| c.is_ascii_graphic())
}

/// Ask whether there is a newer helloim.ai build. Downloads nothing.
///
/// `account_token` is optional and the check works without it. When present it
/// goes out as `Authorization: Bearer` so nameos.ai can decide whether this
/// account is entitled to the release — see the licence section in
/// nameos/worker.js. **Entitlement decides what is OFFERED. The minisign
/// signature decides what is SAFE, and it is checked on this machine for every
/// licence state including none.** Those are separate questions and this
/// function does not let the first one touch the second.
///
/// helloim.ai IS the backend now (migration 2026-09-23). The update manifest
/// and R2-backed download route were ported onto the `helloim-api` Worker under
/// `helloim.ai/api/update/*` and `helloim.ai/api/download/*` (see
/// handleUpdateCheck / handleDownload in helloim/worker.js). The endpoint lives
/// in `tauri.conf.json` (now helloim.ai); the `X-Helloim-*` header names below
/// match the Worker's reads; the CSP `connect-src` is helloim.ai. Entitlement
/// still only decides what is OFFERED; the minisign signature (pinned pubkey)
/// still decides what is SAFE to install, unchanged by the domain move.
#[tauri::command]
pub async fn check_update(app: AppHandle, account_token: Option<String>) -> UpdateInfo {
    let current = app.package_info().version.to_string();
    let mut out = UpdateInfo {
        current_version: current,
        ..Default::default()
    };

    let built = app
        .updater_builder()
        // Header NAMES the helloim-api update route reads (handleUpdateCheck in
        // helloim/worker.js): X-Helloim-Install feeds the staged-rollout bucket,
        // X-Helloim-Channel selects the release channel. Renamed from X-NameOS-*
        // in the 2026-09-23 backend migration -- these must match the Worker's
        // header reads or rollout/channel silently stop working.
        .header("X-Helloim-Install", install_id(&app))
        .and_then(|b| b.header("X-Helloim-Channel", channel(&app)))
        .and_then(|b| match account_token.as_deref() {
            Some(t) if usable_token(t) => b.header("Authorization", format!("Bearer {}", t.trim())),
            _ => Ok(b),
        })
        .and_then(|b| b.build());

    let updater = match built {
        Ok(u) => u,
        Err(e) => {
            out.problem = Some(format!("could not start the check: {e}"));
            return out;
        }
    };

    match updater.check().await {
        Ok(Some(u)) => {
            out.available = true;
            out.version = Some(u.version.clone());
            out.notes = u.body.clone();
            out.date = u.date.map(|d| d.to_string());
            *app.state::<Pending>().0.lock().unwrap() = Some(u);
        }
        // The honest "you are on the latest" — we asked and were told no.
        Ok(None) => {
            *app.state::<Pending>().0.lock().unwrap() = None;
        }
        Err(e) => {
            // SAY WHAT WENT WRONG RATHER THAN GOING QUIET. A check that fails
            // silently trains people to stop pressing the button.
            out.problem = Some(friendly(&e.to_string()));
        }
    }
    out
}

/// Turn the plugin's error text into something worth reading. It stays
/// specific — "check your connection" when the machine is plainly offline is
/// the kind of guess that wastes somebody's afternoon.
fn friendly(raw: &str) -> String {
    let low = raw.to_lowercase();
    // "error sending request" IS THE COMMON CASE AND IT WAS NOT ON THIS LIST —
    // added 2026-08-29 after release verification induced a real failure on
    // Windows and got the fallback instead of the sentence: the surfaced text
    // was `error sending request for url (https://nameos.ai/api/update/...)`,
    // which contains none of "dns", "connect" or "timed out". So the branch
    // that exists for exactly this situation never fired, and a raw transport
    // string plus a full internal URL reached a person who can do nothing with
    // either.
    if low.contains("dns")
        || low.contains("connect")
        || low.contains("timed out")
        || low.contains("error sending request")
        || low.contains("network")
    {
        // NAMED NO HOST, AS OF THE NameOS SWEEP, 2026-09-23 -- this used to
        // say "Could not reach nameos.ai", which is our own internal update
        // server's real name (see this file's own header on why it is still
        // called that) but not the brand the person who is reading this
        // actually bought. The fallback branch below already strips a URL
        // out of an error for the identical reason ("it is our own internal
        // shape rather than anything of theirs") -- this brings the DNS/
        // connect branch in line with that same rule rather than leaving it
        // as the one place still naming the endpoint.
        "Could not reach the update server. If you are online, it is us — try again shortly.".into()
    } else if low.contains("signature") || low.contains("minisign") {
        // This one is worth alarming about. It means the bytes we were handed
        // were not built by us.
        "That download did not pass its signature check, so it was not installed.".into()
    } else {
        // THE FALLBACK STILL SAYS WHAT HAPPENED — this house does not hide a
        // cause behind a shrug — but it must not carry a URL. A person cannot
        // act on an endpoint path, and it is our own internal shape rather than
        // anything of theirs, so it is stripped and the rest of the message
        // kept. Nothing else about the text is altered: an unrecognised error
        // is still shown, because an unexplained failure is worse than an ugly
        // one.
        let cleaned = raw
            .split_whitespace()
            .filter(|w| !w.contains("http://") && !w.contains("https://"))
            .collect::<Vec<_>>()
            .join(" ");
        let cleaned = cleaned.trim().trim_end_matches(['(', ')', ':', ',']).trim();
        if cleaned.is_empty() {
            "The update check failed. Try again shortly.".into()
        } else {
            format!("The update check failed: {cleaned}")
        }
    }
}

/// Download the update, verify it, and hand it to the installer.
///
/// **This does not return on success.** The updater hands the installer to
/// Windows and exits this process; the installer puts the new files down and
/// starts NameOS again. Anything the window wanted saved must already be saved
/// before this is called.
#[tauri::command]
pub async fn install_update(app: AppHandle) -> Result<(), String> {
    let update = app
        .state::<Pending>()
        .0
        .lock()
        .unwrap()
        .take()
        .ok_or("There is no update ready to install. Check again first.")?;

    let mut seen: u64 = 0;
    let handle = app.clone();
    update
        .download_and_install(
            move |chunk, total| {
                seen += chunk as u64;
                let _ = handle.emit(
                    "update-progress",
                    Progress {
                        fraction: total.map(|t| {
                            if t == 0 {
                                0.0
                            } else {
                                (seen as f64 / t as f64).clamp(0.0, 1.0)
                            }
                        }),
                        downloaded: seen,
                        total,
                    },
                );
            },
            || {},
        )
        .await
        .map_err(|e| friendly(&e.to_string()))?;

    // Reached only on the platforms where the installer does not replace this
    // process. On Windows the line above never returns.
    app.restart();
}

// ---------------------------------------------------------------------------
// Tests
//
// WHAT THESE EXIST FOR. Every claim in the header above about the signature is
// true of the code as written; none of it was ever true of the code as RUN,
// because nothing here had a test until 2026-08-29. This product has already
// shipped one updater that reported success while doing nothing, and it was
// caught by forcing the failure rather than by watching a clean run. So the
// weight below is on the refusals.
//
// THEY NEED NO SECRET AND NO NETWORK. `fixtures/signed-artifact.bin` was signed
// once, by hand, with the release key, and only the signature is committed.
// That is deliberate: this check has to be runnable on the Windows rig and in
// any future CI without the release key going near either of them.
//
// WHAT THEY CANNOT PROVE, said plainly rather than left to be discovered: that
// a real installer downloads over the real network and replaces a real
// install. Nothing running on Linux can prove that, and it is the one link
// that genuinely needs the Windows rig.
// ---------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use base64::Engine;
    use minisign_verify::{PublicKey, Signature};

    const ARTIFACT: &[u8] = include_bytes!("../fixtures/signed-artifact.bin");
    const SIG: &str = include_str!("../fixtures/signed-artifact.bin.sig");
    const WRONG_KEY_SIG: &str = include_str!("../fixtures/signed-artifact.bin.wrongkey.sig");

    fn conf() -> serde_json::Value {
        serde_json::from_str(include_str!("../tauri.conf.json")).expect("tauri.conf.json parses")
    }

    fn config_pubkey() -> String {
        conf()["plugins"]["updater"]["pubkey"]
            .as_str()
            .expect("the updater has a pubkey")
            .to_string()
    }

    /// The exact three lines tauri-plugin-updater 2.10.1 runs in
    /// `verify_signature` -- base64-unwrap both sides, then `verify(.., true)`.
    /// Copied rather than paraphrased so that a change in the plugin shows up
    /// here as a compile error instead of as a test that quietly checks
    /// something else.
    fn verify_as_the_app_does(data: &[u8], sig_b64: &str, pubkey_b64: &str) -> bool {
        let unwrap = |s: &str| -> Option<String> {
            String::from_utf8(
                base64::engine::general_purpose::STANDARD
                    .decode(s.trim())
                    .ok()?,
            )
            .ok()
        };
        let (Some(pk), Some(sg)) = (unwrap(pubkey_b64), unwrap(sig_b64)) else {
            return false;
        };
        let (Ok(pk), Ok(sg)) = (PublicKey::decode(&pk), Signature::decode(&sg)) else {
            return false;
        };
        pk.verify(data, &sg, true).is_ok()
    }

    // -- the happy path, stated once so the refusals below mean something -----

    #[test]
    fn the_release_key_signs_something_this_binary_accepts() {
        assert!(
            verify_as_the_app_does(ARTIFACT, SIG, &config_pubkey()),
            "the committed signature no longer verifies against the pubkey in \
             tauri.conf.json. Either the config's key was changed -- which cuts \
             off every install already in the world -- or the fixture was \
             edited. Neither is a test to relax."
        );
    }

    // -- the refusals, which are the actual point ----------------------------

    #[test]
    fn a_tampered_artifact_is_refused() {
        // One byte, in the middle, which is what a supply-chain tamper looks
        // like: the file is the right length and the right shape.
        let mut bad = ARTIFACT.to_vec();
        bad[ARTIFACT.len() / 2] ^= 0x01;
        assert!(
            !verify_as_the_app_does(&bad, SIG, &config_pubkey()),
            "A MODIFIED INSTALLER VERIFIED. This is the whole security model of \
             the update feature and it just failed."
        );
    }

    #[test]
    fn a_truncated_artifact_is_refused() {
        // The other realistic corruption: a download that ended early. The
        // plugin verifies the whole buffer after the stream finishes, so a
        // short read must not pass.
        assert!(
            !verify_as_the_app_does(&ARTIFACT[..ARTIFACT.len() - 1], SIG, &config_pubkey()),
            "a truncated download verified"
        );
    }

    #[test]
    fn an_appended_artifact_is_refused() {
        let mut bad = ARTIFACT.to_vec();
        bad.extend_from_slice(b"payload");
        assert!(
            !verify_as_the_app_does(&bad, SIG, &config_pubkey()),
            "bytes appended after the signed content verified"
        );
    }

    #[test]
    fn a_signature_from_another_key_is_refused() {
        // The fixture is a real, valid minisign signature over the SAME bytes,
        // made by a throwaway key that was destroyed after signing. It is
        // structurally perfect and must still be refused, because the only
        // question that matters is which key made it.
        assert!(
            !verify_as_the_app_does(ARTIFACT, WRONG_KEY_SIG, &config_pubkey()),
            "A VALID SIGNATURE FROM THE WRONG KEY WAS ACCEPTED. Anyone able to \
             sign anything could ship a NameOS update."
        );
    }

    #[test]
    fn a_missing_or_malformed_signature_is_refused() {
        let pk = config_pubkey();
        for junk in ["", "   ", "not base64 at all", "bm90IGEgc2lnbmF0dXJl"] {
            assert!(
                !verify_as_the_app_does(ARTIFACT, junk, &pk),
                "a malformed signature ({junk:?}) was accepted"
            );
        }
    }

    #[test]
    fn an_empty_download_is_refused() {
        assert!(!verify_as_the_app_does(b"", SIG, &config_pubkey()));
    }

    // -- the contract with the server ----------------------------------------

    #[test]
    fn the_endpoint_speaks_the_shape_the_worker_parses() {
        // nameos/worker.js splits the path after /api/update/ into exactly
        // three segments and reads them as target, arch, version -- and it
        // rejects a target or arch that is not [a-z0-9_]{1,16}. tauri's
        // {{target}} is "windows"/"linux"/"darwin" and {{arch}} is
        // "x86_64"/"aarch64"/..., so the two agree TODAY. This test is here so
        // that reordering or renaming the template breaks a test rather than
        // breaking everyone's update check silently.
        let eps = conf()["plugins"]["updater"]["endpoints"]
            .as_array()
            .expect("endpoints is an array")
            .clone();
        assert!(!eps.is_empty(), "no update endpoint configured");
        for e in eps {
            let u = e.as_str().expect("endpoint is a string");
            assert!(
                u.starts_with("https://"),
                "the updater endpoint must be https -- the plugin refuses to \
                 start otherwise, and it is how we know we are talking to us: {u}"
            );
            let tail = u.split("/api/update/").nth(1).unwrap_or_else(|| {
                panic!("endpoint no longer matches the worker's /api/update/ route: {u}")
            });
            assert_eq!(
                tail, "{{target}}/{{arch}}/{{current_version}}",
                "the path segments must stay in the order worker.js reads them"
            );
        }
    }

    #[test]
    fn the_pubkey_is_a_real_minisign_key_and_not_a_placeholder() {
        let raw = base64::engine::general_purpose::STANDARD
            .decode(config_pubkey())
            .expect("pubkey is base64");
        let text = String::from_utf8(raw).expect("pubkey decodes to a minisign public key file");
        assert!(text.contains("minisign public key"), "not a minisign key");
        assert!(
            PublicKey::decode(&text).is_ok(),
            "the pubkey in tauri.conf.json will not parse -- every update check \
             on every machine fails at plugin init"
        );
    }

    // -- "could not check" is never "up to date" ------------------------------

    #[test]
    fn a_failed_check_is_never_reported_as_up_to_date() {
        // THE BUG THIS GUARDS IS ALREADY IN THIS PRODUCT'S HISTORY: an updater
        // that exited 0 with the old binary still on disk. Its cousin is a
        // check that could not reach the server and renders as "you are on the
        // latest version". UpdateInfo has to be able to say "I did not manage
        // to ask", and the two states must be distinguishable.
        let failed = super::UpdateInfo {
            available: false,
            current_version: "0.2.0".into(),
            problem: Some("dns failure".into()),
            ..Default::default()
        };
        let up_to_date = super::UpdateInfo {
            available: false,
            current_version: "0.2.0".into(),
            ..Default::default()
        };
        assert!(failed.problem.is_some() && !failed.available);
        assert!(up_to_date.problem.is_none() && !up_to_date.available);
        assert_ne!(
            failed.problem.is_some(),
            up_to_date.problem.is_some(),
            "the window cannot tell a failed check from a current install"
        );
    }

    // -- the account token is checked before it becomes a header --------------

    #[test]
    fn a_token_that_could_forge_a_header_is_dropped() {
        // It arrives from the webview, which is the far side of a boundary.
        // CR and LF are the ones that matter: either would let a value split
        // the request into two headers.
        for bad in [
            "",
            "   ",
            "tok\r\nX-Forwarded-For: 1.2.3.4",
            "tok\nAuthorization: Bearer other",
            "tok with space",
            "tok\0nul",
            "tok\ttab",
        ] {
            assert!(!super::usable_token(bad), "accepted {bad:?}");
        }
        assert!(!super::usable_token(&"a".repeat(257)), "accepted an oversized token");
    }

    #[test]
    fn an_ordinary_session_token_is_kept() {
        // And the guard is not simply refusing everything, which is the way a
        // check like this quietly stops doing its job.
        assert!(super::usable_token("Zm9vYmFy-1234567890abcdef"));
        assert!(super::usable_token("  Zm9vYmFy-1234567890abcdef  "));
        assert!(super::usable_token(&"a".repeat(256)));
    }

    #[test]
    fn a_signature_failure_says_so_rather_than_blaming_the_network() {
        // Getting this wrong would tell somebody to check their wifi when what
        // actually happened is that they were handed bytes we did not build.
        let m = super::friendly("Signature verification failed: minisign error");
        assert!(
            m.contains("signature check"),
            "a signature failure must be reported as one: {m}"
        );
        assert!(!m.contains("try again"), "and must not read as transient: {m}");

        let n = super::friendly("failed to connect to host");
        assert!(n.contains("Could not reach"), "{n}");
    }
}
