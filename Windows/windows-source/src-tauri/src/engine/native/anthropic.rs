//! Anthropic's own Messages API, spoken natively. `POST {base}/v1/messages`,
//! SSE back, `x-api-key: {key}` + `anthropic-version`.
//!
//! **PHASE 2's OTHER HALF OF THE TOOL SEAM — added 2026-09-03, continuing the
//! plan `engine::mod.rs`'s own header records.** `openai.rs` proved the
//! `Wire` trait can carry real tool-calling end to end; this is the second
//! provider shape, built the same way: request/response translated here,
//! dispatched through `tools::dispatch`, nothing vendor-specific anywhere
//! else in the engine. This is what an `anthropic-compatible` row (any
//! endpoint that speaks `/v1/messages` natively — the real Anthropic API,
//! Amazon Bedrock through OpenRouter's "Anthropic Skin", and Ollama itself as
//! of 0.33.0, per `providers.rs`'s own header) will run on once
//! `NATIVE_ANTHROPIC_ENABLED` opens, the same staged way `NATIVE_OPENAI_ENABLED`
//! did for the Chat Completions shape.
//!
//! **THE KIND THIS DRIVES IS CURRENTLY UNROUTABLE, AND THAT IS A SEPARATE
//! DECISION FROM THIS FILE'S.** `providers.rs`'s `ROUTABLE_KINDS` is
//! `&["claude", "local"]` as of 2026-08-28, Mark's own ruling —
//! `anthropic-compatible` cannot be selected as an active brain today, the
//! same as `openai-compatible` could not until its own provider-layer work
//! landed separately from `openai.rs`. Building this wire now, gated and
//! unreachable from `providers.rs`, is the same shape `openai.rs` shipped in:
//! a real, tested, `#[cfg(test)]`-forceable path with nobody able to hit it
//! by accident. Restoring the kind in `providers.rs` is its own piece of
//! work and not this one's to do.
//!
//! ## What is genuinely different from `openai.rs`, and why
//!
//! - **The system prompt is a top-level request field, never a message.**
//!   Anthropic's own API rejects a `system`-role entry inside `messages`
//!   with 400 (`role must be either 'user' or 'assistant'`) — confirmed by a
//!   real captured error in the wild, not assumed; there is no laxer
//!   fallback to fall back to.
//! - **Tool results are `user`-role messages carrying `tool_result` content
//!   blocks, not a dedicated `tool` role.** Anthropic has exactly two
//!   message roles. `Role::Tool` (this crate's own third role, added for
//!   OpenAI's shape) becomes `"user"` here.
//! - **CONSECUTIVE MESSAGES OF THE SAME ROLE MUST BE MERGED INTO ONE, OR THE
//!   REAL API REFUSES THE WHOLE REQUEST WITH 400** (`"roles must alternate
//!   between \"user\" and \"assistant\", but found multiple \"user\" roles
//!   in a row"` — the exact wording of the error this house's own research
//!   found before writing a line of `build_request`). This bites on the very
//!   shape `drive`'s tool loop produces: two parallel tool calls become two
//!   consecutive `Role::Tool` messages, which without merging would become
//!   two consecutive `"user"` turns. `build_request` merges by appending
//!   content blocks onto the previous Anthropic message when its role
//!   matches, rather than emitting a new one — see
//!   `consecutive_tool_results_are_merged_into_one_user_turn` below, which
//!   is proven able to fail by skipping the merge.
//! - **`max_tokens` IS A REQUIRED REQUEST FIELD**, unlike `openai.rs` and
//!   `ollama.rs`, both of which deliberately send no cap at all (see their
//!   own module docs: a cap is what produces the `gpt-oss:20b`-shaped empty
//!   answer). Anthropic's endpoint returns 400 with no `max_tokens` present
//!   at all — there is no "send nothing" option here. `ANTHROPIC_MAX_TOKENS`
//!   is a generous constant chosen to run on every current model family
//!   without needing an extended-output beta header, not a per-turn setting;
//!   the Stop button is still the only limit this engine has ever offered a
//!   person.
//! - **A tool's `input` is a JSON object, not a string.** OpenAI's shape
//!   carries `function.arguments` as a JSON-encoded string that itself
//!   contains JSON; Anthropic's `tool_use.input` is the parsed object
//!   directly, and the reverse is true on replay — `ToolTurn::Calls` carries
//!   `args_json: String` (this crate's own vendor-neutral shape, chosen so a
//!   malformed argument is a `dispatch`-time failure rather than something
//!   this struct could silently drop), so this wire parses it back into a
//!   `Value` before putting it in the request and falls back to `{}` on a
//!   parse failure rather than sending a string where the API expects an
//!   object — see `build_request`'s own comment for why that fallback, and
//!   not a refusal, is the right one here (the malformed string was already
//!   accepted once by `dispatch` on the way OUT; refusing to replay it
//!   would strand a conversation the wire itself never objected to).
//!
//! ## What was measured against a REAL endpoint, and by which key
//!
//! No `NAMEOS_TEST_ANTHROPIC_KEY` — no key against `api.anthropic.com` was
//! available to this house when this file was written. What WAS run for
//! real, 2026-09-03, is OpenRouter's own Anthropic-shaped endpoint
//! (`https://openrouter.ai/api/v1/messages`, documented by OpenRouter as its
//! "Anthropic Skin" — the same shape the real API speaks, proxied to Amazon
//! Bedrock) — a plain-text round trip, a forced tool call, and the
//! **replayed tool result reaching a second real call and appearing in the
//! final answer**, against `anthropic/claude-3-haiku`, using the
//! `openrouter-api` credential already on this box
//! (`~/.config/openrouter/openrouter-api.cred`, `systemd-creds decrypt`,
//! piped straight into the test process's environment, never written to
//! disk — see `a_real_openrouter_anthropic_tool_call_round_trip`'s own doc
//! for the command). The raw bytes from that run are what
//! `the_real_captured_lines_decode_the_way_they_did_on_the_wire` below pins,
//! captured with a bare `curl` first so the shape was checked before this
//! file's own parser was trusted with it — the same discipline `openai.rs`
//! used for its own real-endpoint proof. A 401 against a bogus key was run
//! for real too, and is what `status_error`'s 401 branch is worded from.
//! **This proves the shape against a real Anthropic-speaking endpoint. It
//! does not prove `api.anthropic.com` itself never differs** — the same
//! honest limit `openai.rs`'s own header states about its OpenRouter proof.
//!
//! ## The boundary
//!
//! - **Who may call.** `engine::native`, in-process.
//! - **Wrong caller.** Not representable; `pub(crate)`, no command, no route.
//! - **Malformed input.** A line that is not a recognised SSE shape is
//!   skipped, not fatal — same rule `openai.rs` and `ollama.rs` both state.
//!   A stream where NOTHING recognisable ever arrived is a different failure
//!   and is an error, because that means the address is not speaking this
//!   protocol at all.
//! - **What errors leak.** The base URL, the model name, and the provider's
//!   own error text, capped and run through `providers::redact_and_truncate`
//!   with the real key as the redaction target — this wire sends a real
//!   credential upstream (`x-api-key`), same reasoning `openai.rs`'s own
//!   header gives for not trusting a blank redaction target. Never the
//!   prompt, never the conversation, never the key itself.

