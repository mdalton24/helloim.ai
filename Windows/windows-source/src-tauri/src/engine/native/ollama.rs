//! Ollama, spoken natively. `POST {base}/api/chat`, NDJSON back, no auth.
//!
//! **THIS IS THE FIRST PROVIDER TO GET A NATIVE PATH AND THAT ORDER IS MARK'S —
//! *"lets connect individually first and then go from there."*** Ollama is the
//! cleanest possible proof of the whole point: no key, no account, and nothing
//! from Anthropic anywhere in the path. When this answers a turn, the strongest
//! claim the product has — *it runs on your own machine* — is finally true
//! without a 214 MB download bolted to the front of it.
//!
//! ## `/api/chat` rather than `/v1/messages`, and it is a deliberate reversal
//!
//! My own design note (2026-08-31) found that Ollama 0.33.1 also serves
//! Anthropic's `/v1/messages` shape, and recommended the internal format be
//! Anthropic's so that two of three providers need no translation. **That was
//! right for a tool-using engine and it is wrong here.** `/v1/messages` arrived
//! in Ollama **0.33.0**; `/api/chat` has been Ollama's own endpoint since long
//! before that. A chat engine that speaks the compatibility shim refuses to
//! work for anybody on an older Ollama, for no benefit at all — this engine has
//! no tools to translate, and a `{role, content}` list is the same list in both
//! shapes.
//!
//! `providers.rs` still TESTS the row against `/v1/messages` and still requires
//! 0.33 for that (`classify_local_404`). **That is a real seam and it is stated
//! rather than papered over: the Test button is stricter than the send path.**
//! Somebody on Ollama 0.32 would fail the Test and be refused a row that this
//! engine could actually have driven. Fixing it means changing what the Test
//! probes, which is a change to a shipped, working path and belongs in its own
//! piece of work — not smuggled in beneath an engine.
//!
//! ## What was measured on this box, by running it, not read on a page
//!
//! Ollama 0.33.1, 2026-08-31. Every one of these changed the code:
//!
//! - **`think: false` DOES NOT STOP A THINKING MODEL. It moves the reasoning
//!   into `content`.** `qwen3:4b` asked to "Say OK" with `think:false` streamed
//!   *"Hmm, the user wants me to say OK and nothing else. They specified…"* as
//!   its answer. **That is the exact shape of a local brain that looks broken —
//!   the failure this house has already shipped once** (b17, the red bubble on
//!   every local turn).
//! - **`think: true` fails outright on a model without the capability** —
//!   `qwen3-coder:30b` answers **400** `{"error":"\"qwen3-coder:30b\" does not
//!   support thinking"}`.
//! - **Omitting the field entirely is the only correct choice.** A thinking
//!   model then separates its reasoning into `message.thinking` on its own, and
//!   `content` carries the answer alone. So this request builder sends no
//!   `think` field, and there is a test that fails if one is ever added.
//! - An unknown model is **404** `{"error":"model 'x' not found"}`.
//! - A request with no messages is **200** with `done_reason:"load"` and empty
//!   content — a legitimate no-op, not an error.
//!
//! ## Tool calling — added 2026-09-03, measured against THIS box's own Ollama
//!
//! **THE MODULE HEADER USED TO SAY "THIS ENGINE HAS NO TOOLS TO TRANSLATE."
//! THAT IS NOW WRONG AND THIS SECTION REPLACES IT**, closing the gap
//! `engine::mod.rs`'s own header named ("Gemini/Anthropic/local translation
//! is a LATER increment") and matching `openai.rs`'s and `anthropic.rs`'s
//! own tool support. Verified LIVE against `127.0.0.1:11434` (this box's own
//! Ollama, 0.33.1, `qwen3:4b`, which the box's own `/api/tags` reports has
//! the `tools` capability) — a forced tool call, dispatched, and the replayed
//! result reaching a second real call and appearing in the final answer — not
//! assumed from `ollama/docs/api.md` alone, which turned out to be wrong on
//! one material point:
//!
//! - **`tool_calls` ARRIVES AS ONE COMPLETE ARRAY ON A SINGLE LINE**, never
//!   fragmented across several the way OpenAI's and Anthropic's streamed
//!   deltas are — confirmed against the real captured line. So there is no
//!   `PendingCall` accumulator here; `Chunk::ToolCalls` carries the whole
//!   thing, once.
//! - **`arguments` IS A JSON OBJECT, NOT A STRING** — the opposite of
//!   OpenAI's shape and the same as Anthropic's `tool_use.input`. This
//!   crate's own vendor-neutral `ToolCallRequest.args_json` is still a
//!   `String`, so it is serialised straight back out of the object Ollama
//!   sent, same reasoning `anthropic.rs`'s own module header gives for why
//!   that type is a string in the first place.
//! - **THE PUBLISHED DOCS SAY THERE IS NO `id` FIELD ON A TOOL CALL. THIS
//!   BOX'S OLLAMA DISAGREES** — the real line captured here carried
//!   `"id":"call_8atmg3k8"`. Read where present (so a genuinely-unique id is
//!   used when the model bothers to send one) and synthesised as
//!   `ollama-{index}` when it is not, rather than trusting either behaviour
//!   as a constant across versions.
//! - **REPLAY USES `tool_name`, NOT AN ID.** A tool result going back to
//!   Ollama is `{"role":"tool","content":"...","tool_name":"..."}` — proven
//!   by a real second round trip on this box, not read off a page: the
//!   model correctly used the replayed result in its final answer. So
//!   `build_request` keeps its own small `id -> name` lookup, filled in as
//!   it walks the message list and read back out the one time a
//!   `ToolTurn::Result` needs to name the tool it is answering for.
//! - **`done_reason` AFTER A TOOL CALL WAS `"stop"`, NOT SOMETHING NAMING
//!   TOOL USE** — Ollama has no dedicated stop reason for it the way
//!   Anthropic's `"tool_use"` or OpenAI's `"tool_calls"` are. So `stream`
//!   below tracks whether any `Chunk::ToolCalls` was seen in THIS turn and
//!   reports `StopReason::ToolUse` on that basis when the stream ends,
//!   regardless of what `done_reason` literally said.
//!
//! ## No `num_predict`, and that is also measured rather than assumed
//!
//! Capping the answer length is the obvious safety knob and it is the thing
//! that produces empty answers. With a small cap, a thinking model spends the
//! entire budget on `thinking` and returns `done_reason:"length"` with **no
//! content at all** — which is precisely `FACTS.md`'s record of `gpt-oss:20b`
//! never completing a turn. So nothing is capped here. **The Stop button is the
//! cap**, and the read timeout is the backstop. A `length` stop is still
//! handled, because a model's own configured limit can produce one.
//!
//! ## The boundary
//!
//! - **Who may call.** `engine::native`, in-process.
//! - **Wrong caller.** Not representable; `pub(crate)`, no command, no route.
//! - **Malformed input.** A line that is not JSON is skipped rather than fatal
//!   — Ollama has never sent one, and a stream that dies on an unexpected line
//!   is a stream that breaks on the next version. **A stream where NOTHING
//!   parsed is a different thing and is an error**, because that means we are
//!   not talking to Ollama at all.
//! - **What errors leak.** The base URL, the model name and Ollama's own error
//!   text, capped and run through `providers::redact_and_truncate` — the same
//!   helper `adapter::relay` and `providers::generic_status_error` share.
//!   Ollama on loopback carries no key today; using the shared helper now is
//!   what stops this growing its own un-redacted copy the way `relay` once did.

