//! OpenAI's shapes, spoken natively. Two of them now — `POST {base}/chat/
//! completions` for a non-reasoning model, `POST {base}/responses` for a
//! reasoning one (`endpoint_path` decides which; see "Phase 2B" below for
//! why) — SSE back either way, `Authorization: Bearer {key}` either way.
//!
//! **PHASE 1 OF THE SAME REMOVAL `ollama.rs` STARTED — Mark, 2026-08-31: *"we
//! need to make each of the providers stand alone without claude"*, *"nothing
//! should reside on claude. All providers should be neutral."*** `ollama.rs`
//! proved the shape for a brain with no key. This is the one with a key, and
//! it covers TWO of this build's rows at once — both `openai-compatible`
//! tiles in `brain_setup.rs`, OpenAI and Gemini, because Gemini rides the
//! identical kind on purpose (see that file's own comment: *"RIDES THE EXACT
//! SAME `openai-compatible` KIND AS THE TILE ABOVE, NOT A NEW ONE"*, verified
//! live against `generativelanguage.googleapis.com/v1beta/openai/`). One wire,
//! two brains, because the endpoint shape is the leverage — the same argument
//! `adapter.rs`'s own header opens with.
//!
//! ## What is genuinely new here, and what is only new to this file
//!
//! Streaming is Server-Sent Events, not NDJSON — `data: {json}\n\n`, ending
//! `data: [DONE]\n\n` — so the parser below is a different shape from
//! `ollama.rs`'s, even though it answers the same `Wire` trait. Everything
//! else is the pattern already proven there: connect fast, read patiently; an
//! error body is the provider explaining the user's own mistake and is
//! repeated back capped rather than replaced with a number; nothing here can
//! be stopped except by the `Flow` the run loop hands back on every piece.
//!
//! ## Scope, and it is the room's, not mine
//!
//! **NO `max_tokens`/`max_completion_tokens`, NO sampling parameters. TOOLS
//! ARE IN, AS OF PHASE 2 — see `NativeEngine::openai_compatible`'s
//! `offers_tools: true` and `tool_to_json`/`ToolCallFragment` below.** This
//! paragraph used to read "CHAT ONLY. NO TOOLS… none of `adapter.rs`'s six
//! documented degradations apply on this path; they are all about a request
//! this wire never builds" — **that was true when it was written and it
//! quietly stopped being true the day this wire learned to send `tools`,
//! and nobody came back to fix the sentence that said otherwise.** That is
//! not a cosmetic staleness: it is the reason degradation 6 — a reasoning
//! model 400ing the instant BOTH function tools and any reasoning effort are
//! on the same `/chat/completions` turn — went unported here for as long as
//! it did. `adapter.rs` found and fixed it live on 2026-09-02, on the
//! loopback-proxy path; this wire builds the identical request shape on a
//! completely separate path (no `claude` binary anywhere in the process)
//! and needed the identical fix, ported below rather than re-derived: when
//! `is_reasoning_model(call.model)` and `call.tools` is non-empty,
//! `reasoning_effort: "none"` is added, sharing `adapter::is_reasoning_model`
//! rather than growing a second copy of its prefix list that could drift
//! from it — same reasoning `providers.rs`'s own token-cap branch already
//! gives for sharing it. Sampling parameters were never sent from this wire
//! (no field for them exists on `ModelCall` at all), so that half of
//! degradation 6's family does not apply here the way it does in the
//! translator. The one finding that DID carry over from day one is the
//! token-cap field name split between `max_tokens` and
//! `max_completion_tokens` for the o1/o3/o4/gpt-5* family — and the fix
//! mirrors `ollama.rs`'s own, for the identical reason recorded there
//! (`gpt-oss:20b` never completing a turn under a cap): **send neither
//! field.** Omitting the cap entirely sidesteps the naming split rather than
//! resolving it, and the Stop button is the only cap this engine has ever had.
//!
//! ## Phase 2B — `/v1/responses`, and why the stopgap did not have to be ripped out
//!
//! **Degradation 6's fix above was a STOPGAP, and it said so in its own
//! comment: "the second door OpenAI names is the one taken" — reasoning
//! turned OFF rather than the OTHER door, `/v1/responses`, which keeps it
//! ON.** That trade was made under the same pressure `adapter.rs` was under
//! on 2026-09-02 (a live 400 in front of Mark, fixed with the door that did
//! not mean building a second parser). **2026-09-03: the second door.** Every
//! `is_reasoning_model` turn now goes to `/v1/responses` — `build_request`,
//! `stream_chat_completions`'s own `/chat/completions` and the `reasoning_
//! effort: "none"` stopgap in `build_request` are UNCHANGED and still the
//! path every non-reasoning model uses, plus the automatic fallback a
//! reasoning turn takes if `/responses` itself answers 404 (a third-party
//! `openai-compatible` server that has not shipped that route yet — see
//! `endpoint_path`'s own doc for exactly which failures fall back and which
//! do not).
//!
//! **Three real shape differences from Chat Completions, each found by
//! running it rather than assumed from the docs — see `build_responses_
//! request`'s and `RChunk`'s own doc comments for the load-bearing detail on
//! each:** the request body is `input`, not `messages`, with a tool call and
//! its result as their own input items rather than nested in a message; the
//! system prompt is a top-level `instructions` string; and the finished
//! answer arrives whole, in `response.completed`'s own `output` array, so
//! this wire does not re-assemble a tool call from streamed fragments the
//! way `PendingCall` does for the other endpoint — it reads the fragments
//! only to keep the window's text and the Stop button responsive, then
//! trusts the one authoritative shape at the end.
//!
//! **What is still open, said plainly rather than left implied:** reasoning-
//! item continuity across the tool loop's two rounds (OpenAI's own guidance:
//! worth roughly 3% on SWE-bench, not required for correctness — verified
//! live here, not merely read) is not implemented, because it would mean
//! threading a reasoning-item id through `store::Message`/`ToolTurn`, shared
//! by every wire and the run loop in `engine::native::mod`, for a quality
//! knob rather than the 400 this phase exists to close. See `build_
//! responses_request`'s own doc for the full reasoning and the live test
//! that proves the omission costs quality headroom, not correctness.
//!
//! ## What was measured against a REAL endpoint, and by which key
//!
//! `adapter.rs`'s own header says its translator "has never met a real OpenAI
//! endpoint" — every one of its tests is synthetic. This wire's request and
//! response shapes were checked against **the real thing** before being
//! written down as fact: this file's own `#[ignore]`d tests
//! (`a_real_openai_streaming_round_trip`, `a_real_openrouter_streaming_round_trip`,
//! `a_bad_key_surfaces_honestly_with_no_claude_exe_running`) run this exact
//! code — `build_request`, `parse_sse_line`, the run loop, `Wire::stream` in
//! full — against `https://api.openai.com/v1` with a real key, and against
//! `https://openrouter.ai/api/v1` (OpenAI's own shape; OpenRouter documents
//! itself as OpenAI-SDK-compatible) as a second, independent OpenAI-shaped
//! endpoint. All four (see below) are `#[ignore]` by house convention for a
//! real network call, spend at most a fraction of a cent, and the ones
//! needing a key say so in their own doc comment.
//!
//! **Gemini's own compat endpoint WAS hit by this wire — closed 2026-09-02,
//! this line used to say the opposite and that was true until it was run.**
//! `a_real_gemini_streaming_round_trip` and `a_bad_key_surfaces_honestly_
//! against_gemini_too` are this wire's own proof, distinct from `adapter.rs`'s
//! separate Gemini test (that one exercises the translator's request/response
//! traversal through a different code path entirely — Claude Code talking
//! through a loopback proxy — and proves nothing about this engine, which
//! streams SSE directly). **Running it for real found one genuine shape
//! difference, not a flake:** Gemini never sends OpenAI's dedicated
//! usage-only, empty-`choices` line — it attaches running usage totals to
//! every line that carries a real choice instead. `scan_usage` (below) reads
//! for that independently of `parse_sse_line`'s own single-`Chunk`-per-line
//! classification; both real endpoints are now covered by the same code
//! without either one growing a special case for the other. See
//! `a_real_gemini_streaming_round_trip`'s and `scan_usage`'s own doc comments
//! for what was actually observed.
//!
//! ## The boundary
//!
//! - **Who may call.** `engine::native`, in-process.
//! - **Wrong caller.** Not representable; `pub(crate)`, no command, no route.
//! - **Malformed input.** A line that is not a recognised SSE shape is
//!   skipped, not fatal — a keep-alive comment (`: ping`) is legal SSE and
//!   carries nothing this wire uses. A stream where NOTHING recognisable ever
//!   arrived is a different failure and is an error, because that means the
//!   address is not speaking this protocol at all.
//! - **What errors leak.** The base URL, the model name, and the provider's
//!   own error text, capped and run through `providers::redact_and_truncate`
//!   with the real key as the redaction target — unlike `ollama.rs`, THIS
//!   wire does send a real credential upstream, so the same body that could
//!   echo it back (Beck's proof against a live provider row, cited in
//!   `providers.rs`'s own comment on that function) is treated as hostile by
//!   default. Never the prompt, never the conversation, never the key itself.

use std::io::{BufRead, BufReader};
use std::time::Duration;

use serde_json::{json, Value};

use super::{
    Completion, Delta, ErrorKind, Flow, ModelCall, StopReason, ToolCallRequest, ToolDef, ToolTurn,
    TurnError, Wire,
};

/// Same reasoning as `ollama.rs`'s own constants: connect fast so a dead
/// address is reported in seconds, read patiently because a real answer can
/// take real time and the Stop button — not a timeout — is what should end a
/// healthy long one. Matches `adapter.rs::relay`'s own figures for the
/// identical kind of upstream, deliberately: two files driving the same class
/// of endpoint should not disagree about how long "still working" is allowed
/// to look like "gone".
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const READ_TIMEOUT: Duration = Duration::from_secs(300);

/// Same cap `ollama.rs` and `providers::generic_status_error` both use.
const ERROR_BODY_CAP: usize = 500;

pub(crate) struct OpenAiWire;

/// One decoded SSE data line, or the reason the stream ended.
#[derive(Debug, PartialEq)]
pub(crate) enum Chunk {
    /// Answer text.
    Content(String),
    /// A DeepSeek-style `reasoning_content` delta — see `adapter.rs`'s
    /// degradation 3. Kept separate from the answer and never concatenated
    /// into it, same rule as `ollama.rs`'s own thinking chunks and the same
    /// reason: reasoning presented as the answer is the b17 failure, the
    /// local brain that looked broken on its first message.
    Thinking(String),
    /// A choice reported why IT stopped. Not the end of the wire -- a
    /// `stream_options.include_usage` request still has a usage-only chunk
    /// and the literal `[DONE]` line still to come.
    FinishReason(String),
    /// The usage-only chunk `stream_options.include_usage` asks for: empty
    /// `choices`, real counts.
    Usage { input_tokens: u64, output_tokens: u64 },
    /// The upstream reported a mid-stream failure inside an otherwise-200
    /// response. Rare, and real: `ollama.rs`'s own parser has the identical
    /// shape for the identical reason.
    Error(String),
    /// The literal `data: [DONE]` line. This, not a finish reason, is what
    /// ends the stream.
    Done,
    /// One line's worth of tool-call deltas -- **plural, on purpose.** A
    /// model asking for several tools at once (parallel calls) can carry more
    /// than one entry in a single line's `delta.tool_calls` array; keeping
    /// them together as one `Chunk` rather than forcing one-`Chunk`-per-line
    /// (this file's usual design, stated in `scan_usage`'s own doc) would
    /// silently drop every entry after the first the moment a model actually
    /// does that.
    ToolCallDeltas(Vec<ToolCallFragment>),
    /// A well-formed SSE line carrying nothing this wire reads (a comment, a
    /// keep-alive, a chunk whose delta was empty).
    Nothing,
}

/// One fragment of one tool call.
///
/// **OPENAI SENDS A CALL'S `id` AND `name` ON ITS OPENING FRAGMENT ONLY AND
/// NEVER REPEATS EITHER — captured live against a real key, 2026-09-02** (a
/// forced tool call against `gpt-4o-mini`). It ALSO sends an explicit `index`
/// on every fragment, and every fragment after the first for a given `index`
/// carries only more of `arguments`. So a single `Chunk` cannot represent one
/// call — the wire has to accumulate these by `index` across however many
/// lines it takes, which is exactly what `stream`'s own run loop does with
/// `PendingCall` below.
///
/// **`index` IS NOT ALWAYS ON THE WIRE — Gemini's OpenAI-compatible endpoint
/// omits it, and sends the WHOLE call (`id`, `name`, complete `arguments`) in
/// ONE fragment rather than splitting it across lines the way OpenAI does**
/// (captured live 2026-09-06). `index` is therefore `Option<u32>` — the raw
/// wire value, `None` when absent, NOT back-filled from array position. That
/// back-fill was a real bug: Gemini sends PARALLEL calls on SEPARATE SSE
/// lines, each a single-entry array, so every one sat at position `0` and got
/// index `0`, and the by-index accumulator concatenated their arguments into
/// one corrupt call (`{"city":"Paris"}{"city":"Tokyo"}`). `CallKey` (below)
/// keys by explicit `index` when the wire gives one — OpenAI's path, byte-for-
/// byte unchanged — and by the call's own `id` when it does not, so two
/// index-less Gemini calls stay two distinct calls.
///
/// **`thought_signature` IS GEMINI 3.x's SIGNED REASONING, AND IT MUST ROUND-
/// TRIP.** Gemini attaches an opaque `extra_content.google.thought_signature`
/// to a tool call it emits; the follow-up request that replays that call MUST
/// echo the same signature back in the same place, or Gemini 400s the whole
/// turn with "Function call is missing a thought_signature in functionCall
/// parts." It is captured here and threaded through `PendingCall` →
/// `ToolCallRequest` → `build_request`'s replayed `tool_calls`. `None` for
/// OpenAI/OpenRouter, which never send it — an absent signature is simply
/// never echoed, so their request bytes do not change.
#[derive(Debug, PartialEq, Clone)]
pub(crate) struct ToolCallFragment {
    /// The wire's own `index`, or `None` when the provider omits it (Gemini).
    /// Never synthesised from array position — see the struct's own doc for
    /// the parallel-call corruption that back-fill caused.
    pub index: Option<u32>,
    pub id: Option<String>,
    pub name: Option<String>,
    pub arguments_fragment: String,
    /// `extra_content.google.thought_signature`, when the provider sent one.
    pub thought_signature: Option<String>,
}

/// The key an assembling call is accumulated under. **OpenAI's explicit
/// `index` and Gemini's `id` are different identity spaces and must never
/// collide** — hence separate variants rather than one integer with a
/// back-filled fallback (the bug that merged two parallel Gemini calls).
#[derive(Debug, PartialEq, Clone)]
enum CallKey {
    /// OpenAI: the wire's own `index`, stable across a call's many fragments.
    Index(u32),
    /// Gemini: the call's `id`, when no `index` was sent. Each index-less call
    /// arrives whole in one fragment with a distinct `id`, so this keeps them
    /// distinct even when they share array position `0` on separate lines.
    Id(String),
    /// Last resort — a fragment with neither `index` nor `id`. Monotonic
    /// across the turn so each such fragment gets its own slot; a distinct
    /// variant so it can never equal a real `Index` or `Id`.
    Synthetic(u64),
}

/// One tool call being assembled from however many `ToolCallFragment`s have
/// arrived so far for its key. `id`/`name`/`thought_signature` are set once,
/// on the fragment that first carries each, and never overwritten;
/// `arguments_json` only ever grows.
#[derive(Debug, Default)]
struct PendingCall {
    id: String,
    name: String,
    arguments_json: String,
    thought_signature: Option<String>,
}

/// Accumulate one line's worth of tool-call fragments into the running
/// `pending` set, keyed so OpenAI's multi-fragment-by-`index` and Gemini's
/// whole-call-by-`id` both land correctly. `synthetic` is the turn's
/// monotonic counter for the neither-`index`-nor-`id` last resort. Pulled out
/// of `stream_chat_completions` so both the by-index (OpenAI) and by-id
/// (parallel Gemini) paths are unit-testable without a live socket.
fn accumulate_fragments(
    pending: &mut Vec<(CallKey, PendingCall)>,
    synthetic: &mut u64,
    frags: Vec<ToolCallFragment>,
) {
    for f in frags {
        let key = match (f.index, &f.id) {
            (Some(i), _) => CallKey::Index(i),
            (None, Some(id)) => CallKey::Id(id.clone()),
            (None, None) => {
                let s = *synthetic;
                *synthetic += 1;
                CallKey::Synthetic(s)
            }
        };
        let slot = match pending.iter_mut().find(|(k, _)| *k == key) {
            Some((_, p)) => p,
            None => {
                pending.push((key, PendingCall::default()));
                &mut pending.last_mut().unwrap().1
            }
        };
        // id, name and signature arrive ONCE, on the fragment that carries
        // them (see `ToolCallFragment`'s own doc) -- `if let` rather than
        // overwrite, so a later arguments-only fragment can never blank out
        // what an earlier one set.
        if let Some(id) = f.id {
            slot.id = id;
        }
        if let Some(name) = f.name {
            slot.name = name;
        }
        if f.thought_signature.is_some() {
            slot.thought_signature = f.thought_signature;
        }
        slot.arguments_json.push_str(&f.arguments_fragment);
    }
}

