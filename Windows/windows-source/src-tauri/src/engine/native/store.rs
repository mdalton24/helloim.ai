//! Our own conversation store.
//!
//! **WHY THIS EXISTS AT ALL: `--resume` BELONGS TO CLAUDE CODE.** Today the app
//! keeps one string — the session id `claude.exe` hands back — and passing it to
//! `--resume` next turn is the entire memory mechanism. The transcript itself is
//! the vendor's, in the vendor's format, in the vendor's directory. A native
//! engine has no such thing, so without this file every turn is a fresh
//! conversation and the app answers *"what did I just ask you?"* with a blank
//! stare.
//!
//! That is a worse failure than the one the chat engine is honest about. *"It
//! cannot edit files"* is a limit somebody chose and can see. *"It forgot what
//! we just said"* reads as the product being broken.
//!
//! ## The commit rule, which is the whole design
//!
//! **NOTHING IS WRITTEN UNTIL THE TURN IS OVER, AND WHAT IS WRITTEN IS WHAT THE
//! PERSON ACTUALLY SAW.** The obvious build — append the user's message, call
//! the model, append the answer — is wrong in three separate ways, and each of
//! them has a name:
//!
//! - **A failed turn would leave a dangling user message.** The next turn
//!   appends a second one, and now the history has two user messages in a row
//!   with nothing between them. Some endpoints tolerate that and some do not,
//!   and the ones that do give the model a conversation that never happened.
//! - **A retry would double it.** The person presses Send again after a dead
//!   port; their words are now in the history twice. `commit` is the only
//!   writer and it runs once per turn, so **a failed turn leaves no trace and
//!   sending again is a clean first attempt.** That is the idempotency story,
//!   stated rather than hoped for.
//! - **A cancelled turn must agree with what the person saw.** The rule is that
//!   **the screen is the truth; the store agrees with the screen or the store is
//!   wrong.**
//!
//! **THAT LAST POINT USED TO SAY "partial text is committed exactly as it was
//! shown", AND BUILDING THE ENGINE FALSIFIED IT — corrected 2026-08-31.** It
//! assumed the answer appears incrementally, and the five-shape contract this
//! product actually has contains no incremental-text shape: every `assistant`
//! event becomes its own bubble in the window and gets spoken aloud, so the
//! answer is emitted once, whole, at the end. **A cancelled turn has therefore
//! shown the person nothing**, and committing a half-answer would put words in
//! the model's mouth that nobody ever read — the same fault as the original,
//! pointed the other way. So `engine::native` commits nothing on a cancelled
//! turn and says so on screen. The rule did not change; the fact underneath it
//! did.
//!
//! ## What is deliberately NOT stored
//!
//! **The system prompt.** It is rebuilt every turn from the voice card and the
//! session bridge, both of which change. Freezing one into a conversation would
//! pin a stale voice into every future turn of it, and the bridge is a file on
//! disk that the app itself rewrites at the end of every run.
//!
//! ## The boundary
//!
//! - **Who may call.** `engine::native`, in-process. Not a command, no HTTP
//!   surface, `pub(crate)`.
//! - **What happens when the id is not one of ours.** `id_is_safe` refuses
//!   anything outside `[a-z0-9-]`, so a conversation id can never contain a
//!   path separator, a `..`, a drive letter or a NUL. **This is not decoration:
//!   `Session::session_id` is populated by `main.rs::session_id_of` off a
//!   `system` line in the event stream, and an engine that ever forwards a
//!   provider's own stream would be letting a remote party name a file on this
//!   disk.** Today every id in native mode is minted here from the OS CSPRNG,
//!   so the check is unreachable — and it is written anyway, because "today's
//!   only caller is trusted" is the sentence that stops being true quietly.
//! - **What happens on malformed input.** An unreadable or unparseable file is
//!   treated as no conversation. It is never repaired in place and never
//!   deleted: a corrupt transcript is somebody's history, and the safest
//!   operation on it is the one we do not perform.
//! - **What errors leak.** Paths and OS messages, which are the person's own
//!   machine. Never a key — no key is in scope here.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// How much conversation history is sent to the model, in characters.
///
/// **TRIMMED TOWARD SAFETY AND SAID OUT LOUD, WHICH IS THE POINT.** The right
/// number is per-model — Ollama's `/api/show` returns the context length, and
/// reading it is the proper fix — and this stage deliberately does not do that.
/// So the budget is set low enough for a small local model, which means a
/// 32k-context 30B is under-used. **That trade is the correct way round:
/// under-using a context window costs a little quality, over-running one costs
/// the model the beginning of the conversation with no error anywhere.**
///
/// ~24,000 characters is roughly 6,000 tokens, which leaves room for the voice
/// card, the session bridge and an answer inside an 8k window.
///
/// **AND WHEN IT BITES, THE PERSON IS TOLD.** `messages_for_send` returns
/// whether it trimmed, and the engine puts a note on screen. A long
/// conversation quietly losing its beginning is the worst kind of bug in this
/// product, because it looks exactly like the model being forgetful — which is
/// the one thing this app exists to fix.
pub(crate) const HISTORY_BUDGET_CHARS: usize = 24_000;