use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::time::Duration;

use serde_json::{json, Value};

use super::{
    Completion, Delta, ErrorKind, Flow, ModelCall, StopReason, ToolCallRequest, ToolDef, ToolTurn,
    TurnError, Wire,
};

/// Connect fast, read patiently.
///
/// The connect timeout is short because a dead port should be reported in
/// seconds, not minutes. The read timeout is long for one specific reason:
/// **a cold Ollama pulls the whole model into VRAM before the first token**,
/// and 17 GB takes real time. `providers.rs` already carries this lesson at
/// `LOCAL_TEST_TIMEOUT` — false-failing a cold load teaches people to distrust
/// the app, which is worse than waiting.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const READ_TIMEOUT: Duration = Duration::from_secs(300);

/// Cap on any upstream error text we repeat back. Same 500 as
/// `generic_status_error` and `relay`, deliberately.
const ERROR_BODY_CAP: usize = 500;

pub(crate) struct OllamaWire;

/// One decoded NDJSON line.
#[derive(Debug, PartialEq)]
pub(crate) enum Chunk {
    /// Answer text. May be empty on a line that only carried thinking.
    Content(String),
    /// The model's reasoning. **Kept separate from the answer and never
    /// concatenated into it** — see the module header for what happens when
    /// these two are allowed to mix.
    Thinking(String),
    /// The final line. Carries why it stopped and the token counts.
    Done { reason: String, input_tokens: Option<u64>, output_tokens: Option<u64> },
    /// The model asked to call tools — **the WHOLE array, arrived on one
    /// line**. See the module header: Ollama does not fragment this the way
    /// OpenAI's and Anthropic's streamed deltas do, confirmed against a real
    /// captured line rather than assumed.
    ToolCalls(Vec<ToolCallRequest>),
    /// A well-formed line carrying nothing we use.
    Nothing,
}

fn tool_to_json(t: &ToolDef) -> Value {
    // Same shape `openai.rs`'s own `tool_to_json` sends, minus `strict` —
    // Ollama's tool schema has no strict-mode concept to opt into (nothing
    // in `ollama/docs/api.md` or the real captured request/response
    // mentions one), so the field is simply not sent rather than sent and
    // hoped to be ignored.
    json!({
        "type": "function",
        "function": {
            "name": t.name,
            "description": t.description,
            "parameters": t.parameters,
        }
    })
}