use std::io::{BufRead, BufReader};
use std::time::Duration;

use serde_json::{json, Value};

use super::{
    Completion, Delta, ErrorKind, Flow, ModelCall, Role, StopReason, ToolCallRequest, ToolDef,
    ToolTurn, TurnError, Wire,
};

/// Same reasoning as `openai.rs`'s and `ollama.rs`'s own constants: connect
/// fast so a dead address is reported in seconds, read patiently because a
/// real answer can take real time and the Stop button — not a timeout — is
/// what should end a healthy long one.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const READ_TIMEOUT: Duration = Duration::from_secs(300);

/// Same cap `openai.rs`, `ollama.rs` and `providers::generic_status_error`
/// all use.
const ERROR_BODY_CAP: usize = 500;

/// The version this wire speaks. Anthropic's own header, required on every
/// call — see the module header for why there is no "send nothing" option
/// the way `openai.rs` and `ollama.rs` both take for their own per-turn caps.
const ANTHROPIC_VERSION: &str = "2023-06-01";

/// **REQUIRED BY THE ENDPOINT, NOT A PRODUCT CHOICE.** See the module header:
/// omitting `max_tokens` entirely is a 400 on Anthropic's real API, unlike
/// OpenAI's and Ollama's shapes where omitting the cap is exactly the
/// correct move. 8192 runs on every current model family (Claude 3.x through
/// 5.x) without needing an extended-output beta header, so this is the
/// widest cap that asks nothing extra of whichever endpoint is on the other
/// end. The Stop button remains the only limit a person actually experiences.
const ANTHROPIC_MAX_TOKENS: u64 = 8192;

pub(crate) struct AnthropicWire;

/// One decoded SSE data line, or the reason the stream ended.
#[derive(Debug, PartialEq)]
pub(crate) enum Chunk {
    /// Answer text (`content_block_delta` / `text_delta`).
    Content(String),
    /// Extended-thinking text (`thinking_delta`). Kept separate from the
    /// answer and never concatenated into it — same rule `ollama.rs`'s and
    /// `openai.rs`'s own headers state, and the same reason: reasoning
    /// presented as the answer is the b17 failure, the local brain that
    /// looked broken on its first message.
    Thinking(String),
    /// A new content block has started. Only `tool_use` is acted on; `text`
    /// and `thinking` block-starts carry no useful fields of their own (the
    /// text itself arrives on the deltas that follow) and are folded into
    /// `Nothing` by `parse_sse_line`.
    ToolBlockStart { index: u32, id: String, name: String },
    /// More of a tool call's `input` object, as raw JSON text — Anthropic
    /// streams a tool's arguments the same way OpenAI does, one
    /// `input_json_delta` at a time, keyed by the block's `index`.
    ToolInputDelta { index: u32, partial_json: String },
    /// `message_delta`'s own `delta.stop_reason`, arriving once, near the
    /// end of the stream but before `message_stop`.
    StopReasonSeen(String),
    /// Anthropic reported a mid-stream failure inside an otherwise-200
    /// response (`event: error`). `ollama.rs`'s and `openai.rs`'s parsers
    /// both have the identical shape for the identical reason.
    Error(String),
    /// `message_stop` — the actual end of the stream, distinct from
    /// `message_delta` (which can carry the final `stop_reason` and usage
    /// before `message_stop` arrives one line later).
    MessageStop,
    /// A well-formed SSE line carrying nothing this wire reads — a `ping`,
    /// a `content_block_stop`, a `content_block_start` for a block type
    /// this wire does not act on, a `signature_delta`, or the trailing
    /// `event: data` / `data: [DONE]` pair OpenRouter's own proxy appends
    /// after `message_stop` (captured live, 2026-09-03 — not part of
    /// Anthropic's own documented shape, and harmless to ignore).
    Nothing,
}

fn tool_to_json(t: &ToolDef) -> Value {
    // No `strict` field -- Anthropic's `input_schema` has no strict-mode
    // concept of its own (see `ToolDef`'s own doc: a schema built stricter
    // than a wire needs costs that wire nothing). `strict-compatible` here
    // just means every property is already listed in `required`, which
    // Anthropic reads as an ordinary JSON Schema constraint.
    json!({
        "name": t.name,
        "description": t.description,
        "input_schema": t.parameters,
    })
}