/// Who said it.
///
/// **THIS USED TO BE TWO VALUES, AND THE THIRD IS DELIBERATELY NARROW — added
/// 2026-09-02 for the tool loop (Phase 2 of the neutral-engine plan;
/// `engine/mod.rs`'s own header names it).** `Tool` is never sent by a
/// person and never shown to one — it exists so a call's own result has a
/// role of its own when it is replayed back to the model, which every wire
/// that can actually call a tool (OpenAI's Chat Completions shape today)
/// requires: a `tool` message answering a call has to carry that role and
/// nothing else, exactly the way `system` never gets stored either (see the
/// module header). A `Tool` message is never committed to the persisted
/// `Conversation` — see `store.rs`'s own commit rule and `Message::tool`'s
/// doc — so this variant only ever exists transiently, inside one turn's
/// in-flight `messages_for_send` copy.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Role {
    User,
    Assistant,
    Tool,
}

impl Role {
    /// The word all three wire formats happen to agree on.
    pub(crate) fn wire(self) -> &'static str {
        match self {
            Role::User => "user",
            Role::Assistant => "assistant",
            Role::Tool => "tool",
        }
    }
}

/// One tool call a model asked for, and the argument string it sent —
/// **NOT parsed here.** `args_json` is handed to `tools::dispatch` whole;
/// this type's job is carrying it, not reading it, so a malformed-JSON
/// argument is a `dispatch`-time failure with the call's own id attached
/// rather than something this struct could silently drop.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ToolCallRequest {
    /// The provider's own id for this call. Echoed back verbatim on the
    /// matching `tool` result — OpenAI's Chat Completions API keys the
    /// result to the call by this string, not by position.
    pub id: String,
    pub name: String,
    pub args_json: String,
    /// Gemini 3.x's opaque `extra_content.google.thought_signature`, when the
    /// provider attached one to this call. **It MUST round-trip:** the
    /// follow-up request that replays this call has to echo the same signature
    /// back in the same place or Gemini 400s the turn ("Function call is
    /// missing a thought_signature in functionCall parts"). Only the OpenAI
    /// Chat Completions wire (`openai.rs`) ever sets or reads it; every other
    /// provider leaves it `None`, and `skip_serializing_if` keeps it out of
    /// their stored/replayed shape entirely, so nothing changes for them.
    /// `#[serde(default)]` loads a conversation written before this field
    /// existed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thought_signature: Option<String>,
}

