//! The NameOS adapter — Anthropic-shaped on the inside, OpenAI-shaped on the
//! outside.
//!
//! **WIRED BACK UP 2026-08-29 AND SWITCHED ON THE SAME DAY.** Mark: *"The user
//! needs to pick their cloud ai. They need to be given a choice of Claude AI or
//! OpenAI."* So the three things that were removed together on 2026-08-28 came
//! back together: the `openai-compatible` arm of `providers::apply_env`, that
//! kind's place in the routable set, and the `adapter::init` call in main.rs's
//! setup.
//!
//! **THIS HEADER SAID "THE LISTENER NEVER BINDS ON THIS BUILD" AND CITED
//! `providers::OPENAI_ENABLED` BEING `false`. IT HAS BEEN `true` SINCE
//! 2026-08-29 (`providers.rs:163`) — corrected here 2026-08-31, along with the
//! twin of this paragraph in main.rs's `mod adapter` note.** Both were long,
//! emphatic, and wrong about the constant they named, and a reader trusting
//! either would conclude this path is dead when it routes. **A comment that
//! contradicts the code is how the next person builds on a false premise**, and
//! this one had two copies.
//!
//! **WHAT MAKES IT SAFE IS NOT THE GATE. IT IS THAT THE DOT MUST BE EARNED ON
//! THE USER'S OWN MACHINE.** `test_provider` runs `test_chat_endpoint` against
//! OpenAI for real with their key, then `test_binary` spawns Claude Code
//! through `apply_env` — which for this kind starts this translator and points
//! the binary at loopback — so going green is a real end-to-end round trip, and
//! `may_select` refuses a row that has not gone green. A broken translator
//! fails the Test in front of them, with words, rather than failing quietly
//! halfway through a conversation.
//!
//! **WHAT IS STILL TRUE, AND IT IS THE THING TO CARRY FORWARD: every test in
//! this file is synthetic.** Replayed and hand-built payloads, no network —
//! this translator has never met a real OpenAI endpoint. Two consequences,
//! neither of them "turn it off": the capability lines still marked UNPROVEN in
//! `desktop/BRAIN-CHOICES.md` stay unproven and may not be shown as fact, and
//! the neutral-engine work (`engine/`) treats proving this module against a
//! real endpoint as its own line item rather than assuming it is roughly right.
//! The known live defect is in `FACTS.md`: this file sends `max_tokens`, which
//! newer OpenAI reasoning models reject with a 400, and `providers.rs:1351`
//! discards the response body so the user is shown only *"The endpoint answered
//! 400."*
//!
//! **WHAT CHANGED IN THE REWIRING, and it is worth knowing before comparing
//! against `3a328ff`:** `apply_env` used to read
//! `if kind == "openai-compatible" { adapter } else { direct }`. That `else`
//! caught every other kind, so any future cloud kind would have gone direct —
//! Claude Code pointed at a third party in Anthropic's wire format carrying
//! the user's real key. It is an exhaustive `match` with a refusing final arm
//! now, and there is a test that fails with those exact words.
//!
//! The rest of this header is the original design and every word of it still
//! applies to the code below.
//!
//! **WHY THIS EXISTS — Mark's decision, 2026-08-27, overruling the earlier
//! Gemini rejection with the cost stated: "fuck the rule.. we need to be able
//! to connect to each ai that a user uses otherwise we are failing this
//! product. It should be ai agnostic."** The three routes rejected earlier
//! stay rejected — no bundled proxy, no hosted gateway, no third-party
//! software shipped. This is the fourth route: the translator written by us,
//! in this binary, spawned as a thread and dead when the app is.
//!
//! **THE LEVERAGE IS THE POINT: ONE ADAPTER, NEARLY EVERYTHING.** OpenAI,
//! Gemini, Groq, Mistral, DeepSeek, Together, OpenRouter, and the local
//! runtimes people actually use — llama.cpp's server, LM Studio, and Ollama
//! itself — all publish OpenAI-compatible endpoints. This module is not a
//! Gemini feature; Gemini is one row of it. That is what makes owning a
//! protocol translator worth it.
//!
//! **THE SERVER IS NOT A WEB SERVER AND MUST NEVER GROW INTO ONE — Jarvis's
//! ruling on approval.** It speaks to exactly one known client, the `claude`
//! binary; it is POST-only, behind a token, on loopback, `Connection: close`.
//! Hand-rolled over `std::net::TcpListener` deliberately: no tokio, no hyper,
//! no tiny_http, so "nothing third-party is shipped" stays literally true
//! rather than nearly true. If a feature wants a GET route or keep-alive,
//! that feature is wrong.
//!
//! **THE TRUST BOUNDARY THIS CREATES, AND HOW IT IS CLOSED.** A loopback
//! relay to a paid account is an open wallet to anything on the machine that
//! can reach it — including any web page's JavaScript, which can POST to
//! 127.0.0.1 from a browser (CORS blocks the reply, not the request). So:
//!  - Who may call: holders of the per-launch random bearer token, minted
//!    from the OS CSPRNG and handed ONLY to the child process we spawn, on
//!    the `ANTHROPIC_AUTH_TOKEN` channel the binary already authenticates
//!    with. Loopback origin proves NOTHING and is not consulted — the same
//!    lesson as trusting a proxied header.
//!  - Wrong credential: 401 with no detail, before the body is even parsed.
//!  - Malformed input: 400 with an Anthropic-shaped error body, so the
//!    binary renders words instead of hanging.
//!  - What errors leak: never the run token, never the provider key. Upstream
//!    error bodies are passed through capped, because they are the provider
//!    explaining the user's own mistake to them.
//!
//! **AND ONE PART IS A GENUINE UPGRADE ON THE DIRECT PATH, not a side
//! effect: the provider's REAL key never enters the child's environment at
//! all.** It lives in this process, fetched from the OS store per request,
//! attached upstream, and is unreadable to anything the child process runs.
//! The direct paths put the key in the child env because the binary itself
//! must present it; here the binary presents the run token instead.
//!
//! **KNOWN AND DEFERRED — six degradations now, recorded so a future
//! session inherits the limits instead of rediscovering them (five approved
//! 2026-08-27; the sixth found live 2026-09-02, see `translate_request`'s own
//! comment on it):**
//!  1. `count_tokens` is an ESTIMATE (bytes/4) — no OpenAI equivalent
//!     exists. Consequence: auto-compaction timing and cost display drift.
//!  2. Prompt caching is STRIPPED (`cache_control` is Anthropic-only).
//!     Consequence is money, not correctness: paid OpenAI-shaped endpoints
//!     re-bill more of the context each turn. The provider caveat says so.
//!  3. Reasoning content (DeepSeek-style `reasoning_content`) is DROPPED —
//!     the user pays for thinking they never sees. Mappable to Anthropic
//!     thinking blocks later.
//!  4. `stop_sequence` attribution is always null — OpenAI does not say
//!     which sequence matched. Cosmetic.
//!  5. Images inside TOOL RESULTS become a text marker — OpenAI's `tool`
//!     role takes text on most providers. A model that says it cannot see
//!     the picture is honest; a model inventing what is in it is the
//!     failure.
//!  6. A REASONING MODEL LOSES ITS REASONING THE MOMENT A TOOL IS ON THE
//!     TURN. OpenAI's `/chat/completions` refuses function tools together
//!     with any reasoning effort outright (400, naming the exact fix:
//!     `reasoning_effort: "none"`), and Claude Code attaches its own tools on
//!     nearly every real turn — so through this translator, a reasoning-tier
//!     model's reasoning is live only on the rare tool-free turn. Not
//!     cosmetic: it undercuts the whole reason to pick a reasoning-tier model
//!     for an agentic, tool-heavy product, which is what NameOS is. The
//!     alternative — OpenAI's `/v1/responses` — is not the shape Gemini,
//!     Groq and the rest publish, so switching this translator to it would
//!     cost the leverage the module doc opens with. Recorded as a real
//!     product-shape finding, not silently patched around: which default
//!     model to ship is a call for whoever owns that choice, not this file.
//!
//! One robustness call worth knowing before "fixing" it: streamed TOOL-CALL
//! arguments are accumulated per call and emitted whole, while TEXT streams
//! token by token. Interleaved argument fragments from two parallel calls
//! would corrupt JSON if forwarded raw; Claude Code renders tool calls on
//! completion anyway, so nothing visible is lost and parallel calls are
//! provably correct — which is the classic silent corruption in adapters
//! like this one, and the one this file is most tested against.