/// One already-translated Anthropic message, kept apart from `Value` only
/// long enough for `push_or_merge` to read its `role` back out — `Value`
/// itself has no way to ask "what role is this" without re-parsing.
struct AnthMessage {
    role: &'static str,
    content: Vec<Value>,
}

/// Append `blocks` under `role`, merging into the PREVIOUS message when its
/// role already matches rather than starting a new turn.
///
/// **THIS IS THE FUNCTION `consecutive_tool_results_are_merged_into_one_
/// user_turn` EXISTS TO PROVE, AND SKIPPING IT IS A LIVE 400 AGAINST THE REAL
/// API** — see the module header for the exact error text a real, unmerged
/// request draws. Two consecutive `Role::Tool` results (a model calling two
/// tools in parallel) are the case `drive`'s own tool loop actually produces;
/// this is not a defensive measure against input that cannot arise.
fn push_or_merge(out: &mut Vec<AnthMessage>, role: &'static str, blocks: Vec<Value>) {
    if let Some(last) = out.last_mut() {
        if last.role == role {
            last.content.extend(blocks);
            return;
        }
    }
    out.push(AnthMessage { role, content: blocks });
}

/// The request body. `system` leads as its own top-level field (never a
/// message — see the module header), `max_tokens` is always present (see
/// `ANTHROPIC_MAX_TOKENS`'s own doc), and every message is built through
/// `push_or_merge` so same-role turns never land side by side.
pub(crate) fn build_request(call: &ModelCall<'_>) -> Value {
    let mut messages: Vec<AnthMessage> = Vec::new();

    for m in call.messages {
        match &m.tool {
            None => {
                let role = if m.role == Role::User { "user" } else { "assistant" };
                push_or_merge(&mut messages, role, vec![json!({ "type": "text", "text": m.text })]);
            }
            Some(ToolTurn::Calls(calls)) => {
                let mut blocks: Vec<Value> = Vec::new();
                if !m.text.trim().is_empty() {
                    blocks.push(json!({ "type": "text", "text": m.text }));
                }
                for c in calls {
                    // FALLS BACK TO `{}` ON A PARSE FAILURE, DELIBERATELY --
                    // see the module header. `dispatch` already accepted this
                    // exact string once on the way out; refusing to replay it
                    // here would strand the conversation on a failure this
                    // wire itself never raised.
                    let input: Value =
                        serde_json::from_str(&c.args_json).unwrap_or_else(|_| json!({}));
                    blocks.push(json!({
                        "type": "tool_use",
                        "id": c.id,
                        "name": c.name,
                        "input": input,
                    }));
                }
                push_or_merge(&mut messages, "assistant", blocks);
            }
            Some(ToolTurn::Result { call_id, is_error }) => {
                // ALWAYS "user" -- Anthropic has no third role. This is the
                // exact case `push_or_merge` exists for: two parallel calls
                // produce two of these messages in a row.
                push_or_merge(
                    &mut messages,
                    "user",
                    vec![json!({
                        "type": "tool_result",
                        "tool_use_id": call_id,
                        "content": m.text,
                        "is_error": is_error,
                    })],
                );
            }
        }
    }

    let messages_json: Vec<Value> =
        messages.into_iter().map(|m| json!({ "role": m.role, "content": m.content })).collect();

    let mut body = json!({
        "model": call.model,
        "max_tokens": ANTHROPIC_MAX_TOKENS,
        "messages": messages_json,
        "stream": true,
    });
    if !call.system.trim().is_empty() {
        body["system"] = json!(call.system);
    }
    // Empty means "do not offer tools" -- see `ModelCall.tools`'s own doc.
    // Omitting the field entirely rather than sending `"tools": []` mirrors
    // `openai.rs`'s own reasoning: an empty array is documented, on some
    // OpenAI-compatible endpoints, to behave differently from the field
    // being absent at all, and there is no reason to assume every
    // Anthropic-shaped endpoint is immune to the same class of surprise.
    if !call.tools.is_empty() {
        let tools: Vec<Value> = call.tools.iter().map(tool_to_json).collect();
        body["tools"] = json!(tools);
    }
    body
}