/// The tool-loop half of a message, present only on the two shapes that
/// carry it. `None` for every plain chat turn, which is still the common
/// case and the only one an engine with `supports_tools() == false` (or a
/// turn where the model simply answered) ever produces.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum ToolTurn {
    /// An ASSISTANT message that asked for these calls instead of (or
    /// alongside) answering in text. Replayed back verbatim on the next
    /// request — OpenAI's own API requires the assistant's `tool_calls` to
    /// precede the `tool` results answering them, so dropping this on replay
    /// would be handing back an answer to a question the model never sees
    /// itself having asked.
    Calls(Vec<ToolCallRequest>),
    /// A Role::Tool message: which call this is the result of, and whether
    /// the tool itself reported failure (a confined-path refusal, a failed
    /// `Bash`, and so on) — carried here rather than folded into `text` so a
    /// wire can tell the model "this one failed" without parsing prose to
    /// find out.
    Result { call_id: String, is_error: bool },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Message {
    pub role: Role,
    pub text: String,
    /// See `ToolTurn`'s own doc. `#[serde(default)]` so a conversation file
    /// written before this field existed still loads — every message on
    /// disk today is `None` in practice anyway, since nothing tool-related
    /// is ever committed (see the module header's commit rule).
    #[serde(default)]
    pub tool: Option<ToolTurn>,
}

/// One conversation, as it sits on disk.
///
/// `provider_id` and `model` are recorded for a reason that is about honesty
/// rather than bookkeeping: a conversation that was held with one brain and is
/// resumed against another is not the same conversation, and the app should be
/// able to say so. Nothing reads them yet; they are cheap to write now and
/// impossible to backfill later.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Conversation {
    pub id: String,
    #[serde(default)]
    pub provider_id: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub messages: Vec<Message>,
}