/// The request body. Pure, so the decisions in it are provable without a
/// socket: **the system prompt is a message with role `system`**, **there is
/// no `think` field**, and **a tool result is replayed as `{"role":"tool",
/// "content":..., "tool_name":...}`** — see the module header for why
/// `tool_name` and not an id.
pub(crate) fn build_request(call: &ModelCall<'_>) -> Value {
    let mut messages: Vec<Value> = Vec::new();
    if !call.system.trim().is_empty() {
        messages.push(json!({ "role": "system", "content": call.system }));
    }
    // Filled in as `ToolTurn::Calls` messages are walked, and read back out
    // the one time a later `ToolTurn::Result` needs to name the tool it
    // answers for -- `drive`'s own loop always pushes a call's `Calls`
    // message before the matching `Result`(s), so the name is always
    // already known by the time it is needed.
    let mut name_of: HashMap<&str, &str> = HashMap::new();
    for m in call.messages {
        match &m.tool {
            None => messages.push(json!({ "role": m.role.wire(), "content": m.text })),
            Some(ToolTurn::Calls(calls)) => {
                let tool_calls: Vec<Value> = calls
                    .iter()
                    .map(|c| {
                        name_of.insert(c.id.as_str(), c.name.as_str());
                        // `arguments` is the parsed OBJECT, not the raw
                        // string -- see the module header. Falls back to
                        // `{}` on a malformed string for the same reason
                        // `anthropic.rs`'s own build_request does: `dispatch`
                        // already accepted this exact string once on the way
                        // out, so refusing to replay it here would strand
                        // the conversation on a failure this wire itself
                        // never raised.
                        let arguments: Value =
                            serde_json::from_str(&c.args_json).unwrap_or_else(|_| json!({}));
                        json!({ "id": c.id, "function": { "name": c.name, "arguments": arguments } })
                    })
                    .collect();
                messages.push(json!({
                    "role": "assistant",
                    "content": m.text,
                    "tool_calls": tool_calls,
                }));
            }
            Some(ToolTurn::Result { call_id, is_error }) => {
                let name = name_of.get(call_id.as_str()).copied().unwrap_or("");
                let content = if *is_error { format!("Error: {}", m.text) } else { m.text.clone() };
                messages.push(json!({ "role": "tool", "content": content, "tool_name": name }));
            }
        }
    }
    let mut body = json!({
        "model": call.model,
        "messages": messages,
        "stream": true,
    });
    // Empty means "do not offer tools" -- see `ModelCall.tools`'s own doc.
    // Omitted entirely rather than sent as `"tools": []`, same reasoning
    // `openai.rs`'s and `anthropic.rs`'s own request builders give.
    if !call.tools.is_empty() {
        let tools: Vec<Value> = call.tools.iter().map(tool_to_json).collect();
        body["tools"] = json!(tools);
    }
    body
}

/// Decode one line of the NDJSON stream. `None` for a line that is not JSON.
pub(crate) fn parse_line(line: &str) -> Option<Chunk> {
    let v: Value = serde_json::from_str(line.trim()).ok()?;

    // Ollama can answer 200 and then report a problem inside the stream. It is
    // rare and it is not a parse failure, so it gets its own shape rather than
    // being silently treated as "nothing".
    if let Some(err) = v.get("error").and_then(|e| e.as_str()) {
        return Some(Chunk::Done {
            reason: format!("error:{err}"),
            input_tokens: None,
            output_tokens: None,
        });
    }

    if v.get("done").and_then(|d| d.as_bool()).unwrap_or(false) {
        return Some(Chunk::Done {
            reason: v
                .get("done_reason")
                .and_then(|r| r.as_str())
                .unwrap_or("stop")
                .to_string(),
            input_tokens: v.get("prompt_eval_count").and_then(|n| n.as_u64()),
            output_tokens: v.get("eval_count").and_then(|n| n.as_u64()),
        });
    }

    // Checked before content/thinking: the real captured line has an empty
    // `content` alongside `tool_calls`, but reading this first costs nothing
    // and means a future Ollama version that DID put text on the same line
    // is still handled rather than silently losing the tool call.
    if let Some(calls) = v.pointer("/message/tool_calls").and_then(|c| c.as_array()) {
        if !calls.is_empty() {
            let parsed: Vec<ToolCallRequest> = calls
                .iter()
                .enumerate()
                .map(|(i, c)| {
                    let id = c
                        .get("id")
                        .and_then(|s| s.as_str())
                        .map(str::to_string)
                        // Not every version sends one -- see the module
                        // header. Synthesised rather than left blank so
                        // `ToolTurn::Result { call_id, .. }` always has
                        // something non-empty to carry.
                        .unwrap_or_else(|| format!("ollama-{i}"));
                    let name =
                        c.pointer("/function/name").and_then(|s| s.as_str()).unwrap_or("").to_string();
                    let args =
                        c.pointer("/function/arguments").cloned().unwrap_or_else(|| json!({}));
                    ToolCallRequest { id, name, args_json: args.to_string(), thought_signature: None }
                })
                .collect();
            return Some(Chunk::ToolCalls(parsed));
        }
    }

    // Content wins over thinking when a line somehow carries both, because the
    // answer is what the person is waiting for. In practice Ollama sends one or
    // the other and content is empty while thinking streams.
    if let Some(text) = v.pointer("/message/content").and_then(|c| c.as_str()) {
        if !text.is_empty() {
            return Some(Chunk::Content(text.to_string()));
        }
    }
    if let Some(text) = v.pointer("/message/thinking").and_then(|c| c.as_str()) {
        if !text.is_empty() {
            return Some(Chunk::Thinking(text.to_string()));
        }
    }
    Some(Chunk::Nothing)
}

/// Turn Ollama's own words about a failure into something a person can act on.
///
/// **NOT A BARE STATUS CODE.** That fault has shown up three times in this repo
/// in a week — `test_binary` discarding stderr, `send`'s diagnostic thread
/// dropping a message's meaning, and `providers.rs:1351` throwing away the
/// body behind *"The endpoint answered 400."* Ollama's error bodies are short,
/// specific and written for the person who caused them, so they are repeated
/// verbatim (capped) rather than replaced with a code.
fn status_error(code: u16, body: String, model: &str, base: &str) -> TurnError {
    let detail = crate::providers::redact_and_truncate(body, "", ERROR_BODY_CAP);
    let said = serde_json::from_str::<Value>(&detail)
        .ok()
        .and_then(|v| v.get("error").and_then(|e| e.as_str()).map(str::to_string))
        .unwrap_or(detail);

    match code {
        404 => TurnError {
            kind: ErrorKind::ModelMissing,
            what: format!("Ollama is running, but it has no model called {model}."),
            fix: format!(
                "Install it once with `ollama pull {model}`, then send again. Nothing was \
                 sent anywhere else and nothing was charged."
            ),
        },
        400 => TurnError {
            kind: ErrorKind::BadRequest,
            what: format!("Ollama refused the request: {said}"),
            fix: "This is usually the model name or a setting it does not support. Open AI \
                  components, check the model, and test the connection again."
                .into(),
        },
        c if c >= 500 => TurnError {
            kind: ErrorKind::Upstream,
            what: format!("Ollama answered {c}: {said}"),
            fix: "That is a fault inside Ollama rather than anything you set. Worth trying \
                  again in a moment; restarting Ollama usually clears it."
                .into(),
        },
        c => TurnError {
            kind: ErrorKind::Upstream,
            what: format!("Ollama at {base} answered {c}: {said}"),
            fix: "Check that the address in AI components points at this computer's own \
                  Ollama, then test the connection again."
                .into(),
        },
    }
}