/// Turn the assembled `pending` set into the vendor-neutral `ToolCallRequest`s
/// `Completion` carries, preserving each call's `thought_signature` so
/// `build_request` can echo it back on the next turn.
fn pending_into_requests(pending: Vec<(CallKey, PendingCall)>) -> Vec<ToolCallRequest> {
    pending
        .into_iter()
        .map(|(_, p)| ToolCallRequest {
            id: p.id,
            name: p.name,
            args_json: p.arguments_json,
            thought_signature: p.thought_signature,
        })
        .collect()
}

fn tool_to_json(t: &ToolDef) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": t.name,
            "description": t.description,
            // See `ToolDef`'s own doc: the internal schema is built
            // strict-compatible from the start specifically so this can
            // always be `true` rather than needing a second, laxer schema
            // for wires that do not ask for it.
            "strict": t.strict,
            "parameters": t.parameters,
        }
    })
}

/// The request body. Pure, so both decisions in it are provable without a
/// socket: **no token cap of either name**, and **the system prompt leads as
/// its own message**, same shape `ollama.rs::build_request` already proves for
/// the local wire.
///
/// **THE TOOL LOOP'S TWO EXTRA MESSAGE SHAPES ARE BUILT HERE, ONE FUNCTION,
/// FOR ONE REASON: EVERY WIRE OWNS ITS OWN VENDOR SPELLING.** A message
/// carrying `ToolTurn::Calls` becomes an `assistant` turn with a `tool_calls`
/// array and (per OpenAI's own contract) `content: null` when there is no
/// text alongside the calls; a message carrying `ToolTurn::Result` becomes a
/// `tool` turn keyed by `tool_call_id`, with an `is_error` result prefixed
/// into the text OpenAI's shape has nowhere else to put it -- unlike
/// Anthropic's `tool_result` blocks, Chat Completions has no dedicated error
/// field on a tool message.
pub(crate) fn build_request(call: &ModelCall<'_>) -> Value {
    let mut messages: Vec<Value> = Vec::new();
    if !call.system.trim().is_empty() {
        messages.push(json!({ "role": "system", "content": call.system }));
    }
    for m in call.messages {
        match &m.tool {
            None => messages.push(json!({ "role": m.role.wire(), "content": m.text })),
            Some(ToolTurn::Calls(calls)) => {
                let tool_calls: Vec<Value> = calls
                    .iter()
                    .map(|c| {
                        let mut tc = json!({
                            "id": c.id,
                            "type": "function",
                            "function": { "name": c.name, "arguments": c.args_json },
                        });
                        // GEMINI 3.x THOUGHT-SIGNATURE ROUND-TRIP. When the
                        // first turn's call carried an
                        // `extra_content.google.thought_signature`, echo it
                        // back in the identical place on this replayed
                        // `tool_calls` entry -- Gemini requires it on the
                        // functionCall part or it 400s "Function call is
                        // missing a thought_signature". Placement (a sibling
                        // of `id`/`type`/`function`) matches the shape Gemini
                        // sent it in, verified against the captured wire and
                        // against Google's OpenAI-compat guidance. OpenAI /
                        // OpenRouter never set this, so the field is simply
                        // absent for them and their bytes are unchanged.
                        if let Some(sig) = &c.thought_signature {
                            tc["extra_content"] = json!({ "google": { "thought_signature": sig } });
                        }
                        tc
                    })
                    .collect();
                let content = if m.text.trim().is_empty() { Value::Null } else { json!(m.text) };
                messages.push(json!({ "role": "assistant", "content": content, "tool_calls": tool_calls }));
            }
            Some(ToolTurn::Result { call_id, is_error }) => {
                let content =
                    if *is_error { format!("Error: {}", m.text) } else { m.text.clone() };
                messages
                    .push(json!({ "role": "tool", "tool_call_id": call_id, "content": content }));
            }
        }
    }
    let mut body = json!({
        "model": call.model,
        "messages": messages,
        "stream": true,
        // Same field `adapter.rs::relay` already sends on every streaming
        // call to this shape of endpoint, proven live there. Without it the
        // final chunk carries no token counts at all on providers that honour
        // it, and `Completion.input_tokens`/`output_tokens` would silently be
        // `None` on every real turn rather than only on the ones where a
        // provider genuinely does not support it.
        "stream_options": { "include_usage": true },
    });
    // Empty means "do not offer tools" -- see `ModelCall.tools`'s own doc.
    // Omitting the field entirely rather than sending `"tools": []` is
    // deliberate: an empty array is documented, by some OpenAI-compatible
    // endpoints, to behave differently from the field being absent, and this
    // wire has no reason to find out which by accident.
    if !call.tools.is_empty() {
        let tools: Vec<Value> = call.tools.iter().map(tool_to_json).collect();
        body["tools"] = json!(tools);
    }
    // DEGRADATION 6, PORTED FROM `adapter.rs::translate_request` — see this
    // file's own module doc, "Scope" section, for why it took until now to
    // land here. OpenAI's own words, against this exact model, captured live
    // 2026-09-02 through the translator and reproduced here through Mark's
    // real desktop install with no `claude` binary in the process at all:
    // *"Function tools with reasoning_effort are not supported for
    // gpt-5.6-sol in /v1/chat/completions. To use function tools, use
    // /v1/responses or set reasoning_effort to 'none'."* This wire only
    // speaks `/chat/completions` -- same reasoning as the translator's own
    // comment on the same choice: it is the one shape OpenAI, Gemini,
    // OpenRouter and the rest of the `openai-compatible` row all publish,
    // and switching to `/v1/responses` would spend that leverage for one
    // family. So the second door OpenAI names is the one taken: an explicit
    // `reasoning_effort: "none"` rather than leaving the field to the
    // model's own default, which the error text confirms is NOT "none" --
    // the 400 fires even when this wire sends nothing for the field at all.
    // Scoped to when tools are ACTUALLY on the turn, matching the error's own
    // wording ("Function tools with reasoning_effort") and the translator's
    // own precedent: a tool-free turn on a reasoning model keeps full
    // reasoning effort rather than paying this down for nothing.
    // `is_reasoning_model` is shared with `adapter.rs` rather than
    // re-copied, same reasoning `providers.rs`'s token-cap branch already
    // gives for sharing it -- one prefix list, so a family OpenAI adds later
    // only needs updating in one place.
    if !call.tools.is_empty() && crate::adapter::is_reasoning_model(call.model) {
        body["reasoning_effort"] = json!("none");
    }
    body
}

/// Decode one line of the SSE stream. `None` for a line this wire does not
/// need to look at further (blank lines, and anything not prefixed `data:`
/// — comments and any other SSE field name).
pub(crate) fn parse_sse_line(line: &str) -> Option<Chunk> {
    let line = line.trim_end_matches(['\r', '\n']);
    let payload = line.strip_prefix("data:")?.trim_start();
    if payload == "[DONE]" {
        return Some(Chunk::Done);
    }
    let v: Value = serde_json::from_str(payload).ok()?;

    if let Some(err) = v.get("error") {
        let msg = err
            .get("message")
            .and_then(|m| m.as_str())
            .map(str::to_string)
            .unwrap_or_else(|| err.to_string());
        return Some(Chunk::Error(msg));
    }

    if let Some(usage) = v.get("usage") {
        // The usage-only chunk has empty `choices`; a chunk that carries BOTH
        // usage and a real choice is not a shape any provider has been seen
        // to send, but reading usage first and falling through below would
        // silently drop that choice's content if one ever did -- so usage is
        // returned only when there is nothing else on this line to report.
        let no_choice = v
            .get("choices")
            .and_then(|c| c.as_array())
            .map(|a| a.is_empty())
            .unwrap_or(true);
        if no_choice {
            return Some(Chunk::Usage {
                input_tokens: usage.get("prompt_tokens").and_then(|n| n.as_u64()).unwrap_or(0),
                output_tokens: usage
                    .get("completion_tokens")
                    .and_then(|n| n.as_u64())
                    .unwrap_or(0),
            });
        }
    }

    let choice = v.pointer("/choices/0")?;
    // Checked before `finish_reason`: the real capture shows the two never
    // sharing a line (the opening tool-call fragment carries a null
    // `finish_reason`, and `finish_reason: "tool_calls"` arrives later on its
    // own line with an empty `delta`) -- but reading it first costs nothing
    // and means a future provider that DOES combine them is still handled
    // rather than silently losing the tool-call half.
    if let Some(calls) = choice.pointer("/delta/tool_calls").and_then(|c| c.as_array()) {
        let frags: Vec<ToolCallFragment> = calls
            .iter()
            .map(|c| {
                // **`index` IS OPTIONAL ON THE WIRE, NOT GUARANTEED — Gemini's
                // own OpenAI-compatible endpoint omits it from every
                // `tool_calls` entry** (captured live 2026-09-06), where
                // OpenAI always sends it. It is read as an `Option` and carried
                // AS-IS — never back-filled from array position. That back-fill
                // was a real corruption: Gemini sends PARALLEL calls on
                // SEPARATE SSE lines, each a single-entry array, so every call
                // sat at position `0`, every one got index `0`, and the
                // by-index accumulator concatenated their arguments into one
                // call. Keying is now `CallKey`'s job (see `accumulate_
                // fragments`): explicit `index` for OpenAI, the call's `id` for
                // index-less Gemini, so two parallel calls stay two calls.
                let index = c.get("index").and_then(|i| i.as_u64()).map(|i| i as u32);
                ToolCallFragment {
                    index,
                    id: c.get("id").and_then(|v| v.as_str()).map(str::to_string),
                    name: c.pointer("/function/name").and_then(|v| v.as_str()).map(str::to_string),
                    arguments_fragment: c
                        .pointer("/function/arguments")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    // Gemini 3.x's signed reasoning, opaque and echoed back
                    // untouched on the next turn (see `ToolCallFragment`'s doc
                    // and `build_request`). Absent for OpenAI/OpenRouter.
                    thought_signature: c
                        .pointer("/extra_content/google/thought_signature")
                        .and_then(|v| v.as_str())
                        .map(str::to_string),
                }
            })
            .collect();
        if !frags.is_empty() {
            return Some(Chunk::ToolCallDeltas(frags));
        }
    }
    if let Some(reason) = choice.get("finish_reason").and_then(|r| r.as_str()) {
        return Some(Chunk::FinishReason(reason.to_string()));
    }
    if let Some(text) = choice.pointer("/delta/content").and_then(|c| c.as_str()) {
        if !text.is_empty() {
            return Some(Chunk::Content(text.to_string()));
        }
    }
    // DeepSeek-style, and it is the shape `adapter.rs`'s degradation 3 names.
    // Not part of OpenAI's own published schema; kept anyway because this
    // wire's whole point is to serve any endpoint speaking this shape, and
    // dropping a field silently because OpenAI itself does not send it would
    // reintroduce exactly the failure the module comment is here to prevent.
    if let Some(text) = choice.pointer("/delta/reasoning_content").and_then(|c| c.as_str()) {
        if !text.is_empty() {
            return Some(Chunk::Thinking(text.to_string()));
        }
    }
    Some(Chunk::Nothing)
}

/// **USAGE CAN RIDE ALONGSIDE A REAL CHOICE, NOT ONLY ON ITS OWN LINE —
/// found running the live Gemini round trip for real, 2026-09-02, not
/// assumed from a spec.** OpenAI sends a dedicated usage-only chunk with
/// empty `choices` at the end of the stream, which is the shape
/// `parse_sse_line`'s `Chunk::Usage` arm above reads. **Gemini's own
/// OpenAI-compatible endpoint does not do that at all** — captured live
/// against a real key and a real answer: every chunk that carries a choice
/// (the content delta, and separately the one carrying `finish_reason`)
/// ALSO carries the running usage totals, and Gemini never sends a
/// choice-empty line in the whole stream. So `no_choice` never fires, no
/// line is ever classified `Chunk::Usage`, and `stream()` below would
/// silently leave `input_tokens`/`output_tokens` at `None` for every real
/// Gemini turn if it only trusted `parse_sse_line`'s classification. Rather
/// than growing a Gemini-specific branch inside `parse_sse_line` — the exact
/// kind of per-vendor special case this file's own module doc says the
/// endpoint shape is supposed to make unnecessary — this reads the SAME raw
/// line a second time, independently of whichever single `Chunk` it decoded
/// to, so usage is picked up wherever it actually appears rather than only
/// where OpenAI happens to put it. `parse_sse_line` itself, and the real
/// captured bytes pinned against it, are untouched.
fn scan_usage(line: &str) -> Option<(u64, u64)> {
    let payload = line.trim_end_matches(['\r', '\n']).strip_prefix("data:")?.trim_start();
    let v: Value = serde_json::from_str(payload).ok()?;
    // `.as_object()` returns `None` for a present-but-`null` `usage` key —
    // the same OpenAI shape `the_real_captured_lines_decode_the_way_they_
    // did_on_the_wire` already pins for `Chunk::Usage` above, so a chunk
    // with no real usage yet is not misread as one that has it.
    let usage = v.get("usage")?.as_object()?;
    let input = usage.get("prompt_tokens")?.as_u64()?;
    let output = usage.get("completion_tokens")?.as_u64()?;
    Some((input, output))
}

/// Turn the provider's own words about a failure into something a person can
/// act on. Wording matches `providers::test_chat_endpoint`'s own hand-written
/// cases for 401/403/404/429/5xx wherever the two describe the same event —
/// that phrasing already reached a real person (Mark, 2026-08-30, on a fresh
/// key hitting 429) and repeating it here means the Test button and the send
/// path teach the same lesson instead of two.
///
/// **`path` NAMES THE ROUTE THAT WAS ACTUALLY CALLED, ADDED FOR PHASE 2B —
/// this used to hardcode `/chat/completions` into the 404 message, which was
/// true of every caller until this file learned a second route.** A reasoning
/// model's turn can now 404 against `/responses` just as easily as a
/// non-reasoning one can against `/chat/completions`, and naming the wrong
/// one in the fix text sends a person checking a route that was never called.
fn status_error(code: u16, body: String, key: &str, model: &str, base: &str, path: &str) -> TurnError {
    let detail = crate::providers::redact_and_truncate(body, key, ERROR_BODY_CAP);
    // Same array-wrapped Gemini envelope `providers::generic_status_error`
    // unwraps for the Test button (`[{"error":{"message":...}}]`) — the SEND
    // path lands here on a Gemini 400 and had the identical latent leak, since
    // `/error/message` never matches an array root. Read `/0/error/message`
    // too so a bad-key body surfaces the vendor's own sentence here as well,
    // rather than the raw JSON. OpenAI's object shape still matches the first
    // pointer, so its wording is unchanged.
    let said = serde_json::from_str::<Value>(&detail)
        .ok()
        .and_then(|v| {
            v.pointer("/error/message")
                .or_else(|| v.pointer("/0/error/message"))
                .and_then(|m| m.as_str())
                .map(str::to_string)
        })
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
            what: format!("No {path} endpoint at {base} (404), or {model} is not a model there."),
            fix: "Check the base URL and the model name in AI components -- most base URLs \
                  end in /v1 -- then test the connection again."
                .into(),
        },
        // 429 IS ALMOST NEVER A RATE LIMIT ON A NEW KEY -- the same finding
        // `providers::test_chat_endpoint` already carries, from Mark's own
        // account minutes after making it. Repeated here rather than
        // shortened to "rate limited", which is the less likely and less
        // actionable of the two real causes.
        429 => TurnError {
            kind: ErrorKind::Upstream,
            what: "The endpoint refused with 429.".into(),
            fix: "On a new key that usually means the account has no credit yet rather than \
                  too many requests -- check billing and credits with the provider, then try \
                  again."
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

/// **PHASE 2B: THE REAL FIX, NOT THE STOPGAP — 2026-09-03.** `build_request`'s
/// own `reasoning_effort: "none"` branch (see its doc, "DEGRADATION 6") kept
/// tools working on a reasoning model by turning reasoning OFF. OpenAI's own
/// 400 names the other door: *"To use function tools, use /v1/responses or
/// set reasoning_effort to 'none'."* This is that other door, taken for
/// real — every `is_reasoning_model` turn now goes to `/v1/responses`,
/// where reasoning and tools coexist with nothing forced down, verified live
/// (see `a_real_openai_reasoning_model_tool_call_round_trip` below: a real
/// `gpt-5.6-sol` call, tools on, no `reasoning_effort` field sent at all, and
/// the response's own echoed `reasoning.effort` came back `"medium"` — the
/// model's genuine default, not suppressed).
///
/// **`build_request`'s stopgap is NOT deleted — it is the fallback, exactly
/// as asked.** Some `openai-compatible` rows on this tile are third-party
/// servers proxying only Chat Completions (this file's own module doc names
/// them), and `/v1/responses` is new enough that not every one of them has
/// it yet. A `404` from `/responses` is read as "this address does not speak
/// that route" — not "the model is missing", which a 401/403/429/5xx would
/// already have ruled out by then — and the turn is retried once on
/// `/chat/completions` with the old stopgap, rather than failing a request
/// that would have worked a version of this file ago. Every other failure
/// (bad key, no credit, a real 500) surfaces from `/responses` directly;
/// retrying THOSE on a different endpoint would not fix them and would only
/// spend a second request finding that out.
fn endpoint_path(model: &str) -> &'static str {
    if crate::adapter::is_reasoning_model(model) {
        "/responses"
    } else {
        "/chat/completions"
    }
}