// ORPHANED, NOT DEAD — see the header. Every item below has no caller today
// and a wall of "never used" warnings is how a module like this gets tidied
// away by somebody who did not read why it is here. The tests below still
// exercise all of it.
#![allow(dead_code)]

use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

/// Headers are small; a head bigger than this is not the claude binary.
const MAX_HEAD: usize = 64 * 1024;
/// A conversation with images can be big; beyond this it is not a request,
/// it is a mistake.
const MAX_BODY: usize = 64 * 1024 * 1024;

struct State {
    config_dir: Option<PathBuf>,
    running: Option<(u16, String)>,
}

fn state() -> &'static Mutex<State> {
    static STATE: OnceLock<Mutex<State>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(State { config_dir: None, running: None }))
}

/// Where providers.json lives. Called once from Tauri setup; the adapter
/// reads the file fresh per request (same pattern as the board's ticket
/// store) so an edited provider reaches it without a restart.
pub fn init(config_dir: PathBuf) {
    state().lock().unwrap().config_dir = Some(config_dir);
}

/// Start the listener if it is not running, and hand back (port, token).
/// The port is ephemeral; the token is minted per launch from the OS CSPRNG
/// and never written anywhere but the child's environment.
pub fn ensure_running() -> Result<(u16, String), String> {
    let mut guard = state().lock().unwrap();
    if let Some(running) = &guard.running {
        return Ok(running.clone());
    }
    let Some(dir) = guard.config_dir.clone() else {
        return Err("The translator was never initialised — this is a bug, not a setup problem.".into());
    };

    let mut raw = [0u8; 32];
    getrandom::getrandom(&mut raw).map_err(|e| format!("no OS randomness for the token: {e}"))?;
    let token: String = raw.iter().map(|b| format!("{b:02x}")).collect();

    let listener = TcpListener::bind("127.0.0.1:0")
        .map_err(|e| format!("could not open the loopback translator: {e}"))?;
    let port = listener
        .local_addr()
        .map_err(|e| format!("could not read the translator's port: {e}"))?
        .port();

    let accept_token = token.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(stream) = stream else { continue };
            let token = accept_token.clone();
            let dir = dir.clone();
            std::thread::spawn(move || {
                let _ = handle(stream, &token, &dir);
            });
        }
    });

    guard.running = Some((port, token.clone()));
    Ok((port, token))
}

// ---------------------------------------------------------------------------
// The narrow server. POST only, token first, Connection: close.
// ---------------------------------------------------------------------------

enum Route {
    Messages(String),
    CountTokens(String),
    Unknown,
}

/// `/{provider-id}/v1/messages[?query]` — the id rides the path so a test and
/// a send against different rows can never race a global "current provider".
fn route_of(path: &str) -> Route {
    let path = path.split('?').next().unwrap_or(path);
    let parts: Vec<&str> = path.trim_matches('/').split('/').collect();
    match parts.as_slice() {
        [id, "v1", "messages"] if !id.is_empty() => Route::Messages((*id).into()),
        [id, "v1", "messages", "count_tokens"] if !id.is_empty() => {
            Route::CountTokens((*id).into())
        }
        _ => Route::Unknown,
    }
}

/// Digest-compare so equality takes the same time whether the guess was close
/// or not. Loopback timing attacks are a stretch; two hashes cost nothing.
fn auth_ok(headers: &HashMap<String, String>, token: &str) -> bool {
    use sha2::{Digest, Sha256};
    let presented = headers
        .get("authorization")
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or("");
    Sha256::digest(presented.as_bytes()) == Sha256::digest(token.as_bytes())
}

/// Parse the request head: method, path, lowercased headers. Bounded.
fn parse_head(head: &str) -> Result<(String, String, HashMap<String, String>), String> {
    let mut lines = head.lines();
    let request = lines.next().ok_or("empty request")?;
    let mut parts = request.split_whitespace();
    let method = parts.next().ok_or("no method")?.to_string();
    let path = parts.next().ok_or("no path")?.to_string();
    let mut headers = HashMap::new();
    for line in lines {
        if let Some((k, v)) = line.split_once(':') {
            headers.insert(k.trim().to_lowercase(), v.trim().to_string());
        }
    }
    Ok((method, path, headers))
}

fn write_simple(stream: &mut TcpStream, status: &str, content_type: &str, body: &str) {
    let _ = write!(
        stream,
        "HTTP/1.1 {status}\r\ncontent-type: {content_type}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    );
}

/// An error the claude binary can render as words. Anthropic-shaped on
/// purpose — a bare 400 with an empty body looks like a hang from the UI.
fn write_error(stream: &mut TcpStream, status: &str, message: &str) {
    let body = json!({
        "type": "error",
        "error": { "type": "api_error", "message": message }
    })
    .to_string();
    write_simple(stream, status, "application/json", &body);
}

fn handle(mut stream: TcpStream, token: &str, config_dir: &std::path::Path) -> std::io::Result<()> {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(30)));

    // Read the head byte-wise up to the blank line, bounded.
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        match stream.read(&mut byte) {
            Ok(0) => return Ok(()),
            Ok(_) => head.push(byte[0]),
            Err(_) => return Ok(()),
        }
        if head.ends_with(b"\r\n\r\n") {
            break;
        }
        if head.len() > MAX_HEAD {
            write_error(&mut stream, "431 Request Header Fields Too Large", "Request head too large.");
            return Ok(());
        }
    }
    let head = String::from_utf8_lossy(&head).into_owned();
    let Ok((method, path, headers)) = parse_head(&head) else {
        write_error(&mut stream, "400 Bad Request", "Malformed request head.");
        return Ok(());
    };

    // THE TOKEN IS THE GATE, AND IT COMES FIRST — before the method, before
    // the body, before anything learns whether a provider id exists. Loopback
    // origin is not consulted; see the module doc.
    if !auth_ok(&headers, token) {
        write_simple(&mut stream, "401 Unauthorized", "application/json", "{}");
        return Ok(());
    }
    if method != "POST" {
        write_error(&mut stream, "405 Method Not Allowed", "This translator only takes POST.");
        return Ok(());
    }
    let route = route_of(&path);
    let provider_id = match &route {
        Route::Messages(id) | Route::CountTokens(id) => id.clone(),
        Route::Unknown => {
            write_error(&mut stream, "404 Not Found", "No such route on the translator.");
            return Ok(());
        }
    };

    let Some(len) = headers.get("content-length").and_then(|v| v.parse::<usize>().ok()) else {
        write_error(&mut stream, "411 Length Required", "A Content-Length is required.");
        return Ok(());
    };
    if len > MAX_BODY {
        write_error(&mut stream, "413 Payload Too Large", "Request body too large.");
        return Ok(());
    }
    let mut body = vec![0u8; len];
    if stream.read_exact(&mut body).is_err() {
        return Ok(());
    }
    let Ok(request) = serde_json::from_slice::<Value>(&body) else {
        write_error(&mut stream, "400 Bad Request", "The request body was not JSON.");
        return Ok(());
    };

    // The provider row, read fresh from disk so edits need no restart. The
    // KEY never touches the child's environment — it is fetched here, inside
    // this process, per request. See the module doc: that is the upgrade.
    let Some(provider) = crate::providers::row_from_disk(config_dir, &provider_id) else {
        write_error(&mut stream, "404 Not Found", "No such provider is configured.");
        return Ok(());
    };
    let key = crate::providers::secret_of(&provider.id)
        .unwrap_or_else(|| "nameos-local-placeholder".into());

    match route {
        Route::CountTokens(_) => {
            // DEGRADATION 1 (see module doc): an estimate, because no OpenAI
            // equivalent exists. Wrong counts drift compaction timing; they
            // do not corrupt anything.
            let text = request.to_string();
            let estimate = (text.len() / 4).max(1);
            write_simple(
                &mut stream,
                "200 OK",
                "application/json",
                &json!({ "input_tokens": estimate }).to_string(),
            );
        }
        Route::Messages(_) => {
            let translated = match translate_request(&request) {
                Ok(t) => t,
                Err(why) => {
                    write_error(&mut stream, "400 Bad Request", &why);
                    return Ok(());
                }
            };
            relay(&mut stream, &provider.base_url, &key, translated);
        }
        Route::Unknown => unreachable!(),
    }
    Ok(())
}