impl Wire for OllamaWire {
    fn label(&self) -> &'static str {
        "ollama"
    }

    fn stream(
        &self,
        call: &ModelCall<'_>,
        on: &mut dyn FnMut(Delta) -> Flow,
    ) -> Result<Completion, TurnError> {
        let base = call.base_url.trim().trim_end_matches('/');
        if !base.starts_with("http://") && !base.starts_with("https://") {
            return Err(TurnError {
                kind: ErrorKind::BadRequest,
                what: format!("The address saved for this brain is not a web address ({base})."),
                fix: "Open AI components and set it to your Ollama address — normally \
                      http://127.0.0.1:11434 — then test the connection."
                    .into(),
            });
        }
        let url = format!("{base}/api/chat");

        let agent = ureq::AgentBuilder::new()
            .timeout_connect(CONNECT_TIMEOUT)
            .timeout_read(READ_TIMEOUT)
            .build();

        // NO AUTHORIZATION HEADER. Ollama takes none, and sending a placeholder
        // bearer would be a credential-shaped string in a request that has no
        // credential in it -- the sort of thing that reads as a key to whoever
        // finds it in a log.
        let response = agent
            .post(&url)
            .set("content-type", "application/json")
            .send_string(&build_request(call).to_string());

        let upstream = match response {
            Ok(r) => r,
            Err(ureq::Error::Status(code, resp)) => {
                return Err(status_error(
                    code,
                    resp.into_string().unwrap_or_default(),
                    call.model,
                    base,
                ))
            }
            Err(ureq::Error::Transport(t)) => {
                return Err(TurnError {
                    kind: ErrorKind::Unreachable,
                    what: format!("Could not reach Ollama at {base} ({t})."),
                    fix: "Ollama has to be installed and running on this computer for a local \
                          brain to answer. Start it, then send again — nothing left your \
                          machine and nothing was charged."
                        .into(),
                })
            }
        };

        let mut text = String::new();
        let mut stop = StopReason::Other("the stream ended without saying why".into());
        let mut input_tokens = None;
        let mut output_tokens = None;
        let mut parsed_anything = false;
        // Collected across the whole stream, then announced once at the end
        // -- same order `openai.rs`'s and `anthropic.rs`'s own wires use,
        // and the same reason: a call can be seen and then have the
        // connection drop before `done` ever arrives, and reporting a call
        // nobody actually finished the turn on would send `drive` off to run
        // a tool the model's own turn never really completed asking for.
        let mut tool_calls: Vec<ToolCallRequest> = Vec::new();

        let mut reader = BufReader::new(upstream.into_reader());
        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) => break,
                Ok(_) => {}
                // A broken pipe mid-answer is not the same as no answer. What
                // arrived is real and the person is looking at it, so it is
                // kept and the stop reason says what happened.
                Err(e) => {
                    stop = StopReason::Other(format!("the connection to Ollama dropped ({e})"));
                    break;
                }
            }
            if line.trim().is_empty() {
                continue;
            }
            let Some(chunk) = parse_line(&line) else { continue };
            parsed_anything = true;
            match chunk {
                Chunk::Content(t) => {
                    text.push_str(&t);
                    if on(Delta::Text(t)) == Flow::Stop {
                        stop = StopReason::Cancelled;
                        break;
                    }
                }
                Chunk::Thinking(t) => {
                    if on(Delta::Thinking(t)) == Flow::Stop {
                        stop = StopReason::Cancelled;
                        break;
                    }
                }
                Chunk::ToolCalls(mut calls) => {
                    // No `on()` callback here -- same rule `openai.rs`'s and
                    // `anthropic.rs`'s own tool-call arms state: a call is
                    // announced once, after the turn is known to have
                    // actually ended on it, not the moment it is seen.
                    tool_calls.append(&mut calls);
                }
                Chunk::Done { reason, input_tokens: i, output_tokens: o } => {
                    input_tokens = i;
                    output_tokens = o;
                    stop = if !tool_calls.is_empty() {
                        // Ollama has no dedicated stop reason for a tool
                        // call -- see the module header: the real
                        // `done_reason` after one was measured as `"stop"`,
                        // not something naming tool use. Seeing a call at
                        // all is the more reliable signal.
                        StopReason::ToolUse
                    } else {
                        match reason.as_str() {
                            "stop" | "load" => StopReason::End,
                            "length" => StopReason::Length,
                            other => StopReason::Other(other.to_string()),
                        }
                    };
                    break;
                }
                Chunk::Nothing => {}
            }
        }

        // NOTHING PARSED IS A DIFFERENT FAILURE FROM AN EMPTY ANSWER, and
        // collapsing the two would report "the model said nothing" when the
        // truth is "that address is not Ollama".
        if !parsed_anything {
            return Err(TurnError {
                kind: ErrorKind::Protocol,
                what: format!("Something answered at {base}, but not in Ollama's language."),
                fix: "Check the address in AI components points at Ollama itself and not at \
                      another program on the same computer, then test the connection."
                    .into(),
            });
        }

        for call in &tool_calls {
            // The Stop button still applies to a turn that ended on a tool
            // call -- same reasoning `openai.rs`'s and `anthropic.rs`'s own
            // wires state: `drive` decides what to do with `Flow::Stop` the
            // same way it would for a cancelled plain answer, by not
            // dispatching.
            if on(Delta::ToolCall {
                id: call.id.clone(),
                name: call.name.clone(),
                args_json: call.args_json.clone(),
            }) == Flow::Stop
            {
                stop = StopReason::Cancelled;
                break;
            }
        }

        Ok(Completion { text, stop, input_tokens, output_tokens, tool_calls })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::native::store::{Message, Role};

    fn a_call<'a>(system: &'a str, messages: &'a [Message]) -> ModelCall<'a> {
        a_call_with_tools(system, messages, &[])
    }

    fn a_call_with_tools<'a>(
        system: &'a str,
        messages: &'a [Message],
        tools: &'a [ToolDef],
    ) -> ModelCall<'a> {
        ModelCall {
            base_url: "http://127.0.0.1:11434",
            model: "qwen3:4b",
            api_key: None,
            system,
            messages,
            tools,
        }
    }

    /// **THE `think` FIELD MUST NOT BE THERE, AND THIS IS THE TEST THAT SAYS
    /// SO.** Both values are wrong and the measurements are in the module
    /// header: `false` moves a thinking model's reasoning into the answer, and
    /// `true` is a 400 on a model without the capability. The only correct
    /// request is one that does not mention it.
    ///
    /// Proven able to fail: adding `"think": false` to `build_request` — the
    /// obvious, plausible thing to write — fails this immediately.
    #[test]
    fn the_request_never_mentions_think() {
        let msgs = vec![Message { role: Role::User, text: "hello".into(), tool: None }];
        let body = build_request(&a_call("be brief", &msgs));
        assert!(
            body.get("think").is_none(),
            "sending `think` at all breaks one model family or the other: {body}"
        );
        assert!(
            !body.to_string().contains("think"),
            "no field anywhere may set thinking: {body}"
        );
    }

    /// No answer cap. See the module header: a cap is what makes a thinking
    /// model return an empty answer, which is the `gpt-oss:20b` failure already
    /// in `FACTS.md`.
    #[test]
    fn the_request_does_not_cap_the_answer_length() {
        let msgs = vec![Message { role: Role::User, text: "hello".into(), tool: None }];
        let body = build_request(&a_call("", &msgs));
        assert!(body.get("options").is_none(), "no options block: {body}");
        assert!(!body.to_string().contains("num_predict"), "{body}");
    }

    /// The system prompt is the FIRST message and carries role `system`. A
    /// version that appended it, or folded it into the user's text, would put
    /// the voice card where the person's words should be.
    ///
    /// Proven able to fail: pushing the system message after the loop, or
    /// prefixing it onto the first user message, both fail here.
    #[test]
    fn the_system_prompt_leads_and_the_history_follows_in_order() {
        let msgs = vec![
            Message { role: Role::User, text: "first".into(), tool: None },
            Message { role: Role::Assistant, text: "second".into(), tool: None },
            Message { role: Role::User, text: "third".into(), tool: None },
        ];
        let body = build_request(&a_call("VOICE", &msgs));
        let m = body["messages"].as_array().unwrap();
        assert_eq!(m.len(), 4);
        assert_eq!(m[0]["role"], "system");
        assert_eq!(m[0]["content"], "VOICE");
        assert_eq!((m[1]["role"].as_str(), m[1]["content"].as_str()), (Some("user"), Some("first")));
        assert_eq!(
            (m[2]["role"].as_str(), m[2]["content"].as_str()),
            (Some("assistant"), Some("second"))
        );
        assert_eq!((m[3]["role"].as_str(), m[3]["content"].as_str()), (Some("user"), Some("third")));
        assert_eq!(body["stream"], true);
        assert_eq!(body["model"], "qwen3:4b");
    }

    /// An empty system prompt sends no system message at all, rather than an
    /// empty one. Some endpoints treat an empty system turn as a real
    /// instruction to say nothing.
    #[test]
    fn an_empty_system_prompt_sends_no_system_message() {
        let msgs = vec![Message { role: Role::User, text: "hi".into(), tool: None }];
        let body = build_request(&a_call("   ", &msgs));
        assert_eq!(body["messages"].as_array().unwrap().len(), 1);
        assert_eq!(body["messages"][0]["role"], "user");
    }

    /// **THE REAL BYTES, COPIED OFF THIS BOX'S OWN OLLAMA 0.33.1 ON
    /// 2026-08-31.** Not a shape imagined from documentation — these four lines
    /// were captured from a live `/api/chat` call and pasted here, which is the
    /// difference between a test of the wire and a test of my memory of it.
    #[test]
    fn the_real_captured_lines_decode_the_way_they_did_on_the_wire() {
        // A content token.
        assert_eq!(
            parse_line(
                r#"{"model":"qwen3:4b","created_at":"2026-09-01T01:45:49.507182209Z","message":{"role":"assistant","content":"Hmm"},"done":false}"#
            ),
            Some(Chunk::Content("Hmm".into()))
        );
        // A thinking token: content is present AND EMPTY, thinking carries the
        // text. A parser that keyed on `content` being present would report an
        // empty answer for the whole reasoning phase.
        assert_eq!(
            parse_line(
                r#"{"model":"qwen3:4b","created_at":"2026-09-01T01:45:58.628297108Z","message":{"role":"assistant","content":"","thinking":"Okay"},"done":false}"#
            ),
            Some(Chunk::Thinking("Okay".into()))
        );
        // The final line, with the counts.
        assert_eq!(
            parse_line(
                r#"{"model":"qwen3:4b","created_at":"2026-09-01T01:45:59.030272974Z","message":{"role":"assistant","content":""},"done":true,"done_reason":"length","total_duration":205192026,"load_duration":717297,"prompt_eval_count":12,"prompt_eval_duration":5267000,"eval_count":40,"eval_duration":194753000}"#
            ),
            Some(Chunk::Done {
                reason: "length".into(),
                input_tokens: Some(12),
                output_tokens: Some(40)
            })
        );
        // The no-op shape: a request with no messages answers 200 with this.
        assert_eq!(
            parse_line(
                r#"{"model":"qwen3:4b","created_at":"2026-09-01T01:46:15.258451933Z","message":{"role":"assistant","content":""},"done":true,"done_reason":"load"}"#
            ),
            Some(Chunk::Done { reason: "load".into(), input_tokens: None, output_tokens: None })
        );
    }

    /// A line that is not JSON is skipped, not fatal. A line that is JSON but
    /// carries nothing is `Nothing` — distinct from unparseable, because only
    /// one of the two means we might not be talking to Ollama.
    #[test]
    fn junk_is_skipped_and_an_empty_line_is_not_junk() {
        assert_eq!(parse_line("not json at all"), None);
        assert_eq!(parse_line(""), None);
        assert_eq!(parse_line(r#"{"model":"x","done":false}"#), Some(Chunk::Nothing));
        assert_eq!(
            parse_line(r#"{"message":{"role":"assistant","content":""},"done":false}"#),
            Some(Chunk::Nothing)
        );
    }

    /// An in-stream error becomes a Done that names it, rather than being
    /// mistaken for content or silently dropped.
    #[test]
    fn an_error_inside_the_stream_ends_the_turn_and_says_why() {
        assert_eq!(
            parse_line(r#"{"error":"model requires more system memory"}"#),
            Some(Chunk::Done {
                reason: "error:model requires more system memory".into(),
                input_tokens: None,
                output_tokens: None
            })
        );
    }

    /// **404 SENDS SOMEBODY TO `ollama pull`, NOT TO AN UPDATE.** This is the
    /// same fault Mason's `classify_local_404` was written to stop on the Test
    /// button, arriving on the send path: Ollama answers 404 for a missing
    /// model, and telling that person to update software that is already
    /// current fixes nothing and wastes their evening.
    ///
    /// Proven able to fail: routing 404 into the generic arm produces a message
    /// with no `ollama pull` in it.
    #[test]
    fn a_missing_model_names_the_command_that_fixes_it() {
        let e = status_error(
            404,
            r#"{"error":"model 'nope:9b' not found"}"#.into(),
            "nope:9b",
            "http://127.0.0.1:11434",
        );
        assert_eq!(e.kind, ErrorKind::ModelMissing);
        assert!(e.fix.contains("ollama pull nope:9b"), "{e:?}");
        assert!(e.what.contains("nope:9b"), "{e:?}");
    }

    /// **OLLAMA'S OWN WORDS SURVIVE, WHICH IS THE WHOLE POINT.** A 400 that
    /// reaches the person as "The endpoint answered 400." is the recurring
    /// fault of this week, in three separate files. The error text is unwrapped
    /// out of the JSON envelope so they read the sentence, not the packaging.
    ///
    /// Proven able to fail: replacing `said` with the status code alone.
    #[test]
    fn a_400_carries_the_reason_rather_than_the_number() {
        let e = status_error(
            400,
            r#"{"error":"\"qwen3-coder:30b\" does not support thinking"}"#.into(),
            "qwen3-coder:30b",
            "http://127.0.0.1:11434",
        );
        assert_eq!(e.kind, ErrorKind::BadRequest);
        assert!(e.what.contains("does not support thinking"), "{e:?}");
        assert!(!e.fix.is_empty(), "every error carries something to do next");
    }

    /// A body that is not JSON still reaches the person — capped, not dropped.
    /// A proxy's HTML error page is ugly and it is more useful than silence.
    #[test]
    fn a_non_json_error_body_is_repeated_capped_rather_than_discarded() {
        let e = status_error(502, "<html><body>Bad Gateway</body></html>".into(), "m", "http://x");
        assert!(e.what.contains("Bad Gateway"), "{e:?}");
        let e = status_error(503, "z".repeat(5000), "m", "http://x");
        assert!(e.what.len() < 700, "the body must be capped: {} chars", e.what.len());
    }

    /// Every error names what happened AND what to do. An error state is where
    /// honesty is worth most, and a sentence with no next step leaves somebody
    /// at a dead end.
    #[test]
    fn every_status_error_says_what_to_do_next() {
        for code in [400u16, 401, 404, 418, 500, 503] {
            let e = status_error(code, "{}".into(), "m", "http://127.0.0.1:11434");
            assert!(!e.what.trim().is_empty(), "{code} has no description");
            assert!(!e.fix.trim().is_empty(), "{code} has no fix hint");
        }
    }

    // -- tool calling, added 2026-09-03 --------------------------------------

    /// No `tools` field at all when none are offered -- same rule
    /// `openai.rs`'s and `anthropic.rs`'s own request builders hold, and
    /// checked here because omitting the field (rather than sending `[]`) is
    /// the decision this file makes for itself.
    #[test]
    fn no_tools_field_when_none_are_offered() {
        let msgs = vec![Message { role: Role::User, text: "hi".into(), tool: None }];
        let body = build_request(&a_call("", &msgs));
        assert!(body.get("tools").is_none(), "{body}");
    }

    /// The real shape a tool definition takes on the wire.
    #[test]
    fn a_tool_definition_is_sent_as_a_function_with_no_strict_field() {
        let t = ToolDef {
            name: "Read".into(),
            description: "read a file".into(),
            strict: true,
            parameters: json!({ "type": "object", "properties": {}, "required": [] }),
        };
        let msgs = vec![Message { role: Role::User, text: "hi".into(), tool: None }];
        let body = build_request(&a_call_with_tools("", &msgs, std::slice::from_ref(&t)));
        let sent = &body["tools"][0];
        assert_eq!(sent["type"], json!("function"));
        assert_eq!(sent["function"]["name"], json!("Read"));
        assert!(sent["function"].get("strict").is_none(), "{sent}");
    }

    /// **`arguments` GOES OUT AS A JSON OBJECT, NOT A STRING** — proven able
    /// to fail: sending `c.args_json` directly (a string) where the real
    /// endpoint expects an object fails this immediately. See the module
    /// header for why this is the opposite of `openai.rs`'s own shape.
    #[test]
    fn a_replayed_tool_calls_arguments_are_a_json_object_not_a_string() {
        let msgs = vec![Message {
            role: Role::Assistant,
            text: String::new(),
            tool: Some(ToolTurn::Calls(vec![ToolCallRequest {
                id: "call1".into(),
                name: "Read".into(),
                args_json: r#"{"path":"a.txt"}"#.into(),
                thought_signature: None,
            }])),
        }];
        let body = build_request(&a_call("", &msgs));
        let sent = &body["messages"][0]["tool_calls"][0];
        assert_eq!(sent["function"]["name"], json!("Read"));
        assert_eq!(sent["function"]["arguments"], json!({ "path": "a.txt" }));
    }

    /// **A TOOL RESULT REPLAYS BY `tool_name`, NEVER BY ID** — proven able to
    /// fail: sending `"tool_call_id": call_id` (OpenAI's own field name)
    /// instead fails this immediately. See the module header for the real
    /// round trip this was checked against.
    #[test]
    fn a_tool_results_replay_names_the_tool_not_an_id() {
        let msgs = vec![
            Message {
                role: Role::Assistant,
                text: String::new(),
                tool: Some(ToolTurn::Calls(vec![ToolCallRequest {
                    id: "call1".into(),
                    name: "get_weather".into(),
                    args_json: "{}".into(),
                    thought_signature: None,
                }])),
            },
            Message {
                role: Role::Tool,
                text: "18C and rainy".into(),
                tool: Some(ToolTurn::Result { call_id: "call1".into(), is_error: false }),
            },
        ];
        let body = build_request(&a_call("", &msgs));
        let sent = &body["messages"][1];
        assert_eq!(sent["role"], json!("tool"));
        assert_eq!(sent["content"], json!("18C and rainy"));
        assert_eq!(sent["tool_name"], json!("get_weather"));
        assert!(sent.get("tool_call_id").is_none(), "{sent}");
    }

    /// A failed tool's result is still readable prose, same convention
    /// `openai.rs`'s own build_request uses for the identical case (Chat
    /// Completions has no dedicated error field on a tool message; neither
    /// does Ollama's).
    #[test]
    fn a_failed_tool_results_content_says_so_in_the_text() {
        let msgs = vec![
            Message {
                role: Role::Assistant,
                text: String::new(),
                tool: Some(ToolTurn::Calls(vec![ToolCallRequest {
                    id: "call1".into(),
                    name: "Read".into(),
                    args_json: "{}".into(),
                    thought_signature: None,
                }])),
            },
            Message {
                role: Role::Tool,
                text: "not found".into(),
                tool: Some(ToolTurn::Result { call_id: "call1".into(), is_error: true }),
            },
        ];
        let body = build_request(&a_call("", &msgs));
        assert_eq!(body["messages"][1]["content"], json!("Error: not found"));
    }

    /// **THE REAL CAPTURED LINE, run against THIS box's own Ollama
    /// (127.0.0.1:11434, 0.33.1, `qwen3:4b`) 2026-09-03 — see the module
    /// header.** Pinned so this parser is checked against what actually
    /// arrived, including the `id` field the published docs say does not
    /// exist.
    #[test]
    fn the_real_captured_tool_call_line_decodes_the_way_it_did_on_the_wire() {
        let line = r#"{"model":"qwen3:4b","created_at":"2026-09-03T06:32:39.141439739Z","message":{"role":"assistant","content":"","tool_calls":[{"id":"call_8atmg3k8","function":{"index":0,"name":"get_weather","arguments":{"city":"Tokyo"}}}]},"done":false}"#;
        assert_eq!(
            parse_line(line),
            Some(Chunk::ToolCalls(vec![ToolCallRequest {
                id: "call_8atmg3k8".into(),
                name: "get_weather".into(),
                args_json: json!({ "city": "Tokyo" }).to_string(),
                thought_signature: None,
            }]))
        );
    }

    /// The immediately-following real line — `done:true`, `done_reason:
    /// "stop"`, empty content — the exact evidence the module header cites
    /// for why `stream` must not trust `done_reason` to name tool use.
    #[test]
    fn the_real_captured_line_after_a_tool_call_has_an_ordinary_stop_reason() {
        let line = r#"{"model":"qwen3:4b","created_at":"2026-09-03T06:32:39.152373339Z","message":{"role":"assistant","content":""},"done":true,"done_reason":"stop","total_duration":5174593292,"load_duration":3340383707,"prompt_eval_count":146,"prompt_eval_duration":63549000,"eval_count":349,"eval_duration":1758079000}"#;
        assert_eq!(
            parse_line(line),
            Some(Chunk::Done {
                reason: "stop".into(),
                input_tokens: Some(146),
                output_tokens: Some(349),
            })
        );
    }

    /// A tool call with no `id` field at all still decodes -- the shape the
    /// published docs describe, kept working alongside the shape this box's
    /// own Ollama actually sends.
    #[test]
    fn a_tool_call_with_no_id_field_gets_a_synthesised_one() {
        let line = r#"{"message":{"role":"assistant","content":"","tool_calls":[{"function":{"name":"get_weather","arguments":{"city":"Tokyo"}}}]},"done":false}"#;
        match parse_line(line) {
            Some(Chunk::ToolCalls(calls)) => {
                assert_eq!(calls.len(), 1);
                assert_eq!(calls[0].id, "ollama-0");
                assert_eq!(calls[0].name, "get_weather");
            }
            other => panic!("expected ToolCalls, got {other:?}"),
        }
    }

    // -- Real endpoint. `#[ignore]`d, same reason `engine::native::tests::
    // -- a_real_ollama_turn_answers_and_is_remembered` already is: it needs a
    // -- live Ollama and pulls a model into VRAM, so a plain `cargo test` must
    // -- not depend on either. --

    /// **THE TOOL LOOP'S LIVE PROOF FOR THIS WIRE — run for real 2026-09-03
    /// against THIS box's own Ollama** (see the module header for the
    /// captured evidence this is built from): a forced tool call, dispatched
    /// through the real `tools::dispatch`, replayed back in the exact shape
    /// `build_request` builds, and a second real call that reads the tool's
    /// answer and finishes in text.
    ///
    /// Run it with:
    ///
    /// ```text
    /// cargo test --manifest-path desktop/src-tauri/Cargo.toml \
    ///   engine::native::ollama::tests::a_real_ollama_tool_call_round_trip \
    ///   -- --ignored --nocapture
    /// ```
    #[test]
    #[ignore = "needs a live Ollama on 127.0.0.1:11434 with a tool-capable model — run with --ignored"]
    fn a_real_ollama_tool_call_round_trip() {
        let workdir = std::env::temp_dir()
            .join(format!("nameos-ollama-tool-live-{}", crate::engine::native::store::new_id()));
        std::fs::create_dir_all(&workdir).unwrap();
        std::fs::write(workdir.join("notes.txt"), "the secret number is 8214").unwrap();

        let read_tool = crate::engine::native::tools::definitions(true, false)
            .into_iter()
            .find(|t| t.name == "Read")
            .expect("the Read tool must be in the real definitions list");

        let round_one_msgs = vec![Message {
            role: Role::User,
            text: "Use the Read tool to read notes.txt, then tell me the secret number in it. \
                   Call the tool first; do not guess."
                .into(),
            tool: None,
        }];
        let call_one = a_call_with_tools("You are terse.", &round_one_msgs, std::slice::from_ref(&read_tool));
        let out_one = OllamaWire
            .stream(&call_one, &mut |_| Flow::Go)
            .expect("round one against the real local Ollama must succeed");
        assert_eq!(
            out_one.stop,
            StopReason::ToolUse,
            "the model did not call the tool: text={:?} calls={:?}",
            out_one.text,
            out_one.tool_calls
        );
        assert_eq!(out_one.tool_calls.len(), 1, "{:?}", out_one.tool_calls);
        let call_req = &out_one.tool_calls[0];
        assert_eq!(call_req.name, "Read");
        let parsed: Value =
            serde_json::from_str(&call_req.args_json).expect("real tool-call arguments must be valid JSON");
        assert_eq!(parsed["path"], "notes.txt", "args={}", call_req.args_json);
        println!("a_real_ollama_tool_call_round_trip: model asked for {call_req:?}");

        let result = crate::engine::native::tools::dispatch(&call_req.name, &call_req.args_json, &workdir, true, false);
        assert!(!result.is_error, "{}", result.output);
        assert!(result.output.contains("8214"), "{}", result.output);

        let round_two_msgs = vec![
            round_one_msgs.into_iter().next().unwrap(),
            Message {
                role: Role::Assistant,
                text: out_one.text.clone(),
                tool: Some(ToolTurn::Calls(out_one.tool_calls.clone())),
            },
            Message {
                role: Role::Tool,
                text: result.output.clone(),
                tool: Some(ToolTurn::Result { call_id: call_req.id.clone(), is_error: false }),
            },
        ];
        let call_two = a_call_with_tools("You are terse.", &round_two_msgs, std::slice::from_ref(&read_tool));
        let out_two = OllamaWire
            .stream(&call_two, &mut |_| Flow::Go)
            .expect("round two against the real local Ollama must succeed");
        assert_eq!(out_two.stop, StopReason::End, "text={:?}", out_two.text);
        assert!(
            out_two.text.contains("8214"),
            "the model did not use the real tool result in its answer: {:?}",
            out_two.text
        );
        println!("a_real_ollama_tool_call_round_trip: final answer = {:?}", out_two.text);

        let _ = std::fs::remove_dir_all(&workdir);
    }
}