/// A conversation id we are willing to turn into a filename.
///
/// Lowercase letters, digits and hyphens, 1..=64. That excludes `/`, `\`, `..`,
/// `:`, NUL and every other thing that turns a name into a path. See the
/// module header for why this is checked rather than assumed.
pub(crate) fn id_is_safe(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// What marks a conversation id as this engine's.
///
/// **IT IS A CHECKED OWNERSHIP MARKER, NOT A LABEL — see `Engine::owns_session`,
/// which reads it from both sides.** `providers::select_provider` does clear the
/// id when somebody switches brains, and that covers the common path; it does
/// not cover the active row being *edited* from one kind to another, which keeps
/// the same id in place. So the prefix is checked at both engines rather than
/// relied on for recognition after the fact.
pub(crate) const CHAT_ID_PREFIX: &str = "helloim-chat-";

/// A fresh id, from the OS CSPRNG.
///
/// **PREFIXED ON PURPOSE.** Two engines write into the same
/// `Session::session_id` slot, and handing one engine's id to the other is a
/// real bug rather than a theoretical one — `claude.exe --resume helloim-chat-…`
/// fails outright, and a claude session id arriving here would name a
/// conversation file after somebody else's transcript.
///
/// Falls back to a time-based id if the CSPRNG is unavailable, because being
/// unable to remember a conversation is a worse outcome than a less random
/// name — and nothing here is a security decision. Collisions are the only
/// risk and the fallback is per-nanosecond.
pub(crate) fn new_id() -> String {
    let mut raw = [0u8; 12];
    if getrandom::getrandom(&mut raw).is_ok() {
        let hex: String = raw.iter().map(|b| format!("{b:02x}")).collect();
        return format!("{CHAT_ID_PREFIX}{hex}");
    }
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{CHAT_ID_PREFIX}{nanos:x}")
}

fn path_of(dir: &Path, id: &str) -> PathBuf {
    dir.join(format!("{id}.json"))
}

impl Conversation {
    pub(crate) fn new(id: String, provider_id: String, model: String) -> Self {
        Conversation { id, provider_id, model, messages: Vec::new() }
    }

    /// Read a conversation, or `None` for anything that is not one.
    ///
    /// **EVERY FAILURE IS `None` AND NOTHING IS REPAIRED.** A missing file is
    /// an ordinary first turn. An unreadable or unparseable one is somebody's
    /// history that we cannot read, and the response to that is to leave it
    /// exactly where it is — not to overwrite it with an empty conversation,
    /// which is what "repair" would mean here.
    pub(crate) fn load(dir: &Path, id: &str) -> Option<Conversation> {
        if !id_is_safe(id) {
            return None;
        }
        let text = std::fs::read_to_string(path_of(dir, id)).ok()?;
        let convo: Conversation = serde_json::from_str(&text).ok()?;
        // A file whose contents disagree with its own name is not this
        // conversation. Trusting the field over the filename would let a
        // renamed file answer for one it is not.
        if convo.id != id {
            return None;
        }
        Some(convo)
    }

    /// Write it, atomically.
    ///
    /// **TEMP FILE THEN RENAME, because the alternative is a truncated
    /// transcript.** A plain write that is interrupted — the app closed, the
    /// machine slept, the disk filled — leaves a half-written JSON file, and
    /// `load` above would then return `None` for it forever. Rename is atomic
    /// on both platforms this ships to, and Rust's `fs::rename` replaces an
    /// existing destination on Windows as well as Unix.
    ///
    /// The temp name carries the conversation id rather than a fixed word, so
    /// two turns in two windows cannot land on the same scratch file.
    pub(crate) fn save(&self, dir: &Path) -> Result<(), String> {
        if !id_is_safe(&self.id) {
            return Err("refusing to write a conversation with an unsafe id".into());
        }
        std::fs::create_dir_all(dir).map_err(|e| format!("could not create {dir:?}: {e}"))?;
        let body = serde_json::to_string(self).map_err(|e| e.to_string())?;
        let tmp = dir.join(format!("{}.json.writing", self.id));
        std::fs::write(&tmp, body).map_err(|e| format!("could not write {tmp:?}: {e}"))?;
        std::fs::rename(&tmp, path_of(dir, &self.id))
            .map_err(|e| format!("could not replace the conversation file: {e}"))
    }

    /// The history to send, newest-first-priority, oldest dropped.
    ///
    /// Returns the messages and **whether anything was dropped**, because the
    /// caller has to say so on screen. Dropping silently is the failure this
    /// whole function is written around.
    ///
    /// **WHOLE MESSAGES ONLY.** Cutting a message in half to fit the budget
    /// would hand the model a sentence that stops mid-word and read, to it, as
    /// the person having typed that. Messages are kept or dropped entire.
    ///
    /// **THE LAST MESSAGE IS ALWAYS KEPT, even if it alone blows the budget.**
    /// That message is what the person just typed. Sending a turn with the
    /// question removed is worse than sending one that is too long — the first
    /// produces a confident answer to nothing, the second produces an error
    /// from the endpoint that names the real problem.
    pub(crate) fn messages_for_send(&self, budget: usize) -> (Vec<Message>, bool) {
        let mut kept: Vec<Message> = Vec::new();
        let mut used = 0usize;
        let mut dropped = false;
        for m in self.messages.iter().rev() {
            let cost = m.text.chars().count();
            if !kept.is_empty() && used + cost > budget {
                dropped = true;
                // Everything older than this is older still. Stop rather than
                // continuing to look for a short message that would fit — a
                // history with a hole in the middle is a conversation that
                // never happened.
                break;
            }
            used += cost;
            kept.push(m.clone());
        }
        kept.reverse();
        (kept, dropped)
    }

    /// Add one completed exchange.
    ///
    /// The only writer. See the module header for why it takes both halves at
    /// once rather than offering an `append`.
    pub(crate) fn commit(&mut self, user: String, assistant: String) {
        self.messages.push(Message { role: Role::User, text: user, tool: None });
        self.messages.push(Message { role: Role::Assistant, text: assistant, tool: None });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn convo_with(texts: &[(Role, &str)]) -> Conversation {
        let mut c = Conversation::new("helloim-chat-aaaa".into(), "p1".into(), "m".into());
        for (role, text) in texts {
            c.messages.push(Message { role: *role, text: (*text).to_string(), tool: None });
        }
        c
    }

    /// **THE PATH CHECK, AND IT IS PROVEN AGAINST THE STRINGS THAT MATTER.**
    /// Every one of these would become a filename if `id_is_safe` returned
    /// true, and the traversal cases would become a filename somewhere other
    /// than the conversations folder.
    ///
    /// Proven able to fail by mutation: replacing the body with `!id.is_empty()`
    /// makes every case below pass except the empty one.
    #[test]
    fn an_id_can_never_become_a_path() {
        for good in ["helloim-chat-0011aabb", "a", "0", "a-b-c", &"x".repeat(64)] {
            assert!(id_is_safe(good), "{good:?} should be accepted");
        }
        for bad in [
            "",
            "../etc/passwd",
            "..",
            "a/b",
            "a\\b",
            "C:name",
            "name.json",
            "Name",           // uppercase: not one of ours, so not trusted
            "nameos chat",    // space
            "a\0b",
            &"x".repeat(65),  // one over the cap
        ] {
            assert!(!id_is_safe(bad), "{bad:?} must be refused");
        }
    }

    /// A minted id is safe by construction, and carries the prefix that makes a
    /// stray one recognisable across the engine boundary.
    #[test]
    fn a_minted_id_is_safe_and_labelled() {
        for _ in 0..50 {
            let id = new_id();
            assert!(id_is_safe(&id), "{id} is not safe to use as a filename");
            assert!(id.starts_with("helloim-chat-"), "{id} is not labelled as ours");
        }
        assert_ne!(new_id(), new_id(), "two conversations must not share an id");
    }

    /// **THE TRIM IS ANNOUNCED, WHICH IS THE HALF THAT MATTERS.** A version
    /// that trimmed correctly and returned `false` would pass every other
    /// assertion here and still ship the silent-forgetting bug.
    ///
    /// Proven able to fail: hardcoding the second return value to `false` fails
    /// this; removing the `used + cost > budget` guard fails it the other way.
    #[test]
    fn an_over_budget_history_drops_the_oldest_and_says_so() {
        let c = convo_with(&[
            (Role::User, "AAAAA"),      // 5
            (Role::Assistant, "BBBBB"), // 5
            (Role::User, "CCCCC"),      // 5
            (Role::Assistant, "DDDDD"), // 5
        ]);
        // Room for two messages, not four.
        let (kept, dropped) = c.messages_for_send(12);
        assert!(dropped, "dropping history must be reported, never silent");
        assert_eq!(
            kept.iter().map(|m| m.text.as_str()).collect::<Vec<_>>(),
            vec!["CCCCC", "DDDDD"],
            "the NEWEST messages are the ones kept"
        );
    }

    /// Under budget, nothing is dropped and nothing is announced. A warning
    /// that fires on an ordinary conversation is a warning people stop reading.
    #[test]
    fn a_short_history_is_untouched_and_unannounced() {
        let c = convo_with(&[(Role::User, "hi"), (Role::Assistant, "hello")]);
        let (kept, dropped) = c.messages_for_send(HISTORY_BUDGET_CHARS);
        assert!(!dropped);
        assert_eq!(kept.len(), 2);
    }

    /// **THE MESSAGE THE PERSON JUST TYPED IS NEVER THE ONE DROPPED**, even
    /// when it alone is over budget. A turn with the question removed produces
    /// a confident answer to nothing; a turn that is too long produces an
    /// error from the endpoint naming the real problem, which is strictly more
    /// useful.
    ///
    /// Proven able to fail: moving the `!kept.is_empty()` guard out of the
    /// condition returns an empty message list here.
    #[test]
    fn the_newest_message_survives_even_when_it_alone_blows_the_budget() {
        let c = convo_with(&[(Role::User, "old"), (Role::User, &"Z".repeat(500))]);
        let (kept, dropped) = c.messages_for_send(10);
        assert_eq!(kept.len(), 1, "the person's own last message must be sent");
        assert_eq!(kept[0].text.len(), 500);
        assert!(dropped);
    }

    /// A budget counted in CHARACTERS, not bytes. A conversation in a language
    /// that is three bytes per character would otherwise be trimmed three
    /// times as hard as an English one for no reason a user could ever guess.
    ///
    /// Proven able to fail: `m.text.len()` in place of `chars().count()` drops
    /// the second message here.
    #[test]
    fn the_budget_counts_characters_rather_than_bytes() {
        let c = convo_with(&[(Role::User, "ありがとうございます"), (Role::Assistant, "はい")]);
        // 10 chars + 2 chars = 12 characters, but 36 bytes.
        let (kept, dropped) = c.messages_for_send(12);
        assert_eq!(kept.len(), 2, "both fit in a CHARACTER budget of 12");
        assert!(!dropped);
    }

    /// A round trip through the real filesystem, including the atomic replace
    /// of an existing file — the second `save` is the one that would fail if
    /// rename could not overwrite.
    #[test]
    fn a_conversation_survives_a_round_trip_and_a_rewrite() {
        let dir = std::env::temp_dir().join(format!("nameos-store-{}", new_id()));
        let mut c = Conversation::new(new_id(), "p1".into(), "qwen3:4b".into());
        c.commit("first question".into(), "first answer".into());
        c.save(&dir).expect("save");

        let read = Conversation::load(&dir, &c.id).expect("load");
        assert_eq!(read.messages.len(), 2);
        assert_eq!(read.messages[0].role, Role::User);
        assert_eq!(read.messages[1].text, "first answer");
        assert_eq!(read.model, "qwen3:4b");

        c.commit("second".into(), "answer".into());
        c.save(&dir).expect("overwriting an existing conversation must work");
        assert_eq!(Conversation::load(&dir, &c.id).unwrap().messages.len(), 4);

        // No scratch file left behind.
        assert!(
            !dir.join(format!("{}.json.writing", c.id)).exists(),
            "the temp file must not survive a successful save"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **A FILE THAT DISAGREES WITH ITS OWN NAME IS NOT THAT CONVERSATION.**
    /// Trusting the field over the filename would let a copied or renamed file
    /// answer for a conversation it is not — which, on a store keyed by a
    /// session id, is one conversation's history appearing inside another.
    ///
    /// Proven able to fail: deleting the `convo.id != id` check returns the
    /// impostor here.
    #[test]
    fn a_file_whose_id_does_not_match_its_name_is_refused() {
        let dir = std::env::temp_dir().join(format!("nameos-store-{}", new_id()));
        std::fs::create_dir_all(&dir).unwrap();
        let impostor = Conversation::new("helloim-chat-somebodyelse".into(), String::new(), String::new());
        std::fs::write(dir.join("helloim-chat-mine.json"), serde_json::to_string(&impostor).unwrap())
            .unwrap();
        assert!(Conversation::load(&dir, "helloim-chat-mine").is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Corrupt, missing and unsafe all answer the same way — `None`, nothing
    /// touched. The corrupt file must still be on disk afterwards: it is
    /// somebody's history, and the safest operation on it is the one we never
    /// perform.
    #[test]
    fn unreadable_history_is_left_exactly_where_it_is() {
        let dir = std::env::temp_dir().join(format!("nameos-store-{}", new_id()));
        std::fs::create_dir_all(&dir).unwrap();
        let corrupt = dir.join("helloim-chat-corrupt.json");
        std::fs::write(&corrupt, "{ this is not json").unwrap();

        assert!(Conversation::load(&dir, "helloim-chat-corrupt").is_none());
        assert!(Conversation::load(&dir, "helloim-chat-missing").is_none());
        assert!(Conversation::load(&dir, "../escape").is_none());
        assert!(
            corrupt.exists(),
            "a corrupt conversation must never be deleted or overwritten by a read"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The commit rule, as a unit: one exchange in, two messages out, in order.
    #[test]
    fn a_commit_writes_the_pair_in_order() {
        let mut c = Conversation::new(new_id(), String::new(), String::new());
        c.commit("q".into(), "a".into());
        assert_eq!(c.messages[0], Message { role: Role::User, text: "q".into(), tool: None });
        assert_eq!(c.messages[1], Message { role: Role::Assistant, text: "a".into(), tool: None });
    }
}
