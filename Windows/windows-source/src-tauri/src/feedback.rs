//! Suggestions & Feedback — the native path from the app to helloim.ai.
//!
//! WHY THIS IS A NATIVE RUST COMMAND AND NOT A `fetch()` FROM THE WEBVIEW.
//! The webview's Content-Security-Policy (`tauri.conf.json` -> `connect-src`)
//! allows `'self'` and `nameos.ai` only. A `fetch('https://helloim.ai/...')`
//! from the page is blocked by the browser before it leaves, and widening the
//! CSP to admit a second origin is a real cost paid on every page for one
//! form. So the page calls `invoke('submit_feedback', ...)` and this process
//! makes the HTTPS request — a native client is not subject to the webview CSP,
//! exactly as `update.rs` reaches nameos.ai without the page doing so.
//!
//! WHY THE ENDPOINT IS NOT MOVED ONTO nameos.ai (the CSP-allowed host). That
//! would put a helloim feature on the NameOS Worker, which serves the desktop
//! app's own update manifest and is at hand-over — every feedback tweak would
//! redeploy the updater. helloim.ai/api/feedback is the right home; the native
//! request is how the app reaches it without touching either.
//!
//! THE TRUST BOUNDARY LIVES ON THE SERVER, NOT HERE. This function does a light
//! trim-and-cap for the sake of a fast, clear error, but the honeypot, the
//! same-origin guard (a native caller sends no Origin and passes it, by
//! design), the length cap, the category enum and the rate limit are all
//! enforced by handleFeedback() in helloim/worker.js. Nothing here is a gate;
//! a caller who bypassed this function entirely would still meet all of those.
//! A native caller sends no `Origin` header, which is precisely what the
//! server's same-origin guard treats as "not a cross-origin browser request"
//! and admits.

use tauri::AppHandle;

/// helloim.ai/api/feedback is bound by helloim/deploy.py to the helloim-api
/// Worker. Lowercase on purpose: Cloudflare route patterns are case-sensitive,
/// and `/API/...` misses the route and falls through to the Pages marketing
/// page (see the 404 comment in worker.js).
const ENDPOINT: &str = "https://helloim.ai/api/feedback";

/// A submit is a single small POST. If it cannot connect in a few seconds the
/// person is offline or we are down; a long spinner helps nobody.
const NET_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// Matches MAX_FEEDBACK_LEN in worker.js. The server's 2 KB body cap is the
/// real bound; capping here keeps a pathological paste from being sent only to
/// be refused, and gives the UI a number to show.
const MAX_MESSAGE: usize = 1800;

/// Send a suggestion / feature request / bug report to helloim.ai.
///
/// Returns `Ok(())` when the server accepted it (HTTP 200). Every other outcome
/// is an `Err(String)` written to be shown to the user as-is — specific enough
/// to act on, never a stack trace.
#[tauri::command]
pub async fn submit_feedback(
    app: AppHandle,
    message: String,
    category: Option<String>,
    contact_email: Option<String>,
) -> Result<(), String> {
    // ureq is blocking; run it off the async runtime so the UI thread is free.
    tauri::async_runtime::spawn_blocking(move || send(app, message, category, contact_email))
        .await
        .map_err(|_| "The app could not start that request. Try again.".to_string())?
}

fn send(
    app: AppHandle,
    message: String,
    category: Option<String>,
    contact_email: Option<String>,
) -> Result<(), String> {
    let msg = message.trim();
    if msg.is_empty() {
        // Local, so the person hears it instantly rather than after a round trip.
        return Err("Add a little detail before sending — even one line helps.".to_string());
    }
    let msg: String = msg.chars().take(MAX_MESSAGE).collect();

    // Build the body with serde_json (present); send it as a STRING, because
    // this crate's ureq is built without the `json` feature (see Cargo.toml) --
    // `send_json`/`into_json` are not compiled in, and `install.rs`/`skills.rs`
    // already read responses with `into_string()` for the same reason.
    let mut body = serde_json::json!({
        "message": msg,
        "app_version": app.package_info().version.to_string(),
    });
    if let Some(c) = category.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        body["category"] = serde_json::Value::String(c.to_string());
    }
    if let Some(e) = contact_email.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        body["contact_email"] = serde_json::Value::String(e.to_string());
    }
    let payload = body.to_string();

    let agent = ureq::AgentBuilder::new().timeout(NET_TIMEOUT).build();
    // NO Origin header is set. That is deliberate: the server's same-origin
    // guard admits a request with no Origin (the native path) and refuses a
    // cross-origin browser one. Setting an Origin here would opt into a check
    // meant for the browser and could only ever hurt.
    let res = agent
        .post(ENDPOINT)
        .set("Content-Type", "application/json")
        .send_string(&payload);

    match res {
        Ok(_) => Ok(()), // helloim-api returns 200 only on {ok:true}.
        Err(ureq::Error::Status(429, _)) => {
            Err("You have sent a few just now — give it a minute and try again.".to_string())
        }
        Err(ureq::Error::Status(400, _)) => {
            // The server rejected the shape (e.g. a category it does not know,
            // or a malformed contact address). The UI constrains these, so this
            // is rare; say something true rather than echoing an error code.
            Err("That did not go through — check the category and your email, then try again.".to_string())
        }
        Err(ureq::Error::Status(503, _)) => {
            Err("Feedback is briefly unavailable. Try once more in a minute, or email hello@markdalton.com.".to_string())
        }
        Err(ureq::Error::Status(code, _)) => {
            Err(format!("helloim.ai answered {code}. Try again shortly, or email hello@markdalton.com."))
        }
        Err(ureq::Error::Transport(t)) => {
            Err(format!("Could not reach helloim.ai — check your connection. ({t})"))
        }
    }
}