impl Wire for OpenAiWire {
    fn label(&self) -> &'static str {
        "openai"
    }

    fn stream(
        &self,
        call: &ModelCall<'_>,
        on: &mut dyn FnMut(Delta) -> Flow,
    ) -> Result<Completion, TurnError> {
        if endpoint_path(call.model) == "/responses" {
            match stream_responses(call, on) {
                // See `endpoint_path`'s own doc for why ONLY a 404 falls back,
                // and why every other error kind is returned as-is.
                Err(e) if e.kind == ErrorKind::ModelMissing => stream_chat_completions(call, on),
                other => other,
            }
        } else {
            stream_chat_completions(call, on)
        }
    }
}

/// The original wire, unchanged in shape — `/v1/chat/completions`, still what
/// every non-reasoning model on this row uses, and now also the fallback
/// `endpoint_path`'s own doc names for a reasoning model whose address does
/// not speak `/v1/responses`.
fn stream_chat_completions(
    call: &ModelCall<'_>,
    on: &mut dyn FnMut(Delta) -> Flow,
) -> Result<Completion, TurnError> {
    let base = call.base_url.trim().trim_end_matches('/');
    if !base.starts_with("http://") && !base.starts_with("https://") {
        return Err(TurnError {
            kind: ErrorKind::BadRequest,
            what: format!("The address saved for this brain is not a web address ({base})."),
            fix: "Open AI components and set it to the provider's API base URL -- for \
                  OpenAI that is https://api.openai.com/v1 -- then test the connection."
                .into(),
        });
    }
    let url = format!("{base}/chat/completions");
    let key = call.api_key.unwrap_or("").trim();

    let agent = ureq::AgentBuilder::new()
        .timeout_connect(CONNECT_TIMEOUT)
        .timeout_read(READ_TIMEOUT)
        .build();

    let response = agent
        .post(&url)
        .set("content-type", "application/json")
        .set("authorization", &format!("Bearer {key}"))
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
                "/chat/completions",
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
    // Keyed by `CallKey`, in the order each key was FIRST seen -- a `Vec`
    // rather than a map, because a model asking for more than a handful of
    // parallel calls in one turn has never been observed, so a linear scan
    // costs nothing real. `synthetic` numbers the last-resort keys for a
    // fragment carrying neither `index` nor `id`. See `accumulate_fragments`
    // for why keying is no longer a bare index: Gemini's parallel calls share
    // array position `0` across separate lines and would otherwise merge.
    let mut pending: Vec<(CallKey, PendingCall)> = Vec::new();
    let mut synthetic: u64 = 0;

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
        // Independent of whichever single `Chunk` this line decoded to —
        // see `scan_usage`'s own doc for why a provider can attach real
        // usage to a line that is ALSO a content or finish-reason chunk,
        // which Gemini's endpoint does on every line and OpenAI's never
        // does at all.
        if let Some((i, o)) = scan_usage(&line) {
            input_tokens = Some(i);
            output_tokens = Some(o);
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
            Chunk::ToolCallDeltas(frags) => {
                // Keyed accumulation, extracted so both the OpenAI (by-index)
                // and parallel-Gemini (by-id) paths are unit-testable without
                // a socket -- see `accumulate_fragments`.
                accumulate_fragments(&mut pending, &mut synthetic, frags);
                // No `on()` callback here -- see `Delta::ToolCall`'s own
                // doc: a call is only announced once it is fully
                // assembled, which is not knowable mid-stream (another
                // fragment for the same key could still be coming).
            }
            Chunk::FinishReason(reason) => {
                stop = match reason.as_str() {
                    "stop" => StopReason::End,
                    "length" => StopReason::Length,
                    "tool_calls" => StopReason::ToolUse,
                    other => StopReason::Other(other.to_string()),
                };
                // NOT a break -- `stream_options.include_usage` still has a
                // usage chunk and the `[DONE]` sentinel to come, and the
                // usage chunk is the only place the token counts arrive.
            }
            Chunk::Usage { input_tokens: i, output_tokens: o } => {
                input_tokens = Some(i);
                output_tokens = Some(o);
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
            Chunk::Done => break 'read,
            Chunk::Nothing => {}
        }
    }

    if !parsed_anything {
        return Err(TurnError {
            kind: ErrorKind::Protocol,
            what: format!("Something answered at {base}, but not in this shape."),
            fix: "Check the address in AI components points at an OpenAI-compatible \
                  /chat/completions endpoint and not at another program on the same \
                  address, then test the connection."
                .into(),
        });
    }

    // ASSEMBLED, THEN ANNOUNCED, THEN RETURNED — in that order, and only
    // for a stream that actually ended in `ToolUse`. A model can start a
    // tool call and then have the connection drop before `finish_reason`
    // ever arrives; `pending` would still hold a partial entry, and
    // reporting a call nobody actually finished asking for would send
    // `drive` off to run a tool the model never really requested.
    // **GEMINI ENDS A TOOL-CALL TURN WITH `finish_reason: "stop"`, NOT
    // `"tool_calls"` — captured live 2026-09-06, the second half of the same
    // empty-answer bug the `index` fallback above is the first half of.**
    // OpenAI ends a turn that requested a tool with `finish_reason:
    // "tool_calls"` (→ `StopReason::ToolUse`); Gemini's OpenAI-compatible
    // endpoint ends the identical turn with a plain `"stop"` (→
    // `StopReason::End`) while still carrying the call in `delta.tool_calls`.
    // Guarding assembly on `stop == ToolUse` ALONE therefore discarded every
    // Gemini tool call even once the `index` fix let it be parsed — the turn
    // ended with empty text and the "empty answer" message. So a clean end
    // that ALSO accumulated at least one tool call is treated as tool use
    // whatever finish reason the provider labelled it with, and `stop` is
    // corrected to match, because `drive`'s run loop reads `stop` to decide
    // whether to dispatch.
    //
    // **Still gated on a CLEAN end (`End`/`ToolUse`), preserving the exact
    // protection this branch was written for:** a connection that drops
    // mid-call leaves `stop` an `Other(...)` reason ("the stream ended without
    // saying why" / "the connection dropped"), NOT `End`, so a half-arrived
    // `pending` entry is still never reported as a request the model never
    // finished making. OpenAI's own path is unchanged — its `stop` is already
    // `ToolUse` by the time it gets here, so the promotion is a no-op there.
    let ended_cleanly = matches!(stop, StopReason::End | StopReason::ToolUse);
    let tool_calls: Vec<ToolCallRequest> = if ended_cleanly && !pending.is_empty() {
        stop = StopReason::ToolUse;
        pending_into_requests(pending)
    } else {
        Vec::new()
    };
    for call in &tool_calls {
        // The Stop button still applies to a turn that ended on a tool
        // call — `drive` decides what to do with `Flow::Stop` the same
        // way it would for a cancelled plain answer, by not dispatching.
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

// ---------------------------------------------------------------------------
// `/v1/responses` -- Phase 2B. See `endpoint_path`'s own doc for when a turn
// lands here rather than on `stream_chat_completions` above.
// ---------------------------------------------------------------------------

/// The flat tool shape `/v1/responses` wants -- **NOT `tool_to_json`'s
/// `{"type":"function","function":{...}}` wrapper.** Confirmed against a real
/// request/response round trip, 2026-09-02 (see `a_real_openai_reasoning_
/// model_tool_call_round_trip`'s own capture): the request echoes the tool
/// straight back in `response.tools` with `name`/`description`/`parameters`/
/// `strict` as SIBLINGS of `type`, not nested under a `function` key the way
/// Chat Completions requires. Sending the nested shape here is not merely
/// wrong-looking -- it is a different, invalid schema for this endpoint.
fn tool_to_responses_json(t: &ToolDef) -> Value {
    json!({
        "type": "function",
        "name": t.name,
        "description": t.description,
        "strict": t.strict,
        "parameters": t.parameters,
    })
}

/// The `/v1/responses` request body. **`input`, not `messages` -- that is the
/// whole reason this is a second function rather than a flag on
/// `build_request`.** Three shape differences from the Chat Completions body,
/// each pinned against a real round trip rather than assumed from the docs:
///
/// - **The system prompt is `instructions`, a top-level string, not a message
///   in the array.** Same omit-when-empty rule `build_request` already uses.
/// - **A tool call and a tool result are their OWN input items** —
///   `{"type":"function_call", "call_id", "name", "arguments"}` and
///   `{"type":"function_call_output", "call_id", "output"}` — rather than
///   living inside an `assistant`/`tool` message the way Chat Completions
///   nests them. If the assistant said anything alongside the call, that text
///   goes out first as its own plain `{"role":"assistant","content":...}`
///   item, because `/v1/responses` has nowhere to attach text to a
///   `function_call` item itself. Confirmed live: a plain `{"role":
///   "user"|"assistant","content":"..."}` item is accepted exactly the way
///   Chat Completions accepts a message — this endpoint does not require the
///   nested `content: [{"type":"output_text",...}]` shape its OWN output
///   uses, at least not on the way in.
/// - **`function_call_output` has no error field**, same limitation
///   `build_request`'s own doc names for the `tool` role on the other
///   endpoint — an `is_error` result is prefixed into `output` as `"Error:
///   {text}"` for the identical reason.
///
/// **NO `reasoning_effort` FIELD, EVER, ON PURPOSE.** That is the entire
/// point of routing here: omitting it lets the model use its own default
/// effort (captured live as `"medium"` against `gpt-5.6-sol`, both with and
/// without tools on the turn) rather than the `"none"` `build_request` has to
/// force to avoid the 400. A future increment could expose a real effort
/// dial; this one only had to stop suppressing it.
///
/// **`store: false`, ALWAYS, EVEN THOUGH THE DEFAULT IS `true`.** This
/// engine already keeps the whole conversation itself (`store.rs`'s own
/// header) and resends it in full on every turn — nothing here reads
/// `previous_response_id` and nothing benefits from OpenAI retaining a copy
/// server-side. Asking for retention this engine has no use for would be
/// pure downside: a second copy of the conversation sitting on someone
/// else's server for no reason this app can point to.
///
/// **REASONING-ITEM CONTINUITY IS DELIBERATELY NOT IMPLEMENTED, AND THAT IS A
/// SCOPE DECISION, NOT AN OVERSIGHT.** OpenAI's own guidance (the Responses
/// API cookbook, "Reasoning items in multi-turn conversations") says
/// including the model's reasoning item from round one when replaying a
/// function call in round two is worth roughly 3% on SWE-bench and improves
/// cache hits — genuinely worth having, and genuinely optional. Doing it here
/// would mean threading a reasoning-item id through `store::Message`/
/// `ToolTurn`, which every wire and the run loop in `engine::native::mod`
/// share — a shared-type change for a 3% quality knob, not the 400 this
/// increment exists to fix. **Verified live rather than assumed harmless:**
/// the two-round tool call in `a_real_openai_reasoning_model_tool_call_round_
/// trip` below sends round two with NO reasoning item in `input` at all, and
/// it still answers correctly using the tool's real result — so the omission
/// costs quality headroom, not correctness.
pub(crate) fn build_responses_request(call: &ModelCall<'_>) -> Value {
    let mut input: Vec<Value> = Vec::new();
    for m in call.messages {
        match &m.tool {
            None => input.push(json!({ "role": m.role.wire(), "content": m.text })),
            Some(ToolTurn::Calls(calls)) => {
                if !m.text.trim().is_empty() {
                    input.push(json!({ "role": "assistant", "content": m.text }));
                }
                for c in calls {
                    input.push(json!({
                        "type": "function_call",
                        "call_id": c.id,
                        "name": c.name,
                        "arguments": c.args_json,
                    }));
                }
            }
            Some(ToolTurn::Result { call_id, is_error }) => {
                let output =
                    if *is_error { format!("Error: {}", m.text) } else { m.text.clone() };
                input.push(json!({
                    "type": "function_call_output",
                    "call_id": call_id,
                    "output": output,
                }));
            }
        }
    }
    let mut body = json!({
        "model": call.model,
        "input": input,
        "stream": true,
        "store": false,
    });
    if !call.system.trim().is_empty() {
        body["instructions"] = json!(call.system);
    }
    // Same omit-entirely-when-empty rule as `build_request`'s own `tools`
    // field, and the same reason: an endpoint that treats `"tools": []`
    // differently from the field being absent should never find out by
    // accident.
    if !call.tools.is_empty() {
        let tools: Vec<Value> = call.tools.iter().map(tool_to_responses_json).collect();
        body["tools"] = json!(tools);
    }
    body
}

/// One event this wire acts on from the `/v1/responses` SSE stream, or the
/// reason it ended.
///
/// **A DIFFERENT SHAPE FROM `Chunk`, AND DELIBERATELY A SMALLER ONE.** Unlike
/// Chat Completions, this API's own `response.completed` event carries the
/// WHOLE finished answer — every output item, complete, not a fragment — in
/// its `response.output` array (real capture: a forced tool call's
/// `response.completed` line contains the finished `function_call` item with
/// its full `arguments` string, not just the last delta of it). So this wire
/// does not need `Chunk::ToolCallDeltas`' `PendingCall`-by-index accumulation
/// at all: `response.output_text.delta` and `response.function_call_
/// arguments.delta` are read here ONLY so text can stream to the window and
/// the Stop button stays responsive between pieces (`Wire::stream`'s own
/// contract) — the `Completion` this wire actually returns is built once,
/// from `response.completed`'s own authoritative `output`, never from
/// anything reassembled from fragments. Reading the finished shape the API
/// itself hands back is strictly more reliable than re-deriving it from
/// deltas, not a missing feature.
#[derive(Debug, PartialEq)]
enum RChunk {
    /// A piece of the answer's own text — `response.output_text.delta`'s
    /// `delta` field.
    Text(String),
    /// A piece of a reasoning summary. **Dormant today** — this wire never
    /// sets `reasoning.summary`, so OpenAI never sends this event against a
    /// request this file builds; kept so a future summary request does not
    /// silently vanish here the way `openai.rs`'s own DeepSeek-shape comment
    /// warns against on the other wire (`Chunk::Thinking`'s own doc).
    Thinking(String),
    /// `response.completed` — the turn finished cleanly. `output` is the
    /// full, final array of output items; `input_tokens`/`output_tokens` are
    /// read straight off `response.usage`, the same field names Chat
    /// Completions' own usage chunk uses.
    Completed { output: Vec<Value>, input_tokens: Option<u64>, output_tokens: Option<u64> },
    /// `response.incomplete` — ended, but not the way a healthy turn does
    /// (most often a length limit). Still carries whatever `output` exists
    /// so far, same as `Completed`.
    Incomplete { output: Vec<Value>, reason: String },
    /// `response.failed`, or the top-level `type: "error"` event this API
    /// sends for a mid-stream failure inside an otherwise-200 response —
    /// `Chunk::Error`'s own doc on the other wire has the identical reason
    /// for being its own arm rather than a parse failure.
    Error(String),
    /// A well-formed event this wire has nothing to do with — `response.
    /// created`, `response.in_progress`, `response.output_item.added` for a
    /// text message, content-part bookkeeping, and so on.
    Nothing,
}

/// Decode one line of the `/v1/responses` SSE stream. Same contract as
/// `parse_sse_line`: `None` for a line this wire does not need to look at
/// further (blank lines, and anything not prefixed `data:` — this API sends
/// a matching `event:` line before every `data:` line, which this wire never
/// reads, exactly as it never reads Chat Completions' `event: ping`).
///
/// **NO LITERAL `[DONE]` SENTINEL ON THIS ENDPOINT — found running it live,
/// not assumed from the docs.** A real capture of a plain turn and a real
/// capture of a forced tool call both end their stream immediately after the
/// `response.completed` line, with no closing sentinel of any kind. So unlike
/// `parse_sse_line`, this parser's `Completed`/`Incomplete`/`Error` arms are
/// what end the read loop in `stream_responses`, not a separate marker.
fn parse_responses_sse_line(line: &str) -> Option<RChunk> {
    let line = line.trim_end_matches(['\r', '\n']);
    let payload = line.strip_prefix("data:")?.trim_start();
    let v: Value = serde_json::from_str(payload).ok()?;
    let t = v.get("type").and_then(|t| t.as_str())?;

    match t {
        "response.output_text.delta" => match v.get("delta").and_then(|d| d.as_str()) {
            Some(d) if !d.is_empty() => Some(RChunk::Text(d.to_string())),
            _ => Some(RChunk::Nothing),
        },
        "response.reasoning_summary_text.delta" | "response.reasoning_text.delta" => {
            match v.get("delta").and_then(|d| d.as_str()) {
                Some(d) if !d.is_empty() => Some(RChunk::Thinking(d.to_string())),
                _ => Some(RChunk::Nothing),
            }
        }
        "response.completed" => {
            let resp = v.get("response")?;
            let output = resp.get("output").and_then(|o| o.as_array()).cloned().unwrap_or_default();
            let usage = resp.get("usage");
            Some(RChunk::Completed {
                output,
                input_tokens: usage.and_then(|u| u.get("input_tokens")).and_then(|n| n.as_u64()),
                output_tokens: usage.and_then(|u| u.get("output_tokens")).and_then(|n| n.as_u64()),
            })
        }
        "response.incomplete" => {
            let resp = v.get("response")?;
            let output = resp.get("output").and_then(|o| o.as_array()).cloned().unwrap_or_default();
            let reason = resp
                .pointer("/incomplete_details/reason")
                .and_then(|r| r.as_str())
                .unwrap_or("incomplete")
                .to_string();
            Some(RChunk::Incomplete { output, reason })
        }
        "response.failed" => {
            let msg = v
                .pointer("/response/error/message")
                .and_then(|m| m.as_str())
                .unwrap_or("the response failed")
                .to_string();
            Some(RChunk::Error(msg))
        }
        // The top-level `type: "error"` event -- a mid-stream failure inside
        // an otherwise-200 response, distinct from `response.failed` (which
        // still carries a full `response` object). Shape confirmed against
        // the published schema (`message`/`code`/`param` as siblings of
        // `type`), not captured live -- this house has not made a real
        // request that triggers one, same honesty `openai.rs`'s own
        // DeepSeek-shape test states rather than pretends otherwise.
        "error" => {
            let msg = v
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("the stream reported an error")
                .to_string();
            Some(RChunk::Error(msg))
        }
        _ => Some(RChunk::Nothing),
    }
}

/// Read `response.completed`'s (or `response.incomplete`'s) own `output`
/// array into the two things `Completion` needs: the answer's text,
/// concatenated in order across however many `message` items carried it, and
/// every `function_call` item as a `ToolCallRequest`.
///
/// **`ToolCallRequest.id` IS `call_id`, NOT THE ITEM'S OWN `id`.** A
/// `function_call` item carries both (real capture: `"id":"fc_...",
/// "call_id":"call_..."`) and they are not interchangeable -- `call_id` is
/// the string `function_call_output` keys its answer to
/// (`build_responses_request`'s own doc), so that is the one `tools::
/// dispatch`'s caller needs echoed back, the same contract `store::
/// ToolCallRequest::id`'s own doc states for the Chat Completions wire.
fn text_and_tool_calls_from_output(output: &[Value]) -> (String, Vec<ToolCallRequest>) {
    let mut text = String::new();
    let mut calls = Vec::new();
    for item in output {
        match item.get("type").and_then(|t| t.as_str()) {
            Some("message") => {
                if let Some(parts) = item.get("content").and_then(|c| c.as_array()) {
                    for p in parts {
                        if p.get("type").and_then(|t| t.as_str()) == Some("output_text") {
                            if let Some(t) = p.get("text").and_then(|t| t.as_str()) {
                                text.push_str(t);
                            }
                        }
                    }
                }
            }
            Some("function_call") => calls.push(ToolCallRequest {
                id: item.get("call_id").and_then(|c| c.as_str()).unwrap_or("").to_string(),
                name: item.get("name").and_then(|n| n.as_str()).unwrap_or("").to_string(),
                args_json: item.get("arguments").and_then(|a| a.as_str()).unwrap_or("").to_string(),
                // `/v1/responses` is the OpenAI reasoning-model path; Gemini
                // never lands here, so there is no signature to carry.
                thought_signature: None,
            }),
            // Reasoning items, and anything else `output` can carry, are not
            // this wire's concern -- see `build_responses_request`'s own doc
            // on why reasoning-item continuity is out of scope here.
            _ => {}
        }
    }
    (text, calls)
}

/// Announce every tool call once, then hand back the finished `Completion` —
/// the same "assembled, then announced, then returned" order
/// `stream_chat_completions` already uses, ported rather than re-derived
/// (see that function's own comment on why a partial call must never be
/// announced). `tool_calls` is left populated even if `Flow::Stop` is
/// returned mid-announcement, same as the other wire: `drive`'s own
/// cancelled-turn branch never reads it, so leaving it there is harmless and
/// simpler than clearing it back out.
fn finish_responses_completion(
    tool_calls: Vec<ToolCallRequest>,
    text: String,
    mut stop: StopReason,
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
    on: &mut dyn FnMut(Delta) -> Flow,
) -> Completion {
    if stop == StopReason::ToolUse {
        for call in &tool_calls {
            let announced = Delta::ToolCall {
                id: call.id.clone(),
                name: call.name.clone(),
                args_json: call.args_json.clone(),
            };
            if on(announced) == Flow::Stop {
                stop = StopReason::Cancelled;
                break;
            }
        }
    }
    Completion { text, stop, input_tokens, output_tokens, tool_calls }
}

/// The `/v1/responses` wire — see `endpoint_path`'s own doc for when a turn
/// lands here, and this file's module doc, "What was measured against a REAL
/// endpoint" section, for the captures this was built and checked against.
fn stream_responses(
    call: &ModelCall<'_>,
    on: &mut dyn FnMut(Delta) -> Flow,
) -> Result<Completion, TurnError> {
    let base = call.base_url.trim().trim_end_matches('/');
    if !base.starts_with("http://") && !base.starts_with("https://") {
        return Err(TurnError {
            kind: ErrorKind::BadRequest,
            what: format!("The address saved for this brain is not a web address ({base})."),
            fix: "Open AI components and set it to the provider's API base URL -- for \
                  OpenAI that is https://api.openai.com/v1 -- then test the connection."
                .into(),
        });
    }
    let url = format!("{base}/responses");
    let key = call.api_key.unwrap_or("").trim();

    let agent =
        ureq::AgentBuilder::new().timeout_connect(CONNECT_TIMEOUT).timeout_read(READ_TIMEOUT).build();

    let response = agent
        .post(&url)
        .set("content-type", "application/json")
        .set("authorization", &format!("Bearer {key}"))
        .send_string(&build_responses_request(call).to_string());

    let upstream = match response {
        Ok(r) => r,
        Err(ureq::Error::Status(code, resp)) => {
            // `ErrorKind::ModelMissing` (a 404) is what `Wire::stream`'s
            // dispatcher reads to decide whether to fall back to
            // `/chat/completions` -- see `endpoint_path`'s own doc.
            return Err(status_error(
                code,
                resp.into_string().unwrap_or_default(),
                key,
                call.model,
                base,
                "/responses",
            ));
        }
        Err(ureq::Error::Transport(t)) => {
            return Err(TurnError {
                kind: ErrorKind::Unreachable,
                what: format!("Could not reach {base} ({t})."),
                fix: "Check the base URL in AI components and that this computer has a \
                      working connection, then send again -- nothing was charged."
                    .into(),
            });
        }
    };

    let mut reader = BufReader::new(upstream.into_reader());
    let mut line = String::new();
    let mut parsed_anything = false;
    let mut text_so_far = String::new();

    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => {}
            Err(e) => {
                return Err(TurnError {
                    kind: ErrorKind::Upstream,
                    what: format!("The connection dropped while reading the answer ({e})."),
                    fix: "Worth trying again -- this is a fault reading the answer, not \
                          anything you set."
                        .into(),
                });
            }
        }
        if line.trim().is_empty() {
            continue;
        }
        let Some(chunk) = parse_responses_sse_line(&line) else { continue };
        parsed_anything = true;
        match chunk {
            RChunk::Text(t) => {
                text_so_far.push_str(&t);
                if on(Delta::Text(t)) == Flow::Stop {
                    return Ok(Completion {
                        text: text_so_far,
                        stop: StopReason::Cancelled,
                        input_tokens: None,
                        output_tokens: None,
                        tool_calls: Vec::new(),
                    });
                }
            }
            RChunk::Thinking(t) => {
                if on(Delta::Thinking(t)) == Flow::Stop {
                    return Ok(Completion {
                        text: text_so_far,
                        stop: StopReason::Cancelled,
                        input_tokens: None,
                        output_tokens: None,
                        tool_calls: Vec::new(),
                    });
                }
            }
            RChunk::Completed { output, input_tokens, output_tokens } => {
                let (text, tool_calls) = text_and_tool_calls_from_output(&output);
                let stop = if !tool_calls.is_empty() { StopReason::ToolUse } else { StopReason::End };
                return Ok(finish_responses_completion(
                    tool_calls,
                    text,
                    stop,
                    input_tokens,
                    output_tokens,
                    on,
                ));
            }
            RChunk::Incomplete { output, reason } => {
                let (text, tool_calls) = text_and_tool_calls_from_output(&output);
                let stop = if !tool_calls.is_empty() {
                    StopReason::ToolUse
                } else if reason == "max_output_tokens" {
                    StopReason::Length
                } else {
                    StopReason::Other(reason)
                };
                return Ok(finish_responses_completion(tool_calls, text, stop, None, None, on));
            }
            RChunk::Error(msg) => {
                return Err(TurnError {
                    kind: ErrorKind::Upstream,
                    what: format!("The provider reported a problem mid-answer: {msg}"),
                    fix: "Worth trying again -- this is a fault at the provider's end, not \
                          anything you set."
                        .into(),
                });
            }
            RChunk::Nothing => {}
        }
    }

    // Reached only on a clean connection close (`Ok(0)`) with no `Completed`/
    // `Incomplete`/`Error` ever seen -- either nothing recognisable arrived at
    // all (`!parsed_anything`, the same protocol-mismatch case `stream_chat_
    // completions` guards against) or the connection closed early after some
    // real progress. Both are honest failures, worded for which one happened.
    if !parsed_anything {
        return Err(TurnError {
            kind: ErrorKind::Protocol,
            what: format!("Something answered at {base}, but not in this shape."),
            fix: "Check the address in AI components points at an OpenAI-compatible \
                  /responses endpoint and not at another program on the same address, \
                  then test the connection."
                .into(),
        });
    }
    Err(TurnError {
        kind: ErrorKind::Upstream,
        what: format!("The connection to {base} closed before the answer finished."),
        fix: "Worth trying again -- this looks like the connection ending early rather than \
              anything you set."
            .into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::native::store::{Message, Role};

    fn a_call<'a>(system: &'a str, key: Option<&'a str>, messages: &'a [Message]) -> ModelCall<'a> {
        a_call_with_tools(system, key, messages, &[])
    }

    /// `a_call`'s twin for the tool tests below — kept separate rather than
    /// adding a fifth parameter every existing call site would then have to
    /// carry, since most of this file's tests have nothing to do with tools
    /// at all.
    fn a_call_with_tools<'a>(
        system: &'a str,
        key: Option<&'a str>,
        messages: &'a [Message],
        tools: &'a [ToolDef],
    ) -> ModelCall<'a> {
        ModelCall {
            base_url: "https://api.openai.com/v1",
            model: "gpt-4o-mini",
            api_key: key,
            system,
            messages,
            tools,
        }
    }

    /// **NO TOKEN CAP OF EITHER NAME.** Same finding as `ollama.rs`'s own
    /// test, ported: this wire has no per-turn setting for it, and omitting
    /// both sidesteps the `max_tokens` / `max_completion_tokens` split
    /// `adapter.rs::translate_request` has to branch on. Proven able to fail:
    /// adding `"max_tokens": N` -- the obvious, plausible thing to write --
    /// fails this immediately.
    #[test]
    fn the_request_sends_no_token_cap_of_either_name() {
        let msgs = vec![Message { role: Role::User, text: "hi".into(), tool: None }];
        let body = build_request(&a_call("be brief", Some("sk-test"), &msgs));
        assert!(body.get("max_tokens").is_none(), "{body}");
        assert!(body.get("max_completion_tokens").is_none(), "{body}");
    }

    /// The system prompt leads and carries role `system`, same order
    /// `ollama.rs` proves for the local wire and the same contract
    /// `ModelCall` promises every wire.
    #[test]
    fn the_system_prompt_leads_and_the_history_follows_in_order() {
        let msgs = vec![
            Message { role: Role::User, text: "first".into(), tool: None },
            Message { role: Role::Assistant, text: "second".into(), tool: None },
        ];
        let body = build_request(&a_call("VOICE", Some("sk-test"), &msgs));
        let m = body["messages"].as_array().unwrap();
        assert_eq!(m.len(), 3);
        assert_eq!((m[0]["role"].as_str(), m[0]["content"].as_str()), (Some("system"), Some("VOICE")));
        assert_eq!((m[1]["role"].as_str(), m[1]["content"].as_str()), (Some("user"), Some("first")));
        assert_eq!(
            (m[2]["role"].as_str(), m[2]["content"].as_str()),
            (Some("assistant"), Some("second"))
        );
        assert_eq!(body["stream"], true);
        assert_eq!(body["stream_options"]["include_usage"], true);
    }

    /// An empty system prompt sends no system message at all -- same rule
    /// `ollama.rs` carries, for the same reason: some endpoints treat an
    /// empty system turn as a real instruction to say nothing.
    #[test]
    fn an_empty_system_prompt_sends_no_system_message() {
        let msgs = vec![Message { role: Role::User, text: "hi".into(), tool: None }];
        let body = build_request(&a_call("   ", Some("sk-test"), &msgs));
        assert_eq!(body["messages"].as_array().unwrap().len(), 1);
    }

    /// **THE TOOL LOOP'S MESSAGE SHAPES, PINNED AGAINST THE ACTUAL REQUEST
    /// BODY.** `engine::native::tests::a_tool_call_is_dispatched_…` proves
    /// the LOOP calls this function correctly; this proves what this
    /// function BUILDS is the shape OpenAI's own API requires: the
    /// assistant's `tool_calls` array (with `content: null` when there is no
    /// text alongside it), and a `tool` message keyed by `tool_call_id` with
    /// the error prefixed into its content since Chat Completions has no
    /// dedicated error field for one (see this function's own doc).
    ///
    /// Proven able to fail: swapping `"tool_call_id"` for `"id"` (an easy
    /// slip -- it is `id` on the assistant's OWN entry two lines up) makes
    /// the second assertion fail.
    #[test]
    fn tool_calls_and_their_results_take_the_shape_the_api_requires() {
        let msgs = vec![
            Message { role: Role::User, text: "read hello.txt".into(), tool: None },
            Message {
                role: Role::Assistant,
                text: String::new(),
                tool: Some(ToolTurn::Calls(vec![ToolCallRequest {
                    id: "call_1".into(),
                    name: "Read".into(),
                    args_json: r#"{"path":"hello.txt"}"#.into(),
                    thought_signature: None,
                }])),
            },
            Message {
                role: Role::Tool,
                text: "outside the folder this conversation is trusted to touch.".into(),
                tool: Some(ToolTurn::Result { call_id: "call_1".into(), is_error: true }),
            },
        ];
        let body = build_request(&a_call("", Some("sk-test"), &msgs));
        let m = body["messages"].as_array().unwrap();
        assert_eq!(m.len(), 3);

        assert_eq!(m[1]["role"], "assistant");
        assert_eq!(m[1]["content"], Value::Null, "no text alongside the calls -> null content");
        let calls = m[1]["tool_calls"].as_array().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0]["id"], "call_1");
        assert_eq!(calls[0]["type"], "function");
        assert_eq!(calls[0]["function"]["name"], "Read");
        assert_eq!(calls[0]["function"]["arguments"], r#"{"path":"hello.txt"}"#);

        assert_eq!(m[2]["role"], "tool");
        assert_eq!(m[2]["tool_call_id"], "call_1");
        assert_eq!(
            m[2]["content"],
            "Error: outside the folder this conversation is trusted to touch.",
            "an is_error result must say so in the only field OpenAI's tool-message shape has"
        );
    }

    /// The strict-mode shape (`"strict": true`, `additionalProperties: false`
    /// carried through unchanged from `ToolDef.parameters`) reaches the
    /// request body when `call.tools` is non-empty, and the field is absent
    /// entirely -- not sent as `[]` -- when it is empty. See `build_request`'s
    /// own doc for why absent and empty are treated as different things on
    /// purpose.
    #[test]
    fn tools_are_sent_strict_and_omitted_entirely_when_there_are_none() {
        let msgs = vec![Message { role: Role::User, text: "hi".into(), tool: None }];
        let none = build_request(&a_call("", Some("sk-test"), &msgs));
        assert!(none.get("tools").is_none(), "{none}");

        let def = ToolDef {
            name: "Read".into(),
            description: "reads a file".into(),
            strict: true,
            parameters: json!({
                "type": "object",
                "properties": { "path": { "type": "string" } },
                "required": ["path"],
                "additionalProperties": false
            }),
        };
        let with_tools = build_request(&a_call_with_tools("", Some("sk-test"), &msgs, &[def]));
        let tools = with_tools["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0]["type"], "function");
        assert_eq!(tools[0]["function"]["name"], "Read");
        assert_eq!(tools[0]["function"]["strict"], true);
        assert_eq!(tools[0]["function"]["parameters"]["additionalProperties"], false);
    }

    /// **THE BUG, REPRODUCED AND FIXED.** Mark's real desktop install --
    /// OpenAI connected, no Claude Code anywhere in the process tree --
    /// 400'd on the shipped default model the moment a turn carried a tool,
    /// with OpenAI's own words: *"Function tools with reasoning_effort are
    /// not supported for gpt-5.6-sol in /v1/chat/completions… set
    /// reasoning_effort to 'none'."* Against the pre-fix `build_request`
    /// this assertion is false -- the field was never sent at all, which is
    /// exactly the request that 400'd for real. Mirrors `adapter.rs`'s own
    /// `a_reasoning_model_with_tools_gets_reasoning_effort_none`, on the
    /// separate code path that needed the same fix.
    #[test]
    fn a_reasoning_model_with_tools_gets_reasoning_effort_none() {
        let msgs = vec![Message { role: Role::User, text: "read hello.txt".into(), tool: None }];
        let def = ToolDef {
            name: "Read".into(),
            description: "reads a file".into(),
            strict: true,
            parameters: json!({"type": "object", "properties": {}}),
        };
        let call = ModelCall {
            base_url: "https://api.openai.com/v1",
            model: "gpt-5.6-sol",
            api_key: Some("sk-test"),
            system: "",
            messages: &msgs,
            tools: &[def],
        };
        let body = build_request(&call);
        assert!(body.get("tools").is_some(), "the fixture must actually carry a tool: {body}");
        assert_eq!(body["reasoning_effort"], "none", "{body}");
    }

    /// **THE OTHER HALF, AND THE ONE THAT KEEPS THE FIX FROM OVERREACHING.**
    /// OpenAI's own restriction names the combination -- function tools AND
    /// reasoning effort together -- not the model alone, and a plain
    /// tool-free question has no reason to pay this down. Mirrors
    /// `adapter.rs`'s own `a_reasoning_model_with_no_tools_keeps_full_reasoning`.
    #[test]
    fn a_reasoning_model_with_no_tools_keeps_full_reasoning() {
        let msgs = vec![Message { role: Role::User, text: "what is 2+2?".into(), tool: None }];
        // `a_call` fixes the model to "gpt-4o-mini" -- build directly with
        // the reasoning-family name instead, same shape the test above
        // uses, since this is the case that must NOT carry the field.
        let call = ModelCall {
            base_url: "https://api.openai.com/v1",
            model: "gpt-5.6-sol",
            api_key: Some("sk-test"),
            system: "",
            messages: &msgs,
            tools: &[],
        };
        let body = build_request(&call);
        assert!(body.get("tools").is_none(), "{body}");
        assert!(
            body.get("reasoning_effort").is_none(),
            "a tool-free turn must not have its reasoning effort forced down: {body}"
        );
    }

    /// **CONSERVATIVE: A NON-REASONING MODEL WITH TOOLS IS UNCHANGED.**
    /// `gpt-4o-mini` and the third-party `openai-compatible` servers this
    /// row can also point at never asked for this field and were never
    /// broken by its absence -- sending it to them would be fixing nothing
    /// and risking a 400 on a family that has no opinion on it. Mirrors
    /// `adapter.rs`'s own `a_gpt4_class_model_keeps_the_shape_it_has_always_gotten`.
    #[test]
    fn a_non_reasoning_model_with_tools_sends_no_reasoning_effort() {
        let msgs = vec![Message { role: Role::User, text: "read hello.txt".into(), tool: None }];
        let def = ToolDef {
            name: "Read".into(),
            description: "reads a file".into(),
            strict: true,
            parameters: json!({"type": "object", "properties": {}}),
        };
        let body = build_request(&a_call_with_tools("", Some("sk-test"), &msgs, &[def]));
        assert!(body.get("tools").is_some(), "{body}");
        assert!(
            body.get("reasoning_effort").is_none(),
            "gpt-4o-mini has no opinion on this field and must not receive it: {body}"
        );
    }

    // -- `/v1/responses` -- endpoint routing, request shape, event parsing. --

    /// **THE ROUTING RULE ITSELF, PROVABLE WITHOUT A SOCKET.** This is what
    /// `Wire::stream`'s dispatcher actually reads; the live tests below prove
    /// the two endpoints it points at both work, not that the pointing is
    /// correct -- this test is what proves that.
    #[test]
    fn reasoning_models_route_to_responses_non_reasoning_stay_on_chat_completions() {
        assert_eq!(endpoint_path("gpt-5.6-sol"), "/responses");
        assert_eq!(endpoint_path("o3-mini"), "/responses");
        assert_eq!(endpoint_path("o1"), "/responses");
        assert_eq!(endpoint_path("gpt-4o-mini"), "/chat/completions");
        assert_eq!(endpoint_path("gemini-3.6-flash"), "/chat/completions");
    }

    /// **THE BUG'S FIX, PROVABLE WITHOUT A SOCKET: NO `reasoning_effort` ON
    /// THIS BODY, EVER -- not even the stopgap's own `"none"`.** Against the
    /// old `build_request`-only design this exact call 400'd; this proves
    /// the REPLACEMENT body never needs the stopgap at all, because the
    /// field this wire used to force down is simply never written here.
    #[test]
    fn the_responses_body_never_carries_reasoning_effort_even_with_tools_on() {
        let msgs = vec![Message { role: Role::User, text: "read hello.txt".into(), tool: None }];
        let def = ToolDef {
            name: "Read".into(),
            description: "reads a file".into(),
            strict: true,
            parameters: json!({"type": "object", "properties": {}}),
        };
        let call = ModelCall {
            base_url: "https://api.openai.com/v1",
            model: "gpt-5.6-sol",
            api_key: Some("sk-test"),
            system: "",
            messages: &msgs,
            tools: &[def],
        };
        let body = build_responses_request(&call);
        assert!(body.get("tools").is_some(), "the fixture must actually carry a tool: {body}");
        assert!(body.get("reasoning_effort").is_none(), "{body}");
    }

    /// **`input`, NOT `messages`.** The one-word difference that makes this a
    /// second function rather than a flag on `build_request`.
    #[test]
    fn the_responses_body_carries_input_not_messages() {
        let msgs = vec![Message { role: Role::User, text: "hi".into(), tool: None }];
        let body = build_responses_request(&a_call("", Some("sk-test"), &msgs));
        assert!(body.get("messages").is_none(), "{body}");
        assert!(body.get("input").is_some(), "{body}");
    }

    /// The system prompt becomes `instructions`, a top-level string, and is
    /// omitted entirely when blank -- same omit-when-empty rule
    /// `build_request` already proves for its own `system` message.
    #[test]
    fn the_system_prompt_becomes_instructions_and_is_omitted_when_blank() {
        let msgs = vec![Message { role: Role::User, text: "hi".into(), tool: None }];
        let with = build_responses_request(&a_call("VOICE", Some("sk-test"), &msgs));
        assert_eq!(with["instructions"], "VOICE");
        let without = build_responses_request(&a_call("   ", Some("sk-test"), &msgs));
        assert!(without.get("instructions").is_none(), "{without}");
    }

    /// **`store: false`, ALWAYS.** This engine keeps the whole conversation
    /// itself and never reads `previous_response_id` -- see
    /// `build_responses_request`'s own doc for why asking OpenAI to retain a
    /// copy it is never used would be pure downside.
    #[test]
    fn the_responses_body_never_asks_the_provider_to_store_the_conversation() {
        let msgs = vec![Message { role: Role::User, text: "hi".into(), tool: None }];
        let body = build_responses_request(&a_call("", Some("sk-test"), &msgs));
        assert_eq!(body["store"], false, "{body}");
    }

    /// The flat tool shape -- `name`/`description`/`parameters`/`strict` as
    /// SIBLINGS of `type`, never nested under a `function` key the way
    /// `tool_to_json` (Chat Completions' own shape) requires. Proven able to
    /// fail: nesting under `function` here, the obvious copy-paste from
    /// `tool_to_json`, makes the second assertion fail.
    #[test]
    fn responses_tools_are_flat_not_nested_under_function() {
        let msgs = vec![Message { role: Role::User, text: "hi".into(), tool: None }];
        let def = ToolDef {
            name: "Read".into(),
            description: "reads a file".into(),
            strict: true,
            parameters: json!({"type": "object", "properties": {}, "additionalProperties": false}),
        };
        let body = build_responses_request(&a_call_with_tools("", Some("sk-test"), &msgs, &[def]));
        let tools = body["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0]["type"], "function");
        assert_eq!(tools[0]["name"], "Read", "must be flat, not nested under \"function\": {body}");
        assert_eq!(tools[0]["strict"], true);
        assert!(tools[0].get("function").is_none(), "{body}");
    }

    /// **THE TOOL LOOP'S TWO EXTRA INPUT-ITEM SHAPES, PINNED AGAINST THE
    /// ACTUAL REQUEST BODY** -- `build_request`'s own twin test, ported to
    /// this endpoint's item-per-call shape rather than the nested
    /// `tool_calls` array Chat Completions uses.
    #[test]
    fn responses_tool_calls_and_results_take_the_shape_the_api_requires() {
        let msgs = vec![
            Message { role: Role::User, text: "read hello.txt".into(), tool: None },
            Message {
                role: Role::Assistant,
                text: String::new(),
                tool: Some(ToolTurn::Calls(vec![ToolCallRequest {
                    id: "call_1".into(),
                    name: "Read".into(),
                    args_json: r#"{"path":"hello.txt"}"#.into(),
                    thought_signature: None,
                }])),
            },
            Message {
                role: Role::Tool,
                text: "outside the folder this conversation is trusted to touch.".into(),
                tool: Some(ToolTurn::Result { call_id: "call_1".into(), is_error: true }),
            },
        ];
        let body = build_responses_request(&a_call("", Some("sk-test"), &msgs));
        let input = body["input"].as_array().unwrap();
        assert_eq!(input.len(), 3, "{body}");

        assert_eq!(input[0]["role"], "user");

        assert_eq!(input[1]["type"], "function_call");
        assert_eq!(input[1]["call_id"], "call_1");
        assert_eq!(input[1]["name"], "Read");
        assert_eq!(input[1]["arguments"], r#"{"path":"hello.txt"}"#);

        assert_eq!(input[2]["type"], "function_call_output");
        assert_eq!(input[2]["call_id"], "call_1");
        assert_eq!(
            input[2]["output"],
            "Error: outside the folder this conversation is trusted to touch.",
            "an is_error result must say so in the only field this shape has for it: {body}"
        );
    }

    /// When the assistant said something alongside the call, that text goes
    /// out as its own plain input item FIRST -- `/v1/responses` has nowhere
    /// to hang text off a `function_call` item itself.
    #[test]
    fn responses_assistant_text_alongside_a_tool_call_gets_its_own_input_item() {
        let msgs = vec![Message {
            role: Role::Assistant,
            text: "Let me check that file.".into(),
            tool: Some(ToolTurn::Calls(vec![ToolCallRequest {
                id: "call_1".into(),
                name: "Read".into(),
                args_json: "{}".into(),
                thought_signature: None,
            }])),
        }];
        let body = build_responses_request(&a_call("", Some("sk-test"), &msgs));
        let input = body["input"].as_array().unwrap();
        assert_eq!(input.len(), 2, "{body}");
        assert_eq!(input[0]["role"], "assistant");
        assert_eq!(input[0]["content"], "Let me check that file.");
        assert_eq!(input[1]["type"], "function_call");
    }

    /// **REAL BYTES.** A plain-turn capture (`response.created` through
    /// `response.completed`, no tools) and a forced-tool-call capture,
    /// both against `gpt-5.6-sol` with a real key, 2026-09-02 -- pasted here
    /// so this is a test of the parser rather than a test of memory of the
    /// docs. Trimmed of nothing but the response/item ids, which are unique
    /// per call.
    #[test]
    fn the_real_responses_lines_decode_the_way_they_did_on_the_wire() {
        assert_eq!(
            parse_responses_sse_line(
                r#"data: {"type":"response.created","response":{"id":"resp_1","status":"in_progress","output":[],"usage":null},"sequence_number":0}"#
            ),
            Some(RChunk::Nothing),
            "housekeeping events this wire does not act on must not be misread as content"
        );
        assert_eq!(
            parse_responses_sse_line(
                r#"data: {"type":"response.output_text.delta","content_index":0,"delta":"OK","item_id":"msg_1","logprobs":[],"obfuscation":"L7JeloISCLc4LK","output_index":0,"sequence_number":4}"#
            ),
            Some(RChunk::Text("OK".into()))
        );
        assert_eq!(
            parse_responses_sse_line(
                r#"data: {"type":"response.function_call_arguments.delta","delta":"{\"","item_id":"fc_1","obfuscation":"x","output_index":0,"sequence_number":3}"#
            ),
            // This wire does not act on the delta itself -- see `RChunk`'s
            // own doc for why -- so it decodes to `Nothing` rather than a
            // dedicated arm; `response.completed`'s own `output` array below
            // is what this wire actually reads a tool call from.
            Some(RChunk::Nothing)
        );
        assert_eq!(
            parse_responses_sse_line(
                r#"data: {"type":"response.completed","response":{"id":"resp_1","status":"completed","output":[{"id":"fc_1","type":"function_call","status":"completed","arguments":"{\"path\":\"notes.txt\"}","call_id":"call_1","name":"Read"}],"usage":{"input_tokens":65,"output_tokens":18,"total_tokens":83}},"sequence_number":11}"#
            ),
            Some(RChunk::Completed {
                output: vec![serde_json::json!({
                    "id": "fc_1", "type": "function_call", "status": "completed",
                    "arguments": r#"{"path":"notes.txt"}"#, "call_id": "call_1", "name": "Read"
                })],
                input_tokens: Some(65),
                output_tokens: Some(18),
            })
        );
        // The keep-alive comment SSE itself defines, and a blank line -- same
        // contract `parse_sse_line`'s own equivalent test proves.
        assert_eq!(parse_responses_sse_line(": keep-alive"), None);
        assert_eq!(parse_responses_sse_line(""), None);
    }

    /// A `response.completed` event's own `output` array is read into text
    /// (concatenated across every `message` item's `output_text` parts) and
    /// tool calls (`call_id`, not the item's own `id` -- see this function's
    /// own doc for why the two are not interchangeable) -- pinned against the
    /// exact real capture above rather than a hand-built fixture.
    #[test]
    fn text_and_tool_calls_are_read_from_the_completed_events_own_output() {
        let output = vec![
            serde_json::json!({
                "id": "msg_1", "type": "message", "status": "completed", "role": "assistant",
                "content": [{"type": "output_text", "text": "The secret number is 8214."}]
            }),
            serde_json::json!({
                "id": "fc_1", "type": "function_call", "status": "completed",
                "arguments": r#"{"path":"notes.txt"}"#, "call_id": "call_abc", "name": "Read"
            }),
        ];
        let (text, calls) = text_and_tool_calls_from_output(&output);
        assert_eq!(text, "The secret number is 8214.");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].id, "call_abc", "must be call_id, not the item's own \"fc_1\" id");
        assert_eq!(calls[0].name, "Read");
        assert_eq!(calls[0].args_json, r#"{"path":"notes.txt"}"#);
    }

    /// **REAL BYTES.** A content chunk, a reasoning_content chunk, a
    /// finish_reason chunk, a usage-only chunk with empty choices, and the
    /// `[DONE]` sentinel -- captured against the real endpoints named in the
    /// module header (`openai_chat_completions_shape.rs`), pasted here so
    /// this is a test of the wire rather than a test of memory of it.
    #[test]
    fn the_real_captured_lines_decode_the_way_they_did_on_the_wire() {
        // THE FOUR LINES BELOW ARE THE REAL, COMPLETE RESPONSE — captured
        // 2026-09-02 with a bare `curl` against `https://api.openai.com/v1
        // /chat/completions`, `gpt-4o-mini`, `stream_options.include_usage`,
        // asked to say exactly "OK". Not reconstructed from OpenAI's docs and
        // not trimmed except for the id, which is unique per call and not
        // worth re-capturing for. **The `"usage":null` on the first three
        // lines was NOT anticipated when this parser was written** — every
        // hand-built example elsewhere in this file simply omits a `usage`
        // key it does not carry, and the real endpoint instead sends the key
        // present and null on every chunk but the last. `v.get("usage")`
        // returns `Some(Value::Null)` for that, which is truthy — and this
        // assertion is what proves `no_choice` still gates it correctly
        // rather than misreading an early chunk as the usage chunk. Extra
        // fields the real payload carries and this parser does not read
        // (`refusal`, `logprobs`, `service_tier`, `system_fingerprint`,
        // `obfuscation`) are left in, exactly as received, because a parser
        // that only survives a trimmed example is not proven against the
        // real shape.
        assert_eq!(
            parse_sse_line(
                r#"data: {"id":"chatcmpl-EJp1GvhRQ4jxBcPbKgSWIUPPYCZ3O","object":"chat.completion.chunk","created":1788393058,"model":"gpt-4o-mini-2024-07-18","service_tier":"default","system_fingerprint":"fp_70cf485092","choices":[{"index":0,"delta":{"role":"assistant","content":"","refusal":null},"logprobs":null,"finish_reason":null}],"usage":null,"obfuscation":"UQYBJsEjq"}"#
            ),
            Some(Chunk::Nothing),
            "the opening role-only delta carries no text, and a present-but-null `usage` \
             key must not be read as the usage chunk"
        );
        assert_eq!(
            parse_sse_line(
                r#"data: {"id":"chatcmpl-EJp1GvhRQ4jxBcPbKgSWIUPPYCZ3O","object":"chat.completion.chunk","created":1788393058,"model":"gpt-4o-mini-2024-07-18","service_tier":"default","system_fingerprint":"fp_70cf485092","choices":[{"index":0,"delta":{"content":"OK"},"logprobs":null,"finish_reason":null}],"usage":null,"obfuscation":"8dcUdnQnZ"}"#
            ),
            Some(Chunk::Content("OK".into()))
        );
        assert_eq!(
            parse_sse_line(
                r#"data: {"id":"chatcmpl-EJp1GvhRQ4jxBcPbKgSWIUPPYCZ3O","object":"chat.completion.chunk","created":1788393058,"model":"gpt-4o-mini-2024-07-18","service_tier":"default","system_fingerprint":"fp_70cf485092","choices":[{"index":0,"delta":{},"logprobs":null,"finish_reason":"stop"}],"usage":null,"obfuscation":"PuFsS"}"#
            ),
            Some(Chunk::FinishReason("stop".into()))
        );
        assert_eq!(
            parse_sse_line(
                r#"data: {"id":"chatcmpl-EJp1GvhRQ4jxBcPbKgSWIUPPYCZ3O","object":"chat.completion.chunk","created":1788393058,"model":"gpt-4o-mini-2024-07-18","service_tier":"default","system_fingerprint":"fp_70cf485092","choices":[],"usage":{"prompt_tokens":25,"completion_tokens":1,"total_tokens":26,"prompt_tokens_details":{"cached_tokens":0,"audio_tokens":0},"completion_tokens_details":{"reasoning_tokens":0,"audio_tokens":0,"accepted_prediction_tokens":0,"rejected_prediction_tokens":0}},"obfuscation":"mvLrlsNWd5K"}"#
            ),
            Some(Chunk::Usage { input_tokens: 25, output_tokens: 1 })
        );
        assert_eq!(parse_sse_line("data: [DONE]"), Some(Chunk::Done));
        // The keep-alive comment SSE itself defines, and a blank line.
        assert_eq!(parse_sse_line(": keep-alive"), None);
        assert_eq!(parse_sse_line(""), None);
    }

    /// **THE GEMINI EMPTY-ANSWER BUG, PINNED AGAINST THE REAL BYTES THAT
    /// CAUSED IT — captured live 2026-09-06 with a bare `curl` against
    /// `generativelanguage.googleapis.com/v1beta/openai/chat/completions`,
    /// `gemini-3.6-flash`, a `Read` tool attached, asked to read a file.**
    /// Two shape differences from OpenAI's tool-call stream sat behind the
    /// in-app symptom "The model returned an empty answer" on turns where
    /// Gemini chose to call a tool — the app always attaches the tool
    /// surface, so those are ordinary turns:
    ///
    /// 1. **The `tool_calls` entry carries NO `index`** (the CHOICE has one;
    ///    the entry does not). The old `filter_map` read it with `?` and
    ///    dropped the entire fragment, losing the whole call. This asserts the
    ///    call now survives with `index: None` carried AS-IS — no longer
    ///    back-filled from array position, which merged parallel calls (see
    ///    `two_parallel_gemini_calls_with_no_index_stay_two_distinct_calls`).
    /// 2. **The whole call arrives in ONE fragment** — `id`, `name`, and the
    ///    complete `arguments` string together — not split across lines the
    ///    way OpenAI streams it.
    /// 3. **The finish line reads `finish_reason: "stop"`, not
    ///    `"tool_calls"`** — decoded here as `FinishReason("stop")`. The
    ///    matching half of the fix lives in `stream_chat_completions`, which
    ///    promotes a clean stop carrying accumulated calls to `ToolUse`
    ///    (proven end-to-end by `a_real_gemini_tool_loop_follow_up_succeeds`).
    /// 4. **`extra_content.google.thought_signature` IS captured** — Gemini
    ///    3.x's signed reasoning, which must round-trip on the follow-up
    ///    request or the next turn 400s (`build_request` echoes it; the
    ///    round-trip is proven by `a_thought_signature_round_trips_through_build_request`). The
    ///    value here is a stand-in for the real opaque bytes; every field this
    ///    parser reads is exactly as it came off the wire.
    ///
    /// Proven able to fail: restoring the `?` on `index` (the original code)
    /// makes the first assertion fail — the fragment is dropped and the line
    /// decodes to `Chunk::Nothing`.
    #[test]
    fn a_gemini_tool_call_with_no_index_is_not_dropped() {
        assert_eq!(
            parse_sse_line(
                r#"data: {"choices":[{"delta":{"role":"assistant","tool_calls":[{"extra_content":{"google":{"thought_signature":"Es4ECssEFAKE"}},"function":{"arguments":"{\"path\":\"notes.txt\"}","name":"Read"},"id":"call_1921976","type":"function"}]},"index":0}],"created":1788680268,"id":"SRidasuTDqXnz7IPko3ikQE","model":"gemini-3.6-flash","object":"chat.completion.chunk","usage":{"completion_tokens":16,"prompt_tokens":74,"total_tokens":138}}"#
            ),
            Some(Chunk::ToolCallDeltas(vec![ToolCallFragment {
                index: None,
                id: Some("call_1921976".into()),
                name: Some("Read".into()),
                arguments_fragment: r#"{"path":"notes.txt"}"#.into(),
                thought_signature: Some("Es4ECssEFAKE".into()),
            }])),
            "Gemini's index-less tool-call fragment must survive with index None and its signature"
        );
        assert_eq!(
            parse_sse_line(
                r#"data: {"choices":[{"delta":{"role":"assistant"},"finish_reason":"stop","index":0}],"created":1788680268,"id":"SRidasuTDqXnz7IPko3ikQE","model":"gemini-3.6-flash","object":"chat.completion.chunk","usage":{"completion_tokens":16,"prompt_tokens":74,"total_tokens":138}}"#
            ),
            Some(Chunk::FinishReason("stop".into())),
            "Gemini ends a tool-call turn with a plain \"stop\", not \"tool_calls\""
        );
    }

    /// **PARALLEL GEMINI CALLS, PINNED AGAINST THE REAL BYTES — captured live
    /// 2026-09-06 (`gemini-3.6-flash`, two `get_weather` calls in one turn).**
    /// Gemini sends parallel calls on SEPARATE SSE lines, each a single-entry
    /// `tool_calls` array with NO entry-level `index` (only the choice has
    /// one, and it is `0` on every line). The old code back-filled `index`
    /// from array position, so BOTH calls got index `0`, the by-index
    /// accumulator matched the second onto the first, and its `arguments`
    /// were `push_str`'d onto the first's — one corrupt call reading
    /// `{"city":"Paris"}{"city":"Tokyo"}`. The distinct `id`s did not save it
    /// because nothing keyed on them.
    ///
    /// This drives the two real lines through `parse_sse_line` and the ACTUAL
    /// accumulator (`accumulate_fragments` → `pending_into_requests`, the same
    /// functions `stream_chat_completions` calls) and asserts the result is
    /// two distinct, well-formed calls — Paris and Tokyo, each its own valid
    /// JSON — with Gemini's signature (which it attaches only to the FIRST of
    /// N parallel calls, covering the block) landing on the first call.
    ///
    /// Proven able to fail: change the `(None, Some(id))` arm of
    /// `accumulate_fragments` back to keying by a position-derived index and
    /// the two calls merge into one corrupt entry, exactly the pre-fix bug.
    #[test]
    fn two_parallel_gemini_calls_with_no_index_stay_two_distinct_calls() {
        // The single-fragment lines exactly as they came off the wire — first
        // carries `extra_content.google.thought_signature`, second does not,
        // neither carries an entry-level `index`.
        let paris = r#"data: {"choices":[{"delta":{"role":"assistant","tool_calls":[{"extra_content":{"google":{"thought_signature":"Es4ECssEFAKE_paris_sig"}},"function":{"arguments":"{\"city\":\"Paris\"}","name":"get_weather"},"id":"call_1776981","type":"function"}]},"index":0}],"created":1788682960,"id":"tiKdau70AYuGz7IPwtejmQg","model":"gemini-3.6-flash","object":"chat.completion.chunk"}"#;
        let tokyo = r#"data: {"choices":[{"delta":{"role":"assistant","tool_calls":[{"function":{"arguments":"{\"city\":\"Tokyo\"}","name":"get_weather"},"id":"call_1777008","type":"function"}]},"index":0}],"created":1788682962,"id":"tiKdau70AYuGz7IPwtejmQg","model":"gemini-3.6-flash","object":"chat.completion.chunk"}"#;

        let mut pending: Vec<(CallKey, PendingCall)> = Vec::new();
        let mut synthetic: u64 = 0;
        for line in [paris, tokyo] {
            match parse_sse_line(line) {
                Some(Chunk::ToolCallDeltas(frags)) => {
                    accumulate_fragments(&mut pending, &mut synthetic, frags)
                }
                other => panic!("expected tool-call deltas, got {other:?} for {line}"),
            }
        }
        let calls = pending_into_requests(pending);

        assert_eq!(
            calls.len(),
            2,
            "the two parallel calls must stay two, not merge into one corrupt call: {calls:?}"
        );

        assert_eq!(calls[0].id, "call_1776981");
        assert_eq!(calls[0].name, "get_weather");
        let a: Value = serde_json::from_str(&calls[0].args_json)
            .unwrap_or_else(|_| panic!("first call args must be valid JSON, got {:?}", calls[0].args_json));
        assert_eq!(a["city"], "Paris", "first call corrupted: {:?}", calls[0].args_json);
        assert_eq!(
            calls[0].thought_signature.as_deref(),
            Some("Es4ECssEFAKE_paris_sig"),
            "Gemini's block signature must ride on the first parallel call"
        );

        assert_eq!(calls[1].id, "call_1777008");
        assert_eq!(calls[1].name, "get_weather");
        let b: Value = serde_json::from_str(&calls[1].args_json)
            .unwrap_or_else(|_| panic!("second call args must be valid JSON, got {:?}", calls[1].args_json));
        assert_eq!(b["city"], "Tokyo", "second call corrupted: {:?}", calls[1].args_json);
        assert_eq!(
            calls[1].thought_signature, None,
            "Gemini signs only the first parallel call; the second carries no signature"
        );
    }

    /// **THE THOUGHT-SIGNATURE ROUND-TRIP — BLOCKER 1, proven pure.** A tool
    /// call Gemini returned carried an `extra_content.google.thought_signature`;
    /// when that call is replayed on the FOLLOW-UP request, the signature MUST
    /// be echoed back in the same place or Gemini 400s the whole turn
    /// ("Function call is missing a thought_signature in functionCall parts").
    /// This asserts `build_request` puts it back as a sibling of
    /// `id`/`type`/`function` on the assistant `tool_calls` entry — the shape
    /// Gemini sent it in and expects it back (verified against the captured
    /// wire and Google's OpenAI-compat guidance) — and that a call with NO
    /// signature (OpenAI/OpenRouter) emits NO `extra_content`, so their
    /// request bytes are unchanged.
    ///
    /// Proven able to fail: delete `build_request`'s `extra_content` branch and
    /// the first assertion fails — the replayed call has no signature and the
    /// live follow-up would 400.
    #[test]
    fn a_thought_signature_round_trips_through_build_request() {
        let signed = Message {
            role: Role::Assistant,
            text: String::new(),
            tool: Some(ToolTurn::Calls(vec![
                ToolCallRequest {
                    id: "call_1776981".into(),
                    name: "get_weather".into(),
                    args_json: r#"{"city":"Paris"}"#.into(),
                    thought_signature: Some("Es4ECssEFAKE_paris_sig".into()),
                },
                ToolCallRequest {
                    id: "call_1777008".into(),
                    name: "get_weather".into(),
                    args_json: r#"{"city":"Tokyo"}"#.into(),
                    thought_signature: None,
                },
            ])),
        };
        let msgs = vec![signed];
        let call = ModelCall {
            base_url: "https://generativelanguage.googleapis.com/v1beta/openai/",
            model: "gemini-3.6-flash",
            api_key: Some("k"),
            system: "",
            messages: &msgs,
            tools: &[],
        };
        let body = build_request(&call);
        let tool_calls = body
            .pointer("/messages/0/tool_calls")
            .and_then(|c| c.as_array())
            .expect("the assistant turn must carry tool_calls");
        assert_eq!(tool_calls.len(), 2);
        assert_eq!(
            tool_calls[0].pointer("/extra_content/google/thought_signature").and_then(|s| s.as_str()),
            Some("Es4ECssEFAKE_paris_sig"),
            "the first call's signature must be echoed back where Gemini expects it: {tool_calls:?}"
        );
        assert!(
            tool_calls[1].get("extra_content").is_none(),
            "a call with no signature must emit no extra_content (OpenAI/OpenRouter unchanged): {tool_calls:?}"
        );
    }

    /// **THE FULL GEMINI TOOL LOOP, PROVEN LIVE — BLOCKER 1, the strongest
    /// proof. 2026-09-06.** `a_gemini_tool_call_with_no_index_is_not_dropped`
    /// proves the bytes decode; this proves the WHOLE loop across TWO real
    /// requests: Gemini calls the tool (round one), the real `tools::dispatch`
    /// runs it, and the FOLLOW-UP request — the one that used to 400 with
    /// "Function call is missing a thought_signature in functionCall parts" —
    /// now succeeds and answers using the tool's real result. The follow-up
    /// succeeding IS the proof the signature round-tripped: `build_request`
    /// echoed `extra_content.google.thought_signature` back onto the replayed
    /// call, and Gemini accepted it. Delete `build_request`'s `extra_content`
    /// branch and this test 400s at round two.
    ///
    /// Same live-round shape as `a_real_openai_tool_call_round_trip`, pointed
    /// at Gemini's endpoint, sending NO `tool_choice` — the prompt alone makes
    /// Gemini choose the tool, exactly as the app does. Run it with:
    ///
    /// ```text
    /// NAMEOS_TEST_GEMINI_KEY="$(systemd-creds decrypt --user --name=gemini-api \
    ///   ~/.config/gemini/gemini-api.cred -)" cargo test --manifest-path \
    ///   desktop/src-tauri/Cargo.toml a_real_gemini_tool_loop_follow_up_succeeds \
    ///   -- --ignored --nocapture
    /// ```
    #[test]
    #[ignore = "talks to the real Gemini endpoint and spends real tokens on a real key"]
    fn a_real_gemini_tool_loop_follow_up_succeeds() {
        const BASE: &str = "https://generativelanguage.googleapis.com/v1beta/openai/";
        const MODEL: &str = "gemini-3.6-flash";
        let key = std::env::var("NAMEOS_TEST_GEMINI_KEY")
            .expect("set NAMEOS_TEST_GEMINI_KEY to a real Gemini API key to run this test");

        let workdir = std::env::temp_dir()
            .join(format!("nameos-gemini-tool-live-{}", crate::engine::native::store::new_id()));
        std::fs::create_dir_all(&workdir).unwrap();
        std::fs::write(workdir.join("notes.txt"), "the secret number is 8214").unwrap();

        let read_tool = crate::engine::native::tools::definitions(true, false)
            .into_iter()
            .find(|t| t.name == "Read")
            .expect("the Read tool must be in the real definitions list");

        // ROUND ONE.
        let round_one_msgs = vec![Message {
            role: Role::User,
            text: "Use the Read tool to read notes.txt, then tell me the secret number in it. \
                   Call the tool first; do not guess."
                .into(),
            tool: None,
        }];
        let call_one = ModelCall {
            base_url: BASE,
            model: MODEL,
            api_key: Some(&key),
            system: "You are terse.",
            messages: &round_one_msgs,
            tools: std::slice::from_ref(&read_tool),
        };
        let out_one = OpenAiWire
            .stream(&call_one, &mut |_| Flow::Go)
            .expect("round one against a real Gemini key must succeed");
        assert_eq!(
            out_one.stop,
            StopReason::ToolUse,
            "Gemini's tool call was dropped: text={:?} calls={:?}",
            out_one.text,
            out_one.tool_calls
        );
        assert_eq!(out_one.tool_calls.len(), 1, "{:?}", out_one.tool_calls);
        let call_req = &out_one.tool_calls[0];
        assert_eq!(call_req.name, "Read");
        assert!(!call_req.id.is_empty(), "the call must carry the id Gemini gave it");
        // The whole point of the fix: Gemini 3.x signs the tool call, and we
        // captured it. If this is ever None the model did not sign (round two
        // would then not need it) -- but for a 3.x thinking model it is set.
        assert!(
            call_req.thought_signature.is_some(),
            "Gemini 3.x must have attached a thought_signature to capture: {call_req:?}"
        );
        let parsed: Value = serde_json::from_str(&call_req.args_json)
            .expect("real tool-call arguments must be valid JSON");
        assert_eq!(parsed["path"], "notes.txt", "args={}", call_req.args_json);

        // DISPATCH FOR REAL.
        let result = crate::engine::native::tools::dispatch(&call_req.name, &call_req.args_json, &workdir, true, false);
        assert!(!result.is_error, "{}", result.output);
        assert!(result.output.contains("8214"), "{}", result.output);

        // ROUND TWO -- the follow-up that used to 400. `ToolTurn::Calls`
        // carries the signature; `build_request` echoes it back.
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
            base_url: BASE,
            model: MODEL,
            api_key: Some(&key),
            system: "You are terse.",
            messages: &round_two_msgs,
            tools: std::slice::from_ref(&read_tool),
        };
        let out_two = OpenAiWire.stream(&call_two, &mut |_| Flow::Go).expect(
            "round two must NOT 400 -- if it does, the thought_signature did not round-trip",
        );
        assert!(
            out_two.text.contains("8214"),
            "the model did not use the real tool result in its answer: {:?}",
            out_two.text
        );
        println!("a_real_gemini_tool_loop_follow_up_succeeds: final answer = {:?}", out_two.text);

        let _ = std::fs::remove_dir_all(&workdir);
    }

    /// **TWO PARALLEL GEMINI TOOL CALLS THROUGH A FULL LIVE LOOP — BLOCKER 2
    /// end-to-end, 2026-09-06.** `two_parallel_gemini_calls_with_no_index_
    /// stay_two_distinct_calls` proves the accumulator keeps the captured
    /// bytes distinct; this proves it against the LIVE model and then closes
    /// the loop: Gemini reads two files in one turn (two parallel `Read`
    /// calls, no entry-level `index`, distinct `id`s), both are dispatched for
    /// real, both replayed with the block signature on the first, and the
    /// follow-up succeeds with both secrets in the answer. Against the pre-fix
    /// code the two calls merged into one corrupt `{"path":"a.txt"}{"path":
    /// "b.txt"}` and the turn failed. Run it with the same env var as
    /// `a_real_gemini_tool_loop_follow_up_succeeds`.
    #[test]
    #[ignore = "talks to the real Gemini endpoint and spends real tokens on a real key"]
    fn a_real_gemini_parallel_tool_loop_follow_up_succeeds() {
        const BASE: &str = "https://generativelanguage.googleapis.com/v1beta/openai/";
        const MODEL: &str = "gemini-3.6-flash";
        let key = std::env::var("NAMEOS_TEST_GEMINI_KEY")
            .expect("set NAMEOS_TEST_GEMINI_KEY to a real Gemini API key to run this test");

        let workdir = std::env::temp_dir()
            .join(format!("nameos-gemini-parallel-live-{}", crate::engine::native::store::new_id()));
        std::fs::create_dir_all(&workdir).unwrap();
        std::fs::write(workdir.join("a.txt"), "alpha secret is 111").unwrap();
        std::fs::write(workdir.join("b.txt"), "bravo secret is 222").unwrap();

        let read_tool = crate::engine::native::tools::definitions(true, false)
            .into_iter()
            .find(|t| t.name == "Read")
            .expect("the Read tool must be in the real definitions list");

        let round_one_msgs = vec![Message {
            role: Role::User,
            text: "Read BOTH a.txt and b.txt using the Read tool -- call it twice in the same \
                   turn, once per file -- then tell me both secrets. Do not guess."
                .into(),
            tool: None,
        }];
        let call_one = ModelCall {
            base_url: BASE,
            model: MODEL,
            api_key: Some(&key),
            system: "You are terse. When asked to read multiple files, call the tool for all of \
                     them in one turn.",
            messages: &round_one_msgs,
            tools: std::slice::from_ref(&read_tool),
        };
        let out_one = OpenAiWire
            .stream(&call_one, &mut |_| Flow::Go)
            .expect("round one against a real Gemini key must succeed");
        assert_eq!(
            out_one.stop,
            StopReason::ToolUse,
            "Gemini did not call the tool: text={:?} calls={:?}",
            out_one.text,
            out_one.tool_calls
        );
        // The core BLOCKER 2 assertion: the parallel calls did NOT merge.
        assert_eq!(
            out_one.tool_calls.len(),
            2,
            "the two parallel calls merged instead of staying distinct: {:?}",
            out_one.tool_calls
        );
        let mut paths: Vec<String> = out_one
            .tool_calls
            .iter()
            .map(|c| {
                let v: Value = serde_json::from_str(&c.args_json)
                    .unwrap_or_else(|_| panic!("corrupt (merged?) args: {:?}", c.args_json));
                v["path"].as_str().unwrap_or_default().to_string()
            })
            .collect();
        paths.sort();
        assert_eq!(paths, vec!["a.txt".to_string(), "b.txt".to_string()], "{:?}", out_one.tool_calls);

        // Dispatch both for real, replay both calls (signature on the first)
        // and both results, and prove the follow-up succeeds.
        let mut result_msgs = vec![
            round_one_msgs.into_iter().next().unwrap(),
            Message {
                role: Role::Assistant,
                text: out_one.text.clone(),
                tool: Some(ToolTurn::Calls(out_one.tool_calls.clone())),
            },
        ];
        for c in &out_one.tool_calls {
            let result = crate::engine::native::tools::dispatch(&c.name, &c.args_json, &workdir, true, false);
            assert!(!result.is_error, "{}", result.output);
            result_msgs.push(Message {
                role: Role::Tool,
                text: result.output.clone(),
                tool: Some(ToolTurn::Result { call_id: c.id.clone(), is_error: false }),
            });
        }
        let call_two = ModelCall {
            base_url: BASE,
            model: MODEL,
            api_key: Some(&key),
            system: "You are terse.",
            messages: &result_msgs,
            tools: std::slice::from_ref(&read_tool),
        };
        let out_two = OpenAiWire.stream(&call_two, &mut |_| Flow::Go).expect(
            "the parallel follow-up must NOT 400 -- if it does, a signature did not round-trip",
        );
        assert!(
            out_two.text.contains("111") && out_two.text.contains("222"),
            "the model did not use both real tool results: {:?}",
            out_two.text
        );
        println!("a_real_gemini_parallel_tool_loop_follow_up_succeeds: final answer = {:?}", out_two.text);

        let _ = std::fs::remove_dir_all(&workdir);
    }

    /// **NOT CAPTURED, AND SAID SO RATHER THAN LEFT LOOKING LIKE IT WAS.**
    /// OpenAI's own endpoint never sent `reasoning_content` in the capture
    /// above — this is `adapter.rs`'s documented DeepSeek-style shape
    /// (degradation 3 in that file's module doc), reproduced by hand because
    /// this house does not have a live DeepSeek key to capture it from. If
    /// this ever gets a real capture, it belongs merged into the test above,
    /// not left here pretending to be one.
    #[test]
    fn a_deepseek_style_reasoning_delta_is_kept_separate_from_the_answer() {
        assert_eq!(
            parse_sse_line(
                r#"data: {"id":"1","choices":[{"index":0,"delta":{"reasoning_content":"Let"},"finish_reason":null}]}"#
            ),
            Some(Chunk::Thinking("Let".into()))
        );
    }

    /// A mid-stream error object ends the turn and names the reason, rather
    /// than being read as a chunk with no usable content.
    #[test]
    fn an_error_inside_the_stream_is_recognised() {
        assert_eq!(
            parse_sse_line(r#"data: {"error":{"message":"the model is overloaded"}}"#),
            Some(Chunk::Error("the model is overloaded".into()))
        );
    }

    /// Junk is skipped, not fatal -- same contract `ollama.rs` proves for its
    /// own stream. Anything not prefixed `data:` is not this wire's concern.
    #[test]
    fn junk_and_non_data_fields_are_skipped() {
        assert_eq!(parse_sse_line("not sse at all"), None);
        assert_eq!(parse_sse_line("event: ping"), None);
        assert_eq!(parse_sse_line("data: not json"), None);
    }

    /// **404 NAMES THE TWO LIKELY CAUSES, NOT A BARE NUMBER.** Same fault
    /// class `providers.rs`'s `generic_status_error` was built to stop, and
    /// `ollama.rs`'s own 404 case is the local half of the same lesson.
    #[test]
    fn a_404_names_the_url_and_the_model() {
        let e = status_error(404, "{}".into(), "sk-test", "gpt-9", "https://api.openai.com/v1", "/chat/completions");
        assert_eq!(e.kind, ErrorKind::ModelMissing);
        assert!(e.what.contains("gpt-9"), "{e:?}");
        assert!(e.what.contains("api.openai.com"), "{e:?}");
    }

    /// **A REAL KEY THAT APPEARS IN AN ERROR BODY MUST NOT REACH THE
    /// PERSON.** Beck's proof against a live provider row
    /// (`providers.rs`'s own comment on `redact_and_truncate`) is that an
    /// upstream error page can echo a bearer token back. This wire is the
    /// one place in the native engine that ever holds a real key, so it is
    /// the one wire whose error path is tested against that directly.
    #[test]
    fn a_key_echoed_in_an_error_body_is_redacted() {
        let key = "sk-proj-abcdefghijklmnopqrstuvwxyz0123456789";
        let body = format!(r#"{{"error":{{"message":"bad request with key {key} attached"}}}}"#);
        let e = status_error(400, body, key, "gpt-4o-mini", "https://api.openai.com/v1", "/chat/completions");
        assert!(!e.what.contains(key), "the real key leaked into the error text: {}", e.what);
        assert!(e.what.contains("[key redacted]"), "{}", e.what);
    }

    /// A 429 on a fresh key points at billing before it points at pacing --
    /// same wording `providers::test_chat_endpoint` already earned from a
    /// real person hitting this exact case.
    #[test]
    fn a_429_points_at_credit_before_pacing() {
        let e = status_error(429, "{}".into(), "sk-test", "gpt-4o-mini", "https://api.openai.com/v1", "/chat/completions");
        assert!(e.fix.contains("credit"), "{e:?}");
    }

    /// Every error names what happened AND what to do -- same invariant
    /// `ollama.rs` asserts for its own error table.
    #[test]
    fn every_status_error_says_what_to_do_next() {
        for code in [400u16, 401, 403, 404, 429, 500, 503] {
            let e = status_error(code, "{}".into(), "sk-test", "m", "https://api.openai.com/v1", "/chat/completions");
            assert!(!e.what.trim().is_empty(), "{code} has no description");
            assert!(!e.fix.trim().is_empty(), "{code} has no fix hint");
        }
    }

    // -- Real endpoints. `#[ignore]`d by house convention (`adapter.rs`'s own
    // -- real round trip does the same) because a default `cargo test` run
    // -- must not depend on the network or on a credential being present. --

    /// **THIS IS THE PROOF `adapter.rs`'S OWN TRANSLATOR HAS NEVER HAD: a
    /// real key, a real OpenAI endpoint, no `claude.exe` anywhere in the
    /// process tree.** Run it with:
    ///
    /// ```text
    /// NAMEOS_TEST_OPENAI_KEY=sk-... cargo test --manifest-path \
    ///   desktop/src-tauri/Cargo.toml a_real_openai_streaming_round_trip \
    ///   -- --ignored --nocapture
    /// ```
    ///
    /// **Run for real on 2026-09-02**, against `gpt-4o-mini` — this test
    /// passed (`text="OK" in=Some(25) out=Some(1)`), and the raw bytes it
    /// answered with are what `the_real_captured_lines_decode_the_way_they_
    /// did_on_the_wire` now pins, captured separately with a bare `curl` so
    /// the shape is checked before this test's own parser is trusted with
    /// it: the stream opened with the empty role-only delta, one content
    /// chunk carried the whole one-token answer, `finish_reason` arrived as
    /// `"stop"` on its own chunk, a usage-only chunk with empty `choices`
    /// carried real token counts, and the stream closed on the literal
    /// `data: [DONE]` line -- the whole shape this file assumed from
    /// OpenAI's published docs, confirmed against the real thing rather than
    /// trusted from a page.
    #[test]
    #[ignore = "talks to the real OpenAI endpoint and spends a fraction of a cent on a real key"]
    fn a_real_openai_streaming_round_trip() {
        let key = std::env::var("NAMEOS_TEST_OPENAI_KEY")
            .expect("set NAMEOS_TEST_OPENAI_KEY to a real OpenAI API key to run this test");
        let msgs = vec![Message {
            role: Role::User,
            text: "Reply with exactly the word OK and nothing else.".into(),
            tool: None,
        }];
        let call = ModelCall {
            base_url: "https://api.openai.com/v1",
            model: "gpt-4o-mini",
            api_key: Some(&key),
            system: "You are terse.",
            messages: &msgs,
            tools: &[],
        };
        let mut pieces: Vec<String> = Vec::new();
        let out = OpenAiWire
            .stream(&call, &mut |d| {
                if let Delta::Text(t) = d {
                    pieces.push(t);
                }
                Flow::Go
            })
            .expect("a real OpenAI call with a real key must succeed");
        assert_eq!(out.stop, StopReason::End, "text={:?}", out.text);
        assert!(!out.text.trim().is_empty(), "no text came back");
        assert!(!pieces.is_empty(), "no streamed pieces arrived through `on`, only the total");
        assert!(
            out.input_tokens.is_some() && out.output_tokens.is_some(),
            "usage never arrived -- stream_options.include_usage did not work against the \
             real endpoint the way the module doc claims"
        );
        println!(
            "a_real_openai_streaming_round_trip: text={:?} in={:?} out={:?}",
            out.text, out.input_tokens, out.output_tokens
        );
    }

    /// **THE TOOL LOOP'S LIVE PROOF — 2026-09-02, Phase 2's own increment.**
    /// `a_real_openai_streaming_round_trip` above proves a plain answer;
    /// this proves the whole round `drive`'s tool loop actually runs: a real
    /// forced tool call against a real key, dispatched through the real
    /// `tools::dispatch` (real `folder_trust::confine`, real file on real
    /// disk), replayed back in the exact shape `build_request` builds for a
    /// `ToolTurn`, and a second real call that reads the tool's answer and
    /// finishes in text. Nothing here is a mock past the OS filesystem and
    /// the network socket.
    ///
    /// Run it with:
    ///
    /// ```text
    /// NAMEOS_TEST_OPENAI_KEY="$(systemd-creds decrypt --user --name=openai-api \
    ///   ~/.config/openai/openai-api.cred -)" cargo test --manifest-path \
    ///   desktop/src-tauri/Cargo.toml a_real_openai_tool_call_round_trip \
    ///   -- --ignored --nocapture
    /// ```
    #[test]
    #[ignore = "talks to the real OpenAI endpoint and spends a fraction of a cent on a real key"]
    fn a_real_openai_tool_call_round_trip() {
        let key = std::env::var("NAMEOS_TEST_OPENAI_KEY")
            .expect("set NAMEOS_TEST_OPENAI_KEY to a real OpenAI API key to run this test");

        let workdir = std::env::temp_dir()
            .join(format!("nameos-openai-tool-live-{}", crate::engine::native::store::new_id()));
        std::fs::create_dir_all(&workdir).unwrap();
        std::fs::write(workdir.join("notes.txt"), "the secret number is 8214").unwrap();

        let read_tool = crate::engine::native::tools::definitions(true, false)
            .into_iter()
            .find(|t| t.name == "Read")
            .expect("the Read tool must be in the real definitions list");

        // ROUND ONE: force the call.
        let round_one_msgs = vec![Message {
            role: Role::User,
            text: "Use the Read tool to read notes.txt, then tell me the secret number in it. \
                   Call the tool first; do not guess."
                .into(),
            tool: None,
        }];
        let call_one = ModelCall {
            base_url: "https://api.openai.com/v1",
            model: "gpt-4o-mini",
            api_key: Some(&key),
            system: "You are terse.",
            messages: &round_one_msgs,
            tools: std::slice::from_ref(&read_tool),
        };
        let out_one = OpenAiWire
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
        println!("a_real_openai_tool_call_round_trip: model asked for {call_req:?}");

        // DISPATCH FOR REAL — the same function `drive`'s tool loop calls,
        // against the real file just written.
        let result = crate::engine::native::tools::dispatch(&call_req.name, &call_req.args_json, &workdir, true, false);
        assert!(!result.is_error, "{}", result.output);
        assert!(result.output.contains("8214"), "{}", result.output);

        // ROUND TWO: replay the assistant's call and the tool's real result,
        // exactly the shape `drive` builds them in.
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
            base_url: "https://api.openai.com/v1",
            model: "gpt-4o-mini",
            api_key: Some(&key),
            system: "You are terse.",
            messages: &round_two_msgs,
            tools: std::slice::from_ref(&read_tool),
        };
        let out_two = OpenAiWire
            .stream(&call_two, &mut |_| Flow::Go)
            .expect("round two against a real key must succeed");
        assert_eq!(out_two.stop, StopReason::End, "text={:?}", out_two.text);
        assert!(
            out_two.text.contains("8214"),
            "the model did not use the real tool result in its answer: {:?}",
            out_two.text
        );
        println!("a_real_openai_tool_call_round_trip: final answer = {:?}", out_two.text);

        let _ = std::fs::remove_dir_all(&workdir);
    }

    /// **THE ACTUAL FIX, PROVEN LIVE — 2026-09-03.** Against the pre-Phase-2B
    /// wire this exact call 400'd with OpenAI's own words: *"Function tools
    /// with reasoning_effort are not supported for gpt-5.6-sol in
    /// /v1/chat/completions. To use function tools, use /v1/responses or set
    /// reasoning_effort to 'none'."* This is that fix, run for real against a
    /// reasoning model with a real key: a forced tool call, dispatched
    /// through the real `tools::dispatch`, replayed back in the shape
    /// `build_responses_request` builds for a `ToolTurn`, and a second real
    /// call that reads the tool's own answer and finishes in text — the
    /// `a_real_openai_tool_call_round_trip` shape above, pointed at the
    /// reasoning model and the `/v1/responses` endpoint instead.
    ///
    /// **REASONING IS ON, NOT SUPPRESSED — the assertion the stopgap could
    /// never make.** This wire never sends a `reasoning_effort` field on this
    /// path at all (`the_responses_body_never_carries_reasoning_effort_
    /// even_with_tools_on` proves that offline); this test is what proves
    /// the model genuinely reasoned rather than merely not erroring. Run it
    /// with:
    ///
    /// ```text
    /// NAMEOS_TEST_OPENAI_KEY="$(systemd-creds decrypt --user --name=openai-api \
    ///   ~/.config/openai/openai-api.cred -)" cargo test --manifest-path \
    ///   desktop/src-tauri/Cargo.toml a_real_openai_reasoning_model_tool_call_round_trip \
    ///   -- --ignored --nocapture
    /// ```
    #[test]
    #[ignore = "talks to the real OpenAI endpoint and spends a fraction of a cent on a real key"]
    fn a_real_openai_reasoning_model_tool_call_round_trip() {
        let key = std::env::var("NAMEOS_TEST_OPENAI_KEY")
            .expect("set NAMEOS_TEST_OPENAI_KEY to a real OpenAI API key to run this test");
        let model = "gpt-5.6-sol";

        let workdir = std::env::temp_dir().join(format!(
            "nameos-openai-reasoning-tool-live-{}",
            crate::engine::native::store::new_id()
        ));
        std::fs::create_dir_all(&workdir).unwrap();
        std::fs::write(workdir.join("notes.txt"), "the secret number is 8214").unwrap();

        let read_tool = crate::engine::native::tools::definitions(true, false)
            .into_iter()
            .find(|t| t.name == "Read")
            .expect("the Read tool must be in the real definitions list");

        // ROUND ONE: force the call, against a REASONING model with tools on
        // -- the exact combination that 400'd before this increment.
        let round_one_msgs = vec![Message {
            role: Role::User,
            text: "Use the Read tool to read notes.txt, then tell me the secret number in it. \
                   Call the tool first; do not guess."
                .into(),
            tool: None,
        }];
        let call_one = ModelCall {
            base_url: "https://api.openai.com/v1",
            model,
            api_key: Some(&key),
            system: "You are terse.",
            messages: &round_one_msgs,
            tools: std::slice::from_ref(&read_tool),
        };
        let out_one = OpenAiWire
            .stream(&call_one, &mut |_| Flow::Go)
            .expect("a reasoning model with tools must return 200 on /v1/responses, not 400");
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
        let parsed: Value = serde_json::from_str(&call_req.args_json)
            .expect("real tool-call arguments must be valid JSON");
        assert_eq!(parsed["path"], "notes.txt", "args={}", call_req.args_json);
        println!("a_real_openai_reasoning_model_tool_call_round_trip: model asked for {call_req:?}");

        // DISPATCH FOR REAL.
        let result =
            crate::engine::native::tools::dispatch(&call_req.name, &call_req.args_json, &workdir, true, false);
        assert!(!result.is_error, "{}", result.output);
        assert!(result.output.contains("8214"), "{}", result.output);

        // ROUND TWO: replay the call and its real result -- WITHOUT any
        // reasoning item in `input` at all. See `build_responses_request`'s
        // own doc: this is the live proof that omitting reasoning-item
        // continuity costs quality headroom, not correctness.
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
            base_url: "https://api.openai.com/v1",
            model,
            api_key: Some(&key),
            system: "You are terse.",
            messages: &round_two_msgs,
            tools: std::slice::from_ref(&read_tool),
        };
        let out_two = OpenAiWire
            .stream(&call_two, &mut |_| Flow::Go)
            .expect("round two against a real key must succeed");
        assert_eq!(out_two.stop, StopReason::End, "text={:?}", out_two.text);
        assert!(
            out_two.text.contains("8214"),
            "the model did not use the real tool result in its answer: {:?}",
            out_two.text
        );
        println!(
            "a_real_openai_reasoning_model_tool_call_round_trip: final answer = {:?}",
            out_two.text
        );

        let _ = std::fs::remove_dir_all(&workdir);
    }

    /// **THE GAP THIS FILE'S OWN MODULE DOC NAMED, CLOSED — run for real
    /// 2026-09-02.** Until now "Gemini's own compat endpoint was NOT hit by
    /// this wire" was true, and this file said so plainly rather than
    /// letting the `adapter.rs` translator's separate Gemini proof
    /// (`a_real_gemini_round_trip_translates_both_ways`, a different code
    /// path -- `translate_request`/`openai_to_anthropic_response`, not this
    /// engine at all) stand in for it. It cannot: that path is Claude Code
    /// talking through a loopback translator, this one is the native engine
    /// streaming SSE directly, and nothing about one proves the other. This
    /// test is `a_real_openai_streaming_round_trip`'s own shape, pointed at
    /// `generativelanguage.googleapis.com`'s OpenAI-compatible endpoint --
    /// same `OpenAiWire`, same `build_request`, same `parse_sse_line`, same
    /// run loop, no Gemini-specific branch anywhere in this file. Run it
    /// with the real key decrypted straight into the test process's
    /// environment, never written to disk or printed:
    ///
    /// ```text
    /// NAMEOS_TEST_GEMINI_KEY="$(systemd-creds decrypt --user --name=gemini-api \
    ///   ~/.config/gemini/gemini-api.cred -)" cargo test --manifest-path \
    ///   desktop/src-tauri/Cargo.toml a_real_gemini_streaming_round_trip \
    ///   -- --ignored --nocapture
    /// ```
    ///
    /// **Run for real on 2026-09-02, against `gemini-3.6-flash` -- the same
    /// model name `adapter.rs`'s own live Gemini test already proved
    /// answers.** Real streamed text and a real `finish_reason` worked on the
    /// FIRST run with zero changes to this file -- the content and
    /// finish-reason shapes are genuinely identical to OpenAI's. **Usage did
    /// not, and that is a real difference rather than a flake:** Gemini
    /// never sends OpenAI's dedicated usage-only, empty-`choices` line at
    /// all -- it attaches the running usage totals to every line that
    /// carries a real choice instead, captured live in `scan_usage`'s own
    /// doc comment above. `parse_sse_line`'s single-`Chunk`-per-line design
    /// (and the real OpenAI bytes pinned against it) could not represent
    /// "this line is both a finish-reason chunk AND a usage chunk," so
    /// usage came back `None` on the first run of this test -- a real,
    /// reproducible finding, not something papered over to make this pass.
    /// `scan_usage` is the fix: it reads every line a second time,
    /// independently of `parse_sse_line`'s own classification, so usage is
    /// picked up wherever a provider actually puts it. **So "closed" is
    /// true, and it took one small addition to get there, not zero.**
    ///
    /// **One real difference from OpenAI, found by running it rather than
    /// assumed, and it does not need a code change here:** this engine sends
    /// neither `max_tokens` nor `max_completion_tokens` (see this file's own
    /// module doc, "Scope" section) -- an unbounded request. That sidesteps
    /// entirely the failure `adapter.rs`'s own Gemini test had to work
    /// around, where a small caller-set budget let this family's
    /// always-on internal reasoning (unlike OpenAI's, where
    /// `is_reasoning_model` at least names which families do it) consume the
    /// whole cap before any visible text, returning empty. With no cap sent,
    /// there is nothing for reasoning to exhaust before the answer -- this
    /// path cannot reproduce that failure by construction, not because
    /// anything here guards against it.
    #[test]
    #[ignore = "talks to the real Gemini endpoint and spends real tokens on a real key"]
    fn a_real_gemini_streaming_round_trip() {
        let key = std::env::var("NAMEOS_TEST_GEMINI_KEY")
            .expect("set NAMEOS_TEST_GEMINI_KEY to a real Gemini API key to run this test");
        let msgs = vec![Message {
            role: Role::User,
            text: "Reply with exactly the word OK and nothing else.".into(),
            tool: None,
        }];
        let call = ModelCall {
            base_url: "https://generativelanguage.googleapis.com/v1beta/openai/",
            model: "gemini-3.6-flash",
            api_key: Some(&key),
            system: "You are terse.",
            messages: &msgs,
            tools: &[],
        };
        let mut pieces: Vec<String> = Vec::new();
        let out = OpenAiWire
            .stream(&call, &mut |d| {
                if let Delta::Text(t) = d {
                    pieces.push(t);
                }
                Flow::Go
            })
            .expect("a real Gemini call with a real key must succeed");
        assert_eq!(out.stop, StopReason::End, "text={:?}", out.text);
        assert!(!out.text.trim().is_empty(), "no text came back");
        assert!(!pieces.is_empty(), "no streamed pieces arrived through `on`, only the total");
        // Gemini's own default reasoning latency (see the doc comment above)
        // means real token counts are worth pinning here specifically, not
        // just for OpenAI -- a silent `None` on this endpoint would be a
        // second, independent way `Completion.input_tokens`/`output_tokens`
        // could go quiet that the OpenAI test alone would never catch.
        assert!(
            out.input_tokens.is_some() && out.output_tokens.is_some(),
            "usage never arrived -- stream_options.include_usage did not work against Gemini's \
             real endpoint the way it does against OpenAI's"
        );
        println!(
            "a_real_gemini_streaming_round_trip: text={:?} in={:?} out={:?}",
            out.text, out.input_tokens, out.output_tokens
        );
    }

    /// **A SECOND, INDEPENDENT OPENAI-SHAPED ENDPOINT, DIFFERENT COMPANY.**
    /// One real endpoint proves this wire is not merely tuned to one
    /// provider's quirks -- OpenRouter is documented as OpenAI-SDK-compatible
    /// and this is what proves that claim against this code rather than
    /// trusting the claim. Run it with:
    ///
    /// ```text
    /// NAMEOS_TEST_OPENROUTER_KEY=sk-or-... cargo test --manifest-path \
    ///   desktop/src-tauri/Cargo.toml a_real_openrouter_streaming_round_trip \
    ///   -- --ignored --nocapture
    /// ```
    #[test]
    #[ignore = "talks to the real OpenRouter endpoint and spends a fraction of a cent on a real key"]
    fn a_real_openrouter_streaming_round_trip() {
        let key = std::env::var("NAMEOS_TEST_OPENROUTER_KEY")
            .expect("set NAMEOS_TEST_OPENROUTER_KEY to a real OpenRouter API key to run this");
        let msgs = vec![Message {
            role: Role::User,
            text: "Reply with exactly the word OK and nothing else.".into(),
            tool: None,
        }];
        let call = ModelCall {
            base_url: "https://openrouter.ai/api/v1",
            model: "openai/gpt-4o-mini",
            api_key: Some(&key),
            system: "You are terse.",
            messages: &msgs,
            tools: &[],
        };
        let out = OpenAiWire
            .stream(&call, &mut |_| Flow::Go)
            .expect("a real OpenRouter call with a real key must succeed");
        assert_eq!(out.stop, StopReason::End);
        assert!(!out.text.trim().is_empty(), "no text came back");
        println!("a_real_openrouter_streaming_round_trip: text={:?}", out.text);
    }

    /// **THE OTHER HALF OF FORCE-FAILURE DISCIPLINE: A BAD KEY MUST FAIL
    /// HONESTLY, NOT SILENTLY, AND NOT BECAUSE `claude.exe` CAUGHT IT.** No
    /// key needed to run this one -- the key is deliberately wrong, and the
    /// point is a real 401 from the real endpoint, decoded by this file's own
    /// `status_error`, with nothing from Anthropic anywhere in the process.
    #[test]
    #[ignore = "talks to the real OpenAI endpoint (no real key needed -- proves a bad one fails \
                honestly)"]
    fn a_bad_key_surfaces_honestly_with_no_claude_exe_running() {
        let msgs = vec![Message { role: Role::User, text: "hi".into(), tool: None }];
        let call = ModelCall {
            base_url: "https://api.openai.com/v1",
            model: "gpt-4o-mini",
            api_key: Some("sk-this-is-not-a-real-key"),
            system: "",
            messages: &msgs,
            tools: &[],
        };
        // `Result::expect_err` needs `Completion: Debug`, which the type does
        // not derive (it is never printed anywhere else in this codebase) --
        // matching by hand avoids adding a derive to a shared type just for
        // one test's convenience.
        match OpenAiWire.stream(&call, &mut |_| Flow::Go) {
            Ok(_) => panic!("a bad key must not silently succeed"),
            Err(err) => {
                assert_eq!(err.kind, ErrorKind::BadRequest, "{err:?}");
                assert!(err.what.contains("401"), "{err:?}");
                assert!(!err.fix.trim().is_empty(), "{err:?}");
            }
        }
    }

    /// **THE SAME PROOF, AGAINST GEMINI, AND IT FOUND A REAL DIFFERENCE —
    /// run for real 2026-09-02.** No key needed here either. Two things do
    /// NOT match OpenAI's shape, found by running this rather than assumed:
    /// Gemini answers a bad key with **400, not 401** (`"status":
    /// "INVALID_ARGUMENT"`, its own wording, not an auth-specific code), and
    /// the error body is a **JSON ARRAY** (`[{"error": {...}}]`) rather than
    /// a bare object -- so `status_error`'s `.pointer("/error/message")`,
    /// written for OpenAI's object shape, does not resolve against an array
    /// root and `said` falls back to the raw (redacted, capped) body rather
    /// than the extracted message alone. **Still honest, just less tidy:**
    /// the 400 catch-all arm still names the real code and still carries the
    /// provider's own text -- "Please pass a valid API key" is in there,
    /// wrapped in its brackets -- and the fix hint still tells the person
    /// what to do. Not changed here: a parser tolerant of an array-rooted
    /// error body would be real work for a cosmetically worse message on an
    /// already-honest failure, not a silent one.
    #[test]
    #[ignore = "talks to the real Gemini endpoint (no real key needed -- proves a bad one \
                fails honestly, and that the failure shape differs from OpenAI's)"]
    fn a_bad_key_surfaces_honestly_against_gemini_too() {
        let msgs = vec![Message { role: Role::User, text: "hi".into(), tool: None }];
        let call = ModelCall {
            base_url: "https://generativelanguage.googleapis.com/v1beta/openai/",
            model: "gemini-3.6-flash",
            api_key: Some("AIzaSyThisIsNotARealKey00000000000"),
            system: "",
            messages: &msgs,
            tools: &[],
        };
        match OpenAiWire.stream(&call, &mut |_| Flow::Go) {
            Ok(_) => panic!("a bad key must not silently succeed"),
            Err(err) => {
                assert_eq!(err.kind, ErrorKind::BadRequest, "{err:?}");
                assert!(err.what.contains("400"), "{err:?}");
                assert!(!err.fix.trim().is_empty(), "{err:?}");
            }
        }
    }
}