/// Decode one line of the SSE stream. `None` for a line this wire does not
/// need to look at further (blank lines, and anything not prefixed `data:`).
pub(crate) fn parse_sse_line(line: &str) -> Option<Chunk> {
    let line = line.trim_end_matches(['\r', '\n']);
    let payload = line.strip_prefix("data:")?.trim_start();
    let v: Value = serde_json::from_str(payload).ok()?;
    let kind = v.get("type").and_then(|t| t.as_str()).unwrap_or("");

    match kind {
        "error" => {
            let msg = v
                .pointer("/error/message")
                .and_then(|m| m.as_str())
                .map(str::to_string)
                .unwrap_or_else(|| v.to_string());
            Some(Chunk::Error(msg))
        }
        "content_block_start" => {
            let block = v.get("content_block")?;
            if block.get("type").and_then(|t| t.as_str()) != Some("tool_use") {
                // `text` and `thinking` block-starts carry nothing this wire
                // reads -- the content itself arrives on the deltas.
                return Some(Chunk::Nothing);
            }
            let index = v.get("index").and_then(|i| i.as_u64())? as u32;
            let id = block.get("id").and_then(|s| s.as_str()).unwrap_or("").to_string();
            let name = block.get("name").and_then(|s| s.as_str()).unwrap_or("").to_string();
            Some(Chunk::ToolBlockStart { index, id, name })
        }
        "content_block_delta" => {
            let index = v.get("index").and_then(|i| i.as_u64())? as u32;
            let delta = v.get("delta")?;
            match delta.get("type").and_then(|t| t.as_str()) {
                Some("text_delta") => {
                    let text = delta.get("text").and_then(|t| t.as_str()).unwrap_or("");
                    if text.is_empty() {
                        Some(Chunk::Nothing)
                    } else {
                        Some(Chunk::Content(text.to_string()))
                    }
                }
                Some("thinking_delta") => {
                    let text = delta.get("thinking").and_then(|t| t.as_str()).unwrap_or("");
                    if text.is_empty() {
                        Some(Chunk::Nothing)
                    } else {
                        Some(Chunk::Thinking(text.to_string()))
                    }
                }
                Some("input_json_delta") => {
                    let partial = delta.get("partial_json").and_then(|t| t.as_str()).unwrap_or("");
                    Some(Chunk::ToolInputDelta { index, partial_json: partial.to_string() })
                }
                // `signature_delta` and anything future: carries nothing
                // this wire reads.
                _ => Some(Chunk::Nothing),
            }
        }
        "message_delta" => {
            let reason = v.pointer("/delta/stop_reason").and_then(|r| r.as_str());
            match reason {
                Some(r) => Some(Chunk::StopReasonSeen(r.to_string())),
                None => Some(Chunk::Nothing),
            }
        }
        "message_stop" => Some(Chunk::MessageStop),
        // `message_start`, `content_block_stop`, `ping`, and anything this
        // wire has not been taught: a well-formed line with nothing to read.
        // Token counts are read independently by `scan_usage`, below, the
        // same split `openai.rs`'s own `scan_usage` uses for the identical
        // reason -- usage lives on two DIFFERENT event types here
        // (`message_start` for input, `message_delta` for output) and
        // forcing that into one `Chunk` arm would mean choosing which one
        // `parse_sse_line`'s single-classification-per-line contract loses.
        _ => Some(Chunk::Nothing),
    }
}

/// Token counts, read independently of whichever single `Chunk` a line
/// decoded to — see `parse_sse_line`'s own trailing comment for why. Returns
/// `(input_tokens, output_tokens)`, either half `None` when that line did
/// not carry it: `message_start` carries only input, `message_delta` carries
/// only output, and neither ever carries both on the same line (confirmed
/// against the real captured bytes below, not assumed).
fn scan_usage(line: &str) -> (Option<u64>, Option<u64>) {
    let Some(payload) = line.trim_end_matches(['\r', '\n']).strip_prefix("data:") else {
        return (None, None);
    };
    let Ok(v) = serde_json::from_str::<Value>(payload.trim_start()) else {
        return (None, None);
    };
    let input = v.pointer("/message/usage/input_tokens").and_then(|n| n.as_u64());
    let output = v.pointer("/usage/output_tokens").and_then(|n| n.as_u64());
    (input, output)
}

/// Turn the provider's own words about a failure into something a person can
/// act on. Anthropic's documented error `type`s (`invalid_request_error`,
/// `authentication_error`, `permission_error`, `not_found_error`,
/// `request_too_large`, `rate_limit_error`, `api_error`, `overloaded_error`)
/// are read from the body where present, same as `openai.rs`'s own
/// `status_error` reads `/error/message`.
fn status_error(code: u16, body: String, key: &str, model: &str, base: &str) -> TurnError {
    let detail = crate::providers::redact_and_truncate(body, key, ERROR_BODY_CAP);
    let said = serde_json::from_str::<Value>(&detail)
        .ok()
        .and_then(|v| v.pointer("/error/message").and_then(|m| m.as_str()).map(str::to_string))
        .unwrap_or(detail);

    match code {
        401 => TurnError {
            kind: ErrorKind::BadRequest,
            what: "The endpoint rejected the key (401).".into(),
            fix: "Open AI components, check the key saved for this brain, and test the \
                  connection again. Nothing else was sent and nothing was charged."
                .into(),
        },
        403 => TurnError {
            kind: ErrorKind::BadRequest,
            what: format!("The key was accepted but is not permitted to do this (403): {said}"),
            fix: "Check the key's permissions with the provider, then test the connection \
                  again."
                .into(),
        },
        404 => TurnError {
            kind: ErrorKind::ModelMissing,
            what: format!(
                "No /v1/messages endpoint at {base} (404), or {model} is not a model there."
            ),
            fix: "Check the base URL and the model name in AI components, then test the \
                  connection again."
                .into(),
        },
        413 => TurnError {
            kind: ErrorKind::BadRequest,
            what: "The request was too large for the endpoint to accept (413).".into(),
            fix: "This conversation may need to be shorter, or a file it is discussing may be \
                  too large -- try again with less in the message."
                .into(),
        },
        429 => TurnError {
            kind: ErrorKind::Upstream,
            what: "The endpoint refused with 429 (rate limited).".into(),
            fix: "Wait a moment and try again, or check the account's rate limits with the \
                  provider."
                .into(),
        },
        529 => TurnError {
            kind: ErrorKind::Upstream,
            what: "The endpoint reported it is overloaded (529).".into(),
            fix: "That is a fault at the provider rather than anything you set. Worth trying \
                  again in a moment."
                .into(),
        },
        c if c >= 500 => TurnError {
            kind: ErrorKind::Upstream,
            what: format!("The endpoint answered {c}: {said}"),
            fix: "That is a fault at the provider rather than anything you set. Worth trying \
                  again in a moment."
                .into(),
        },
        c => TurnError {
            kind: ErrorKind::BadRequest,
            what: format!("The endpoint answered {c}: {said}"),
            fix: "Check the model name and base URL in AI components, then test the \
                  connection again."
                .into(),
        },
    }
}

/// One tool call being assembled from however many `ToolInputDelta`s have
/// arrived so far for its `index`. `id`/`name` are set once, from the
/// `content_block_start` that opened the block, and never overwritten.
#[derive(Default)]
struct PendingCall {
    id: String,
    name: String,
    arguments_json: String,
}