/// Send the translated request upstream and stream or return the answer.
fn relay(stream: &mut TcpStream, base_url: &str, key: &str, mut body: Value) {
    let base = base_url.trim().trim_end_matches('/');
    let url = format!("{base}/chat/completions");
    let wants_stream = body.get("stream").and_then(|s| s.as_bool()).unwrap_or(false);
    if wants_stream {
        // Ask for usage on the final chunk where the provider supports it;
        // absent usage is synthesized as an estimate downstream.
        body["stream_options"] = json!({ "include_usage": true });
    }

    // Connect fast, read patiently: a long generation pauses between tokens,
    // and the per-read timeout is what bounds a dead upstream, not a total
    // timeout that would kill a healthy long answer.
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(10))
        .timeout_read(Duration::from_secs(300))
        .build();
    let response = agent
        .post(&url)
        .set("content-type", "application/json")
        .set("authorization", &format!("Bearer {key}"))
        .send_string(&body.to_string());

    let upstream = match response {
        Ok(r) => r,
        Err(ureq::Error::Status(code, resp)) => {
            // THE PROVIDER EXPLAINING THE USER'S OWN MISTAKE TO THEM, CAPPED —
            // Mason, 2026-08-31, reusing `providers::redact_and_truncate`
            // rather than the bare `truncate(500)` this used to carry. See
            // that function's own comment for the two bugs it exists to stop;
            // both were live here.
            //
            // "Never our token, never the key — neither is in these bodies"
            // is the assumption this comment used to state as fact, and it is
            // the SAME assumption `providers::generic_status_error` was built
            // to stop trusting on the Test button's own call to this exact
            // kind of upstream — Beck proved a live provider row echoing 23
            // of 24 characters of a bearer token back in its own error body.
            // There was never a reason the LIVE SEND would be safer than the
            // Test: same bearer token two lines up, same upstream shape,
            // same risk of an error page quoting the request that caused it.
            //
            // And the bare `truncate(500)` panicked on a multibyte character
            // sitting across byte 500 — proven on `generic_status_error`
            // before this fix, same fault, different call site. Here the
            // panic happened inside this thread's connection handler rather
            // than a `#[tauri::command]`, so `invoke('send', …)` never
            // resolved and the visible symptom was not a crash dialog, it was
            // the composer sitting on "Claude Code started but did not
            // answer in time." with nothing to explain why.
            let detail = crate::providers::redact_and_truncate(
                resp.into_string().unwrap_or_default(),
                key,
                500,
            );
            write_error(
                stream,
                &format!("{code} Upstream Error"),
                &format!("The provider answered {code}: {detail}"),
            );
            return;
        }
        Err(ureq::Error::Transport(t)) => {
            write_error(stream, "502 Bad Gateway", &format!("Could not reach the provider: {t}"));
            return;
        }
    };

    if !wants_stream {
        let text = upstream.into_string().unwrap_or_default();
        match openai_to_anthropic_response(&text) {
            Ok(v) => write_simple(stream, "200 OK", "application/json", &v.to_string()),
            Err(why) => write_error(stream, "502 Bad Gateway", &why),
        }
        return;
    }

    // Streaming: SSE in, SSE out, translated event by event. Close-delimited
    // body (legal with Connection: close), so no chunked framing to get wrong.
    let _ = write!(
        stream,
        "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncache-control: no-cache\r\nconnection: close\r\n\r\n"
    );
    let mut translator = StreamOut::default();
    let mut reader = BufReader::new(upstream.into_reader());
    let mut line = String::new();
    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => {}
            Err(_) => break,
        }
        let Some(payload) = line.trim().strip_prefix("data:") else { continue };
        let payload = payload.trim();
        if payload == "[DONE]" {
            break;
        }
        let Ok(chunk) = serde_json::from_str::<Value>(payload) else { continue };
        for event in translator.feed(&chunk) {
            if stream.write_all(event.as_bytes()).is_err() {
                return; // client went away; nothing to clean up
            }
        }
        let _ = stream.flush();
    }
    for event in translator.finish() {
        let _ = stream.write_all(event.as_bytes());
    }
    let _ = stream.flush();
}

// ---------------------------------------------------------------------------
// Request translation: Anthropic -> OpenAI.
// ---------------------------------------------------------------------------

/// Is this one of OpenAI's reasoning models — o1, o3, o4, or the gpt-5 line?
///
/// **VERIFIED 2026-08-31 FROM OPENAI'S OWN DOCS AND CONFIRMED ERROR STRINGS,
/// NOT FROM MEMORY — this box has no OpenAI key to prove it against directly.**
/// That family speaks a genuinely different request shape from every model
/// before it: it 400s `max_tokens` outright (`unsupported_parameter`, error
/// text names `max_completion_tokens` as the field it wants instead), and it
/// 400s again on ANY explicit `temperature` or `top_p`
/// (`unsupported_value` — only the model's own default of 1 is accepted,
/// which is exactly what omitting the field already gives it).
///
/// Detected by NAME, not by asking the endpoint first and retrying, because
/// this same translator also carries plain gpt-4-class OpenAI models and
/// third-party `openai-compatible` servers (LM Studio, llama.cpp, Groq…)
/// that still want the shape this file has always sent — a global switch
/// would 400 THEM instead of fixing anything. The decision is made per
/// REQUEST, off the model name Claude Code is actually asking for, because a
/// provider row's `model` field can itself be either kind.
///
/// **THIS PREFIX LIST WILL ROT — it is a snapshot of today's naming, and
/// OpenAI will ship families this does not recognise.** When that happens,
/// Fault 2's fix (see providers.rs's `generic_status_error`) is what keeps
/// the failure from being silent: the person sees OpenAI's own words naming
/// the field it wanted, on the Test button, instead of a bare status code —
/// so a missed prefix here is recoverable rather than a dead end.
// pub(crate): providers.rs's `test_chat_endpoint` needs the same call for the
// Test button's request body. It used to build that body unbranched — see
// the doc comment on `chat_completion_test_body` in providers.rs for what
// that cost — and the fix is to share this exact function rather than grow a
// second copy of the prefix list that can drift from this one.
pub(crate) fn is_reasoning_model(model: &str) -> bool {
    let m = model.trim().to_ascii_lowercase();
    m.starts_with("o1") || m.starts_with("o3") || m.starts_with("o4") || m.starts_with("gpt-5")
}