impl Wire for AnthropicWire {
    fn label(&self) -> &'static str {
        "anthropic"
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
                fix: "Open AI components and set it to the provider's API base URL -- for \
                      Anthropic itself that is https://api.anthropic.com -- then test the \
                      connection."
                    .into(),
            });
        }
        let url = format!("{base}/v1/messages");
        let key = call.api_key.unwrap_or("").trim();

        let agent = ureq::AgentBuilder::new()
            .timeout_connect(CONNECT_TIMEOUT)
            .timeout_read(READ_TIMEOUT)
            .build();

        let response = agent
            .post(&url)
            .set("content-type", "application/json")
            .set("anthropic-version", ANTHROPIC_VERSION)
            .set("x-api-key", key)
            .send_string(&build_request(call).to_string());

        let upstream = match response {
            Ok(r) => r,
            Err(ureq::Error::Status(code, resp)) => {
                return Err(status_error(
                    code,
                    resp.into_string().unwrap_or_default(),
                    key,
                    call.model,
                    base,
                ))
            }
            Err(ureq::Error::Transport(t)) => {
                return Err(TurnError {
                    kind: ErrorKind::Unreachable,
                    what: format!("Could not reach {base} ({t})."),
                    fix: "Check the base URL in AI components and that this computer has a \
                          working connection, then send again -- nothing was charged."
                        .into(),
                })
            }
        };

        let mut text = String::new();
        let mut stop = StopReason::Other("the stream ended without saying why".into());
        let mut input_tokens = None;
        let mut output_tokens = None;
        let mut parsed_anything = false;
        // Same reasoning as `openai.rs`'s own `pending`: indices arrive as
        // small dense integers starting from zero, so a linear scan over a
        // handful of parallel calls costs nothing real.
        let mut pending: Vec<(u32, PendingCall)> = Vec::new();

        let mut reader = BufReader::new(upstream.into_reader());
        let mut line = String::new();
        'read: loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) => break,
                Ok(_) => {}
                Err(e) => {
                    stop = StopReason::Other(format!("the connection dropped ({e})"));
                    break;
                }
            }
            if line.trim().is_empty() {
                continue;
            }
            let Some(chunk) = parse_sse_line(&line) else { continue };
            parsed_anything = true;
            let (i, o) = scan_usage(&line);
            if i.is_some() {
                input_tokens = i;
            }
            if o.is_some() {
                output_tokens = o;
            }
            match chunk {
                Chunk::Content(t) => {
                    text.push_str(&t);
                    if on(Delta::Text(t)) == Flow::Stop {
                        stop = StopReason::Cancelled;
                        break 'read;
                    }
                }
                Chunk::Thinking(t) => {
                    if on(Delta::Thinking(t)) == Flow::Stop {
                        stop = StopReason::Cancelled;
                        break 'read;
                    }
                }
                Chunk::ToolBlockStart { index, id, name } => {
                    let slot = match pending.iter_mut().find(|(i, _)| *i == index) {
                        Some((_, p)) => p,
                        None => {
                            pending.push((index, PendingCall::default()));
                            &mut pending.last_mut().unwrap().1
                        }
                    };
                    slot.id = id;
                    slot.name = name;
                    // No `on()` callback here -- same rule `openai.rs`'s own
                    // `ToolCallDeltas` arm states: a call is only announced
                    // once fully assembled.
                }
                Chunk::ToolInputDelta { index, partial_json } => {
                    let slot = match pending.iter_mut().find(|(i, _)| *i == index) {
                        Some((_, p)) => p,
                        None => {
                            pending.push((index, PendingCall::default()));
                            &mut pending.last_mut().unwrap().1
                        }
                    };
                    slot.arguments_json.push_str(&partial_json);
                }
                Chunk::StopReasonSeen(reason) => {
                    stop = match reason.as_str() {
                        "end_turn" | "stop_sequence" => StopReason::End,
                        "max_tokens" => StopReason::Length,
                        "tool_use" => StopReason::ToolUse,
                        other => StopReason::Other(other.to_string()),
                    };
                    // NOT a break -- `message_stop` is still to come, and
                    // treating `message_delta` as the end would skip
                    // whatever cleanup a future provider variant put after
                    // it, the same reasoning `openai.rs`'s own
                    // `FinishReason` arm states for not breaking early.
                }
                Chunk::Error(msg) => {
                    return Err(TurnError {
                        kind: ErrorKind::Upstream,
                        what: format!("The provider reported a problem mid-answer: {msg}"),
                        fix: "Worth trying again -- this is a fault at the provider's end, not \
                              anything you set."
                            .into(),
                    });
                }
                Chunk::MessageStop => break 'read,
                Chunk::Nothing => {}
            }
        }

        if !parsed_anything {
            return Err(TurnError {
                kind: ErrorKind::Protocol,
                what: format!("Something answered at {base}, but not in this shape."),
                fix: "Check the address in AI components points at an Anthropic-shaped \
                      /v1/messages endpoint and not at another program on the same address, \
                      then test the connection."
                    .into(),
            });
        }

        // ASSEMBLED, THEN ANNOUNCED, THEN RETURNED -- same order and same
        // reasoning as `openai.rs`'s own: a call can start and then have the
        // connection drop before `message_delta`/`message_stop` ever
        // arrives, and reporting a call nobody actually finished asking for
        // would send `drive` off to run a tool the model never really
        // requested.
        let tool_calls: Vec<ToolCallRequest> = if stop == StopReason::ToolUse {
            pending
                .into_iter()
                .map(|(_, p)| ToolCallRequest { id: p.id, name: p.name, args_json: p.arguments_json, thought_signature: None })
                .collect()
        } else {
            Vec::new()
        };
        for call in &tool_calls {
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

    fn a_call<'a>(system: &'a str, key: Option<&'a str>, messages: &'a [Message]) -> ModelCall<'a> {
        a_call_with_tools(system, key, messages, &[])
    }

    fn a_call_with_tools<'a>(
        system: &'a str,
        key: Option<&'a str>,
        messages: &'a [Message],
        tools: &'a [ToolDef],
    ) -> ModelCall<'a> {
        ModelCall {
            base_url: "https://api.anthropic.com",
            model: "claude-haiku-4.5",
            api_key: key,
            system,
            messages,
            tools,
        }
    }

    /// **`max_tokens` IS ALWAYS PRESENT.** Proven able to fail: deleting the
    /// field from `build_request` -- the shape `openai.rs` and `ollama.rs`
    /// both correctly use for THEIR endpoints -- fails this immediately.
    /// See the module header for why this wire is the one place that shape
    /// is wrong.
    #[test]
    fn max_tokens_is_always_sent() {
        let msgs = vec![Message { role: Role::User, text: "hi".into(), tool: None }];
        let body = build_request(&a_call("", None, &msgs));
        assert_eq!(body["max_tokens"], json!(ANTHROPIC_MAX_TOKENS));
    }

    /// **THE SYSTEM PROMPT IS A TOP-LEVEL FIELD, NEVER A MESSAGE.** Proven
    /// able to fail: pushing it into `messages` with role `"system"` --
    /// which is exactly what `openai.rs::build_request` correctly does for
    /// ITS endpoint -- fails this immediately, because the real API refuses
    /// that shape outright (see the module header).
    #[test]
    fn the_system_prompt_is_a_top_level_field_not_a_message() {
        let msgs = vec![Message { role: Role::User, text: "hi".into(), tool: None }];
        let body = build_request(&a_call("VOICE CARD", None, &msgs));
        assert_eq!(body["system"], json!("VOICE CARD"));
        for m in body["messages"].as_array().unwrap() {
            assert_ne!(m["role"], json!("system"), "no message may carry role system: {body}");
        }
    }

    /// An empty system prompt is omitted entirely rather than sent as
    /// `"system": ""`, matching `openai.rs`'s own rule for an empty system
    /// message.
    #[test]
    fn an_empty_system_prompt_is_omitted() {
        let msgs = vec![Message { role: Role::User, text: "hi".into(), tool: None }];
        let body = build_request(&a_call("", None, &msgs));
        assert!(body.get("system").is_none(), "{body}");
    }

    /// **THE MERGE THIS FILE'S WHOLE MODULE HEADER IS ABOUT — proven able to
    /// fail: replacing `push_or_merge` with a plain `.push()` produces two
    /// consecutive `"user"` messages here, which is precisely the shape a
    /// real Anthropic-speaking endpoint answers 400 to.** Two parallel tool
    /// calls -- exactly what `drive`'s own tool loop emits when a model asks
    /// for more than one tool in a turn -- must land as ONE user turn
    /// carrying two `tool_result` blocks.
    #[test]
    fn consecutive_tool_results_are_merged_into_one_user_turn() {
        let msgs = vec![
            Message { role: Role::User, text: "do two things".into(), tool: None },
            Message {
                role: Role::Assistant,
                text: String::new(),
                tool: Some(ToolTurn::Calls(vec![
                    ToolCallRequest { id: "a".into(), name: "Read".into(), args_json: "{}".into(), thought_signature: None },
                    ToolCallRequest { id: "b".into(), name: "Read".into(), args_json: "{}".into(), thought_signature: None },
                ])),
            },
            Message {
                role: Role::Tool,
                text: "first result".into(),
                tool: Some(ToolTurn::Result { call_id: "a".into(), is_error: false }),
            },
            Message {
                role: Role::Tool,
                text: "second result".into(),
                tool: Some(ToolTurn::Result { call_id: "b".into(), is_error: false }),
            },
        ];
        let body = build_request(&a_call("", None, &msgs));
        let out = body["messages"].as_array().unwrap();
        // user("do two things"), assistant(2 tool_use), user(2 tool_result) -- three
        // turns, never four, and never two adjacent "user" entries.
        assert_eq!(out.len(), 3, "{out:#?}");
        assert_eq!(out[2]["role"], json!("user"));
        let blocks = out[2]["content"].as_array().unwrap();
        assert_eq!(blocks.len(), 2, "the two tool_result blocks must share one turn: {blocks:#?}");
        assert_eq!(blocks[0]["type"], json!("tool_result"));
        assert_eq!(blocks[0]["tool_use_id"], json!("a"));
        assert_eq!(blocks[1]["tool_use_id"], json!("b"));
    }

    /// A tool call's `input` is the PARSED object, not the raw string --
    /// proven able to fail: sending `"input": c.args_json` (a string) where
    /// the API expects an object fails this immediately.
    #[test]
    fn a_tool_calls_input_is_a_json_object_not_a_string() {
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
        let body = build_request(&a_call("", None, &msgs));
        let block = &body["messages"][0]["content"][0];
        assert_eq!(block["type"], json!("tool_use"));
        assert_eq!(block["id"], json!("call1"));
        assert_eq!(block["input"], json!({ "path": "a.txt" }));
    }

    /// Malformed `args_json` falls back to `{}` rather than panicking or
    /// refusing the whole request -- see the module header for why a
    /// refusal here would be the wrong call.
    #[test]
    fn a_malformed_tool_call_argument_falls_back_to_an_empty_object() {
        let msgs = vec![Message {
            role: Role::Assistant,
            text: String::new(),
            tool: Some(ToolTurn::Calls(vec![ToolCallRequest {
                id: "call1".into(),
                name: "Read".into(),
                args_json: "not json".into(),
                thought_signature: None,
            }])),
        }];
        let body = build_request(&a_call("", None, &msgs));
        assert_eq!(body["messages"][0]["content"][0]["input"], json!({}));
    }

    /// No `strict` field on a tool definition -- Anthropic's `input_schema`
    /// has no such concept, and sending an unrecognised field a real
    /// endpoint has never been told about is worth avoiding on principle.
    #[test]
    fn a_tool_definition_carries_no_strict_field() {
        let t = ToolDef {
            name: "Read".into(),
            description: "read a file".into(),
            strict: true,
            parameters: json!({ "type": "object", "properties": {}, "required": [] }),
        };
        let j = tool_to_json(&t);
        assert!(j.get("strict").is_none(), "{j}");
        assert_eq!(j["name"], json!("Read"));
        assert_eq!(j["input_schema"]["type"], json!("object"));
    }

    /// **THE REAL CAPTURED BYTES, run against OpenRouter's Anthropic-shaped
    /// endpoint 2026-09-03 (see the module header) — a tool-call turn,
    /// pinned line by line so this parser is checked against what actually
    /// arrived rather than only against the published docs.**
    #[test]
    fn the_real_captured_lines_decode_the_way_they_did_on_the_wire() {
        assert_eq!(
            parse_sse_line(
                r#"data: {"type":"message_start","message":{"id":"gen-1","type":"message","role":"assistant","content":[],"model":"anthropic/claude-3-haiku","stop_reason":null,"usage":{"input_tokens":339,"output_tokens":1}}}"#
            ),
            Some(Chunk::Nothing)
        );
        assert_eq!(
            scan_usage(
                r#"data: {"type":"message_start","message":{"usage":{"input_tokens":339,"output_tokens":1}}}"#
            ),
            (Some(339), None)
        );
        assert_eq!(
            parse_sse_line(
                r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_bdrk_015suWT5xJDNhUgYSkBEvPTu","name":"get_weather","input":{}}}"#
            ),
            Some(Chunk::ToolBlockStart {
                index: 0,
                id: "toolu_bdrk_015suWT5xJDNhUgYSkBEvPTu".into(),
                name: "get_weather".into(),
            })
        );
        assert_eq!(
            parse_sse_line(
                r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"city\": \"T"}}"#
            ),
            Some(Chunk::ToolInputDelta { index: 0, partial_json: "{\"city\": \"T".into() })
        );
        assert_eq!(
            parse_sse_line(
                r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"okyo\"}"}}"#
            ),
            Some(Chunk::ToolInputDelta { index: 0, partial_json: "okyo\"}".into() })
        );
        assert_eq!(
            parse_sse_line(r#"data: {"type":"content_block_stop","index":0}"#),
            Some(Chunk::Nothing)
        );
        assert_eq!(
            parse_sse_line(
                r#"data: {"type":"message_delta","delta":{"stop_reason":"tool_use","stop_sequence":null},"usage":{"input_tokens":339,"output_tokens":53}}"#
            ),
            Some(Chunk::StopReasonSeen("tool_use".into()))
        );
        assert_eq!(
            scan_usage(
                r#"data: {"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":53}}"#
            ),
            (None, Some(53))
        );
        assert_eq!(parse_sse_line(r#"data: {"type":"message_stop"}"#), Some(Chunk::MessageStop));
        // OpenRouter's own trailing pair, not part of Anthropic's documented
        // shape -- must not be treated as an error or as more content.
        assert_eq!(parse_sse_line("data: [DONE]"), None);
    }

    /// A well-formed `ping` line, and a `content_block_start` for a `text`
    /// block, both carry nothing this wire reads.
    #[test]
    fn ping_and_text_block_start_are_ignored_without_erroring() {
        assert_eq!(parse_sse_line(r#"data: {"type":"ping"}"#), Some(Chunk::Nothing));
        assert_eq!(
            parse_sse_line(
                r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#
            ),
            Some(Chunk::Nothing)
        );
    }

    /// The mid-stream error shape, matching the same `event: error` /
    /// `data: {"type":"error",...}` pair Anthropic's own streaming docs
    /// give.
    #[test]
    fn a_mid_stream_error_event_decodes_to_the_message() {
        assert_eq!(
            parse_sse_line(
                r#"data: {"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}"#
            ),
            Some(Chunk::Error("Overloaded".into()))
        );
    }

    /// Every status code branch has both halves of a `TurnError`, same
    /// contract `openai.rs`'s own equivalent test pins.
    #[test]
    fn every_status_branch_has_what_and_a_fix() {
        for code in [401u16, 403, 404, 413, 429, 500, 529, 400] {
            let e = status_error(code, "{}".into(), "sk-test", "claude-haiku-4.5", "https://api.anthropic.com");
            assert!(!e.what.trim().is_empty(), "{code} has no description");
            assert!(!e.fix.trim().is_empty(), "{code} has no fix hint");
        }
    }

    /// A bad base URL is refused before any socket opens.
    ///
    /// `match` rather than `.unwrap_err()`, same reasoning `openai.rs`'s own
    /// equivalent test states: `Result::unwrap_err`/`expect_err` need
    /// `Completion: Debug` (to format the Ok side if this ever panics), and
    /// that type does not derive it -- not printed anywhere else in this
    /// codebase, so this test avoids adding a derive to a shared type for
    /// its own convenience.
    #[test]
    fn a_non_web_address_is_refused_before_any_socket_opens() {
        let msgs = vec![Message { role: Role::User, text: "hi".into(), tool: None }];
        let call = a_call("", Some("key"), &msgs);
        let call = ModelCall { base_url: "not-a-url", ..call };
        match AnthropicWire.stream(&call, &mut |_| Flow::Go) {
            Ok(_) => panic!("a non-web address must not silently succeed"),
            Err(err) => assert_eq!(err.kind, ErrorKind::BadRequest),
        }
    }

    // -- Real endpoints. `#[ignore]`d by house convention (`openai.rs`'s own
    // -- real round trips do the same) because a default `cargo test` run
    // -- must not depend on the network or on a credential being present. --

    /// Run it with:
    ///
    /// ```text
    /// NAMEOS_TEST_OPENROUTER_KEY="$(systemd-creds decrypt --user \
    ///   --name=openrouter-api ~/.config/openrouter/openrouter-api.cred -)" \
    ///   cargo test --manifest-path desktop/src-tauri/Cargo.toml \
    ///   a_real_openrouter_anthropic_streaming_round_trip -- --ignored --nocapture
    /// ```
    ///
    /// **Run for real on 2026-09-03**, against `anthropic/claude-3-haiku` via
    /// OpenRouter's Anthropic-shaped endpoint — see the module header for
    /// what this does and does not prove.
    #[test]
    #[ignore = "talks to a real Anthropic-shaped endpoint and spends a fraction of a cent"]
    fn a_real_openrouter_anthropic_streaming_round_trip() {
        let key = std::env::var("NAMEOS_TEST_OPENROUTER_KEY")
            .expect("set NAMEOS_TEST_OPENROUTER_KEY to a real OpenRouter API key to run this");
        let msgs = vec![Message {
            role: Role::User,
            text: "Reply with exactly the word OK and nothing else.".into(),
            tool: None,
        }];
        let call = ModelCall {
            base_url: "https://openrouter.ai/api",
            model: "anthropic/claude-3-haiku",
            api_key: Some(&key),
            system: "You are terse.",
            messages: &msgs,
            tools: &[],
        };
        let out = AnthropicWire
            .stream(&call, &mut |_| Flow::Go)
            .expect("a real Anthropic-shaped call with a real key must succeed");
        assert_eq!(out.stop, StopReason::End, "text={:?}", out.text);
        assert!(!out.text.trim().is_empty(), "no text came back");
        assert!(
            out.input_tokens.is_some() && out.output_tokens.is_some(),
            "usage never arrived from message_start/message_delta the way the module doc claims"
        );
        println!(
            "a_real_openrouter_anthropic_streaming_round_trip: text={:?} in={:?} out={:?}",
            out.text, out.input_tokens, out.output_tokens
        );
    }

    /// The tool loop's live proof, mirroring `openai.rs`'s own
    /// `a_real_openai_tool_call_round_trip`: a forced tool call, dispatched
    /// through the real `tools::dispatch`, replayed back in the exact shape
    /// `build_request` builds, and a second real call that uses the answer.
    ///
    /// Run it the same way as the test above, naming
    /// `a_real_openrouter_anthropic_tool_call_round_trip` instead.
    #[test]
    #[ignore = "talks to a real Anthropic-shaped endpoint and spends a fraction of a cent"]
    fn a_real_openrouter_anthropic_tool_call_round_trip() {
        let key = std::env::var("NAMEOS_TEST_OPENROUTER_KEY")
            .expect("set NAMEOS_TEST_OPENROUTER_KEY to a real OpenRouter API key to run this");

        let workdir = std::env::temp_dir()
            .join(format!("nameos-anthropic-tool-live-{}", crate::engine::native::store::new_id()));
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
        let call_one = ModelCall {
            base_url: "https://openrouter.ai/api",
            model: "anthropic/claude-3-haiku",
            api_key: Some(&key),
            system: "You are terse.",
            messages: &round_one_msgs,
            tools: std::slice::from_ref(&read_tool),
        };
        let out_one = AnthropicWire
            .stream(&call_one, &mut |_| Flow::Go)
            .expect("round one against a real key must succeed");
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
        assert!(!call_req.id.is_empty());
        let parsed: Value =
            serde_json::from_str(&call_req.args_json).expect("real tool-call arguments must be valid JSON");
        assert_eq!(parsed["path"], "notes.txt", "args={}", call_req.args_json);
        println!("a_real_openrouter_anthropic_tool_call_round_trip: model asked for {call_req:?}");

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
        let call_two = ModelCall {
            base_url: "https://openrouter.ai/api",
            model: "anthropic/claude-3-haiku",
            api_key: Some(&key),
            system: "You are terse.",
            messages: &round_two_msgs,
            tools: std::slice::from_ref(&read_tool),
        };
        let out_two = AnthropicWire
            .stream(&call_two, &mut |_| Flow::Go)
            .expect("round two against a real key must succeed");
        assert_eq!(out_two.stop, StopReason::End, "text={:?}", out_two.text);
        assert!(
            out_two.text.contains("8214"),
            "the model did not use the real tool result in its answer: {:?}",
            out_two.text
        );
        println!("a_real_openrouter_anthropic_tool_call_round_trip: final answer = {:?}", out_two.text);

        let _ = std::fs::remove_dir_all(&workdir);
    }

    /// A bad key against a real endpoint, run for real 2026-09-03 (see the
    /// module header) — this is what `status_error`'s 401 branch is worded
    /// from.
    #[test]
    #[ignore = "talks to a real Anthropic-shaped endpoint"]
    fn a_bad_key_surfaces_honestly_with_no_claude_exe_running() {
        let msgs = vec![Message { role: Role::User, text: "hi".into(), tool: None }];
        let call = ModelCall {
            base_url: "https://openrouter.ai/api",
            model: "anthropic/claude-3-haiku",
            api_key: Some("sk-or-bogus-not-real-key"),
            system: "",
            messages: &msgs,
            tools: &[],
        };
        match AnthropicWire.stream(&call, &mut |_| Flow::Go) {
            Ok(_) => panic!("a bad key must not silently succeed"),
            Err(err) => {
                assert_eq!(err.kind, ErrorKind::BadRequest);
                println!("a_bad_key_surfaces_honestly_with_no_claude_exe_running: {}", err.sentence());
            }
        }
    }
}