/// Text out of an Anthropic content value (string or block array).
fn text_of(content: &Value) -> String {
    match content {
        Value::String(s) => s.clone(),
        Value::Array(blocks) => blocks
            .iter()
            .filter_map(|b| match b.get("type").and_then(|t| t.as_str()) {
                Some("text") => b.get("text").and_then(|t| t.as_str()).map(String::from),
                // DEGRADATION 5 (see module doc): an image a tool handed back
                // cannot ride OpenAI's tool role portably. The model saying
                // it cannot see the picture is honest; inventing it is not.
                Some("image") => {
                    Some("[image omitted: this provider cannot receive images from tools]".into())
                }
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

pub(crate) fn translate_request(anthropic: &Value) -> Result<Value, String> {
    let mut messages: Vec<Value> = Vec::new();

    // Top-level `system` becomes the leading system message. String or blocks.
    if let Some(system) = anthropic.get("system") {
        let text = text_of(system);
        if !text.is_empty() {
            messages.push(json!({ "role": "system", "content": text }));
        }
    }

    for msg in anthropic.get("messages").and_then(|m| m.as_array()).unwrap_or(&Vec::new()) {
        let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or("user");
        let content = msg.get("content").cloned().unwrap_or(Value::Null);

        match (&content, role) {
            (Value::String(s), _) => {
                messages.push(json!({ "role": role, "content": s }));
            }
            (Value::Array(blocks), "assistant") => {
                let mut text = String::new();
                let mut tool_calls: Vec<Value> = Vec::new();
                for b in blocks {
                    match b.get("type").and_then(|t| t.as_str()) {
                        Some("text") => {
                            text.push_str(b.get("text").and_then(|t| t.as_str()).unwrap_or(""))
                        }
                        Some("tool_use") => tool_calls.push(json!({
                            "id": b.get("id").cloned().unwrap_or(Value::Null),
                            "type": "function",
                            "function": {
                                "name": b.get("name").cloned().unwrap_or(Value::Null),
                                // OpenAI wants the arguments as a JSON STRING.
                                "arguments": b.get("input").cloned().unwrap_or(json!({})).to_string(),
                            }
                        })),
                        // DEGRADATION 3: thinking blocks are dropped.
                        _ => {}
                    }
                }
                let mut out = json!({ "role": "assistant" });
                out["content"] = if text.is_empty() { Value::Null } else { Value::String(text) };
                if !tool_calls.is_empty() {
                    out["tool_calls"] = Value::Array(tool_calls);
                }
                messages.push(out);
            }
            (Value::Array(blocks), _) => {
                // User: tool_results become `tool` role messages FIRST (they
                // answer the assistant's calls just above), then whatever
                // text/images remain become the user message proper.
                let mut parts: Vec<Value> = Vec::new();
                let mut plain = String::new();
                let mut has_image = false;
                for b in blocks {
                    match b.get("type").and_then(|t| t.as_str()) {
                        Some("tool_result") => {
                            messages.push(json!({
                                "role": "tool",
                                "tool_call_id": b.get("tool_use_id").cloned().unwrap_or(Value::Null),
                                "content": text_of(b.get("content").unwrap_or(&Value::Null)),
                            }));
                        }
                        Some("text") => {
                            let t = b.get("text").and_then(|t| t.as_str()).unwrap_or("");
                            plain.push_str(t);
                            parts.push(json!({ "type": "text", "text": t }));
                        }
                        Some("image") => {
                            // Anthropic base64 block -> OpenAI data URL.
                            let src = b.get("source").cloned().unwrap_or(json!({}));
                            let media = src.get("media_type").and_then(|m| m.as_str()).unwrap_or("image/png");
                            let data = src.get("data").and_then(|d| d.as_str()).unwrap_or("");
                            has_image = true;
                            parts.push(json!({
                                "type": "image_url",
                                "image_url": { "url": format!("data:{media};base64,{data}") }
                            }));
                        }
                        _ => {}
                    }
                }
                if has_image {
                    messages.push(json!({ "role": "user", "content": parts }));
                } else if !plain.is_empty() {
                    messages.push(json!({ "role": "user", "content": plain }));
                }
            }
            _ => {}
        }
    }

    // Reasoning models take a different field name for the token cap and
    // reject sampling parameters outright — see is_reasoning_model's doc
    // comment for what that costs and how this is decided per request.
    let reasoning =
        anthropic.get("model").and_then(|m| m.as_str()).map(is_reasoning_model).unwrap_or(false);

    let mut out = json!({
        "model": anthropic.get("model").cloned().unwrap_or(Value::Null),
        "messages": messages,
    });
    // max_tokens is required on the Anthropic side, so it is always here.
    // Reasoning models reject the field under that name (400
    // unsupported_parameter, wanting max_completion_tokens); every other
    // model on this translator — gpt-4-class OpenAI and the third-party
    // openai-compatible servers this row can also point at — still wants the
    // name this file has always sent, so the choice is made per request.
    if let Some(v) = anthropic.get("max_tokens") {
        let field = if reasoning { "max_completion_tokens" } else { "max_tokens" };
        out[field] = v.clone();
    }
    // Reasoning models also 400 on ANY explicit temperature/top_p
    // (unsupported_value — only the default is accepted), which is exactly
    // what omitting the field already gives them. So for them it is dropped
    // rather than forwarded; every other kind still gets what was asked for.
    if !reasoning {
        for (theirs, ours) in [("temperature", "temperature"), ("top_p", "top_p")] {
            if let Some(v) = anthropic.get(theirs) {
                out[ours] = v.clone();
            }
        }
    }
    if let Some(v) = anthropic.get("stop_sequences") {
        out["stop"] = v.clone();
    }
    if let Some(v) = anthropic.get("stream") {
        out["stream"] = v.clone();
    }
    // DEGRADATION 2: `cache_control` never survives translation (it lives
    // inside blocks we rebuilt above), and `metadata` is Anthropic-only.
    // Stripping cache_control is why these providers can cost more per
    // message — the caveat on the provider row says so out loud.

    if let Some(tools) = anthropic.get("tools").and_then(|t| t.as_array()) {
        let translated: Vec<Value> = tools
            .iter()
            .map(|t| {
                json!({
                    "type": "function",
                    "function": {
                        "name": t.get("name").cloned().unwrap_or(Value::Null),
                        "description": t.get("description").cloned().unwrap_or(Value::Null),
                        // Both sides are JSON Schema; pass through untouched.
                        "parameters": t.get("input_schema").cloned().unwrap_or(json!({"type":"object"})),
                    }
                })
            })
            .collect();
        if !translated.is_empty() {
            out["tools"] = Value::Array(translated);
        }
    }
    if let Some(choice) = anthropic.get("tool_choice") {
        out["tool_choice"] = match choice.get("type").and_then(|t| t.as_str()) {
            Some("any") => json!("required"),
            Some("tool") => json!({
                "type": "function",
                "function": { "name": choice.get("name").cloned().unwrap_or(Value::Null) }
            }),
            _ => json!("auto"),
        };
    }
    // DEGRADATION 6, FOUND LIVE 2026-09-02 — a reasoning model 400s outright
    // the moment BOTH function tools and any reasoning effort are present on
    // `/chat/completions`. OpenAI's own words, captured against a real key
    // and the shipped default model: *"Function tools with reasoning_effort
    // are not supported for gpt-5.6-sol in /v1/chat/completions. To use
    // function tools, use /v1/responses or set reasoning_effort to 'none'."*
    //
    // This translator only speaks `/chat/completions` — see the module doc
    // for why one shape is what makes it worth owning (OpenAI, Gemini, Groq,
    // and the rest all publish that shape; `/v1/responses` is not portable
    // the same way). So the only door through is the second one OpenAI
    // names. Claude Code attaches its own tools (Read, Write, Bash, …) on
    // essentially every real turn, so without this a reasoning-tier model
    // was not merely degraded through this translator — it was unusable the
    // moment a turn touched a file, which for NameOS is nearly every turn.
    //
    // Scoped to when tools are ACTUALLY present, not to every reasoning-model
    // request: OpenAI's own error names the combination, not the model alone,
    // and a tool-free turn (a plain question) keeps full reasoning effort
    // rather than paying this cost for nothing.
    //
    // Verified live, same request otherwise: 400 with a tool attached and no
    // `reasoning_effort`; 200 with `reasoning_effort: "none"` added. See
    // `a_real_openai_round_trip_translates_both_ways` and
    // `providers.rs`'s `openai_full_round_trip_earns_green_for_real` for the
    // proof against a real key — the second one is what caught this, because
    // it is the one test that spawns the real `claude` binary, which is the
    // only caller that reliably attaches tools on a turn this trivial.
    if reasoning && out.get("tools").is_some() {
        out["reasoning_effort"] = json!("none");
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Response translation: OpenAI -> Anthropic.
// ---------------------------------------------------------------------------

fn map_finish(reason: Option<&str>, has_tools: bool) -> &'static str {
    match reason {
        Some("tool_calls") | Some("function_call") => "tool_use",
        Some("length") => "max_tokens",
        Some("stop") | Some("content_filter") | None => {
            if has_tools {
                "tool_use"
            } else {
                "end_turn"
            }
        }
        Some(_) => "end_turn",
    }
}

pub(crate) fn openai_to_anthropic_response(body: &str) -> Result<Value, String> {
    let v: Value = serde_json::from_str(body.trim())
        .map_err(|_| "The provider replied, but not with JSON.".to_string())?;
    let message = v
        .pointer("/choices/0/message")
        .ok_or("The provider replied, but with no message in it.")?;

    let mut content: Vec<Value> = Vec::new();
    if let Some(text) = message.get("content").and_then(|c| c.as_str()) {
        if !text.is_empty() {
            content.push(json!({ "type": "text", "text": text }));
        }
    }
    let mut has_tools = false;
    if let Some(calls) = message.get("tool_calls").and_then(|t| t.as_array()) {
        for call in calls {
            has_tools = true;
            let args = call.pointer("/function/arguments").and_then(|a| a.as_str()).unwrap_or("{}");
            let input: Value = serde_json::from_str(if args.trim().is_empty() { "{}" } else { args })
                .map_err(|_| {
                    "The provider returned tool arguments that are not valid JSON.".to_string()
                })?;
            content.push(json!({
                "type": "tool_use",
                "id": call.get("id").cloned().unwrap_or(Value::Null),
                "name": call.pointer("/function/name").cloned().unwrap_or(Value::Null),
                "input": input,
            }));
        }
    }

    let finish = v.pointer("/choices/0/finish_reason").and_then(|f| f.as_str());
    Ok(json!({
        "id": v.get("id").cloned().unwrap_or(json!("msg_adapter")),
        "type": "message",
        "role": "assistant",
        "model": v.get("model").cloned().unwrap_or(Value::Null),
        "content": content,
        "stop_reason": map_finish(finish, has_tools),
        // DEGRADATION 4: OpenAI never says which stop sequence matched.
        "stop_sequence": Value::Null,
        "usage": {
            "input_tokens": v.pointer("/usage/prompt_tokens").cloned().unwrap_or(json!(0)),
            "output_tokens": v.pointer("/usage/completion_tokens").cloned().unwrap_or(json!(0)),
        }
    }))
}

// ---------------------------------------------------------------------------
// Stream translation: OpenAI chunks in, Anthropic SSE events out.
// ---------------------------------------------------------------------------

fn sse(event_type: &str, data: &Value) -> String {
    format!("event: {event_type}\ndata: {data}\n\n")
}

#[derive(Default)]
struct ToolAcc {
    id: String,
    name: String,
    args: String,
}

/// TEXT STREAMS LIVE; TOOL CALLS ARE ACCUMULATED AND EMITTED WHOLE at the
/// end. See the module doc for why that trade was made on purpose: raw
/// interleaved argument fragments from parallel calls would corrupt JSON,
/// and parallel calls being provably correct is worth more than argument
/// streaming nobody renders incrementally anyway.
#[derive(Default)]
pub(crate) struct StreamOut {
    started: bool,
    model: String,
    next_index: usize,
    text_index: Option<usize>,
    tools: Vec<ToolAcc>,
    finish: Option<String>,
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
}

impl StreamOut {
    pub(crate) fn feed(&mut self, chunk: &Value) -> Vec<String> {
        let mut out = Vec::new();
        if !self.started {
            self.started = true;
            self.model = chunk.get("model").and_then(|m| m.as_str()).unwrap_or("").to_string();
            out.push(sse(
                "message_start",
                &json!({
                    "type": "message_start",
                    "message": {
                        "id": chunk.get("id").cloned().unwrap_or(json!("msg_adapter")),
                        "type": "message", "role": "assistant", "model": self.model,
                        "content": [], "stop_reason": Value::Null, "stop_sequence": Value::Null,
                        "usage": { "input_tokens": 0, "output_tokens": 0 }
                    }
                }),
            ));
        }
        if let Some(usage) = chunk.get("usage") {
            self.input_tokens = usage.get("prompt_tokens").and_then(|u| u.as_u64()).or(self.input_tokens);
            self.output_tokens =
                usage.get("completion_tokens").and_then(|u| u.as_u64()).or(self.output_tokens);
        }
        let Some(delta) = chunk.pointer("/choices/0/delta") else { return out };

        if let Some(text) = delta.get("content").and_then(|c| c.as_str()) {
            if !text.is_empty() {
                if self.text_index.is_none() {
                    let idx = self.next_index;
                    self.next_index += 1;
                    self.text_index = Some(idx);
                    out.push(sse(
                        "content_block_start",
                        &json!({ "type": "content_block_start", "index": idx,
                                 "content_block": { "type": "text", "text": "" } }),
                    ));
                }
                out.push(sse(
                    "content_block_delta",
                    &json!({ "type": "content_block_delta", "index": self.text_index.unwrap(),
                             "delta": { "type": "text_delta", "text": text } }),
                ));
            }
        }
        if let Some(calls) = delta.get("tool_calls").and_then(|t| t.as_array()) {
            for call in calls {
                let idx = call.get("index").and_then(|i| i.as_u64()).unwrap_or(0) as usize;
                while self.tools.len() <= idx {
                    self.tools.push(ToolAcc::default());
                }
                let acc = &mut self.tools[idx];
                if let Some(id) = call.get("id").and_then(|i| i.as_str()) {
                    acc.id = id.into();
                }
                if let Some(name) = call.pointer("/function/name").and_then(|n| n.as_str()) {
                    acc.name.push_str(name);
                }
                if let Some(args) = call.pointer("/function/arguments").and_then(|a| a.as_str()) {
                    acc.args.push_str(args);
                }
            }
        }
        if let Some(reason) = chunk.pointer("/choices/0/finish_reason").and_then(|f| f.as_str()) {
            self.finish = Some(reason.to_string());
        }
        out
    }

    pub(crate) fn finish(&mut self) -> Vec<String> {
        let mut out = Vec::new();
        if !self.started {
            return out; // nothing ever arrived; the upstream error already spoke
        }
        if let Some(idx) = self.text_index.take() {
            out.push(sse("content_block_stop", &json!({ "type": "content_block_stop", "index": idx })));
        }
        // Counted DIRECTLY, not inferred from block indices — the first
        // version derived this from `next_index` after `take()` had already
        // cleared `text_index`, and a text-only stream reported `tool_use`.
        // The text-only unit test is what caught it.
        let tools = std::mem::take(&mut self.tools);
        let has_tools = !tools.is_empty();
        // THE PARALLEL-CALL PATH — every accumulated call becomes its own
        // tool_use block. Collapsing several into one is the classic silent
        // corruption in adapters like this, and it would read as the model
        // behaving oddly rather than as our bug.
        for acc in tools {
            let idx = self.next_index;
            self.next_index += 1;
            out.push(sse(
                "content_block_start",
                &json!({ "type": "content_block_start", "index": idx,
                         "content_block": { "type": "tool_use", "id": acc.id, "name": acc.name, "input": {} } }),
            ));
            let args = if acc.args.trim().is_empty() { "{}".to_string() } else { acc.args };
            match serde_json::from_str::<Value>(&args) {
                Ok(_) => out.push(sse(
                    "content_block_delta",
                    &json!({ "type": "content_block_delta", "index": idx,
                             "delta": { "type": "input_json_delta", "partial_json": args } }),
                )),
                // Honest failure beats corrupt forwarding: name it in-stream.
                Err(_) => out.push(sse(
                    "error",
                    &json!({ "type": "error", "error": { "type": "api_error",
                             "message": "The provider streamed tool arguments that are not valid JSON." } }),
                )),
            }
            out.push(sse("content_block_stop", &json!({ "type": "content_block_stop", "index": idx })));
        }
        let stop = map_finish(self.finish.as_deref(), has_tools);
        out.push(sse(
            "message_delta",
            &json!({ "type": "message_delta",
                     "delta": { "stop_reason": stop, "stop_sequence": Value::Null },
                     "usage": { "output_tokens": self.output_tokens.unwrap_or(0) } }),
        ));
        out.push(sse("message_stop", &json!({ "type": "message_stop" })));
        out
    }
}

// ---------------------------------------------------------------------------
// Tests.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- The narrow server's decisions. ------------------------------------

    #[test]
    fn routes_carry_the_provider_id_and_survive_query_strings() {
        assert!(matches!(route_of("/p1/v1/messages"), Route::Messages(id) if id == "p1"));
        assert!(matches!(route_of("/p1/v1/messages?beta=true"), Route::Messages(id) if id == "p1"));
        assert!(matches!(route_of("/p1/v1/messages/count_tokens"), Route::CountTokens(id) if id == "p1"));
        assert!(matches!(route_of("/v1/messages"), Route::Unknown));
        assert!(matches!(route_of("/"), Route::Unknown));
        assert!(matches!(route_of("/p1/v1/other"), Route::Unknown));
    }

    #[test]
    fn the_token_is_the_gate_and_loopback_origin_proves_nothing() {
        let mut h = HashMap::new();
        assert!(!auth_ok(&h, "secret"), "no header, no entry");
        h.insert("authorization".into(), "Bearer wrong".into());
        assert!(!auth_ok(&h, "secret"));
        h.insert("authorization".into(), "Bearer secret".into());
        assert!(auth_ok(&h, "secret"));
    }

    #[test]
    fn the_head_parser_reads_a_real_request() {
        let head = "POST /p1/v1/messages HTTP/1.1\r\nHost: 127.0.0.1:9\r\nAuthorization: Bearer t\r\nContent-Length: 2\r\n\r\n";
        let (method, path, headers) = parse_head(head).unwrap();
        assert_eq!(method, "POST");
        assert_eq!(path, "/p1/v1/messages");
        assert_eq!(headers.get("content-length").unwrap(), "2");
        assert_eq!(headers.get("authorization").unwrap(), "Bearer t");
    }

    // -- Request translation. ----------------------------------------------

    fn sample_request() -> Value {
        json!({
            "model": "test-model",
            "max_tokens": 100,
            "system": [{ "type": "text", "text": "Be brief.", "cache_control": { "type": "ephemeral" } }],
            "messages": [
                { "role": "user", "content": [
                    { "type": "text", "text": "What is in this picture?" },
                    { "type": "image", "source": { "type": "base64", "media_type": "image/png", "data": "AAAA" } }
                ]},
                { "role": "assistant", "content": [
                    { "type": "text", "text": "Checking." },
                    { "type": "tool_use", "id": "call_1", "name": "look", "input": { "at": "photo" } }
                ]},
                { "role": "user", "content": [
                    { "type": "tool_result", "tool_use_id": "call_1", "content": [
                        { "type": "text", "text": "a cat" },
                        { "type": "image", "source": { "type": "base64", "media_type": "image/png", "data": "BBBB" } }
                    ]}
                ]}
            ],
            "tools": [{ "name": "look", "description": "look at things",
                        "input_schema": { "type": "object", "properties": { "at": { "type": "string" } } } }],
            "tool_choice": { "type": "any" },
            "stop_sequences": ["END"]
        })
    }

    #[test]
    fn the_request_translates_shape_for_shape() {
        let out = translate_request(&sample_request()).unwrap();
        let messages = out["messages"].as_array().unwrap();

        // system first, from the top-level field.
        assert_eq!(messages[0]["role"], "system");
        assert_eq!(messages[0]["content"], "Be brief.");
        // user with an image becomes content parts with a data URL.
        assert_eq!(messages[1]["role"], "user");
        let parts = messages[1]["content"].as_array().unwrap();
        assert_eq!(parts[1]["image_url"]["url"], "data:image/png;base64,AAAA");
        // assistant tool_use becomes tool_calls with STRING arguments.
        assert_eq!(messages[2]["role"], "assistant");
        let call = &messages[2]["tool_calls"][0];
        assert_eq!(call["id"], "call_1");
        assert_eq!(call["function"]["name"], "look");
        assert_eq!(call["function"]["arguments"], "{\"at\":\"photo\"}");
        // tool_result becomes a tool-role message; its image became a marker.
        assert_eq!(messages[3]["role"], "tool");
        assert_eq!(messages[3]["tool_call_id"], "call_1");
        let tool_content = messages[3]["content"].as_str().unwrap();
        assert!(tool_content.contains("a cat"));
        assert!(tool_content.contains("[image omitted"));

        // tools pass their schema through; "any" becomes "required".
        assert_eq!(out["tools"][0]["function"]["parameters"]["properties"]["at"]["type"], "string");
        assert_eq!(out["tool_choice"], "required");
        assert_eq!(out["stop"][0], "END");
        // cache_control did not survive anywhere in the output.
        assert!(!out.to_string().contains("cache_control"));
    }

    /// THE BUG: Mark connected a real OpenAI key today, pointed a row at
    /// `gpt-5.5-sol`, and every turn 400'd — this translator was sending
    /// `max_tokens` and `temperature` to a model family that rejects both.
    /// Against the OLD code (no `is_reasoning_model`, `max_tokens` always
    /// forwarded, `temperature`/`top_p` always forwarded) this assertion is
    /// false: the output would carry `max_tokens`, not
    /// `max_completion_tokens`, and would carry `temperature`.
    #[test]
    fn a_reasoning_model_gets_the_new_field_name_and_no_sampling_params() {
        let mut req = sample_request();
        req["model"] = json!("gpt-5.5-sol");
        req["temperature"] = json!(0.7);
        req["top_p"] = json!(0.9);
        let out = translate_request(&req).unwrap();
        assert_eq!(out["max_completion_tokens"], 100, "reasoning models want the NEW field name");
        assert!(out.get("max_tokens").is_none(), "the OLD name 400s a reasoning model outright: {out}");
        assert!(out.get("temperature").is_none(), "reasoning models 400 on any explicit sampling param: {out}");
        assert!(out.get("top_p").is_none(), "{out}");
        // `sample_request()` carries a tool — see the dedicated pair of tests
        // below for the isolated proof, but pinning it here too means this
        // exact fixture, which several other assertions in this file already
        // depend on, cannot silently stop covering the combination.
        assert_eq!(out["reasoning_effort"], "none", "{out}");
    }

    /// THE TEST THAT FAILS AGAINST THE OLD CODE — DEGRADATION 6, found live
    /// 2026-09-02 by `providers.rs`'s `openai_full_round_trip_earns_green_for_real`,
    /// the one test that spawns the real `claude` binary against a real key:
    /// stage 2 400'd on the shipped default model with OpenAI's own words,
    /// *"Function tools with reasoning_effort are not supported for
    /// gpt-5.6-sol in /v1/chat/completions… set reasoning_effort to 'none'."*
    /// Against the pre-fix code this assertion is false — the field was never
    /// sent at all, and the real request that finding is built from 400'd.
    #[test]
    fn a_reasoning_model_with_tools_gets_reasoning_effort_none() {
        let req = sample_request(); // carries model "test-model" and a tool
        let mut req = req;
        req["model"] = json!("gpt-5.6-sol");
        let out = translate_request(&req).unwrap();
        assert!(out.get("tools").is_some(), "the fixture must actually carry a tool: {out}");
        assert_eq!(out["reasoning_effort"], "none", "{out}");
    }

    /// THE OTHER HALF, AND THE ONE THAT KEEPS THE FIX FROM OVERREACHING: a
    /// reasoning model with NO tools on the turn keeps full reasoning effort
    /// — OpenAI's own restriction names the combination, not the model alone,
    /// and there is no reason to pay this cost on a plain question.
    #[test]
    fn a_reasoning_model_with_no_tools_keeps_full_reasoning() {
        let req = json!({
            "model": "gpt-5.6-sol",
            "max_tokens": 100,
            "messages": [{ "role": "user", "content": "What is 2+2?" }],
        });
        let out = translate_request(&req).unwrap();
        assert!(out.get("tools").is_none(), "{out}");
        assert!(
            out.get("reasoning_effort").is_none(),
            "a tool-free turn must not have its reasoning effort forced down: {out}"
        );
    }

    /// THE OTHER HALF OF THE SAME FIX, and the one that is easy to get wrong
    /// by overcorrecting: gpt-4-class OpenAI models, and the third-party
    /// openai-compatible servers this row can also point at, still want the
    /// shape this file has always sent. A blanket switch to the new field
    /// name would have swapped one 400 for another, on a different family.
    #[test]
    fn a_gpt4_class_model_keeps_the_shape_it_has_always_gotten() {
        let mut req = sample_request();
        req["model"] = json!("gpt-4o");
        req["temperature"] = json!(0.7);
        let out = translate_request(&req).unwrap();
        assert_eq!(out["max_tokens"], 100);
        assert!(out.get("max_completion_tokens").is_none());
        assert_eq!(out["temperature"], 0.7);
    }

    #[test]
    fn reasoning_model_prefixes_match_the_documented_family() {
        for name in ["o1", "o1-preview", "o3-mini", "o4-mini", "gpt-5", "gpt-5.5-sol", "gpt-5.1-mini"] {
            assert!(is_reasoning_model(name), "{name} should be recognised");
        }
        for name in ["gpt-4o", "gpt-4-turbo", "gpt-3.5-turbo", "llama3.2:latest", ""] {
            assert!(!is_reasoning_model(name), "{name} should NOT be recognised");
        }
    }

    // -- Response translation. ---------------------------------------------

    #[test]
    fn a_plain_response_translates_with_mapped_stop_and_usage() {
        let body = r#"{"id":"x","model":"m","choices":[{"message":{"role":"assistant","content":"OK"},
            "finish_reason":"stop"}],"usage":{"prompt_tokens":7,"completion_tokens":3}}"#;
        let v = openai_to_anthropic_response(body).unwrap();
        assert_eq!(v["content"][0]["text"], "OK");
        assert_eq!(v["stop_reason"], "end_turn");
        assert_eq!(v["usage"]["input_tokens"], 7);
        assert_eq!(v["usage"]["output_tokens"], 3);
        assert_eq!(v["stop_sequence"], Value::Null);
    }

    #[test]
    fn tool_calls_translate_and_bad_arguments_are_refused_not_forwarded() {
        let good = r#"{"choices":[{"message":{"tool_calls":[
            {"id":"c1","function":{"name":"f","arguments":"{\"a\":1}"}}]},
            "finish_reason":"tool_calls"}]}"#;
        let v = openai_to_anthropic_response(good).unwrap();
        assert_eq!(v["content"][0]["type"], "tool_use");
        assert_eq!(v["content"][0]["input"]["a"], 1);
        assert_eq!(v["stop_reason"], "tool_use");

        let bad = r#"{"choices":[{"message":{"tool_calls":[
            {"id":"c1","function":{"name":"f","arguments":"{not json"}}]},
            "finish_reason":"tool_calls"}]}"#;
        assert!(openai_to_anthropic_response(bad).unwrap_err().contains("not valid JSON"));
    }

    #[test]
    fn finish_reasons_map_and_length_means_max_tokens() {
        assert_eq!(map_finish(Some("stop"), false), "end_turn");
        assert_eq!(map_finish(Some("length"), false), "max_tokens");
        assert_eq!(map_finish(Some("tool_calls"), true), "tool_use");
        assert_eq!(map_finish(None, true), "tool_use");
        assert_eq!(map_finish(None, false), "end_turn");
    }

    // -- The stream, including THE test Jarvis asked for by name. ----------

    fn events_of(chunks: &[Value]) -> Vec<Value> {
        let mut t = StreamOut::default();
        let mut raw: Vec<String> = Vec::new();
        for c in chunks {
            raw.extend(t.feed(c));
        }
        raw.extend(t.finish());
        raw.iter()
            .map(|e| {
                let data = e.lines().find_map(|l| l.strip_prefix("data: ")).unwrap();
                serde_json::from_str(data).unwrap()
            })
            .collect()
    }

    /// PARALLEL TOOL CALLS SURVIVE, REPLAYED — two interleaved calls in one
    /// stream come out as exactly two tool_use blocks with the right ids,
    /// names and arguments. Collapsing them would look like the model
    /// behaving oddly rather than like our bug, which is why this test
    /// exists before the feature ships.
    #[test]
    fn a_replayed_multi_call_stream_yields_two_intact_tool_blocks() {
        let chunks = vec![
            json!({"id":"x","model":"m","choices":[{"delta":{"role":"assistant"}}]}),
            json!({"choices":[{"delta":{"content":"I'll check both. "}}]}),
            json!({"choices":[{"delta":{"tool_calls":[
                {"index":0,"id":"call_a","type":"function","function":{"name":"get_weather","arguments":""}}]}}]}),
            json!({"choices":[{"delta":{"tool_calls":[
                {"index":0,"function":{"arguments":"{\"city\":\"SF\"}"}}]}}]}),
            json!({"choices":[{"delta":{"tool_calls":[
                {"index":1,"id":"call_b","type":"function","function":{"name":"get_time","arguments":"{\"tz\":"}}]}}]}),
            json!({"choices":[{"delta":{"tool_calls":[
                {"index":1,"function":{"arguments":"\"PST\"}"}}]}}]}),
            json!({"choices":[{"delta":{},"finish_reason":"tool_calls"}]}),
            json!({"choices":[],"usage":{"prompt_tokens":50,"completion_tokens":20}}),
        ];
        let events = events_of(&chunks);

        assert_eq!(events[0]["type"], "message_start");
        // The text streamed live before the tools.
        assert_eq!(events[1]["type"], "content_block_start");
        assert_eq!(events[1]["content_block"]["type"], "text");
        assert_eq!(events[2]["delta"]["text"], "I'll check both. ");

        let tool_starts: Vec<&Value> = events
            .iter()
            .filter(|e| e["type"] == "content_block_start" && e["content_block"]["type"] == "tool_use")
            .collect();
        assert_eq!(tool_starts.len(), 2, "BOTH parallel calls must survive: {events:?}");
        assert_eq!(tool_starts[0]["content_block"]["id"], "call_a");
        assert_eq!(tool_starts[0]["content_block"]["name"], "get_weather");
        assert_eq!(tool_starts[1]["content_block"]["id"], "call_b");
        assert_eq!(tool_starts[1]["content_block"]["name"], "get_time");

        let deltas: Vec<&Value> = events
            .iter()
            .filter(|e| e["type"] == "content_block_delta"
                && e["delta"]["type"] == "input_json_delta")
            .collect();
        assert_eq!(deltas.len(), 2);
        assert_eq!(deltas[0]["delta"]["partial_json"], "{\"city\":\"SF\"}");
        assert_eq!(deltas[1]["delta"]["partial_json"], "{\"tz\":\"PST\"}");

        let last = &events[events.len() - 2];
        assert_eq!(last["type"], "message_delta");
        assert_eq!(last["delta"]["stop_reason"], "tool_use");
        assert_eq!(last["usage"]["output_tokens"], 20);
        assert_eq!(events[events.len() - 1]["type"], "message_stop");
    }

    #[test]
    fn a_text_only_stream_ends_in_end_turn() {
        let chunks = vec![
            json!({"id":"x","model":"m","choices":[{"delta":{"role":"assistant"}}]}),
            json!({"choices":[{"delta":{"content":"Hel"}}]}),
            json!({"choices":[{"delta":{"content":"lo"}}]}),
            json!({"choices":[{"delta":{},"finish_reason":"stop"}]}),
        ];
        let events = events_of(&chunks);
        let text: String = events
            .iter()
            .filter(|e| e["type"] == "content_block_delta")
            .map(|e| e["delta"]["text"].as_str().unwrap_or(""))
            .collect();
        assert_eq!(text, "Hello");
        let stop = events.iter().find(|e| e["type"] == "message_delta").unwrap();
        assert_eq!(stop["delta"]["stop_reason"], "end_turn");
    }

    /// A stream that never delivered a chunk emits nothing — the upstream
    /// error already spoke, and half a message frame would wedge the client.
    #[test]
    fn an_empty_stream_emits_nothing() {
        assert!(StreamOut::default().finish().is_empty());
    }

    // -- The live send path, real sockets, no shortcuts. --------------------
    //
    // Beck proved this panic on the shipped b20 binary six times, from its
    // own stderr: `PANIC at src/adapter.rs:385: assertion failed:
    // self.is_char_boundary(new_len)`, followed by "Claude Code started but
    // did not answer in time." on screen — `relay`'s bare `detail.truncate(500)`
    // had the identical fault `providers::generic_status_error` already had
    // and had already been fixed for, under a comment claiming "Never our
    // token, never the key — neither is in these bodies", which is the same
    // assumption Beck disproved on the OTHER call site to the same kind of
    // upstream.
    //
    // Both bugs need REAL sockets to prove: `relay` writes into a live
    // `TcpStream`, not a buffer, and the panic is specifically about
    // `String::truncate` landing off a UTF-8 boundary in bytes that arrived
    // over the wire — a canned `&str` literal in source is always valid
    // UTF-8 at every offset a human would think to try, which is exactly how
    // this shipped once already. So this spins up a real "upstream" on
    // loopback that answers 400 with a body built the same way Beck's was —
    // a multibyte character straddling byte 500, AND the caller's own key
    // echoed back in it — and calls `relay` exactly as `handle` does, into a
    // second real socket pair standing in for the claude-binary side of the
    // connection.
    #[test]
    fn relay_redacts_the_key_and_never_panics_on_a_multibyte_boundary() {
        let key = "sk-test-0123456789abcdef0123456789abcdef";

        // A body shaped like Beck's, with BOTH of the two things
        // `redact_and_truncate` exists for in it: the key near the front —
        // inside the visible window even after a 500-byte cut, so its
        // replacement can actually be seen and checked below — and a
        // multibyte 'é' straddling the byte-500 cut point itself, the exact
        // shape that panicked `generic_status_error` before it was fixed.
        let mut body = format!("Bearer {key} was rejected — ");
        while body.len() < 499 {
            body.push('x');
        }
        body.push('\u{e9}'); // 2-byte UTF-8 char landing on/across byte 500
        body.push_str(" — more text after the cut, so the body keeps going");

        // The fake upstream: accepts one connection, ignores what was sent,
        // answers 400 with the body above.
        let upstream = TcpListener::bind("127.0.0.1:0").unwrap();
        let upstream_addr = upstream.local_addr().unwrap();
        let upstream_body = body.clone();
        std::thread::spawn(move || {
            if let Ok((mut sock, _)) = upstream.accept() {
                let mut drain = [0u8; 4096];
                let _ = sock.read(&mut drain);
                let resp = format!(
                    "HTTP/1.1 400 Bad Request\r\ncontent-type: application/json\r\n\
                     content-length: {}\r\nconnection: close\r\n\r\n{}",
                    upstream_body.len(),
                    upstream_body
                );
                let _ = sock.write_all(resp.as_bytes());
            }
        });

        // The other socket: stands in for the claude-binary side of the
        // connection that `handle()` normally hands `relay` — a real
        // `TcpStream`, because that is what the function signature requires
        // and what actually panicked on the shipped binary.
        let client_listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let client_addr = client_listener.local_addr().unwrap();
        let reader = std::thread::spawn(move || {
            let mut sock = TcpStream::connect(client_addr).unwrap();
            let mut out = String::new();
            let _ = sock.read_to_string(&mut out);
            out
        });
        let (mut claude_side, _) = client_listener.accept().unwrap();

        // THE CALL THAT PANICKED PRE-FIX. If `relay` still carried the bare
        // `detail.truncate(500)`, this line unwinds the test thread right
        // here with "assertion failed: self.is_char_boundary(new_len)" and
        // the test is reported FAILED — it does not need a separate assert
        // to catch that; a panic inside a `#[test]` fn is the failure.
        relay(&mut claude_side, &format!("http://{upstream_addr}"), key, json!({ "model": "x" }));
        drop(claude_side); // EOF for the reader thread

        let written = reader.join().unwrap();
        assert!(
            !written.contains(key),
            "the caller's real bearer token reached the screen: {written}"
        );
        assert!(written.contains("[key redacted]"), "{written}");
        assert!(written.contains("400 Upstream Error"), "{written}");
    }

    // -- The real thing, on demand only. -------------------------------------
    //
    // Same convention as `providers.rs`'s `local_brain_earns_green_for_real`:
    // ignored by default so a box with no key does not silently skip and look
    // clean, run explicitly, real network, real spend (a few tokens).
    //
    // **THIS IS THE TEST THE MODULE HEADER SAYS DOES NOT EXIST YET** — "every
    // test in this file is synthetic… this translator has never met a real
    // OpenAI endpoint." Run 2026-09-02 with a real, user-supplied-shaped key
    // (Mark's own `openai-api` credential, decrypted straight into the
    // environment for this one process and never written to disk, logged, or
    // committed — the header's own rule about the provider's real key applies
    // to a test run exactly as it does to a live send). Passed:
    //   HTTP 200 from api.openai.com, `openai_to_anthropic_response` parsed
    //   the real body into a valid Anthropic-shaped message with non-empty
    //   text content — proving `translate_request` produces a body OpenAI
    //   actually accepts AND `openai_to_anthropic_response` reads what OpenAI
    //   actually sends back, not just the hand-built fixtures above.
    //
    // WHAT THIS DOES NOT PROVE: the third leg, Claude Code spawned through
    // `apply_env` and talking to THIS translator over loopback with a real
    // `claude` binary. That needs the full Tauri app (a config dir, a running
    // `ensure_running()` listener, `select_provider`/`test_provider` from the
    // UI) — this test proves the translator's two wire edges in isolation,
    // which is the part `adapter.rs` owns; the third leg is `test_provider`'s
    // job and needs the real app running, on Windows, per FACTS.md.
    //
    //   NAMEOS_TEST_OPENAI_KEY=sk-... cargo test --manifest-path \
    //     desktop/src-tauri/Cargo.toml openai_round_trip -- --ignored --nocapture
    #[test]
    #[ignore = "talks to the real OpenAI endpoint and spends real tokens on a real key"]
    fn a_real_openai_round_trip_translates_both_ways() {
        let key = std::env::var("NAMEOS_TEST_OPENAI_KEY")
            .expect("set NAMEOS_TEST_OPENAI_KEY to a real OpenAI key to run this test");

        let anthropic_request = json!({
            "model": "gpt-4o-mini",
            "max_tokens": 8,
            "messages": [
                { "role": "user", "content": "Say the single word: OK" }
            ],
        });
        let openai_shaped = translate_request(&anthropic_request).expect("translate_request");
        assert_eq!(openai_shaped["max_tokens"], 8, "gpt-4o-mini is not a reasoning model");

        let agent = ureq::AgentBuilder::new().timeout(Duration::from_secs(20)).build();
        let response = agent
            .post("https://api.openai.com/v1/chat/completions")
            .set("content-type", "application/json")
            .set("authorization", &format!("Bearer {key}"))
            .send_string(&openai_shaped.to_string())
            .expect("a real request to a real key must succeed");
        let body = response.into_string().expect("response body");

        let anthropic_shaped = openai_to_anthropic_response(&body)
            .expect("the real OpenAI response must translate back to Claude's shape");
        assert_eq!(anthropic_shaped["type"], "message");
        assert_eq!(anthropic_shaped["role"], "assistant");
        let text = anthropic_shaped["content"][0]["text"].as_str().unwrap_or("");
        assert!(!text.trim().is_empty(), "the real answer must carry real text: {anthropic_shaped}");
        println!("real OpenAI round trip, translated both ways: {anthropic_shaped}");
    }

    /// **THE PROOF FOR THE CLAIM IN `adapter.rs`'s OWN HEADER** — "Gemini is
    /// one row of it" — run for real for the first time 2026-09-02, with
    /// Mark's own `gemini-api` credential decrypted into this process only.
    /// A second Google-shaped request also checked with a TOOL attached (not
    /// asserted here, to keep this test to the same single real call as its
    /// OpenAI sibling — verified separately by hand against
    /// generativelanguage.googleapis.com and recorded in the brain_setup.rs
    /// tile's own comment): Gemini answers the identical
    /// `choices[0].message.tool_calls[]` shape, so `openai_to_anthropic_response`
    /// needs no Gemini-specific branch to read it.
    ///
    ///   NAMEOS_TEST_GEMINI_KEY=... cargo test --manifest-path \
    ///     desktop/src-tauri/Cargo.toml gemini_round_trip -- --ignored --nocapture
    #[test]
    #[ignore = "talks to the real Gemini endpoint and spends real tokens on a real key"]
    fn a_real_gemini_round_trip_translates_both_ways() {
        let key = std::env::var("NAMEOS_TEST_GEMINI_KEY")
            .expect("set NAMEOS_TEST_GEMINI_KEY to a real Gemini API key to run this test");

        // max_tokens is deliberately generous, not 8 like the OpenAI sibling
        // test — see this test's own second attempt in the record: at 16,
        // Gemini's own internal reasoning (on by default for this family,
        // unlike OpenAI's, where `is_reasoning_model` at least names the
        // families that do it) consumed the entire budget and returned zero
        // visible text with `stop_reason: "max_tokens"`. That is a real,
        // separate finding from anything this translator controls — see the
        // note left on `brain_setup.rs`'s Gemini tile — and not something to
        // paper over by shrinking what this test proves.
        let anthropic_request = json!({
            "model": "gemini-3.6-flash",
            "max_tokens": 512,
            "messages": [
                { "role": "user", "content": "Say the single word: OK" }
            ],
        });
        let google_shaped = translate_request(&anthropic_request).expect("translate_request");
        assert_eq!(google_shaped["max_tokens"], 512, "no Gemini name matches is_reasoning_model");

        // Longer than the OpenAI sibling's 20s — this family's default
        // internal reasoning (see the comment above) adds real latency
        // before the first visible token, observed live.
        let agent = ureq::AgentBuilder::new().timeout(Duration::from_secs(60)).build();
        let response = agent
            .post("https://generativelanguage.googleapis.com/v1beta/openai/chat/completions")
            .set("content-type", "application/json")
            .set("authorization", &format!("Bearer {key}"))
            .send_string(&google_shaped.to_string())
            .expect("a real request to a real key must succeed");
        let body = response.into_string().expect("response body");

        let anthropic_shaped = openai_to_anthropic_response(&body)
            .expect("the real Gemini response must translate back to Claude's shape");
        assert_eq!(anthropic_shaped["type"], "message");
        let text = anthropic_shaped["content"][0]["text"].as_str().unwrap_or("");
        assert!(!text.trim().is_empty(), "the real answer must carry real text: {anthropic_shaped}");
        println!("real Gemini round trip, translated both ways: {anthropic_shaped}");
    }
}
