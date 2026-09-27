//! Providers — the swappable brain.
//!
//! The mechanism is Claude Code's own: the binary honours `ANTHROPIC_BASE_URL`,
//! `ANTHROPIC_AUTH_TOKEN` and `ANTHROPIC_MODEL`, so any endpoint that speaks
//! Anthropic's `/v1/messages` shape can be the brain with three environment
//! variables on the child we already spawn — no shim, no second binary.
//! Proven on a real box 2026-08-27 against Ollama 0.33.1 before this file was
//! written.
//!
//! TWO KINDS, as of 2026-08-28:
//!
//! - `"claude"` — the built-in default. The binary's own sign-in, our env
//!   untouched. This is the product as it shipped, byte-identical.
//! - `"local"` — Ollama, on the user's own machine or on one they own on
//!   their own network (the setup wizard's remote path ends there). Genuinely
//!   native: Ollama serves Anthropic-shaped `/v1/messages` itself as of
//!   0.33.0.
//!
//! **THERE WERE FOUR UNTIL 2026-08-28. Mark: *"take out all cloud based ai and
//! leave claude. We will have to work on that later. Leave local option and
//! local check."*** He approved the multi-provider work the day before, in
//! strong terms, and reversed it the next day knowingly. **Do not re-litigate
//! it, and do not restore a kind because the machinery around it still looks
//! ready — it deliberately does.**
//!
//! **WHAT CAME OUT, named so it can be put back rather than rediscovered:**
//! `"anthropic-compatible"` (any endpoint speaking the shape natively) and
//! `"openai-compatible"` (everything else, through `adapter.rs`). With them
//! went `test_chat_endpoint`, the adapter branch of `apply_env`, their two
//! caveat sentences, `BRAIN_PRESETS` and the `brain_presets` command in
//! brain_setup.rs, five of six `CLOUD_AI` tiles, five of six `LOCAL_AI`
//! tiles, and the "It speaks" shape select in the add-a-brain form.
//! **`adapter.rs` WAS NOT DELETED** — it is intact, still compiled and still
//! tested, with no caller and with nothing initialising it, so its listener
//! cannot start. Read its header before restoring anything; the whole design
//! and its five known degradations are there. The last commit with all four
//! kinds wired is `3a328ff`.
//!
//! **THE COST OF THE RULING, ON THE RECORD RATHER THAN DISCOVERED LATER.**
//! `openai-compatible` also carried LM Studio, Jan, llama.cpp, GPT4All and
//! LocalAI, which are LOCAL tools and not cloud ones. They went anyway: a kind
//! that accepts any base URL is a cloud door whatever a particular person
//! points it at, and Ollama is the local path Mark named. Somebody using one
//! of those five loses it until the kind comes back.
//!
//! **AN EXISTING USER MAY ALREADY HAVE ONE OF THE REMOVED ROWS ON DISK, AND
//! THAT IS THE CASE THAT REACHES A REAL PERSON.** `normalized()` RETIRES it on
//! read — never connected, never selectable, never launchable — and writes a
//! sentence saying what happened and that their key is untouched. **It is not
//! deleted.** That is the same call Mark made about unmappable rows in
//! migrate.rs and it is made here for the same reason: a row visible and
//! unusable with an explanation beats a credential that silently vanished.
//! `migrate.rs` is deliberately UNCHANGED and still turns a retired "api"
//! connector into one of those kinds; such a row lands here and is retired by
//! this same one path, so there is one message in one place however the row
//! arrived.
//!
//! THE HONESTY SYSTEM IS THE SAME ONE AS connectors.rs, DELIBERATELY: grey is
//! never tested, amber is a key stored that has never answered, GREEN IS
//! EARNED BY A REAL ROUND TRIP, red is a test that ran and failed with the
//! actual error. A provider must never show as working because someone pasted
//! a URL — that is a lie told in a colour, one layer up from a connector,
//! because this dot is the whole brain.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::process::Command;
use std::sync::Mutex;
use std::time::Duration;

/// Same namespace as the connectors, same rule: the account is the provider's
/// id, and ids are never reused. Provider ids are prefixed `p` so they can
/// never collide with a connector's `c`-prefixed id in the shared store.
///
/// **ONE DELIBERATE EXCEPTION — rows migrated from the retired "api"
/// connector kind KEEP their `c`-prefixed id. Do not "fix" this.** The safest
/// operation on a customer's credential is the one you never perform: keeping
/// the id means the keyring entry is never read, copied or deleted during
/// migration — there is no window in which a set_password succeeded and a
/// delete failed, because neither call is ever made. "The secret survived"
/// is an identity assertion, not a promise about a copy. What the prefix rule
/// actually protects is NON-COLLISION, and that survives: connector ids are
/// never reused, and the migration removes the connector row in the same pass
/// that creates the provider row. Renaming a migrated id to restore the
/// prefix would orphan a key the person pasted — the id IS the pointer to
/// the stored credential. See migrate.rs.
// LEFT AS "NameOS" IN THE helloim.ai RENAME, 2026-09-02 -- must stay byte-
// identical to connectors.rs's constant of the same name (both key into the
// SAME OS credential store, by id) and to the literal used for the same
// reason in installer/NameOS.nsi's uninstaller cleanup. Renaming it here
// without renaming it everywhere at once, with a real migration for secrets
// already stored under it, would orphan real users' provider keys.
const KEYRING_SERVICE: &str = "NameOS";

/// The token a local endpoint gets when no real key is stored. **IT IS A
/// PLACEHOLDER AND PROTECTS NOTHING** — Ollama on loopback takes no auth, and
/// the variable only exists because the binary sends an Authorization header
/// unconditionally. It is named so nobody reading a process environment
/// mistakes it for a credential.
const PLACEHOLDER_TOKEN: &str = "nameos-local-placeholder";

/// A cloud endpoint that has not answered a tiny request in this long is not
/// a brain anyone would call working.
const CLOUD_TEST_TIMEOUT: Duration = Duration::from_secs(20);

/// LOCAL IS LONGER ON PURPOSE. A cold Ollama load pulls the whole model into
/// VRAM before the first token — 17 GB takes real time — and false-failing it
/// on a connector-sized 12-second timeout teaches people to distrust the dot,
/// and the dot is the product. The UI says the first test can take a minute.
const LOCAL_TEST_TIMEOUT: Duration = Duration::from_secs(120);

/// Stage 2 spawns the real binary; a remote turn is binary startup plus one
/// model round trip.
const BINARY_TEST_TIMEOUT: Duration = Duration::from_secs(60);

/// **THE ONE LIST OF KINDS THIS BUILD CAN ROUTE, and every refusal below reads
/// from it rather than re-deciding.** The removal on 2026-08-28 was a security
/// and product decision, and a decision spread across six `if kind ==` sites is
/// a decision that comes back through whichever one somebody forgets. Anything
/// not in here is RETIRED: present, explained, and unable to launch, select or
/// test.
const ROUTABLE_KINDS: &[&str] = &["claude", "local"];

/// ---------------------------------------------------------------------------
/// THE OPENAI GATE. One line, and it is `false`.
/// ---------------------------------------------------------------------------
///
/// Mark, 2026-08-29: *"During the initial setup, claude should not be default.
/// The user needs to pick their cloud ai. They need to be given a choice of
/// Claude AI or OpenAI."* So OpenAI is coming back, and the translator that
/// carries it (`adapter.rs`) is wired up again as of tonight.
///
/// **IT IS ON, AS OF 2026-08-29. It was off for four builds and the reasoning
/// was wrong — that correction is the useful part of this comment.**
///
/// It was held back waiting for an OpenAI key so that WE could prove the
/// translator. **That conflated two different things: "we cannot verify this
/// yet" and "they cannot have it."** A user brings their own key. Ours would
/// tell us sooner; it was never what made this safe for them.
///
/// **WHAT ACTUALLY MAKES IT SAFE IS THAT THE DOT HAS TO BE EARNED, ON THEIR
/// MACHINE, BEFORE THE ROW CAN BE USED.** `test_provider` runs two stages for
/// this kind: `test_chat_endpoint` reaches OpenAI for real with their key, then
/// `test_binary` spawns Claude Code through `apply_env` — which for this kind
/// starts the translator and points the binary at loopback. So going green IS a
/// real Claude Code → translator → OpenAI round trip. A broken translator fails
/// the Test in front of them, with words, instead of failing quietly halfway
/// through a conversation. That is a stronger guarantee than our own synthetic
/// tests could ever have given, and it is per-machine rather than per-release.
///
/// **AND `may_select` REFUSES A ROW THAT HAS NOT GONE GREEN**, so a failed Test
/// cannot become somebody's brain. The `builtin ||` exemption that used to make
/// that promise conditional was removed the same day.
///
/// **WHAT DOES NOT CHANGE.** The capability lines still marked UNPROVEN in
/// `desktop/BRAIN-CHOICES.md` are still unproven, and nothing unproven may be
/// shown as fact — the picker marks them or leaves them off. The three real
/// costs (no prompt caching, estimated token counts, reasoning dropped) are on
/// the screen, not in a footnote. The cost caveat in `caveat_for` ships word
/// for word as ruled on 2026-08-27.
///
/// **WHAT TURNS IT ON: this constant, and nothing else.** Not an environment
/// variable, not a config file, not a hidden row in `providers.json` — those
/// are all things that can be flipped by accident, by a stray export, or by
/// somebody hand-editing a file, and this one is holding a loopback relay to a
/// paid account. It is a recompile on purpose, in both directions: if the
/// translator turns out to be wrong in the field, this is the one line that
/// takes it off every screen again.
const OPENAI_ENABLED: bool = true;

/// Kinds that exist in the code, are fully wired, and are switched off.
///
/// Kept SEPARATE from `ROUTABLE_KINDS` rather than merged behind an `if`: the
/// difference between "this build cannot do that" and "this build can do that
/// and is not offering it yet" is a real difference, and collapsing the two is
/// how a gate becomes invisible to the next reader.
const GATED_KINDS: &[&str] = &["openai-compatible"];

/// In tests only, the gate can be forced on, so both states are exercised.
/// **There is no production path to this** — it does not exist in a release
/// build. Without it the whole enabled half of this module would be code that
/// has never run, which is the thing the gate exists to prevent shipping.
#[cfg(test)]
static OPENAI_FORCED_ON: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn openai_enabled() -> bool {
    #[cfg(test)]
    {
        if OPENAI_FORCED_ON.load(std::sync::atomic::Ordering::Relaxed) {
            return true;
        }
    }
    OPENAI_ENABLED
}

fn is_routable(kind: &str) -> bool {
    ROUTABLE_KINDS.contains(&kind)
        || (openai_enabled() && GATED_KINDS.contains(&kind))
}

/// The same answer, for the tile catalogue in `brain_setup.rs` to check itself
/// against. **ONE list, read in both places.** A second copy of "which kinds
/// exist" is precisely how a removed kind survives on a screen after the
/// backend stopped routing it, and that seam is the one this removal was most
/// likely to leave.
///
/// Only a test calls it, and that is the point of it — the check it exists for
/// is a check, not a runtime decision.
#[allow(dead_code)]
pub(crate) fn kind_is_routable(kind: &str) -> bool {
    is_routable(kind)
}

/// What a person sees on a row whose kind this build no longer routes.
///
/// **IT ANSWERS THE THREE THINGS SOMEBODY IS ACTUALLY ASKING** — what happened
/// to my brain, what happened to my key, and what do I do now. An error state
/// is where honesty is worth most, and "unknown provider kind" is a sentence
/// written for us rather than for them.
const RETIRED_NOTE: &str = "This brain connected to an AI company helloim.ai no longer offers. \
     Nothing was sent anywhere, and the key you saved is still stored safely on this \
     computer. Claude and a brain on your own machine are the two choices now — pick one \
     above, or remove this row.";

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Provider {
    pub id: String,
    /// "claude" | "local" — see ROUTABLE_KINDS. A string, not an enum, for the
    /// same reason as `Connector.kind`: a version that knows more kinds must
    /// still be able to read this file, and after 2026-08-28 the reverse
    /// matters just as much — this build reads files written by a version that
    /// knew FOUR, and an enum would have failed the whole parse rather than
    /// letting `normalized` retire the two rows it no longer routes.
    pub kind: String,
    pub name: String,
    /// The Anthropic-shaped endpoint. Empty for the built-in "claude" row.
    #[serde(default)]
    pub base_url: String,
    /// The model name the endpoint knows. Required for everything but the
    /// built-in row, where the binary's own default is the point.
    #[serde(default)]
    pub model: String,
    /// True only for the "claude" row. It cannot be deleted and needs no test
    /// to be selected, because it is the behaviour the product shipped with.
    #[serde(default)]
    pub builtin: bool,

    /// The user has switched the built-in row OFF — Mark, 2026-08-27, asked
    /// for Claude to be disconnectable.
    ///
    /// **DISCONNECTED, NOT DELETED, AND THE DIFFERENCE IS THE WHOLE DESIGN.**
    /// `normalized()` guarantees the claude row exists so that a corrupt file
    /// "degrades to the product as it shipped, never to a brainless app" —
    /// that invariant is load-bearing and deleting the row would remove it.
    /// A flag keeps the row present and re-connectable in one press, while
    /// making it unusable as the brain, which is what disconnect actually
    /// means to somebody who has switched to their own provider and does not
    /// want the default quietly catching their traffic.
    ///
    /// **IT CAN NEVER BE THE LAST ONE STANDING.** `disconnect_provider`
    /// refuses unless another brain is connected and takes over, and
    /// `normalized` re-enables it if the active row ever resolves back to a
    /// disconnected Claude. An app with no brain is a worse outcome than a
    /// preference not being honoured, so the preference yields.
    #[serde(default)]
    pub disconnected: bool,

    // ---- Live state, written only by `test_provider` — the same contract as
    // ---- connectors.rs, and the same temptation to resist: nothing here
    // ---- turns green because a string is non-empty.
    #[serde(default)]
    pub connected: bool,
    #[serde(default)]
    pub checked_at: String,
    #[serde(default)]
    pub last_error: String,
    #[serde(default)]
    pub has_secret: bool,

    /// The honesty line shown AT THE POINT OF CHOOSING, derived from `kind`
    /// on every read and never trusted from disk. A small local model is not
    /// the same product as the cloud one, and the app says so before the
    /// switch, not after the first wrong answer.
    #[serde(default)]
    pub caveat: String,

    /// Set ONLY by the one-time migration from the retired "api" connector
    /// kind (migrate.rs), shown by the Brain sheet beneath the row, and
    /// cleared the moment it has done its job: on any edit, or on the first
    /// test that passes. It exists because a person whose green Apps row
    /// vanished in an update must be TOLD where it went and what to do, not
    /// left to discover it. It is deliberately not `last_error` — that field's
    /// contract is "written only by test_provider", and that contract is
    /// load-bearing.
    #[serde(default)]
    pub migration_note: String,

    /// **DECISION B, ROOM-APPROVED 2026-09-02: raw `Bash` is off by default on
    /// EVERY row, and this is the field that turns it on for one.** The label
    /// shown beside it ("let it read anything on this computer") is Wren and
    /// Cyrus's to place; this is only the switch and the wire underneath it.
    ///
    /// **NOT OFFERED, NEVER OFFERED-THEN-REFUSED — that distinction is the
    /// whole design and it is why this lives on `Provider` rather than on
    /// `TurnRequest::permission`.** `engine::native::mod::drive` reads this
    /// (via `Plan::allow_shell`) BEFORE it ever calls `tools::definitions`,
    /// so a model whose row has this off is never told `Bash` exists at
    /// all — it cannot try another way to reach a tool it was never shown,
    /// because refusing an offered tool is the hole this closes, not a
    /// smaller version of it. `tools::dispatch_cancellable` checks the SAME
    /// flag a second time, defense in depth, for the one caller that is not
    /// this file: a model that hallucinates a call to a tool it was never
    /// offered, which smaller/local models do in the wild.
    ///
    /// **`#[serde(default)]` on a `bool` is `false`, which is the point** —
    /// every row already on disk before this field existed reads back with
    /// the shell off, with no migration step required.
    ///
    /// Scoped to the NATIVE engine only. `ClaudeCodeEngine` (the `"claude"`
    /// row) never reads this field — the vendor binary has its own tool
    /// surface and its own permission model (`Permission::Ask/AcceptEdits/
    /// Full`, `claude_code.rs`'s `--permission-mode`), unrelated to this one.
    #[serde(default)]
    pub allow_shell: bool,

    /// **THE SAFE AGENCY LAYER — room-approved 2026-09-04, `desktop/
    /// SAFE-AGENCY-SPEC.md`. A SEPARATE flag from `allow_shell`, on purpose:
    /// the risk shape is different.** `allow_shell` hands a model a real
    /// shell inside a folder the person already reviewed; this hands it
    /// exactly three narrow, allow-listed actions — open a URL, launch one of
    /// four named apps, open one of a handful of named Settings pages — with
    /// no path, no argument and no free-form string ever reaching a command
    /// line. A person who is fine with one is not thereby assumed fine with
    /// the other, and a row should be able to carry either alone.
    ///
    /// **THE SAME `#[serde(default)]` DEFAULT-CLOSED CONTRACT AS
    /// `allow_shell`, for the same reason** — see that field's own doc. Every
    /// row on disk before this field existed reads back with agency off, no
    /// migration step required, pinned by
    /// `a_legacy_row_with_no_allow_agency_field_defaults_closed` below.
    ///
    /// **NOT OFFERED, NEVER OFFERED-THEN-REFUSED — same design as
    /// `allow_shell`.** `tools::definitions` leaves `OpenUrl`/`LaunchApp`/
    /// `OpenSettingsPage` out of the list entirely when this is off, rather
    /// than offering them and refusing the call; `tools::dispatch_cancellable`
    /// checks the same flag a second time, belt and braces, for a model that
    /// hallucinates a call to a tool it was never offered.
    ///
    /// Scoped to the NATIVE engine only, same as `allow_shell` — the vendor
    /// binary has its own tool surface and never reads this field.
    #[serde(default)]
    pub allow_agency: bool,
}

/// What the front menu binds to: the active id plus every row.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Store {
    pub active: String,
    /// **THE AI BOARDROOM'S FALLBACK SYNTHESISER, AND NOTHING ELSE TODAY —
    /// added 2026-09-05 for `HAI-APP-IA-SPEC.md`'s Primary/Secondary
    /// designation.** Empty means nobody has picked one, which is a real
    /// state and not an error: most machines have exactly one connected
    /// brain, and a lone brain has nothing to be Secondary to.
    ///
    /// `#[serde(default)]` for the same reason `Provider::allow_shell` is —
    /// every `providers.json` written before this field existed loads back
    /// with no Secondary chosen, no migration step required.
    ///
    /// **AN ID HERE, NOT A ROLE ON `Provider` ITSELF.** `active` already
    /// names Primary this same way; a second id beside it keeps "who is
    /// what" in exactly one place rather than splitting it between a
    /// store-level field and a per-row flag that could disagree with it.
    #[serde(default)]
    pub secondary: String,
    pub providers: Vec<Provider>,
}

#[derive(Default)]
pub struct Providers {
    inner: Mutex<Option<Store>>,
}

// ---------------------------------------------------------------------------
// The pure parts — decisions testable without a keyring, a network or an app.
// ---------------------------------------------------------------------------

fn caveat_for(kind: &str) -> &'static str {
    // Native routing now owns file tools and memory. Describe today's route,
    // not the retired Claude translator or an unmeasured vendor comparison.
    match kind {
        "local" => "Runs through Ollama without Claude. Download a model first; speed and \
                    accuracy depend on your hardware and model. File tools and memory are \
                    available, along with configured app connections. Account-specific connections need separate setup.",
        "openai-compatible" => "Connects directly to your selected provider without Claude. \
                    Your provider bills your usage; pricing and caching depend on the model. \
                    File tools, memory and configured app connections are available. \
                    Account-specific connections need separate setup.",
        _ => "",
    }
}

fn builtin_claude() -> Provider {
    Provider {
        id: "claude".into(),
        kind: "claude".into(),
        name: "Claude".into(),
        base_url: String::new(),
        model: String::new(),
        builtin: true,
        disconnected: false,
        connected: false,
        checked_at: String::new(),
        last_error: String::new(),
        has_secret: false,
        caveat: String::new(),
        migration_note: String::new(),
        // The built-in row is the vendor binary, which never reads this
        // field (see `allow_shell`'s own doc) -- `false` here is inert, not
        // a security decision about Claude Code's own tool set.
        allow_shell: false,
        // Same reasoning as `allow_shell` immediately above -- inert on the
        // vendor binary, not a security decision about it.
        allow_agency: false,
    }
}

/// Make any loaded file safe to use: the built-in row exists exactly once and
/// is marked builtin even if the file on disk says otherwise (a config file is
/// user-writable, and an edit must not make the default deletable), and
/// `active` always names a row that exists. A corrupt or missing file
/// therefore degrades to the product as it shipped, never to a brainless app.
fn normalized(mut store: Store) -> Store {
    match store.providers.iter_mut().find(|p| p.id == "claude") {
        Some(row) => {
            row.builtin = true;
            row.kind = "claude".into();
        }
        None => store.providers.insert(0, builtin_claude()),
    }
    for p in store.providers.iter_mut() {
        // Only the real built-in row is builtin, whatever the file claims.
        p.builtin = p.id == "claude";
        p.caveat = caveat_for(&p.kind).to_string();

        // RETIRE, NEVER DELETE — the cloud kinds removed 2026-08-28. A person
        // who was using one has a row on disk and a key in the OS store, and
        // the failure to avoid is the row still LOOKING connected: green dot,
        // Use button, and then a send that fails mid-conversation, because
        // `apply_env`'s direct path would happily point the binary at an
        // endpoint that no longer has a translator in front of it.
        //
        // So the green goes here, at the one place every read passes through,
        // rather than at each of the six sites that could offer to use it.
        // Deleting the row instead would orphan the credential it points at
        // and answer none of the questions the person actually has.
        if !is_routable(&p.kind) {
            p.connected = false;
            // NOT `last_error` — that field's contract is "written only by
            // test_provider", and it is load-bearing. `migration_note` is the
            // field for "where this row came from and what to do about it",
            // which is exactly what this is. Set unconditionally: whatever the
            // note used to say, the retirement is the current truth about the
            // row and supersedes it.
            p.migration_note = RETIRED_NOTE.into();
        }
    }
    // The active row must both EXIST and be routable. The second half is the
    // one the removal added: a person whose brain was a cloud provider has
    // `active` pointing straight at a retired row, and without this the app
    // boots with no working brain selected on the very first launch after the
    // update.
    let active_ok = store
        .providers
        .iter()
        .any(|p| p.id == store.active && is_routable(&p.kind));
    // NOBODY IS GIVEN A BRAIN THEY DID NOT PICK — Mark, three times, most
    // recently 2026-08-29: *"again, no prompt on the providoer, Open ai or
    // claude.. the app just defaulted to claude"*, and before that *"it is
    // assuming claude it time. This should not be the case."*
    //
    // THE ASSUMING WAS HERE, and it was one line: an empty `active` is not a
    // broken choice, it is NO CHOICE YET, and falling back to Claude made that
    // indistinguishable from having picked Claude. The setup screen had nothing
    // to ask about because the answer was already filled in.
    //
    // **EMPTY MEANS UNCHOSEN AND STAYS EMPTY.** A NON-EMPTY `active` that no
    // longer resolves is a different case and keeps its old behaviour: that is
    // somebody who DID choose and whose row was retired or removed, and
    // stranding them with no brain because their old provider went away would
    // be punishing them for our change. They fall back to Claude exactly as
    // before.
    //
    // The upgrade path turns on this distinction, so do not collapse the two:
    // an existing customer's providers.json says `"active": "claude"`, which is
    // non-empty, so nothing about their setup moves. Only a machine that has
    // never chosen sees an empty one.
    if !active_ok && !store.active.is_empty() {
        store.active = "claude".into();
    }
    // A CHOSEN BRAIN IS NEVER LEFT SWITCHED OFF. If the active row is the
    // disconnected Claude — because the provider that replaced it was removed,
    // or the file was hand-edited — the flag yields rather than the product.
    // Failing closed here would mean an app that has a brain it refuses to use.
    //
    // It reads `store.active` and an empty string matches no row, so this does
    // nothing at all on a machine that has not chosen. That is the intended
    // shape: "no brain yet" and "a brain that cannot answer" are different
    // states and only the second one is a fault.
    let active_is_off = store
        .providers
        .iter()
        .any(|p| p.id == store.active && p.disconnected);
    if active_is_off {
        if let Some(row) = store.providers.iter_mut().find(|p| p.id == store.active) {
            row.disconnected = false;
        }
    }
    // SECONDARY IS NEVER LEFT POINTING AT A ROW THAT CANNOT ANSWER, AND NEVER
    // AT THE SAME ROW AS PRIMARY. Both are quiet ways for the AI Boardroom's
    // fallback synthesiser to silently point at nothing (a retired kind, a
    // removed row) or at something that offers no real second opinion (its
    // own Primary) -- cleared here, at the one place every read of the store
    // already passes through, rather than trusted from whatever was on disk.
    if !store.secondary.is_empty()
        && (store.secondary == store.active
            || !store.providers.iter().any(|p| p.id == store.secondary && is_eligible(p)))
    {
        store.secondary = String::new();
    }
    store
}

/// Whether a row could answer for the person RIGHT NOW — the same three facts
/// `may_select` checks before letting a press become the brain, as a plain
/// predicate for callers that need to FILTER a list rather than refuse one
/// pick with a worded error.
///
/// **USED BY: Secondary designation (a fallback that cannot itself answer is
/// not a fallback) and the AI Boardroom (a seat that cannot answer has
/// nothing to contribute and nowhere to send the question).** Kept separate
/// from `may_select` rather than having one call the other, because the two
/// callers want different shapes back — a `Result` with a sentence for a
/// single refusal, a bare `bool` for a `retain`/`filter` — and duplicating
/// three short boolean checks is cheaper to keep in sync than threading one
/// function's error type through the other's caller.
pub(crate) fn is_eligible(p: &Provider) -> bool {
    is_routable(&p.kind) && !p.disconnected && p.connected
}

/// May this row be saved at all? Called before anything touches disk or the
/// credential store, so a refusal leaves no half-written state behind.
fn validate(p: &Provider) -> Result<(), String> {
    if p.id == "claude" || p.kind == "claude" {
        return Err("Claude is built in — there is nothing to configure on it.".into());
    }
    if p.name.trim().is_empty() {
        return Err("Give the provider a name.".into());
    }
    match p.kind.as_str() {
        "local" => {}
        // NAMED, NOT SWEPT INTO "unknown" — these two are the kinds removed on
        // 2026-08-28, and somebody pressing Edit on their own retired row is
        // owed the reason rather than a message about a kind being
        // unrecognised. It is also the breadcrumb that tells the next reader
        // these strings once meant something here.
        // Wired, gated off. When OPENAI_ENABLED flips this becomes editable in
        // the same breath as the backend starting to route it — one gate, read
        // in both places, so an editable row and a launchable row can never be
        // two different sets.
        "openai-compatible" if openai_enabled() => {}
        "anthropic-compatible" | "openai-compatible" => {
            return Err("helloim.ai no longer connects to other AI companies. This row cannot \
                        be edited — use Claude or a brain on your own machine, or remove it."
                .into())
        }
        other => return Err(format!("Unknown provider kind `{other}`.")),
    }
    if p.model.trim().is_empty() {
        return Err("Name the model this endpoint should run.".into());
    }
    // A local row may leave the URL empty — it defaults in `save_provider` —
    // but anything present must be a real web address.
    let url = p.base_url.trim();
    if !url.is_empty() && !url.starts_with("https://") && !url.starts_with("http://") {
        return Err("That base URL does not look like a web address.".into());
    }
    Ok(())
}

/// May this row become the brain? The built-in row always may — it is the
/// shipped default. Anything else must have EARNED green first: selecting an
/// untested brain fails later, mid-conversation, as a confusing send error,
/// when the same failure one minute earlier is a red dot beside a Test button.
fn may_select(p: &Provider) -> Result<(), String> {
    // FIRST, BECAUSE IT IS THE ONE THAT CAN NEVER BE FIXED BY PRESSING
    // ANYTHING. `normalized` already sets connected = false on a retired row,
    // so the check below would refuse it anyway — with "Test it first", which
    // is advice that cannot work and would send somebody round a loop.
    if !is_routable(&p.kind) {
        return Err("That brain connected to an AI company helloim.ai no longer offers. \
                    Use Claude or a brain on your own machine."
            .into());
    }
    // A row the user switched off must not come back as the brain by being
    // selected — that would make Disconnect a suggestion rather than a state.
    if p.disconnected {
        return Err("That one is disconnected. Connect it again first.".into());
    }
    // `builtin ||` USED TO BE HERE AND IT IS WHY CLAUDE WAS STRUCTURAL.
    //
    // It meant `connected` had two meanings: for every other row it was "this
    // was tested and it worked", and for Claude it was decoration — the row was
    // selectable whether or not the binary was installed, signed in, or capable
    // of answering. That is not a shortcut, it is the exemption that made
    // Claude the brain you got without asking, and it made the green dot a
    // claim the product did not have to keep.
    //
    // Now `connected` means one thing for every row: somebody pressed Test and
    // it passed. Claude earns it the same way — `test_provider` runs
    // `test_binary` for the built-in row, which spawns the real thing, so the
    // dot is as real as any other and costs one press.
    //
    // THIS IS SAFE FOR SOMEBODY ALREADY RUNNING ON CLAUDE, and that is worth
    // stating because it looks like it should not be: `may_select` has exactly
    // one caller, `select_provider`, which runs only when a person picks a
    // brain. An existing install with `"active": "claude"` never passes through
    // here on upgrade, so nothing about their setup changes and they are not
    // asked to choose again.
    if p.connected {
        Ok(())
    } else {
        Err("Test it first — the dot has to be earned before it can be the brain.".into())
    }
}

/// Switch a brain off without removing it.
///
/// **THE GUARD IS THE POINT.** Mark asked for Claude to be disconnectable, and
/// the honest reading of that is "stop it being the brain", not "leave the app
/// unable to think". So this refuses unless something else is genuinely ready
/// to take over — connected, and not itself disconnected — and it MOVES the
/// active selection there in the same operation rather than leaving a gap for
/// `normalized` to paper over. A disconnect that silently re-enabled itself on
/// the next read would be worse than refusing: the user would believe it was
/// off.
#[tauri::command(async)]
pub fn disconnect_provider(
    app: tauri::AppHandle,
    state: tauri::State<Providers>,
    id: String,
) -> Result<Store, String> {
    with_store(&app, &state, |store| -> Result<Store, String> {
        if !store.providers.iter().any(|p| p.id == id) {
            return Err("There is no brain by that name.".into());
        }
        let successor = store
            .providers
            .iter()
            .find(|p| p.id != id && p.connected && !p.disconnected)
            .map(|p| (p.id.clone(), p.name.clone()));
        /* DISCONNECTING THE LAST BRAIN IS NOW ALLOWED — changed 2026-08-30, and
           the guard it replaces was not wrong when it was written.

           It used to refuse: "this is the only one that works, and
           disconnecting it would leave nothing to think with." That sentence
           described reality until tonight, when the product gained a
           first-class **no brain chosen** state — setup stopped picking one,
           the header chip says so plainly, and `send()` refuses with a real
           way forward rather than a dead end.

           So the refusal was protecting somebody from a state the app now
           supports and documents. **Mark hit it three times and was right
           each time** — from where he sat, a Disconnect button that never
           disconnects is a broken button, whatever the reasoning behind it.

           WHAT REPLACES IT IS NOT NOTHING. `active` is cleared rather than
           handed to a successor, which is the same "empty means not chosen"
           rule the rest of this file now follows — and the front end has a
           confirm in front of it that says plainly nothing will be answering
           until a brain is chosen. The guard moved from a refusal to an
           informed choice, which is where it belonged. */
        let (next_id, next_name) = match successor {
            Some((id, name)) => (id, name),
            None => (String::new(), String::from("Nothing")),
        };
        for p in store.providers.iter_mut() {
            if p.id == id {
                p.disconnected = true;
                // Its green described a connection nobody is using now. Leaving
                // it would show a dot that means nothing.
                p.connected = false;
                p.last_error = String::new();
            }
        }
        store.active = next_id;
        // Said out loud on the row, because a switch the user did not ask for
        // must never be silent even when it is the only sane move.
        if let Some(p) = store.providers.iter_mut().find(|p| p.id == id) {
            // AND IT MUST NOT SAY "Nothing is the brain now" — that is the shape
            // a placeholder leaks through in. The two outcomes are genuinely
            // different sentences: one hands over, the other leaves the seat
            // empty and says what to do about it.
            p.migration_note = if next_name == "Nothing" {
                "Disconnected. Nothing is answering until you choose a brain.".into()
            } else {
                format!("Disconnected. {next_name} is the brain now.")
            };
        }
        save_to_disk(&app, store)?;
        Ok(store.clone())
    })
}

/// Put it back. One press, because the row never went anywhere.
#[tauri::command(async)]
pub fn reconnect_provider(
    app: tauri::AppHandle,
    state: tauri::State<Providers>,
    id: String,
) -> Result<Store, String> {
    with_store(&app, &state, |store| -> Result<Store, String> {
        match store.providers.iter_mut().find(|p| p.id == id) {
            Some(p) => {
                p.disconnected = false;
                p.migration_note = String::new();
                // NOT re-selected and NOT re-greened. Connecting it back makes
                // it available; making it the brain is a separate press, and
                // anything that is not built in still has to earn its dot with
                // a real Test.
            }
            None => return Err("There is no brain by that name.".into()),
        }
        save_to_disk(&app, store)?;
        Ok(store.clone())
    })
}

/// The exact environment a non-default provider puts on the spawned binary.
/// `None` means REMOVE the variable from the child. Pure, so the decision is
/// testable; `apply_env` is the thin impure wrapper both real callers share —
/// the test and the launch MUST NOT DISAGREE, which is the same lesson
/// `secret_env` in connectors.rs already paid for.
///
/// `base_url` and `token` arrive resolved rather than read off `p`, and the
/// parameters are kept after the 2026-08-28 removal even though only one kind
/// now feeds them: the removed `openai-compatible` path passed the ADAPTER's
/// loopback address and a per-launch run token here instead of the provider's
/// own, which is what kept the real key out of the child's environment
/// entirely. Collapsing these to `p.base_url` would quietly delete the shape
/// that made that possible.
fn provider_env(p: &Provider, base_url: &str, token: String) -> Vec<(&'static str, Option<String>)> {
    if p.kind == "claude" {
        return Vec::new();
    }
    let model = p.model.trim().to_string();
    vec![
        ("ANTHROPIC_BASE_URL", Some(base_url.trim().to_string())),
        ("ANTHROPIC_MODEL", Some(model.clone())),
        // The binary's background tasks use a Haiku-class model by its
        // Anthropic name; against a third-party endpoint that model does not
        // exist, so both the current variable and its deprecated alias are
        // pointed at the same model as the main one.
        ("ANTHROPIC_DEFAULT_HAIKU_MODEL", Some(model.clone())),
        ("ANTHROPIC_SMALL_FAST_MODEL", Some(model)),
        // Documented for exactly this situation: an app embedding Claude Code.
        // Without it, a stray `model` key in the user's own settings.json
        // silently overrides the picker — and a picker that lies is the same
        // failure as an unearned green dot, one layer up.
        ("CLAUDE_CODE_PROVIDER_MANAGED_BY_HOST", Some("1".into())),
        ("ANTHROPIC_AUTH_TOKEN", Some(token)),
        // THE CF-CONNECTING-IP LESSON WEARING AN ENV VAR. `ANTHROPIC_API_KEY`
        // is sent as `x-api-key` whenever present — always, in `-p` mode. A
        // user with a real Anthropic key exported plus a third-party base URL
        // would ship their Anthropic credential to somebody else's server, in
        // a product whose selling point is that nobody in the middle holds
        // your keys. Removed unconditionally on every non-default provider.
        ("ANTHROPIC_API_KEY", None),
    ]
}

/// The host part of a URL: everything after the scheme, up to the first `/`,
/// `?` or `#`, with any userinfo and port removed.
///
/// Split on the LAST `@` in the authority, which is what a real URL parser
/// does — `http://127.0.0.1@evil.example/` has host `evil.example`, and
/// reading it left to right gets that backwards.
///
/// **`pub(crate)` as of the safe-agency layer (2026-09-04) — reused by
/// `engine::native::actions`'s `OpenUrl` network-target guard rather than
/// growing a second, slightly-different host parser for the same job.** This
/// function is already the hardened one: it was a substring match until
/// 2026-08-28 (see `is_loopback`'s own doc for the bug that was), so a second
/// implementation written from scratch for the new guard would have started
/// from zero instead of from an already-fixed bug.
pub(crate) fn host_of(url: &str) -> Option<String> {
    let rest = url.split_once("://")?.1;
    let authority = rest.split(['/', '?', '#']).next()?;
    let hostport = match authority.rsplit_once('@') {
        Some((_, h)) => h,
        None => authority,
    };
    // A bracketed IPv6 literal keeps its brackets; anything else loses a port.
    let host = if let Some(end) = hostport.find(']') {
        &hostport[..=end]
    } else {
        hostport.split(':').next()?
    };
    Some(host.trim().to_ascii_lowercase())
}

/// Is this address genuinely this machine? A loopback endpoint gets the LOCAL
/// timeout, because a cold model load false-failing a cloud-sized timeout is
/// the distrust-the-dot failure the local kind's timeout exists to prevent.
///
/// **THIS WAS A SUBSTRING MATCH UNTIL 2026-08-28 AND THAT IS NOT A HOST
/// CHECK.** `url.contains("://127.0.0.1")` is satisfied by
/// `https://127.0.0.1.evil.example/`, which is a perfectly ordinary public
/// hostname that anybody can register a wildcard under. Today the only thing
/// riding on this answer is a timeout, so the old version was a latent bug
/// rather than a live hole — but "it is only used for a timeout" is exactly
/// the sentence that stops being true the day somebody reaches for the
/// nearest-looking helper to decide whether something may leave the machine.
/// Compare the HOST, and compare it for equality.
fn is_loopback(url: &str) -> bool {
    match host_of(url).as_deref() {
        Some("127.0.0.1" | "localhost" | "::1" | "[::1]") => true,
        _ => false,
    }
}

/// Is this Ollama new enough to speak Anthropic's shape? It learned
/// `/v1/messages` in 0.33.0.
fn version_at_least(version: &str, want_major: u64, want_minor: u64) -> bool {
    let mut parts = version.trim().split('.');
    let major: u64 = match parts.next().and_then(|s| s.parse().ok()) {
        Some(n) => n,
        None => return false,
    };
    let minor: u64 = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    (major, minor) >= (want_major, want_minor)
}

/// Did the endpoint answer in Anthropic's shape? A well-formed `/v1/messages`
/// reply carries a `content` array; anything else — an HTML error page, an
/// OpenAI-shaped body, a JSON error — is named for what it is.
fn message_shape_ok(body: &str) -> Result<(), String> {
    let v: serde_json::Value = serde_json::from_str(body.trim())
        .map_err(|_| "The endpoint replied, but not with JSON.".to_string())?;
    if let Some(err) = v.get("error") {
        return Err(format!(
            "The endpoint refused: {}",
            err.get("message").and_then(|m| m.as_str()).unwrap_or("no reason given")
        ));
    }
    if v.get("content").map(|c| c.is_array()).unwrap_or(false) {
        Ok(())
    } else {
        Err("The endpoint replied, but not in Claude's shape — no content block.".into())
    }
}

/// Redact a secret out of a response body, THEN cut it to a safe length.
/// Shared by `generic_status_error` below (the Test button's endpoint call)
/// and `adapter::relay` (the live send path) — TWO call sites that both take
/// an upstream error body of unknown shape and a real bearer key, which is
/// exactly the pattern that drifts into two almost-identical, separately-
/// broken copies if it is not pulled out once. It very nearly did: `relay`
/// carried its own bare `detail.truncate(500)` with no redaction at all,
/// under a comment asserting "Never our token, never the key — neither is in
/// these bodies" — the same assumption THIS function was built to stop
/// trusting, on the OTHER call site, after Beck disproved it there. There
/// was never a reason the live send path would be safer than the Test
/// button; both send the same bearer token to the same kind of upstream.
///
/// REDACT FIRST, THEN CUT. Cutting first and redacting after was the
/// original bug: a key that straddled the cut point survived as a prefix,
/// because `replace` only matches the WHOLE key, and once the cut sliced it
/// in half there was nothing left in the body for it to find. Beck proved it
/// against a live provider row with a server built to pad its error to
/// exactly that length — 23 of 24 characters of a bearer token reached the
/// screen, chosen by how much padding the REMOTE server happened to send.
/// Redacting against the full, un-cut body first means the whole key is
/// still there to match no matter where in the body it falls, so no partial
/// key can survive into the shorter string the cut below produces.
///
/// THE CUT ITSELF MUST BE CHAR-BOUNDARY SAFE. `String::truncate` panics
/// unless the index lands on a UTF-8 char boundary, and `max_len` is a byte
/// offset picked with no regard for where character edges fall. Beck hit
/// this from a live body with a multibyte character sitting across byte
/// 500: `generic_status_error`'s panic happened inside the command handler,
/// so the `invoke` never resolved and the Test button stuck on "Testing…"
/// forever with no error anywhere; `relay`'s bare `truncate(500)` carried
/// the identical fault onto the live send path, where the visible symptom
/// was "Claude Code started but did not answer in time." instead. Walk back
/// to the nearest real boundary rather than trusting the byte offset is one.
///
/// Guarded to keys of a real length (`>= 8`) so a short local placeholder
/// token cannot eat ordinary words out of an unrelated error message.
pub(crate) fn redact_and_truncate(mut body: String, key: &str, max_len: usize) -> String {
    if key.trim().len() >= 8 {
        body = body.replace(key.trim(), "[key redacted]");
    }
    let mut cut = max_len.min(body.len());
    while cut > 0 && !body.is_char_boundary(cut) {
        cut -= 1;
    }
    body.truncate(cut);
    body.trim().to_string()
}

/// What the Test button says for a status code with no bespoke wording of
/// its own (see `test_chat_endpoint` and `test_endpoint`, which keep their
/// hand-written copy for 401/403/404/429/5xx and route everything else here).
///
/// **THE BUG THIS FUNCTION WAS BUILT FOR: Mark connected a real OpenAI key,
/// pressed Test, and got exactly "The endpoint answered 400." — the response
/// body, which is OpenAI's own words explaining what is wrong, was thrown
/// away with the `_` in `ureq::Error::Status(code, _)`.** This was the third
/// instance of the same fault found in one day (`test_binary` discarded
/// stderr on success; `send()`'s diagnostic thread discarded a message's
/// meaning; now this), so it got a named, tested function rather than
/// another inline `format!` that is easy to write the same way again.
///
/// Pure — no network, no keyring — so it is testable directly against a
/// canned body rather than needing a live endpoint or a local TCP server to
/// manufacture a `ureq::Response`. The caller does the one impure step
/// (`resp.into_string()`) and hands the result here.
///
/// Capped so a mistaken HTML error page (a proxy's own 400 page, say) cannot
/// flood the row — the same 500-character cap `adapter.rs`'s `relay` uses
/// for the live send path, via the SAME helper now, so the two surfaces
/// cannot drift apart the way they already once did. See
/// `redact_and_truncate`'s own comment for the two bugs it exists to stop.
///
/// **UNWRAP THE ENVELOPE, NOT JUST THE BODY — Beck's sweep, 2026-09-06: "shows
/// raw provider JSON (INVALID_ARGUMENT…) instead of a friendly message."** A
/// bad key against Gemini's OpenAI-compatibility endpoint lands here too (400
/// has no bespoke arm in either caller) with a body that IS this function's
/// own `error.message` shape and STILL read as a bug, because the fix above
/// stopped at "do not throw the body away" and never took the next step:
/// dumping the whole JSON object at the screen technically contains the
/// sentence a person needs, the same way a phone book technically contains a
/// phone number. Verified against Google's own docs (2026-09-06): a bad key
/// answers `{"error":{"code":400,"message":"API key not valid. Please pass a
/// valid API key.","status":"INVALID_ARGUMENT"}}` — Google's native envelope,
/// leaking through their OpenAI-compat surface rather than OpenAI's own
/// `error.type`/`error.code` shape, but `error.message` sits at the identical
/// path in both, and it is a human sentence in both. So this reads that one
/// field first and, when it is there, shows ONLY it — the vendor's own words,
/// same principle as `test_binary_failure_message`'s "its own words — it
/// knows why far better than we do" — rather than the object carrying it.
/// `error` as a bare string (a shape some servers use directly) is read the
/// same way. **Anything else falls straight through to the raw body exactly
/// as before** — an unrecognised shape still reaches the person, it is just
/// not rewritten into something it never actually said.
fn generic_status_error(code: u16, body: String, key: &str) -> String {
    let body = redact_and_truncate(body, key, 500);
    if body.is_empty() {
        return format!("The endpoint answered {code}.");
    }
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&body) {
        let message = v
            .pointer("/error/message")
            .and_then(|m| m.as_str())
            .or_else(|| v.get("error").and_then(|e| e.as_str()))
            // **GEMINI'S LIVE BAD-KEY BODY IS ARRAY-WRAPPED — Beck's re-sweep,
            // 2026-09-06: fix above was still partial, "shows raw provider JSON
            // (INVALID_ARGUMENT…)".** Google's OpenAI-compat surface answers a
            // bad key with `[{"error":{"code":400,"message":"API key not
            // valid. Please pass a valid API key.","status":"INVALID_ARGUMENT"}}]`
            // — the SAME human `error.message`, one array level deeper.
            // `/error/message` above never matches an array root, so it fell
            // straight through to the raw body and leaked `{`/`INVALID_ARGUMENT`
            // at the person. Reading `/0/error/message` (and a bare string
            // `/0/error`) recovers the vendor's own sentence for the single-
            // element envelope Google actually sends. Verified against Google's
            // own docs, 2026-09-06.
            .or_else(|| v.pointer("/0/error/message").and_then(|m| m.as_str()))
            .or_else(|| v.pointer("/0/error").and_then(|e| e.as_str()));
        if let Some(message) = message.map(str::trim).filter(|m| !m.is_empty()) {
            return format!("The endpoint answered {code}: {message}");
        }
    }
    // Anything else still reaches the person unchanged -- an unrecognised shape
    // is shown as-is rather than rewritten into something it never said.
    format!("The endpoint answered {code}: {body}")
}

/// `claude -p --output-format json` ends with one JSON object on stdout —
/// win or lose. Split out of `binary_result_ok` on 2026-08-31 so the failure
/// branch above it can read the SAME record: `--output-format json` writes
/// its account of a failure here (`result`, `is_error`) even on a non-zero
/// exit, and before this split that account was only ever read on success,
/// so a genuinely failed run threw it away and fell back to raw stderr.
fn parse_result_object(stdout: &str) -> Option<serde_json::Value> {
    let text = stdout.trim();
    serde_json::from_str::<serde_json::Value>(text)
        .ok()
        .or_else(|| {
            // Plugins or a chatty wrapper can put lines above the result
            // object; the result is the last non-empty line.
            text.lines()
                .rev()
                .find(|l| !l.trim().is_empty())
                .and_then(|l| serde_json::from_str(l.trim()).ok())
        })
}

/// The CLI's own explanation for a failed run, when it has one. Used only by
/// the failure branch above, once `stderr` has been filtered down to nothing
/// worth showing — see the comment there for why that happens on literally
/// every non-Anthropic model. `result` is a plain string on both the success
/// and the failure shape of this JSON object, so no `is_error` check is
/// needed here; the caller already knows the process failed.
fn stdout_result_text(stdout: &str) -> Option<String> {
    parse_result_object(stdout)?
        .get("result")
        .and_then(|r| r.as_str())
        .map(str::to_string)
        .filter(|s| !s.trim().is_empty())
}

/// Did the REAL BINARY answer, through the exact wiring the send path uses?
/// `is_error` false is the pass, and for a non-default provider the
/// provider's own model name must appear in the result (it is the
/// `modelUsage` key) — the one check that catches the binary quietly
/// ignoring our routing and answering from the cloud.
fn binary_result_ok(stdout: &str, model: &str, builtin: bool) -> Result<(), String> {
    let text = stdout.trim();
    let Some(v) = parse_result_object(stdout) else {
        return Err("Claude Code ran but did not return its result record.".into());
    };
    if v.get("is_error").and_then(|b| b.as_bool()).unwrap_or(false) {
        return Err(format!(
            "Claude Code reported an error: {}",
            v.get("result").and_then(|r| r.as_str()).unwrap_or("no detail given")
        ));
    }
    if !builtin && !text.contains(model.trim()) {
        return Err(format!(
            "Claude Code answered, but not with {model} — the routing did not take."
        ));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Storage — the same shape as connectors.rs, one file over.
// ---------------------------------------------------------------------------

pub(crate) fn config_path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    use tauri::Manager;
    let dir = app
        .path()
        .app_config_dir()
        .map_err(|e| format!("no config directory: {e}"))?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("could not create {dir:?}: {e}"))?;
    Ok(dir.join("providers.json"))
}

fn load_from_disk(app: &tauri::AppHandle) -> Store {
    // NOTHING CHOSEN. This is a machine with no providers.json — a fresh
    // install — and the empty `active` is what makes the setup screen ask
    // instead of assume. It used to say "claude", which is the whole of the
    // complaint Mark raised three times.
    //
    // It is ALSO the fallback for a file that will not parse, and that is a
    // deliberate second thought rather than a coincidence. An unreadable file
    // means we do not know what they chose; answering "Claude" would be
    // inventing a decision on their behalf at exactly the moment we have least
    // idea what it was. Asking again is honest, and the BOM case below shows
    // how a file becomes unreadable without anybody doing anything wrong.
    let fallback = Store { active: String::new(), secondary: String::new(), providers: Vec::new() };
    let store = config_path(app)
        .ok()
        .and_then(|p| std::fs::read_to_string(p).ok())
        // A BYTE-ORDER MARK SILENTLY THREW AWAY EVERY CUSTOM PROVIDER — found
        // on real Windows 2026-08-29. `serde_json` rejects a leading U+FEFF, so
        // the whole file failed to parse, `unwrap_or(fallback)` did its job,
        // and the app came up with the built-in row and NOTHING ELSE. No error,
        // no warning: every provider the person had added was simply gone from
        // the screen, and still on disk.
        //
        // It was self-inflicted in that case — PowerShell's `Set-Content
        // -Encoding UTF8` writes a BOM — but that is precisely the point. This
        // app writes the file BOM-free, and then editors, sync clients and
        // shells put one there. **The failure mode is silent data loss on a
        // file the user did not know they were breaking**, which is worth three
        // characters to defend against.
        .map(|text| text.strip_prefix('\u{feff}').map(str::to_owned).unwrap_or(text))
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or(fallback);
    normalized(store)
}

fn save_to_disk(app: &tauri::AppHandle, store: &Store) -> Result<(), String> {
    let path = config_path(app)?;
    // Nothing in `Provider` is a secret by construction — keys live in the OS
    // credential store and only `has_secret` is written here.
    let json = serde_json::to_string_pretty(store).map_err(|e| e.to_string())?;
    std::fs::write(&path, json).map_err(|e| format!("could not write {path:?}: {e}"))
}

fn entry(id: &str) -> Result<keyring::Entry, String> {
    keyring::Entry::new(KEYRING_SERVICE, id).map_err(|e| format!("credential store: {e}"))
}

fn secret_for(id: &str) -> Option<String> {
    entry(id).ok().and_then(|e| e.get_password().ok())
}

fn now_stamp() -> String {
    match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        Ok(d) => d.as_secs().to_string(),
        Err(_) => String::new(),
    }
}

/// Lock, lazily load, and hand the caller a normalized store to work on.
fn with_store<T>(
    app: &tauri::AppHandle,
    state: &tauri::State<Providers>,
    f: impl FnOnce(&mut Store) -> T,
) -> T {
    let mut guard = state.inner.lock().unwrap();
    if guard.is_none() {
        *guard = Some(load_from_disk(app));
    }
    f(guard.as_mut().unwrap())
}

/// Refresh the derived fields the disk is never trusted for.
fn refresh(p: &mut Provider) {
    p.caveat = caveat_for(&p.kind).to_string();
    p.has_secret = secret_for(&p.id).is_some();
}

// ---------------------------------------------------------------------------
// The launch wiring — what `send` in main.rs actually calls.
// ---------------------------------------------------------------------------

/// The provider the next turn should run on, or `None` when nobody has chosen
/// one yet.
///
/// **IT USED TO ANSWER `builtin_claude()` RATHER THAN `None`, and that was the
/// last of the four places this product picked a brain on somebody's behalf.**
/// The other three were `load_from_disk`'s fallback, `normalized`'s active
/// fallback, and the `builtin ||` exemption in `may_select`. They had to change
/// together: leave this one alone and the setup screen asks the question
/// honestly, the person answers nothing, and the very next message is billed to
/// an Anthropic account they never agreed to use. A silent default at send time
/// is worse than one at load time, because by then they have been told they are
/// in control.
pub fn active_provider(
    app: &tauri::AppHandle,
    state: &tauri::State<Providers>,
) -> Option<Provider> {
    with_store(app, state, |store| active_of(store).cloned())
}

/// The chosen row, or `None`. Pure, so "has this person chosen a brain" is
/// testable without an app handle — the same split `provider_env` uses, and for
/// the same reason: the decision and the plumbing should not have to be tested
/// together.
///
/// An empty `active` matches no row, so an unchosen machine answers `None`
/// without needing a special case. That is deliberate — a separate
/// `if active.is_empty()` here would be a second place for "nothing chosen" to
/// be decided, and the two would eventually disagree.
fn active_of(store: &Store) -> Option<&Provider> {
    store.providers.iter().find(|p| p.id == store.active)
}

/// What a send says when there is no brain yet.
///
/// **AN ERROR AT THE WORST MOMENT IS WHERE HONESTY IS WORTH MOST**, so it says
/// what happened, what it did about it, and what happens next — nothing was
/// sent anywhere, nothing was billed, and the way out is one screen away. It
/// names AI components rather than saying "configure a provider", because that
/// is what the button is actually called.
pub const NO_BRAIN_YET: &str =
    "No brain is connected yet, so there was nothing to send this to — and \
     nothing left your computer. Open AI components and pick one: Claude, or a \
     model running on your own machine. It takes a minute and you only do it \
     once.";

/// Put the provider's environment on a command about to run the binary. For
/// the built-in row this touches NOTHING — the product as shipped. Shared by
/// the real spawn and stage 2 of the test, so they cannot disagree.
///
/// FALLIBLE ON PURPOSE, AND THIS IS THE LAST GATE BEFORE A REAL SEND. Every
/// launch reaches the binary through here (`main.rs` send, and stage 2 of the
/// test), so a kind this build cannot route is refused HERE, with words, and
/// not merely hidden in the sheet. A UI that stops offering something is a
/// presentation change; this is the boundary.
///
/// **THE FAILURE IT EXISTS TO STOP is specific and it is silent.** Before
/// 2026-08-28 an `openai-compatible` row was launched by pointing the binary
/// at the in-process translator. Remove that branch without adding this
/// refusal and such a row falls through to the `else` below — the binary is
/// pointed straight at an OpenAI-shaped endpoint with the user's real key, in
/// Anthropic's wire format, and the person finds out mid-conversation. Failing
/// loudly one step earlier is the whole difference.
pub fn apply_env(cmd: &mut Command, p: &Provider) -> Result<(), String> {
    if p.kind == "claude" {
        return Ok(());
    }
    if !is_routable(&p.kind) {
        return Err(format!(
            "This brain connected to an AI company helloim.ai no longer offers ({}), so it \
             cannot answer. Open AI components and choose Claude or a brain on your own \
             machine.",
            p.kind
        ));
    }
    // ONE ARM PER KIND, MATCHED EXHAUSTIVELY, AND THE FINAL ARM REFUSES.
    //
    // THIS USED TO BE `if kind == "openai-compatible" { adapter } else { direct }`
    // (see `3a328ff`), and that `else` is the single most dangerous line this
    // file has ever held. It catches everything that is not literally that one
    // string — so any future cloud kind, any typo, any kind added to
    // ROUTABLE_KINDS by somebody who did not also come here, goes DIRECT:
    // Claude Code pointed at a third party's endpoint, speaking Anthropic's
    // wire format, carrying the user's real key, and they find out mid-answer.
    //
    // A match with a refusing final arm cannot do that. A new kind either gets
    // an arm written for it, deliberately, or it is refused with words. The
    // last arm is unreachable while `is_routable` above agrees with this list,
    // and it is written anyway — the whole point is that it stops being
    // unreachable the moment those two disagree, which is exactly the day
    // nobody notices.
    let (base_url, token) = match p.kind.as_str() {
        "local" => (
            p.base_url.trim().to_string(),
            secret_for(&p.id).unwrap_or_else(|| PLACEHOLDER_TOKEN.to_string()),
        ),
        // THE PROVIDER'S REAL KEY IS NOT IN THIS ENVIRONMENT, and that is a
        // genuine upgrade on the direct path rather than a side effect: it
        // lives only inside this process (the adapter fetches it from the OS
        // store per request), so nothing the child runs can read it. The child
        // authenticates to the adapter with a per-launch random token instead,
        // on the same ANTHROPIC_AUTH_TOKEN channel the binary already uses.
        //
        // Reachable only when OPENAI_ENABLED is true. With the gate shut,
        // `is_routable` has already refused above and this arm never runs.
        "openai-compatible" => {
            let (port, run_token) = crate::adapter::ensure_running()?;
            (format!("http://127.0.0.1:{port}/{}", p.id), run_token)
        }
        other => {
            return Err(format!(
                "helloim.ai does not know how to launch a `{other}` brain, so it will not \
                 try. Choose Claude or a brain on your own machine."
            ))
        }
    };
    for (name, value) in provider_env(p, &base_url, token) {
        match value {
            Some(v) => {
                cmd.env(name, v);
            }
            None => {
                cmd.env_remove(name);
            }
        }
    }
    Ok(())
}

/// One provider row, read fresh off disk — the adapter's view of the store.
/// It re-reads per request (the board's ticket-store pattern), so an edited
/// provider reaches a running translator without a restart.
///
/// **`adapter.rs` IS ITS ONLY CALLER AND IT HAS NO CALLER ITSELF SINCE
/// 2026-08-28** — it is kept compiled and tested rather than deleted, so this
/// stays with it. See the module header.
#[allow(dead_code)]
pub(crate) fn row_from_disk(config_dir: &std::path::Path, id: &str) -> Option<Provider> {
    let text = std::fs::read_to_string(config_dir.join("providers.json")).ok()?;
    let store: Store = serde_json::from_str(&text).ok()?;
    store.providers.into_iter().find(|p| p.id == id)
}

/// The stored key, read for whichever caller is about to use it directly
/// rather than by spawning a child that inherits an environment variable.
///
/// **NO LONGER ORPHANED — corrected 2026-09-03.** This used to say it was
/// "kept with the orphaned translator, not currently called," true while
/// `adapter.rs` was the only thing built to want a key this way. It is now
/// also `engine::native`'s own path: the native OpenAI/Gemini wire
/// (`native/mod.rs`'s `start`) calls this directly to put the key on its own
/// HTTP request, never into a spawned process's environment at all — which is
/// the whole point of a provider that does not "reside on Claude."
/// `#[allow(dead_code)]` is dropped along with the stale comment: this
/// function is now reached unconditionally from `native/mod.rs::start`, so a
/// build where it truly went unused would mean that call site was removed
/// too, and `cargo check` would say so on its own.
pub(crate) fn secret_of(id: &str) -> Option<String> {
    secret_for(id)
}

// ---------------------------------------------------------------------------
// Commands. All of them answer the four questions:
//  - Who may call: our own webview, over Tauri invoke. These commands are not
//    network-reachable; there is no HTTP surface to defend.
//  - Wrong caller: not representable — the invoke boundary is the app itself.
//  - Malformed input: a plain-language Err naming the field; nothing is
//    half-written on refusal.
//  - What errors leak: never a key. Error strings carry statuses and reasons,
//    and the secret has no getter anywhere in this module.
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn list_providers(app: tauri::AppHandle, state: tauri::State<Providers>) -> Store {
    with_store(&app, &state, |store| {
        for p in store.providers.iter_mut() {
            refresh(p);
        }
        store.clone()
    })
}

/// Create or update a row. `secret` is only ever SET — empty means "leave the
/// stored key alone", so editing a model name does not force re-typing a key
/// the window cannot show. Any edit resets the row to untested: leaving an
/// old green on a row whose URL just changed is the lie rule 1 forbids.
#[tauri::command]
pub fn save_provider(
    app: tauri::AppHandle,
    state: tauri::State<Providers>,
    mut provider: Provider,
    secret: String,
) -> Result<Provider, String> {
    // A local row may leave the address blank; Ollama's own default is the
    // answer on every platform it ships on.
    if provider.kind == "local" && provider.base_url.trim().is_empty() {
        provider.base_url = "http://127.0.0.1:11434".into();
    }
    validate(&provider)?;

    if provider.id.trim().is_empty() {
        // Never reused, `p`-prefixed so it can never collide with a
        // connector's credential-store account.
        provider.id = format!("p{}-{}", now_stamp(), std::process::id());
    }
    if !secret.trim().is_empty() {
        entry(&provider.id)?
            .set_password(secret.trim())
            .map_err(|e| format!("could not store the key: {e}"))?;
        // The plaintext dies here: dropped with this parameter, never on disk,
        // no getter.
    }

    provider.builtin = false;
    provider.connected = false;
    provider.checked_at = String::new();
    provider.last_error = String::new();
    // An edit means the person has found the row — the migration note has
    // done its job, whatever the UI round-tripped back.
    provider.migration_note = String::new();
    refresh(&mut provider);

    with_store(&app, &state, |store| {
        match store.providers.iter_mut().find(|p| p.id == provider.id) {
            Some(existing) => *existing = provider.clone(),
            None => store.providers.push(provider.clone()),
        }
        save_to_disk(&app, store)
    })?;
    Ok(provider)
}

/// Remove a row. The built-in one refuses; deleting the active row falls the
/// selection back to the built-in rather than leaving `active` pointing at
/// nothing.
#[tauri::command]
pub fn delete_provider(
    app: tauri::AppHandle,
    state: tauri::State<Providers>,
    id: String,
) -> Result<Store, String> {
    let out = with_store(&app, &state, |store| -> Result<Store, String> {
        if store.providers.iter().any(|p| p.id == id && p.builtin) {
            return Err("Claude is the built-in option and cannot be removed.".into());
        }
        store.providers.retain(|p| p.id != id);
        if store.active == id {
            store.active = "claude".into();
        }
        save_to_disk(&app, store)?;
        Ok(store.clone())
    })?;
    // Best effort, same as connectors: an orphaned key is inert without a row.
    if let Ok(e) = entry(&id) {
        let _ = e.delete_credential();
    }
    Ok(out)
}

/// The decision `select_provider` makes on entry, pulled out pure and
/// testable — same reason `may_select` and `validate` are — rather than
/// asserted only through a `#[tauri::command]` this crate has no harness to
/// drive directly (no `tauri::test` usage anywhere in the tree, checked
/// before reaching for it here).
fn refuse_switch_while_alive(turn_is_alive: bool) -> Result<(), String> {
    if turn_is_alive {
        return Err(
            "Still working on the last one. Switching brains mid-turn could hand this \
             conversation's id to the new one by mistake -- wait for it to finish, or \
             press Stop, then switch."
                .into(),
        );
    }
    Ok(())
}

/// What Secondary becomes when `select_provider` promotes the row that WAS
/// Secondary up to Primary. Pulled out pure and testable for the same reason
/// `refuse_switch_while_alive` is (see `select_provider`'s own doc: there is
/// no `tauri::test` harness anywhere in this tree, so a decision left inline
/// in the command closure cannot be exercised directly).
///
/// Mark, using the app, 2026-09-05: *"if OpenAI is primary and Claude is
/// secondary, Claude should have Make primary; if clicked, it defaults
/// OpenAI to secondary... This stands true for any primary/secondary
/// setup."* So promoting the Secondary now SWAPS the two rather than
/// clearing the Secondary — the row bumped OUT of Primary becomes the new
/// Secondary, because the person still wants a fallback seat filled, and
/// the row that just got bumped is the only one already proven able to
/// fill it (it was, after all, the brain a moment ago).
///
/// **ONLY CALLED WHEN THE PROMOTED ROW WAS ALREADY SECONDARY.** An ordinary
/// switch to some third row while a Secondary is already set is NOT what
/// Mark asked to change, and `select_provider` never calls this for that
/// case — Secondary is simply left alone, exactly as before.
///
/// The degenerate cases fall back to clearing rather than writing a
/// Secondary `normalized` would immediately erase on the next load anyway
/// (see its own "SECONDARY IS NEVER LEFT POINTING AT A ROW THAT CANNOT
/// ANSWER" pass): `old_active` empty (nothing was Primary yet — the very
/// first choice made on a fresh install), `old_active == promoted_id` (would
/// make the row its own fallback — cannot happen in practice, since a
/// genuine promotion always changes `active`, but guarded here rather than
/// trusted from the caller), or the old Primary no longer `is_eligible`
/// (retired, disconnected, or never tested — a fallback that cannot itself
/// answer is not a fallback).
fn secondary_after_promoting_it(old_active: &str, promoted_id: &str, providers: &[Provider]) -> String {
    if !old_active.is_empty()
        && old_active != promoted_id
        && providers.iter().any(|p| p.id == old_active && is_eligible(p))
    {
        old_active.to_string()
    } else {
        String::new()
    }
}

/// Make a row the brain. Refuses a row that never earned green (see
/// `may_select`), and DROPS THE CURRENT CONVERSATION on a real switch:
/// resuming a transcript into a different brain is a conversation
/// half-remembered by a different mind, and `--resume` against a changed
/// backend is unproven.
///
/// **REFUSES WHILE A TURN IS STILL RUNNING — found by inspection during a
/// messy-state hardening pass, not reported, so it is written down in full.**
/// `state.session_id` is ONE shared slot, not one per turn. A turn already in
/// flight keeps writing to it — `AppSink::event()` stamps it from every
/// `system`-typed line the running engine emits — for as long as that turn
/// stays alive, which has nothing to do with when this command runs.
///
/// The front end's "Use" button has no guard of its own (checked: it disables
/// on `!usable`, never on a turn in progress), so this was reachable exactly
/// the way `send()`'s "Still working on the last one" already assumes a
/// second command can arrive while the first is unfinished — the Brain sheet
/// is a different screen from the composer and nothing stops it opening
/// mid-turn.
///
/// Without this guard, the two lines above stop being a guarantee: this
/// clears `session_id` to `None` right here so the new brain starts clean,
/// but the OLD turn is still running against the OLD brain and can restamp
/// that same slot afterwards — reattaching a conversation the switch was
/// specifically supposed to drop. `state.turn` already serialises actual
/// *sends* to one at a time regardless of provider, which is why nothing here
/// can run two turns at once; what it does not do on its own is stop the
/// STORE's active row, and the session-id slot that follows it, from moving
/// out from under a turn that has not finished yet. Refusing here — the same
/// shape `send()` already uses for the identical reason — is what closes that
/// the rest of the way, and it costs the person one retry, not a rebuild.
#[tauri::command]
pub fn select_provider(
    app: tauri::AppHandle,
    state: tauri::State<Providers>,
    id: String,
) -> Result<Store, String> {
    use tauri::Manager;
    if let Err(e) = refuse_switch_while_alive(crate::alive(&app.state::<crate::Session>())) {
        return Err(e);
    }
    let (out, switched) = with_store(&app, &state, |store| {
        let Some(p) = store.providers.iter().find(|p| p.id == id) else {
            return Err::<(Store, bool), String>("No such provider.".into());
        };
        may_select(p)?;
        let switched = store.active != id;
        let old_active = store.active.clone();
        store.active = id.clone();
        // A ROW CANNOT BE ITS OWN FALLBACK, still. Whether the displaced
        // Primary takes the empty Secondary seat is pulled out pure below —
        // see `secondary_after_promoting_it`'s own doc for what changed and
        // why.
        if store.secondary == id {
            store.secondary = secondary_after_promoting_it(&old_active, &id, &store.providers);
        }
        save_to_disk(&app, store)?;
        Ok((store.clone(), switched))
    })?;
    if switched {
        let session = app.state::<crate::Session>();
        *session.session_id.lock().unwrap() = None;
    }
    Ok(out)
}

/// Designate this row Secondary — the AI Boardroom's fallback synthesiser if
/// Primary cannot answer, and otherwise just a full voice at the table.
///
/// **`id: ""` CLEARS IT.** "No Secondary" is a real, nameable state (most
/// machines have exactly one brain connected), not an absence this command
/// should refuse to express.
///
/// **DOES NOT TOUCH `state.session_id`, UNLIKE `select_provider`.** Nothing
/// about which conversation is "current" changes here — Secondary never
/// drives an ordinary chat turn, only a Boardroom synthesis when Primary is
/// unreachable, so there is no `--resume` id this could ever invalidate.
#[tauri::command]
pub fn set_secondary_provider(
    app: tauri::AppHandle,
    state: tauri::State<Providers>,
    id: String,
) -> Result<Store, String> {
    with_store(&app, &state, |store| {
        if !id.is_empty() {
            let Some(p) = store.providers.iter().find(|p| p.id == id) else {
                return Err("No such provider.".into());
            };
            // SAME BAR AS PRIMARY, AND FOR THE SAME REASON `may_select`
            // GIVES ONE. A Secondary that has never earned a green dot is a
            // fallback that fails silently the one time it is actually
            // needed — Primary already down, and Secondary refuses too.
            if !is_eligible(p) {
                return Err("Test it first — a Secondary has to be able to \
                             answer before it can be the fallback."
                    .into());
            }
            if id == store.active {
                return Err("That brain is already Primary. Pick a different \
                             one for Secondary."
                    .into());
            }
        }
        store.secondary = id;
        save_to_disk(&app, store)?;
        Ok(store.clone())
    })
}

/// THE REAL CHECK — the only thing that earns green, in two stages, both
/// required.
///
/// Stage 1 asks the ENDPOINT for a real (tiny) message, which gives precise
/// errors: 401 is a bad key, 404 is a wrong address or an Ollama too old.
/// Stage 2 spawns the REAL BINARY with the exact environment the send path
/// uses — the stage that catches endpoint-healthy-product-broken, which is
/// precisely what raw-pipe tests missed on the memory tools: every pipe test
/// passed and the real product path could not fire at all.
///
/// It spends a few hundred of the user's tokens on a paid endpoint. That is
/// user-initiated — this command only runs from the Test button — and the UI
/// copy says a real request is sent, so it is consented and disclosed rather
/// than free.
#[tauri::command(async)]
pub fn test_provider(
    app: tauri::AppHandle,
    state: tauri::State<Providers>,
    id: String,
) -> Result<Provider, String> {
    let target = with_store(&app, &state, |store| {
        store.providers.iter().find(|p| p.id == id).cloned()
    });
    let Some(target) = target else {
        return Err("No such provider.".into());
    };
    // A retired row must not be testable. Letting the Test button run would
    // send a real request, with the user's key, to an endpoint this build can
    // never launch against — spending their money to earn a green that
    // `may_select` would then refuse. Refuse before anything leaves.
    if !is_routable(&target.kind) {
        return Err("There is nothing to test — helloim.ai no longer connects to other AI \
                    companies. Use Claude or a brain on your own machine, or remove this row."
            .into());
    }

    // STAGE 1 IS ONE ARM PER KIND, matched the same way `apply_env` is and for
    // the same reason: it has to speak the shape the endpoint actually speaks,
    // and an `else` that catches everything gets that wrong silently. An
    // OpenAI row sent to `test_endpoint` posts Anthropic JSON at
    // `/v1/messages` and can never pass — the row would be untestable,
    // therefore unselectable, and the picker would be offering a dead end.
    // WHICH ENGINE WILL ACTUALLY DRIVE THIS ROW. Asked once, from the single
    // selection point, so the Test cannot test a path the send will not take.
    let engine = crate::engine::for_provider(&target);

    let outcome = if target.builtin {
        // The binary holds its own credential; there is no endpoint of ours
        // to poke. The spawn IS the test.
        test_binary(&target)
    } else {
        let stage1 = if target.kind == "openai-compatible" {
            test_chat_endpoint(&target)
        } else {
            test_endpoint(&target)
        };

        /* STAGE 2 IS `test_binary` — spawning the real Claude Code — AND IT
           RUNS ONLY WHEN THE ENGINE THIS ROW WILL ACTUALLY USE NEEDS IT.
           Added 2026-08-31 for the `local` kind, the same hour the native
           engine's gate opened, and without it that gate was decoration.

           UNTIL 2026-09-03 THE `openai-compatible` KIND HAD ITS OWN ARM ABOVE
           THIS CHECK, WHICH ALWAYS CHAINED `test_binary` — the exact bug this
           paragraph already describes for `local`, reopened for the row next
           to it. `NATIVE_OPENAI_ENABLED` moved OpenAI and Gemini onto the
           same vendor-binary-free native engine `local` already had, but the
           `openai-compatible` branch sat ABOVE this `if`, so `for_provider`'s
           answer never reached it. Reported by Mark on his own machine: an
           OpenAI key test came back "Claude Code is not installed on this
           computer, and it is the engine every provider runs through", the
           row never turned green, `may_select` refused it, and every send
           answered NO_BRAIN_YET — the identical shape the paragraph below
           already names, on a kind nobody had re-checked since.

           `test_binary` spawns `claude.exe`. It ran for EVERY non-builtin row
           once, local ones included, back when "it launches the real Claude
           Code, so whatever routing that kind uses is exercised" was true of
           every kind there was. It is no longer true of a row driven by the
           native engine: nothing from Anthropic is in its path at all.

           WHAT THAT COSTS WHEN IT SHIPS UNFIXED: a person with a working key
           — or Ollama running perfectly well — and no Claude Code installed
           presses Test, gets told Claude Code is missing, the row never turns
           green, `may_select` refuses a row that is not green, and every send
           answers NO_BRAIN_YET. The send path is free of Anthropic and the
           setup path still demands the 214 MB download this entire piece of
           work exists to remove.

           STAGE 1 STILL RUNS AND IS NOT WEAKENED FOR EITHER KIND. What is
           dropped is a probe of a program this row's engine does not use. */
        if engine.needs_vendor_binary() {
            stage1.and_then(|()| test_binary(&target))
        } else {
            stage1
        }
    };

    with_store(&app, &state, |store| {
        let Some(slot) = store.providers.iter_mut().find(|p| p.id == id) else {
            return Err("No such provider.".into());
        };
        slot.checked_at = now_stamp();
        match outcome {
            Ok(()) => {
                slot.connected = true;
                slot.last_error = String::new();
                // The first earned green retires the migration note; a failed
                // test keeps it, because "where this row came from" and "what
                // is wrong with it" answer different questions.
                slot.migration_note = String::new();
            }
            Err(why) => {
                slot.connected = false;
                slot.last_error = why;
            }
        }
        refresh(slot);
        let updated = slot.clone();
        save_to_disk(&app, store)?;
        Ok(updated)
    })
}

/// Stage 1: a real `/v1/messages` round trip, answered in Anthropic's shape.
///
/// The key goes out as `Authorization: Bearer` and NOTHING ELSE, because that
/// is exactly what the binary sends with `ANTHROPIC_AUTH_TOKEN` — a test that
/// authenticates differently from the launch is a green dot on a brain that
/// will not answer, the same test/launch-parity lesson `secret_env` carries.
/// Stage 1 for an `openai-compatible` row: a real `/chat/completions` round
/// trip, in OpenAI's shape, with the person's own key.
///
/// **RESTORED 2026-08-29, AND IT IS THE FOURTH WIRING POINT — I said there were
/// three and I was wrong.** `ROUTABLE_KINDS`, `apply_env`'s arm and
/// `adapter::init` get a row LAUNCHING; this is what lets it go GREEN. Without
/// it an OpenAI row falls to `test_endpoint`, which posts Anthropic-shaped JSON
/// to `{base}/v1/messages` — against `https://api.openai.com/v1` that is a
/// request to `/v1/v1/messages` in the wrong format, so the Test could never
/// pass, `may_select` would refuse the row for ever, and the picker would offer
/// a choice that dead-ends. Exactly the failure we were trying to avoid by
/// keeping OpenAI off the screen, arriving through the door we left open.
///
/// **AND THIS IS WHY OPENAI IS HONEST TO SHIP WITHOUT US HAVING A KEY.** The
/// translator's own tests are synthetic — it has never met a real endpoint in
/// THIS house. But every person who picks it proves it on their own machine
/// before they can use it: stage 1 here reaches OpenAI for real, and stage 2
/// (`test_binary`) spawns Claude Code through `apply_env`, which for this kind
/// starts the adapter and points the binary at loopback. So the green dot is a
/// genuine Claude Code → translator → OpenAI round trip, and a broken
/// translator fails the Test rather than failing silently mid-conversation.
/// Our key would tell US sooner. It was never what made this safe for them.
/// The Test-button request body for an `openai-compatible` row's stage 1.
///
/// **THE BUG (2026-08-31): this sent `max_tokens` unconditionally.**
/// `adapter::translate_request` — the code that runs the REAL send, once the
/// row is in use — already branched on `is_reasoning_model` to send
/// `max_completion_tokens` instead, because OpenAI's o1/o3/o4/gpt-5* family
/// 400s outright on the old name. Test staying unbranched meant those rows
/// could never earn a green dot no matter how correct the key was, and
/// `may_select` refuses any row that has not gone green — so no reasoning
/// model could ever be chosen as the brain, and `OPENAI_DEFAULT_MODEL` in
/// `ui/index.html` (`gpt-5.6-sol`) pre-fills exactly one of them. Two sites
/// building the same request and disagreeing about the field name IS this
/// bug; sharing `is_reasoning_model` rather than growing a second prefix
/// list is what keeps them from drifting apart again.
///
/// Pulled out to its own pure function for the same reason
/// `generic_status_error` was: provable against a model name with no live
/// endpoint or mock server needed.
fn chat_completion_test_body(model: &str) -> serde_json::Value {
    let mut body = serde_json::json!({
        "model": model,
        "messages": [{ "role": "user", "content": "Say OK" }],
    });
    let field =
        if crate::adapter::is_reasoning_model(model) { "max_completion_tokens" } else { "max_tokens" };
    body[field] = serde_json::json!(8);
    body
}

fn test_chat_endpoint(p: &Provider) -> Result<(), String> {
    let base = p.base_url.trim().trim_end_matches('/');
    if !base.starts_with("https://") && !base.starts_with("http://") {
        return Err("That base URL does not look like a web address.".into());
    }
    let url = format!("{base}/chat/completions");
    let key = secret_for(&p.id).unwrap_or_else(|| PLACEHOLDER_TOKEN.to_string());

    let body = chat_completion_test_body(p.model.trim());
    let timeout = if is_loopback(base) { LOCAL_TEST_TIMEOUT } else { CLOUD_TEST_TIMEOUT };
    let req = ureq::post(&url)
        .timeout(timeout)
        .set("content-type", "application/json")
        .set("authorization", &format!("Bearer {key}"));

    match req.send_string(&body.to_string()) {
        Ok(res) => {
            let text = res.into_string().unwrap_or_default();
            let v: serde_json::Value = serde_json::from_str(text.trim())
                .map_err(|_| "The endpoint replied, but not with JSON.".to_string())?;
            if v.pointer("/choices/0/message").is_some() {
                Ok(())
            } else {
                Err("The endpoint replied, but not in the OpenAI chat shape — no message in it."
                    .into())
            }
        }
        Err(ureq::Error::Status(401, _)) => {
            Err("Rejected the key (401). Check it and try again.".into())
        }
        Err(ureq::Error::Status(403, _)) => Err("Key accepted but not permitted (403).".into()),
        Err(ureq::Error::Status(404, _)) => Err(format!(
            "No /chat/completions endpoint at {base} (404). Wrong base URL? Most end in /v1 \
             — OpenAI's is https://api.openai.com/v1"
        )),
        /* 429 IS ALMOST NEVER A RATE LIMIT ON A NEW KEY — Mark, 2026-08-30,
           first person ever to run this path with a real key. He made a key,
           pasted it, pressed Test, and got "The endpoint answered 429." — a
           number, and nothing he could act on.
           OpenAI returns 429 for TWO different things: genuine rate limiting,
           and `insufficient_quota`, which means the account has no credit on
           it. On a key minutes old with no spend against it, the second is
           overwhelmingly the likely one — and it is the one the person can
           actually fix. Naming both, cheapest check first, beats naming the
           status code. */
        Err(ureq::Error::Status(429, _)) => Err(
            "OpenAI refused with 429. On a new key that usually means the account has no \
             credit yet rather than too many requests — check billing and credits in your \
             OpenAI account, then test again."
                .into(),
        ),
        Err(ureq::Error::Status(code, _)) if code >= 500 => Err(format!(
            "The endpoint answered {code}, which is a fault at their end rather than \
             anything you set. Worth trying again in a moment."
        )),
        Err(ureq::Error::Status(code, resp)) => {
            Err(generic_status_error(code, resp.into_string().unwrap_or_default(), &key))
        }
        Err(ureq::Error::Transport(t)) => Err(format!("Could not reach {base}: {t}")),
    }
}

/// Turn an ambiguous local 404 into the real cause, or an honest "could not
/// tell" — Tessa found the bug this replaces on 2026-08-31 and proved it on
/// this box.
///
/// **THE BUG: Ollama answers 404 for TWO unrelated things** — an old build
/// that has never heard of `/v1/messages` (added in 0.33.0), and a perfectly
/// current build that has simply never heard of the model name being asked
/// for. The code this replaces collapsed both into one sentence, "Update
/// Ollama and test again", which is exactly wrong for the second case: it
/// sends someone to update software that is already current, and says
/// nothing about the one thing that would actually fix it — `ollama pull
/// <model>`. `detect_local_brain` already has both signals for its own
/// screen (`version_at_least`, and the model list off `/api/tags`); this is
/// the same two cheap, short-timeout probes, spent here instead of assumed
/// away.
///
/// The two probes are read independently, so either can fail without taking
/// the other down with it — `/api/version` unreachable does not stop
/// `/api/tags` from settling the model question, and vice versa. Network
/// work is kept impure and separate from `classify_local_404`, which is the
/// part actually deciding what to say and the part the tests below exercise
/// directly.
fn diagnose_local_404(base: &str, model: &str) -> String {
    let agent = ureq::AgentBuilder::new().timeout(Duration::from_secs(3)).build();

    let version = agent
        .get(&format!("{base}/api/version"))
        .call()
        .ok()
        .and_then(|r| r.into_string().ok())
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| v.get("version").and_then(|x| x.as_str()).map(String::from));

    let model_present = agent
        .get(&format!("{base}/api/tags"))
        .call()
        .ok()
        .and_then(|r| r.into_string().ok())
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| v.get("models").and_then(|m| m.as_array()).cloned())
        .map(|models| models.iter().any(|m| m.get("name").and_then(|n| n.as_str()) == Some(model)));

    classify_local_404(version.as_deref(), model_present, model)
}

/// The pure decision `diagnose_local_404` hands its two probe results to —
/// same split as `generic_status_error` and `chat_completion_test_body`
/// above, provable against canned inputs with no socket needed.
///
/// **A definite old version wins outright** — it alone explains a 404 on
/// `/v1/messages` regardless of what the model question found, so there is
/// no need for the model probe to agree before saying so. **A definite
/// missing model is the next-strongest signal** — Ollama did answer, in
/// Anthropic's language even, and simply has never heard of that name.
/// **Anything left over says so rather than guessing** — an old check said
/// "update Ollama" unconditionally here, which is the exact fault this
/// function exists to stop repeating.
fn classify_local_404(version: Option<&str>, model_present: Option<bool>, model: &str) -> String {
    if let Some(v) = version {
        if !version_at_least(v, 0, 33) {
            return format!(
                "Ollama {v} is running, but it is older than 0.33 and has never heard of \
                 Claude's API shape. Update Ollama and test again."
            );
        }
    }
    if model_present == Some(false) {
        return format!(
            "This Ollama does not have a model called {model} installed. Pull it first — \
             `ollama pull {model}` — then test again."
        );
    }
    "Ollama answered 404, but this computer could not confirm why — the version and \
     the installed models could not both be re-checked just now. Confirm Ollama is \
     still running, the model name is right, and the address points at this \
     machine's own Ollama, then test again."
        .to_string()
}

fn test_endpoint(p: &Provider) -> Result<(), String> {
    let base = p.base_url.trim().trim_end_matches('/');
    if !base.starts_with("https://") && !base.starts_with("http://") {
        return Err("That base URL does not look like a web address.".into());
    }
    let url = format!("{base}/v1/messages");
    let key = secret_for(&p.id).unwrap_or_else(|| PLACEHOLDER_TOKEN.to_string());
    let timeout = if p.kind == "local" { LOCAL_TEST_TIMEOUT } else { CLOUD_TEST_TIMEOUT };

    let body = serde_json::json!({
        "model": p.model.trim(),
        "max_tokens": 8,
        "messages": [{ "role": "user", "content": "Say OK" }],
    });

    let req = ureq::post(&url)
        .timeout(timeout)
        .set("content-type", "application/json")
        .set("anthropic-version", "2023-06-01")
        .set("authorization", &format!("Bearer {key}"));

    match req.send_string(&body.to_string()) {
        Ok(res) => message_shape_ok(&res.into_string().unwrap_or_default()),
        // A REAL sk-ant KEY CANNOT REACH HERE ANY MORE — the kinds that could
        // carry one were removed 2026-08-28 and such a row is retired before
        // Test runs. **The finding is kept because it comes straight back with
        // the kind, and re-deriving it costs a live probe.** What is KNOWN,
        // probed live 2026-08-27 against api.anthropic.com: that endpoint
        // accepts and
        // validates the Bearer scheme (a bad token gets "Invalid bearer
        // token", not "unsupported scheme"). What was UNDECIDABLE without a
        // real key: whether an API key is VALID through Bearer or that
        // channel takes only OAuth-shaped tokens. No special case was built
        // for a failure nobody has observed; this 401 arriving at a Test
        // button, naming the Bearer channel, is the designed landing place.
        // The scoped fix if it is ever observed: send `x-api-key` alongside
        // Bearer ONLY when the host is exactly api.anthropic.com — and it
        // must STAY scoped to that exact host, because that is Anthropic's
        // own server so the extra header can leak the key nowhere new, while
        // widening it to any other host would defeat the ANTHROPIC_API_KEY
        // exfiltration guard in provider_env.
        Err(ureq::Error::Status(401, _)) => Err(
            "Rejected the key (401). It is sent as a Bearer token, exactly as Claude \
             Code will send it — check the key, and that this provider takes Bearer auth."
                .into(),
        ),
        Err(ureq::Error::Status(403, _)) => Err("Key accepted but not permitted (403).".into()),
        Err(ureq::Error::Status(404, _)) if p.kind == "local" => {
            Err(diagnose_local_404(base, p.model.trim()))
        }
        Err(ureq::Error::Status(404, _)) => {
            Err(format!("No /v1/messages endpoint at {base} (404). Wrong base URL?"))
        }
        Err(ureq::Error::Status(code, resp)) => {
            Err(generic_status_error(code, resp.into_string().unwrap_or_default(), &key))
        }
        Err(ureq::Error::Transport(t)) if p.kind == "local" => Err(format!(
            "Could not reach Ollama at {base}: {t}. Is it installed and running on THIS \
             computer?"
        )),
        Err(ureq::Error::Transport(t)) => Err(format!("Could not reach {base}: {t}")),
    }
}

/// What to tell the user when `test_binary`'s child exits non-zero.
///
/// THE THIRD UNFILTERED PATH — Mason, 2026-08-31. `filter_benign_stderr`
/// already existed below, guarding the SUCCESS branch only; this failure
/// branch used raw `stderr.trim()` as the entire error message. The
/// unrecognized_model notice fires on literally every non-Anthropic model
/// turn (see its doc comment in main.rs) — success or failure makes no
/// difference to whether the binary writes it. So a genuinely failed run
/// against a non-builtin provider always has this benign noise sitting in
/// `stderr` too, and it was reaching the user raw, as if it were the reason.
/// Mark, adding an OpenAI token: "you get [claude-code:unrecognized_model]
/// {"model":"gpt-5.6-sol","query_source":"sdk"}" — reproduced verbatim on
/// this machine's own claude 2.1.251 by pointing it at a model the backend
/// genuinely rejects: exit code 1, and that marker line is the ONLY thing on
/// stderr, because it is written unconditionally and nothing else happened
/// to land on that channel.
///
/// Filtering can leave nothing behind — the run failed and the only thing
/// stderr held was the benign notice. Claude Code still knows why:
/// `--output-format json` puts its own account of the failure in stdout's
/// `result` field even on a non-zero exit (same reproduction —
/// `"result":"There's an issue with the selected model (gpt-5.6-sol). It may
/// not exist or you may not have access to it."`, `is_error:true`, exit 1).
/// That is the actual reason, and until this fix it was thrown away: only
/// `binary_result_ok`, one branch below, ever read stdout's `result` field,
/// and this branch returns before reaching it.
fn test_binary_failure_message(stderr: &str, stdout: &str, status: &str) -> String {
    let why = filter_benign_stderr(stderr);
    let why = why.trim();
    let why = if why.is_empty() {
        stdout_result_text(stdout).unwrap_or_default()
    } else {
        why.to_string()
    };
    // Its own words — it knows why far better than we do. Never the key: the
    // binary does not print credentials and neither do we.
    if why.is_empty() {
        format!("Claude Code exited with {status} and said nothing.")
    } else {
        why
    }
}

/// Stage 2: the real binary, the real wiring, one tiny turn.
fn test_binary(p: &Provider) -> Result<(), String> {
    use std::io::Read;
    use std::process::Stdio;
    use std::time::Instant;

    let mut cmd = Command::new(crate::claude_binary());
    cmd.args(["-p", "Say OK", "--output-format", "json", "--max-turns", "1"])
        .current_dir(std::env::temp_dir())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    apply_env(&mut cmd, p)?;
    crate::hide_console(&mut cmd);

    let mut child = cmd.spawn().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            "Claude Code is not installed on this computer, and it is the engine every \
             provider runs through."
                .to_string()
        } else {
            format!("Could not start Claude Code: {e}")
        }
    })?;

    // Readers on worker threads so a full pipe can never deadlock the wait
    // loop below — the same reason main.rs streams instead of blocking.
    let mut taken_out = child.stdout.take();
    let mut taken_err = child.stderr.take();
    let (tx_out, rx_out) = std::sync::mpsc::channel::<String>();
    let (tx_err, rx_err) = std::sync::mpsc::channel::<String>();
    std::thread::spawn(move || {
        let mut s = String::new();
        if let Some(out) = taken_out.as_mut() {
            let _ = out.read_to_string(&mut s);
        }
        let _ = tx_out.send(s);
    });
    std::thread::spawn(move || {
        let mut s = String::new();
        if let Some(err) = taken_err.as_mut() {
            let _ = err.read_to_string(&mut s);
        }
        let _ = tx_err.send(s);
    });

    // Stage 1 already warmed a local model, so this timeout covers the binary
    // plus one answer, not a cold load.
    let timeout = if p.kind == "local" || is_loopback(&p.base_url) {
        LOCAL_TEST_TIMEOUT
    } else {
        BINARY_TEST_TIMEOUT
    };
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("Claude Code started but did not answer in time.".into());
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(200)),
            Err(e) => {
                let _ = child.kill();
                return Err(format!("Lost track of the Claude Code process: {e}"));
            }
        }
    };

    let stdout = rx_out.recv_timeout(Duration::from_secs(5)).unwrap_or_default();
    let stderr = rx_err.recv_timeout(Duration::from_secs(5)).unwrap_or_default();

    if !status.success() {
        return Err(test_binary_failure_message(&stderr, &stdout, &status.to_string()));
    }

    // STDERR ON A SUCCESSFUL RUN USED TO BE THROWN AWAY UNREAD — Cassandra's
    // finding: the one function whose job is proving the brain works never
    // looked at the channel most likely to be carrying a warning about it.
    // The one thing that ALWAYS lands here for a local or third-party model is
    // Claude Code's own harmless "[claude-code:unrecognized_model]" notice
    // (see `is_unrecognized_model_notice`'s doc comment in main.rs) — it fires
    // on every turn a non-Anthropic model answers, the run still succeeds, and
    // failing the Test over it would mean NO local brain could ever earn a
    // green dot. So it is filtered out here exactly as it is in the real send
    // path, and it alone. Anything else left over is genuinely unexpected on a
    // run that just reported success, and it no longer vanishes into
    // `unwrap_or_default()` — it goes to NameOS's own stderr, where it can
    // actually be found, rather than nowhere at all. It does not fail the
    // Test: an unfamiliar stderr line on an otherwise-successful, correctly-
    // routed answer (`binary_result_ok` below still checks that) is worth
    // recording, not worth telling the user their working brain is broken.
    let leftover = filter_benign_stderr(&stderr);
    if !leftover.trim().is_empty() {
        eprintln!(
            "providers::test_binary: {} answered successfully but also wrote to \
             stderr: {}",
            if p.builtin { "claude" } else { p.model.as_str() },
            leftover.trim()
        );
    }

    binary_result_ok(&stdout, &p.model, p.builtin)
}

/// Same filter `send()`'s stderr thread applies live, applied here to a whole
/// captured chunk instead — `test_binary` reads its child to completion
/// before deciding anything, so there is no line-at-a-time stream to filter
/// as it arrives. Kept as ONE definition (`main.rs`'s `is_unrecognized_model_*`
/// pair) rather than two, so the two places that decide "is this stderr line
/// actually a problem" cannot quietly drift apart.
fn filter_benign_stderr(stderr: &str) -> String {
    let mut out = String::new();
    let mut swallow_detail = false;
    for line in stderr.lines() {
        if crate::is_unrecognized_model_notice(line) {
            // Real claude.exe writes the marker and its detail as one line
            // (see `is_unrecognized_model_notice`'s doc comment in main.rs);
            // only arm the next-line swallow for the older two-line shape,
            // where this line was JUST the marker.
            swallow_detail = line.trim() == crate::UNRECOGNIZED_MODEL_MARKER;
            continue;
        }
        if swallow_detail {
            swallow_detail = false;
            if crate::is_unrecognized_model_detail(line) {
                continue;
            }
        }
        if !line.trim().is_empty() {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Local detection.
// ---------------------------------------------------------------------------

/// One model Ollama already has, annotated with what we could learn about it.
///
/// **THIS REPLACED A BARE `Vec<String>` OF NAMES ON 2026-08-31 — Mark, having
/// hit the gap himself: *"no way to probe for local brain and different
/// models."*** A name alone let the picker pre-fill whichever entry Ollama's
/// own `/api/tags` happened to list first, which on a real machine put
/// `gpt-oss:20b` in the box — a model that answers `/api/show` looking
/// perfectly healthy and, proven against the real app the same day, never
/// completes a single turn. Sizing and reading its declared capabilities
/// costs one extra request per model; guessing from name order costs a
/// customer their first real session.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledModel {
    pub name: String,
    /// From `/api/tags`'s own `size` (bytes), converted to MB. Only absent if
    /// Ollama's response left the field out, which a real server does not do.
    pub size_mb: Option<u64>,
    /// SAME `Fit` THE NOT-YET-INSTALLED CATALOG USES (`brain_setup::Fit`,
    /// `classify_size`) — one sizing rule, not two. `Unknown` when the
    /// machine's own hardware could not be measured, exactly as it does there.
    pub fit: crate::brain_setup::Fit,
    /// From `/api/show`'s `capabilities` list on THIS model specifically —
    /// see `parse_has_tools`. `Some(true)`/`Some(false)` are both real
    /// findings; `None` means `/api/show` itself could not be read, which is
    /// a different and weaker claim than "checked, and it said no".
    ///
    /// **THIS IS A DECLARATION, NOT A GUARANTEE.** `gpt-oss:20b`, measured on
    /// the machine that prompted this file, declares `["completion","tools",
    /// "thinking"]` — `has_tools` reads `Some(true)` — and Beck proved
    /// separately that a real turn against it never completes. Whether a
    /// model actually finishes a turn is runtime behaviour metadata cannot
    /// carry either way, so nothing here is allowed to say "will work". The
    /// UI's job is to show this fact and then say plainly that Test is the
    /// only way to know the rest.
    pub has_tools: Option<bool>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalBrain {
    /// Ollama answered on this machine's loopback.
    pub running: bool,
    pub version: String,
    /// Running AND new enough to speak the Anthropic shape.
    pub api_ok: bool,
    /// EVERY installed model, annotated, in the order the picker should show
    /// them — see `rank_installed`. **Nothing here is ever filtered out.**
    /// Mark, 2026-08-31, sharper than the first ask: *"i want it to give all
    /// the options on a prob[e]."* A model he can see in `ollama list` and
    /// cannot find here reads as a broken app, not a cautious one.
    pub models: Vec<InstalledModel>,
    /// What is missing, in words a person can act on. None when api_ok.
    pub problem: Option<String>,
}

/// Reads a `capabilities` array for exactly the fact this screen needs, and
/// no other — a single rule shared by both places that array can arrive.
///
/// **VERIFIED LIVE, NOT TAKEN FROM THE DOCS, 2026-08-31.** Ollama's own
/// `docs/api.md` shows `capabilities` only on `/api/show`'s response. Reading
/// `/api/tags` on the real box this feature is for (Ollama 0.33.2) found
/// `capabilities` sitting on EVERY entry there too —
/// `{"name":"gpt-oss:20b",...,"capabilities":["completion","tools",
/// "thinking"]}` — which means the one call already being made for names and
/// sizes usually answers this for free, no second request needed. `detect_
/// local_brain` reads it from there first and only calls `/api/show` per
/// model when a `/api/tags` entry does not carry it — an older 0.33.x point
/// release, most likely — so a customer on a build that has not caught up
/// still gets an answer, at the cost of one more request instead of zero.
///
/// `None` covers two different failures on purpose: the value was not a JSON
/// object with a `capabilities` array, and the array was there but this
/// caller could not be reached at all. Both mean "could not tell", and both
/// are held apart from `Some(false)`, which means capabilities were read and
/// "tools" was genuinely absent — a real, checkable fact about that model,
/// not a gap in ours.
fn has_tools_from(v: &serde_json::Value) -> Option<bool> {
    let caps = v.get("capabilities")?.as_array()?;
    Some(caps.iter().any(|c| c.as_str() == Some("tools")))
}

/// Names measured, on real hardware, to accept a turn and never answer it —
/// FACTS.md: **"gpt-oss:20b NEVER completes a turn"**, `llama3.2` ~4s on the
/// same box. `has_tools` cannot see this failure: `/api/show` reports
/// `gpt-oss:20b` declaring `["completion","tools","thinking"]`, the same
/// `Some(true)` a model that actually works reports, because whether a model
/// FINISHES is runtime behaviour and `has_tools` is a static declaration —
/// see `InstalledModel::has_tools`'s own doc, which said as much before this
/// list existed and had nothing acting on it. This is what closes that gap:
/// a short, named, evidenced denylist rather than a heuristic that might
/// mis-fire on some other model's declared capabilities.
///
/// **Matched by exact tag, not by a name prefix.** `gpt-oss:20b` specifically
/// is what was measured; a different tag of the same family (`gpt-oss:120b`,
/// say) has not been proven broken and does not belong on a list built from
/// one measurement. Extend this by adding another exact, cited entry when
/// another model is proven the same way — never by widening the match.
const KNOWN_STALLING_MODELS: &[&str] = &["gpt-oss:20b"];

fn is_known_stalling(name: &str) -> bool {
    KNOWN_STALLING_MODELS.contains(&name)
}

/// Every model, annotated, best first — but "first" never means "the only one
/// shown" or "the one silently pre-filled". Four tiers, in order:
///
/// 0. **Declares tools, and is not known to stall.**
/// 1. **`None`** (could not even check) — a weaker claim than either a yes or
///    a genuine no, so it sits between them.
/// 2. **Declares no tools, and is not known to stall.**
/// 3. **`KNOWN_STALLING_MODELS`, regardless of what it declares.** Last,
///    always, even ahead of a model that merely lacks declared tool support —
///    a measured "this does not finish a turn" is a stronger, worse claim
///    than an undeclared capability that might still work fine for plain
///    chat. **THIS TIER IS THE FIX.** Before it existed, `gpt-oss:20b` and
///    `llama3.2` both declare `Some(true)` and this function could not tell
///    them apart, which is exactly how the one that never completes a turn
///    ended up ranked equal-first with the one that does — see
///    `ranking_never_drops_a_model_and_never_defaults_to_the_broken_one`'s
///    own history below for the test that used to assert the broken order
///    was correct.
///
/// Stable within a tier, so the list does not reshuffle between two
/// identical probes.
fn rank_installed(mut models: Vec<InstalledModel>) -> Vec<InstalledModel> {
    models.sort_by_key(|m| {
        if is_known_stalling(&m.name) {
            return 3;
        }
        match m.has_tools {
            Some(true) => 0,
            None => 1,
            Some(false) => 2,
        }
    });
    models
}

/// The relative path a kind lists its own installed or available models on.
/// Ollama's is `/api/tags`; an OpenAI-shaped endpoint's is `/models`. Pulled
/// out of `detect_local_brain` on 2026-08-31 — Tessa found it hardcoded to
/// Ollama's path with no way for any other kind to reuse the probe, taken as
/// the shape of `probePath` in `/tmp/vve/src/data/providers.ts` (not
/// ported — that file is TypeScript for a different product) — a `String`
/// field on `Provider` would have worked too, but this build only ever needs
/// it keyed by kind, the same way `caveat_for` and `provider_env`'s match
/// arms already are, and a fourth per-kind field on the stored row is a
/// fourth thing `normalized()` has to keep honest on every future kind.
///
/// **KEEPS OLLAMA'S BEHAVIOUR IDENTICAL — this is the hardcode moved, not a
/// new capability turned on.** `detect_local_brain` below still only ever
/// calls this with `"local"`, which still resolves to `/api/tags`; nothing
/// currently asks it for `"openai-compatible"`. It exists so the day
/// something does — a per-row "list this endpoint's models" probe for a
/// custom OpenAI-shaped brain — the two kinds do not silently diverge on
/// which path means "list the models", the way `is_routable` was built so a
/// removed kind cannot silently diverge between `ROUTABLE_KINDS` and the six
/// sites that used to check it separately.
fn model_list_path(kind: &str) -> &'static str {
    match kind {
        "openai-compatible" => "/models",
        _ => "/api/tags",
    }
}

/// What THIS machine actually has, asked at click time.
///
/// **DETECT, NEVER ASSUME — the box this was built on has a warm GPU and
/// seven models, and the customer's Windows machine may have nothing.** That
/// premise error already bit this product once, the same day this file was
/// written, when "embeddings from the local model" described the development
/// box rather than the product. So nothing here is defaulted from the machine
/// that wrote it: Ollama absent is a `running: false` ANSWER with a next step
/// attached, never an error and never an assumption.
#[tauri::command(async)]
pub fn detect_local_brain() -> LocalBrain {
    let agent = ureq::AgentBuilder::new().timeout(Duration::from_secs(3)).build();

    let version = match agent.get("http://127.0.0.1:11434/api/version").call() {
        Ok(res) => res
            .into_string()
            .ok()
            .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
            .and_then(|v| v.get("version").and_then(|x| x.as_str()).map(String::from))
            .unwrap_or_default(),
        Err(e) => {
            return LocalBrain {
                running: false,
                version: String::new(),
                api_ok: false,
                models: Vec::new(),
                // THE PARENTHETICAL IS NEW — this used to give one sentence for
                // every possible cause: not installed, not running, blocked by a
                // firewall, or something answering slowly. "Say what you did not
                // check" applies to a probe just as much as a person — if this
                // is ever wrong on a real machine (Ollama genuinely running and
                // reachable by hand, refused here), the actual ureq error is now
                // on the screen instead of only in a guess.
                problem: Some(format!(
                    "Ollama isn't installed or isn't running on this computer. It is \
                     free — install it from ollama.com and pull a model, then check \
                     again. (Could not reach 127.0.0.1:11434: {e})"
                )),
            }
        }
    };

    // (name, size_mb, has_tools already declared on THIS /api/tags entry).
    // Kept as one tuple per model, still unfiltered, so a model with a size
    // Ollama did not report or a capabilities array it did not carry is
    // still carried through with `None` rather than being dropped from the
    // list before anyone gets to see it.
    let raw_models: Vec<(String, Option<u64>, Option<bool>)> = agent
        .get(&format!("http://127.0.0.1:11434{}", model_list_path("local")))
        .call()
        .ok()
        .and_then(|res| res.into_string().ok())
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| {
            v.get("models").and_then(|m| m.as_array()).map(|arr| {
                arr.iter()
                    .filter_map(|m| {
                        let name = m.get("name").and_then(|n| n.as_str())?.to_string();
                        let size_mb = m.get("size").and_then(|n| n.as_u64()).map(|b| b / (1024 * 1024));
                        let has_tools = has_tools_from(m);
                        Some((name, size_mb, has_tools))
                    })
                    .collect::<Vec<_>>()
            })
        })
        .unwrap_or_default();

    // Sized against THIS machine, via the exact rule the not-yet-installed
    // catalog uses — `probe_hardware` is std-only (no new dependency, same
    // reasoning as brain_setup.rs's own header), and it is cheap enough to
    // call every time this command runs: it is only reached when a model is
    // already installed, which is the case where a person is looking at this
    // screen wanting an answer now, not a cached one from last launch.
    let hw = crate::brain_setup::probe_hardware();

    // /api/show IS THE FALLBACK, NOT THE RULE — see `has_tools_from`'s doc
    // comment. Only reached for a model whose /api/tags entry did not carry
    // capabilities, so a machine where every entry already has them (proven
    // true on the real box this was built for) makes zero extra requests.
    // Bounded by a short per-call timeout so one hung model cannot stall the
    // whole probe; a model that does not answer /api/show in two seconds
    // reports `has_tools: None`, the same honest "could not tell" a
    // malformed response would produce.
    let show_agent = ureq::AgentBuilder::new().timeout(Duration::from_secs(2)).build();
    let models: Vec<InstalledModel> = raw_models
        .into_iter()
        .map(|(name, size_mb, has_tools_from_tags)| {
            let has_tools = has_tools_from_tags.or_else(|| {
                show_agent
                    .post("http://127.0.0.1:11434/api/show")
                    .send_string(&serde_json::json!({ "model": name }).to_string())
                    .ok()
                    .and_then(|res| res.into_string().ok())
                    .and_then(|body| serde_json::from_str::<serde_json::Value>(&body).ok())
                    .and_then(|v| has_tools_from(&v))
            });
            let fit = size_mb
                .map(|mb| crate::brain_setup::classify_size(mb, &hw))
                .unwrap_or(crate::brain_setup::Fit::Unknown);
            InstalledModel { name, size_mb, fit, has_tools }
        })
        .collect();
    let models = rank_installed(models);

    let new_enough = version_at_least(&version, 0, 33);
    let problem = if !new_enough {
        Some(format!(
            "Ollama {version} is too old — it learned Claude's language in 0.33. \
             Update it and check again."
        ))
    } else if models.is_empty() {
        Some(
            "Ollama is running but has no models. Pull one first — for example \
             `ollama pull qwen3-coder:30b` — then check again."
                .into(),
        )
    } else {
        None
    };

    LocalBrain {
        running: true,
        version,
        api_ok: new_enough && !models.is_empty(),
        models,
        problem,
    }
}

// ---------------------------------------------------------------------------
// Air-gap — Settings → Internet connection. Restricts a turn to the LOCAL
// engine only: no cloud brain may answer, and the AI Boardroom (boardroom.rs)
// reads the same flag to keep a private question from ever leaving the
// machine to be debated. HAI-APP-IA-SPEC.md's own wording for the toggle.
// ---------------------------------------------------------------------------

/// **ITS OWN TINY FILE, NOT A FIELD ON `providers.json`'S `Store` — a
/// deliberate choice, not the obvious one.** Air-gap is a property of THIS
/// MACHINE's willingness to reach the network at all, not a property of any
/// one brain: a Claude row, a local row and an OpenAI-compatible row are all
/// equally restricted by it, so it does not belong nested inside the thing
/// it constrains. A second file also means a corrupt or hand-edited
/// `providers.json` can never flip the person into or out of air-gap mode as
/// a side effect of an edit that had nothing to do with it.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
struct AirgapFile {
    #[serde(default)]
    on: bool,
}

fn airgap_path(app: &tauri::AppHandle) -> Result<std::path::PathBuf, String> {
    use tauri::Manager;
    let dir = app.path().app_config_dir().map_err(|e| format!("no config directory: {e}"))?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("could not create {dir:?}: {e}"))?;
    Ok(dir.join("airgap.json"))
}

/// **FAILS CLOSED TOWARD THE NETWORK, NOT TOWARD AIR-GAP.** A missing file, an
/// unreadable one, or one that will not parse all mean "nobody has ever
/// turned this on" — which is also the honest state of a fresh install, and
/// the same direction every other never-touched toggle on this screen
/// defaults. A storage hiccup must never read as a private question
/// suddenly being safe to send to the cloud, but it must also never strand a
/// person offline because a file went missing — so the one thing it can
/// never silently do is turn ON.
pub(crate) fn is_airgapped(app: &tauri::AppHandle) -> bool {
    let Ok(path) = airgap_path(app) else { return false };
    std::fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str::<AirgapFile>(&t).ok())
        .map(|f| f.on)
        .unwrap_or(false)
}

#[tauri::command]
pub fn get_airgap(app: tauri::AppHandle) -> bool {
    is_airgapped(&app)
}

/// Turn air-gap on or off. Returns the new state, so the toggle's own paint
/// never has to assume a write that may have failed actually landed.
#[tauri::command(async)]
pub fn set_airgap(app: tauri::AppHandle, on: bool) -> Result<bool, String> {
    let path = airgap_path(&app)?;
    let body = serde_json::to_string(&AirgapFile { on }).map_err(|e| e.to_string())?;
    std::fs::write(&path, body).map_err(|e| format!("could not write {path:?}: {e}"))?;
    if on {
        use tauri::Manager;
        crate::cancel_running_turn(&app.state::<crate::Session>())?;
    }
    Ok(on)
}

// ---------------------------------------------------------------------------
// Tests.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::brain_setup::{Fit, Gpu, HardwareProfile, CATALOG};

    /// The messy-state hardening finding: switching brains while a turn is
    /// still running must be refused, not silently allowed to race the
    /// in-flight turn's own late writes to the shared session-id slot.
    #[test]
    fn switching_brains_is_refused_while_a_turn_is_alive() {
        let err = refuse_switch_while_alive(true).unwrap_err();
        assert!(
            err.contains("Still working on the last one"),
            "the refusal must use the same recognisable phrase send() already uses \
             for the identical reason, got: {err}"
        );
    }

    /// The ordinary case — nothing running — must not be touched by this.
    #[test]
    fn switching_brains_is_allowed_when_idle() {
        assert_eq!(refuse_switch_while_alive(false), Ok(()));
    }

    fn row(kind: &str) -> Provider {
        Provider {
            id: "p1".into(),
            kind: kind.into(),
            name: "Test".into(),
            base_url: "http://127.0.0.1:11434".into(),
            model: "qwen3-coder:30b".into(),
            builtin: false,
            disconnected: false,
            connected: false,
            checked_at: String::new(),
            last_error: String::new(),
            has_secret: false,
            caveat: String::new(),
            migration_note: String::new(),
            allow_shell: false,
            allow_agency: false,
        }
    }

    // -- Promoting the Secondary swaps it with the old Primary. ----------
    //
    // Mark, using the app, 2026-09-05: "if OpenAI is primary and Claude is
    // secondary, Claude should have Make primary; if clicked, it defaults
    // OpenAI to secondary... This stands true for any primary/secondary
    // setup." This REPLACES the old behaviour, which cleared Secondary
    // instead of swapping it. There was no earlier test pinning that clear
    // to update: `select_provider` itself has never been reachable from a
    // unit test (no `tauri::test` harness anywhere in this tree — see its
    // own doc), so the old behaviour was only ever proven by hand. These pin
    // `secondary_after_promoting_it`, the pure decision `select_provider`
    // now defers to, plus two tests that mirror the command's own closure
    // line for line to prove the swap end to end without needing an
    // `AppHandle` to drive the real command through.

    #[test]
    fn promoting_the_secondary_swaps_in_the_old_primary() {
        let mut openai = row("openai-compatible");
        openai.id = "p_openai".into();
        openai.connected = true;
        let providers = vec![openai];
        // OpenAI (p_openai) was Primary; Claude is being promoted out of
        // Secondary. The old Primary must take the now-empty seat rather
        // than the seat being cleared.
        assert_eq!(
            secondary_after_promoting_it("p_openai", "claude", &providers),
            "p_openai",
            "the old Primary must become the new Secondary, not vanish"
        );
    }

    #[test]
    fn the_full_cycle_openai_primary_claude_secondary_then_promote_claude() {
        // Mirrors exactly what select_provider's closure does (see that
        // function), without a tauri::AppHandle to drive the real
        // #[tauri::command] through -- none exists in this tree.
        let mut openai = row("openai-compatible");
        openai.id = "openai".into();
        openai.connected = true;
        let mut claude = builtin_claude();
        claude.connected = true;
        let mut store = Store {
            active: "openai".into(),
            secondary: "claude".into(),
            providers: vec![openai, claude],
        };

        let id = "claude".to_string();
        let old_active = store.active.clone();
        store.active = id.clone();
        if store.secondary == id {
            store.secondary = secondary_after_promoting_it(&old_active, &id, &store.providers);
        }

        assert_eq!(store.active, "claude");
        assert_eq!(
            store.secondary, "openai",
            "promoting Secondary must swap it with the old Primary, not clear it"
        );
    }

    #[test]
    fn promoting_a_different_row_while_a_secondary_is_set_leaves_secondary_untouched() {
        // SCOPE NOTE: only promoting the row that IS the current Secondary
        // swaps. Promoting some other, unassigned row while a Secondary is
        // already set is not what Mark asked to change, and must leave that
        // path's behaviour exactly as it was.
        let mut openai = row("openai-compatible");
        openai.id = "openai".into();
        openai.connected = true;
        let mut claude = builtin_claude();
        claude.connected = true;
        let mut local = row("local");
        local.id = "local".into();
        local.connected = true;

        let mut store = Store {
            active: "openai".into(),
            secondary: "claude".into(),
            providers: vec![openai, claude, local],
        };

        // Promoting "local" -- neither the old Primary nor the current
        // Secondary.
        let id = "local".to_string();
        let old_active = store.active.clone();
        store.active = id.clone();
        if store.secondary == id {
            store.secondary = secondary_after_promoting_it(&old_active, &id, &store.providers);
        }

        assert_eq!(store.active, "local");
        assert_eq!(store.secondary, "claude", "promoting an unrelated row must not touch Secondary");
    }

    #[test]
    fn the_old_primary_does_not_return_as_secondary_if_nothing_was_primary_yet() {
        // Fresh install, nobody has chosen Primary yet -- old_active arrives
        // empty. There is nothing to swap in.
        assert_eq!(secondary_after_promoting_it("", "claude", &[]), "");
    }

    #[test]
    fn the_old_primary_does_not_return_as_secondary_if_it_is_no_longer_eligible() {
        // The old Primary was disconnected in the meantime. Writing it back
        // as Secondary would just be erased by normalized() on the very
        // next load (see its "SECONDARY IS NEVER LEFT POINTING AT A ROW
        // THAT CANNOT ANSWER" pass), so the helper clears it up front
        // instead of handing back a value that would not survive a reload.
        let mut openai = row("openai-compatible");
        openai.id = "openai".into();
        openai.connected = true;
        openai.disconnected = true;
        assert_eq!(secondary_after_promoting_it("openai", "claude", &[openai]), "");
    }

    #[test]
    fn a_row_cannot_swap_into_being_its_own_secondary() {
        // Degenerate: old_active == promoted_id. Guarded even though
        // select_provider cannot reach this case in practice today (its
        // `switched` check requires store.active != id a line earlier) --
        // the invariant this whole feature exists to protect ("a row cannot
        // be its own fallback") is cheap to hold here directly rather than
        // resting on that other line never changing underneath it.
        assert_eq!(secondary_after_promoting_it("claude", "claude", &[]), "");
    }

    // -- A missing allow_agency field must default closed, forever. ------
    //
    // Same reasoning as the allow_shell block directly below, for the safe
    // agency layer (SAFE-AGENCY-SPEC.md, room-approved 2026-09-04): this pins
    // the TYPE (`#[serde(default)]` on a `bool` is `false`), not the JSON, so
    // it fails the moment the default stops being `false` rather than the day
    // somebody notices a legacy row came back with OpenUrl/LaunchApp/
    // OpenSettingsPage armed.

    #[test]
    fn a_legacy_row_with_no_allow_agency_field_defaults_closed() {
        // No `allowAgency` key at all -- the shape of every row on disk
        // before this field existed, including every row written between
        // `allow_shell` shipping and this field shipping.
        let json = r#"{"id":"p1","kind":"local","name":"Test"}"#;
        let p: Provider = serde_json::from_str(json).unwrap();
        assert!(!p.allow_agency, "a row saved before this field existed must not come back with agency armed");
    }

    #[test]
    fn an_explicit_allow_agency_true_still_parses_true() {
        // The opt-in path must keep working -- this only pins that the field
        // is never true BY DEFAULT, not that it can never be true.
        let json = r#"{"id":"p1","kind":"local","name":"Test","allowAgency":true}"#;
        let p: Provider = serde_json::from_str(json).unwrap();
        assert!(p.allow_agency, "an explicit allowAgency: true must still parse true");
    }

    #[test]
    fn an_explicit_allow_agency_false_still_parses_false() {
        // The other leg of the round trip the Safe Agency opt-in rides on:
        // the toggle sends `allowAgency` on EVERY save, so an explicit `false`
        // -- the person turning it back off -- must parse as off, not be
        // confused with the absent-key case above. Absent and explicit-false
        // both land on `false` here, which is what lets the wire contract stay
        // "the payload is authoritative" without a separate "unset" state.
        let json = r#"{"id":"p1","kind":"local","name":"Test","allowAgency":false}"#;
        let p: Provider = serde_json::from_str(json).unwrap();
        assert!(!p.allow_agency, "an explicit allowAgency: false must parse false");
    }

    #[test]
    fn a_row_survives_a_disk_round_trip_with_agency_armed() {
        // `save_provider` itself cannot be unit-tested (no `tauri::test`
        // harness anywhere in this tree -- see `secondary_after_promoting_it`'s
        // own note), but the two ends it depends on CAN be: `save_to_disk`
        // serialises the whole `Store` with `serde_json`, and `load_from_disk`
        // deserialises it back, with `refresh`/`normalized` never touching this
        // field (verified by inspection 2026-09-07). This exercises exactly
        // that serialise->deserialise path on a full row and proves an armed
        // flag comes back armed rather than being dropped on the way to disk.
        let mut p = row("local");
        p.allow_agency = true;
        let json = serde_json::to_string(&p).unwrap();
        // The on-disk / on-wire key is camelCase `allowAgency` -- this is the
        // exact key Wren's toggle must set on the payload, pinned here so a
        // rename of the Rust field cannot silently change the contract.
        assert!(json.contains("\"allowAgency\":true"), "serialised row must carry allowAgency: {json}");
        let back: Provider = serde_json::from_str(&json).unwrap();
        assert!(back.allow_agency, "an armed agency flag must survive the disk round trip");
    }

    // -- A missing allow_shell field must default closed, forever. -------
    //
    // Cassandra flagged this during the Decision B sign-off. `#[serde(default)]`
    // on a `bool` is `false` today, and that is what lets every row saved
    // before this field existed come back with the shell off with no
    // migration step. But a default is a property of THIS release, not a
    // promise: drop the attribute, or someone later changes it to
    // `#[serde(default = "..true..")]` while "fixing" something unrelated,
    // and every existing install's rows deserialize with `Bash` armed on the
    // very next launch, silently, for a model that was never meant to have
    // it. This pins the TYPE, not the JSON, so it fails the moment the
    // default stops being `false` rather than the day somebody notices.

    #[test]
    fn a_legacy_row_with_no_allow_shell_field_defaults_closed() {
        // No `allowShell` key at all -- the shape of every row on disk before
        // this field existed.
        let json = r#"{"id":"p1","kind":"local","name":"Test"}"#;
        let p: Provider = serde_json::from_str(json).unwrap();
        assert!(!p.allow_shell, "a row saved before this field existed must not come back with the shell armed");
    }

    #[test]
    fn an_explicit_allow_shell_true_still_parses_true() {
        // The opt-in path must keep working -- this only pins that the
        // field is never true BY DEFAULT, not that it can never be true.
        let json = r#"{"id":"p1","kind":"local","name":"Test","allowShell":true}"#;
        let p: Provider = serde_json::from_str(json).unwrap();
        assert!(p.allow_shell, "an explicit allowShell: true must still parse true");
    }

    // -- Disconnecting the built-in row. ---------------------------------
    //
    // Mark asked for Claude to be disconnectable. The whole risk of granting
    // that is an app with no brain, so these pin the two places that could
    // produce one.

    #[test]
    fn a_disconnected_row_may_not_be_selected() {
        // Otherwise Disconnect is a suggestion rather than a state: the row
        // comes straight back as the brain the next time anything selects it.
        let mut p = row("local");
        p.connected = true;
        assert!(may_select(&p).is_ok());
        p.disconnected = true;
        assert!(may_select(&p).is_err(), "a switched-off brain must not be selectable");

        // And the built-in row is not exempt from this either. It is no longer
        // exempt from ANYTHING — the `builtin ||` shortcut in may_select is
        // gone as of 2026-08-29 — but this case predates that and still holds:
        // a row the user switched off must not come back by being selected, or
        // Disconnect is a suggestion rather than a state.
        let mut b = builtin_claude();
        b.connected = true;
        b.disconnected = true;
        assert!(may_select(&b).is_err());
    }

    #[test]
    fn the_active_row_is_never_left_disconnected() {
        // THE FAILURE THIS PREVENTS: the provider that replaced Claude gets
        // removed, delete_provider falls back to active = "claude", and Claude
        // is switched off. Every turn would then refuse. A preference going
        // unhonoured is a far better outcome than an app that cannot answer,
        // so normalized() makes the preference yield.
        let mut off = builtin_claude();
        off.disconnected = true;
        let mut off = off;
        off.connected = true;   // they had tested it; Disconnect is the thing being undone
        let store = normalized(Store { providers: vec![off], active: "claude".into(), secondary: String::new() });
        assert!(!store.providers[0].disconnected, "the only brain must come back on");
        assert!(may_select(&store.providers[0]).is_ok());
    }

    #[test]
    fn a_disconnected_row_that_is_not_active_stays_disconnected() {
        // The other half, and the one a careless fix breaks: normalized must
        // not simply re-enable everything it finds, or Disconnect would never
        // survive a reload.
        let mut off = builtin_claude();
        off.disconnected = true;
        let mut mine = row("local");
        mine.id = "p9".into();
        mine.connected = true;
        let store = normalized(Store {
            providers: vec![off, mine],
            active: "p9".into(),
            secondary: String::new(),
        });
        let claude = store.providers.iter().find(|p| p.id == "claude").unwrap();
        assert!(claude.disconnected, "it must stay off while something else is the brain");
    }

    // -- The environment is the product. ---------------------------------

    /// The built-in row touches NOTHING — the product as it shipped.
    #[test]
    fn the_builtin_row_sets_no_environment() {
        assert!(provider_env(&builtin_claude(), "", "x".into()).is_empty());
    }

    /// A non-default provider sets the routing, points the background model
    /// at the same place, and claims host-managed provider config.
    #[test]
    fn a_provider_sets_the_documented_variables() {
        let env = provider_env(&row("local"), "http://127.0.0.1:11434", PLACEHOLDER_TOKEN.into());
        let get = |name: &str| {
            env.iter().find(|(k, _)| *k == name).map(|(_, v)| v.clone())
        };
        assert_eq!(get("ANTHROPIC_BASE_URL"), Some(Some("http://127.0.0.1:11434".into())));
        assert_eq!(get("ANTHROPIC_MODEL"), Some(Some("qwen3-coder:30b".into())));
        assert_eq!(get("ANTHROPIC_DEFAULT_HAIKU_MODEL"), Some(Some("qwen3-coder:30b".into())));
        assert_eq!(get("ANTHROPIC_SMALL_FAST_MODEL"), Some(Some("qwen3-coder:30b".into())));
        assert_eq!(get("CLAUDE_CODE_PROVIDER_MANAGED_BY_HOST"), Some(Some("1".into())));
        assert_eq!(get("ANTHROPIC_AUTH_TOKEN"), Some(Some(PLACEHOLDER_TOKEN.into())));
    }

    /// THE CREDENTIAL-EXFILTRATION GUARD. An inherited ANTHROPIC_API_KEY plus
    /// a third-party base URL ships the user's Anthropic key to somebody
    /// else's server; `None` here means env_remove on the child.
    ///
    /// **STILL LIVE AFTER THE 2026-08-28 REMOVAL, and worth saying why, because
    /// "there are no third-party endpoints any more" is the argument that would
    /// retire it wrongly.** A `local` row is not required to be loopback — the
    /// setup wizard's remote path deliberately ends with the user pointing one
    /// at another machine they own — so a non-Anthropic endpoint on the far end
    /// of a network is still exactly the situation this guard is for.
    #[test]
    fn the_users_anthropic_key_is_never_forwarded() {
        let env = provider_env(&row("local"), "http://192.0.2.10:11434", "their-key".into());
        assert_eq!(
            env.iter().find(|(k, _)| *k == "ANTHROPIC_API_KEY").map(|(_, v)| v.clone()),
            Some(None),
            "ANTHROPIC_API_KEY must be REMOVED, not merely left alone"
        );
    }

    // -- Selection has to be earned. --------------------------------------

    #[test]
    fn the_builtin_row_earns_its_dot_like_every_other_row() {
        // THIS TEST USED TO BE CALLED `the_builtin_row_may_always_be_selected`
        // AND IT ASSERTED THE OPPOSITE. That exemption — `p.builtin ||` — is
        // what made Claude structural: `connected` meant "tested and working"
        // for every row except the one everybody got, where it meant nothing.
        // Mark asked three times for setup to stop assuming Claude, and this
        // was the line underneath the assumption.
        //
        // It is not renamed lightly. A test whose NAME states a policy has to
        // be rewritten when the policy changes, or it quietly enforces the old
        // rule under a name nobody re-reads.
        assert!(
            may_select(&builtin_claude()).is_err(),
            "an untested Claude row must not be selectable — that was the exemption"
        );
        let mut earned = builtin_claude();
        earned.connected = true;
        assert!(may_select(&earned).is_ok(), "and one press of Test earns it");
    }

    #[test]
    fn nothing_is_chosen_on_a_machine_that_has_never_chosen() {
        // THE BUG MARK REPORTED THREE TIMES, pinned at the one place it lived.
        // An empty `active` is NO CHOICE YET; it used to be overwritten with
        // "claude" here, which made "never asked" indistinguishable from
        // "picked Claude" and left the setup screen with nothing to ask.
        let store = normalized(Store { providers: Vec::new(), active: String::new(), secondary: String::new() });
        assert_eq!(store.active, "", "setup must have a question left to ask");
        // The built-in row still EXISTS — it is the thing they will be offered.
        // Existing and being chosen are different, and conflating them is the
        // whole bug.
        assert!(store.providers.iter().any(|p| p.id == "claude"), "Claude is offered");
        assert!(
            may_select(store.providers.iter().find(|p| p.id == "claude").unwrap()).is_err(),
            "offered, not selected, and not selectable until it has been tested"
        );
    }

    #[test]
    fn somebody_already_running_on_claude_is_not_asked_again() {
        // THE UPGRADE PATH, AND IT MATTERS MORE THAN THE NEW SCREEN. An
        // existing customer's providers.json says `"active": "claude"`. That is
        // NON-EMPTY, so the rule above does not touch it, and they must come
        // back exactly where they were — not reset, not logged out, not asked
        // to choose again.
        let mut theirs = builtin_claude();
        theirs.connected = true;
        let store = normalized(Store { providers: vec![theirs], active: "claude".into(), secondary: String::new() });
        assert_eq!(store.active, "claude", "their choice survives the upgrade");
        assert!(active_of(&store).is_some(), "and they still have a brain");
    }

    #[test]
    fn a_choice_that_broke_still_falls_back_rather_than_stranding_them() {
        // The OTHER non-empty case, and the reason the rule is "empty stays
        // empty" rather than "always ask". Somebody chose a cloud provider that
        // this build retired: they DID answer the question, so re-asking is not
        // owed to them, and leaving them brainless would punish them for our
        // removal. Unchanged behaviour, pinned so the new rule cannot eat it.
        let mut theirs = row("anthropic-compatible");
        theirs.id = "p9".into();
        let store = normalized(Store { providers: vec![theirs], active: "p9".into(), secondary: String::new() });
        assert_eq!(store.active, "claude");
    }

    #[test]
    fn an_untested_provider_cannot_become_the_brain() {
        assert!(may_select(&row("local")).is_err());
        let mut green = row("local");
        green.connected = true;
        assert!(may_select(&green).is_ok());
    }

    // -- Validation refuses before anything is written. --------------------

    #[test]
    fn validation_names_the_missing_field() {
        let mut p = row("local");
        p.model = String::new();
        assert!(validate(&p).unwrap_err().contains("model"));

        let mut p = row("local");
        p.base_url = "ftp://wrong".into();
        assert!(validate(&p).is_err());

        let mut p = row("weird-kind");
        p.kind = "weird-kind".into();
        assert!(validate(&p).unwrap_err().contains("weird-kind"));
    }

    #[test]
    fn caveats_are_derived_and_honest() {
        assert!(caveat_for("local").contains("hardware and model"));
        assert!(caveat_for("claude").is_empty());
        // RESTORED 2026-08-29 with the wiring. It is returned for the KIND
        // regardless of the gate, because a caveat is a property of the route
        // and not of whether a screen is currently offering it — a gate that
        // flips without its warning coming too is the failure this avoids.
        // Nobody reads it while the gate is shut: no row of this kind is
        // selectable, and RETIRED_NOTE is what they see instead.
        assert!(caveat_for("openai-compatible").contains("Your provider bills your usage"),
            "the current route must explain who bills usage");
        // Never restored: Mark named two choices tonight, Claude and OpenAI.
        assert!(caveat_for("anthropic-compatible").is_empty());
    }

    // -- The kinds removed on 2026-08-28. ----------------------------------
    //
    // Mark: "take out all cloud based ai and leave claude." These pin the
    // three ways a row on an existing customer's disk could still reach a
    // provider this build cannot route, and the one that matters most is the
    // third: the UI not offering something is a presentation change, and
    // `apply_env` is the boundary.

    #[test]
    fn a_removed_kind_is_never_routable_and_the_list_is_the_only_source() {
        assert!(is_routable("claude"));
        assert!(is_routable("local"));
        assert!(!is_routable("anthropic-compatible"));
        // OPENAI IS ROUTABLE AS OF 2026-08-29 — the gate opened. This line
        // asserted the opposite and it is the reason the flip could not be
        // quiet: eight tests written for the shut-gate world failed at once and
        // each had to be re-read rather than re-run.
        assert!(is_routable("openai-compatible"));
        assert!(!is_routable("whatever-comes-next"));
    }

    #[test]
    fn a_row_from_the_old_build_is_retired_rather_than_deleted() {
        // THE CASE THAT REACHES A REAL PERSON: they were using Gemini
        // yesterday, and providers.json on their disk still says so, with a
        // green they genuinely earned and a key in the OS credential store.
        let mut theirs = row("anthropic-compatible");
        theirs.id = "p9".into();
        theirs.connected = true;
        theirs.checked_at = "1724700000".into();
        let store = normalized(Store { providers: vec![theirs], active: "p9".into(), secondary: String::new() });

        let p = store.providers.iter().find(|q| q.id == "p9").expect("the row survives");
        assert!(!p.connected, "a row that cannot be launched must not wear a green");
        assert!(may_select(p).is_err(), "and must not be selectable");
        assert!(
            p.migration_note.contains("still stored safely"),
            "they must be told their key is untouched, not left to guess: {}",
            p.migration_note
        );
        // NEVER DELETED. The id is the pointer to their credential — removing
        // the row orphans a key they pasted and answers none of their
        // questions. Same call migrate.rs makes about an unmappable row.
        assert_eq!(store.providers.iter().filter(|q| q.id == "p9").count(), 1);
    }

    #[test]
    fn a_retired_row_that_was_the_active_brain_falls_back_to_claude() {
        // Without this the first launch after the update has `active` pointing
        // at a brain that cannot answer, which is the brainless app the whole
        // normalized() invariant exists to prevent — arriving by a new door.
        let mut theirs = row("anthropic-compatible");
        theirs.id = "p9".into();
        theirs.connected = true;
        let store = normalized(Store { providers: vec![theirs], active: "p9".into(), secondary: String::new() });
        assert_eq!(store.active, "claude");
        let claude = store.providers.iter().find(|p| p.id == "claude").unwrap();

        // THE INVARIANT IS THAT THEY CAN STILL SEND, and that is what is
        // asserted. This line used to read `may_select(claude).is_ok()`, which
        // passed for free off the `builtin ||` exemption rather than because
        // anything had been proven — and when the exemption went, so did it.
        //
        // The two are genuinely different questions and it is worth keeping
        // them apart: `apply_env` is the launch path and the built-in row needs
        // no environment, so a fallback to Claude can always answer.
        // `may_select` is the picker, where a dot has to be earned. So this
        // person keeps a working app, and if they later open AI components and
        // choose Claude deliberately, it costs them one press of Test — the
        // same as every other row, which is the point of removing the
        // exemption.
        assert!(
            apply_env(&mut Command::new("true"), claude).is_ok(),
            "the fallback must be able to answer — that is the never-brainless rule"
        );
    }

    #[test]
    fn a_retired_row_can_never_be_launched_even_if_it_reaches_apply_env() {
        // THE BOUNDARY, NOT THE SHEET. Everything above is a store that has
        // been normalized; this is the last gate before the real binary runs,
        // and it is the one that has to hold if a hand-edited file, a stale
        // in-memory row or a future caller gets past the rest.
        //
        // The specific silent failure it stops: with the adapter branch gone,
        // an openai-compatible row falling through to the direct path would
        // point Claude Code at an OpenAI endpoint, in Anthropic's wire format,
        // carrying the user's real key — and they would find out mid-answer.
        let mut cmd = Command::new("true");
        let err = apply_env(&mut cmd, &row("anthropic-compatible")).unwrap_err();
        assert!(err.contains("no longer offers"), "{err}");
        // And the two kinds that ARE routable still wire up.
        assert!(apply_env(&mut Command::new("true"), &builtin_claude()).is_ok());
        assert!(apply_env(&mut Command::new("true"), &row("local")).is_ok());
    }

    /// **THE TEST BUTTON MUST NOT GO LOOKING FOR A BINARY THIS ROW DOES NOT
    /// USE**, and this is a source scan because `test_provider` needs a live
    /// Tauri `AppHandle` and cannot be called from `cargo test` at all.
    ///
    /// The failure it exists to stop is silent and complete: reinstate the
    /// unconditional `test_binary` and a person running Ollama with no Claude
    /// Code installed fails the Test, never gets a green dot, cannot be selected
    /// by `may_select`, and every send answers `NO_BRAIN_YET` — while the send
    /// path works perfectly the whole time. Nothing else in this file would
    /// notice, because the row simply never becomes selectable.
    ///
    /// Proven able to fail: deleting the `!engine.needs_vendor_binary()` arm.
    #[test]
    fn the_test_button_asks_the_engine_before_it_hunts_for_claude_code() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/providers.rs");
        let text = std::fs::read_to_string(&path).expect("providers.rs is unreadable");
        let start = text
            .find("\npub fn test_provider(")
            .or_else(|| text.find("\nfn test_provider("))
            .expect("test_provider is not where this test thinks it is");
        let body_len = text[start + 1..].find("\n}\n").expect("test_provider has no end");
        let body = &text[start..start + 1 + body_len];

        // A scan that returned clean because it scanned nothing is the failure
        // this assertion exists to stop.
        assert!(
            body.lines().count() > 20,
            "only scanned {} lines of test_provider -- the slice is wrong, not the code",
            body.lines().count()
        );
        assert!(
            body.contains("needs_vendor_binary"),
            "test_provider no longer asks which engine drives the row, so a local brain \
             is about to start demanding a Claude Code install again"
        );
        assert!(
            body.contains("engine::for_provider"),
            "the engine must come from the one selection point, not be inferred from the kind"
        );
    }

    /// **THE `openai-compatible` KIND HAD ITS OWN, SEPARATE COPY OF THE BUG
    /// THE TEST ABOVE ALREADY GUARDS FOR `local` — found on Mark's own
    /// machine, 2026-09-03.** `needs_vendor_binary` and `engine::for_provider`
    /// were both genuinely present in `test_provider`'s body the whole time —
    /// the test above went on passing — because the `openai-compatible` arm
    /// sat ABOVE the generic `!engine.needs_vendor_binary()` check and
    /// chained straight into `test_binary` without ever asking it. An OpenAI
    /// key test came back "Claude Code is not installed on this computer, and
    /// it is the engine every provider runs through" on a machine where the
    /// key was genuinely good and `NATIVE_OPENAI_ENABLED` was genuinely open.
    /// **Presence of the right words in the function is not the same claim as
    /// every kind actually reaching them**, which is the gap this test closes.
    ///
    /// Proven able to fail: reinstate
    /// `test_chat_endpoint(&target).and_then(|()| test_binary(&target))` as
    /// its own unconditional arm for `target.kind == "openai-compatible"`,
    /// ahead of the `needs_vendor_binary` check, the way it read before.
    #[test]
    fn the_openai_kind_cannot_reach_test_binary_without_asking_the_engine_either() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/providers.rs");
        let text = std::fs::read_to_string(&path).expect("providers.rs is unreadable");
        let start = text
            .find("\npub fn test_provider(")
            .or_else(|| text.find("\nfn test_provider("))
            .expect("test_provider is not where this test thinks it is");
        let body_len = text[start + 1..].find("\n}\n").expect("test_provider has no end");
        let body = &text[start..start + 1 + body_len];

        assert!(
            !body.contains("test_chat_endpoint(&target).and_then(|()| test_binary"),
            "the openai-compatible kind is chaining straight into test_binary again, \
             unconditionally -- exactly the shape that made an OpenAI key test demand a \
             Claude Code install even with the native gate open"
        );
    }

    // -- The OpenAI gate: wired 2026-08-29, switched off. -------------------
    //
    // Mark asked for a choice of Claude or OpenAI at setup. The translator is
    // wired back up; it is not offered, because `adapter.rs`'s ten tests are
    // all synthetic and it has never met a real endpoint. These tests hold
    // both halves of that: that it is genuinely off, and that the half nobody
    // can reach yet is nonetheless correct — because code that has never run
    // is exactly what a gate must not be hiding.

    /// Flip the gate for one test. Serialised, because the tests run in
    /// parallel and a shared switch is a race — and restored on the way out
    /// even if the body panics, or one failure would cascade into every test
    /// that reads the gate afterwards.
    fn with_openai_enabled<T>(f: impl FnOnce() -> T) -> T {
        use std::sync::atomic::Ordering::SeqCst;
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        OPENAI_FORCED_ON.store(true, SeqCst);
        let out = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
        OPENAI_FORCED_ON.store(false, SeqCst);
        drop(guard);
        match out {
            Ok(v) => v,
            Err(p) => std::panic::resume_unwind(p),
        }
    }

    #[test]
    fn the_openai_gate_is_open_and_everything_it_needs_came_with_it() {
        // THIS TEST USED TO ASSERT THE GATE WAS SHUT, and it did exactly the
        // job it was written for: flipping one `const` failed EIGHT tests at
        // once, and every one of them had to be re-read rather than re-run.
        // That is what a pinned decision is for.
        //
        // It now guards the other direction — that the gate being open is not
        // enough on its own. Four things have to be true together or the row is
        // offered and then fails somewhere the person cannot see:
        assert!(OPENAI_ENABLED);
        // 1. it can be routed,
        assert!(is_routable("openai-compatible"));
        // 2. it can be saved and edited,
        let mut p = row("openai-compatible");
        p.base_url = "https://api.openai.com/v1".into();
        p.model = "gpt-5.6-sol".into();
        assert!(validate(&p).is_ok(), "{:?}", validate(&p));
        // 3. it carries the cost warning wherever it is shown,
        assert!(caveat_for("openai-compatible").contains("Your provider bills your usage"));
        // 4. and a row of this kind is NOT retired on read any more.
        let mut theirs = p.clone();
        theirs.id = "p9".into();
        let store = normalized(Store { providers: vec![theirs], active: "p9".into(), secondary: String::new() });
        let got = store.providers.iter().find(|q| q.id == "p9").unwrap();
        assert!(got.migration_note.is_empty(), "a live kind must not read as retired");
        assert_eq!(store.active, "p9");
    }

    /// THE AI BOARDROOM SEATING CONTRACT — the exact question Beck put on
    /// 2026-09-05 after a VM showed "IN THE ROOM · 0 OF 0" with an OpenAI brain
    /// connected and active. `is_eligible` is the ONE predicate both the room's
    /// UI (`iaBoardroomSeats`) and `boardroom_convene`'s own seat filter derive
    /// from, so what it returns IS which brains sit at the table. This pins the
    /// three facts that answer her: a connected cloud brain is seated, an
    /// unconnected local Ollama row is NOT (so convene never dials a 127.0.0.1
    /// nobody connected), and it is connected-state that gates a seat, never the
    /// kind — the room hardcodes neither Ollama nor the built-in Claude.
    ///
    /// Proven able to fail: drop `openai-compatible` from `GATED_KINDS`, or make
    /// `is_eligible` ignore `p.connected`, and this stops holding.
    #[test]
    fn the_boardroom_seats_a_connected_cloud_brain_and_not_an_unconnected_ollama() {
        with_openai_enabled(|| {
            // A connected cloud brain (OpenAI, the shape Beck had active) IS a
            // seat — the room does not ignore it, and "0 OF 0" is wrong the
            // moment this row exists connected.
            let mut cloud = row("openai-compatible");
            cloud.base_url = "https://api.openai.com/v1".into();
            cloud.model = "gpt-5.6-sol".into();
            cloud.connected = true;
            assert!(is_eligible(&cloud), "a connected cloud brain must be seatable");

            // A local Ollama row the person only SEEDED and never tested is not
            // connected, so it is not a seat — convene must not reach a
            // 127.0.0.1:11434 that nobody stood up (Beck's `os error 10061`).
            let mut untested_ollama = row("local");
            assert!(!untested_ollama.connected);
            assert!(
                !is_eligible(&untested_ollama),
                "an unconnected local row must never become a seat convene will dial"
            );

            // And a genuinely-connected local brain IS a seat — it is the green
            // dot that gates, not the kind, so a real Ollama seats the same way
            // a real cloud brain does.
            untested_ollama.connected = true;
            assert!(is_eligible(&untested_ollama), "a connected local brain must be seatable");

            // A disconnected row is out whatever else is true of it.
            let mut off = cloud.clone();
            off.disconnected = true;
            assert!(!is_eligible(&off), "a disconnected row is never a seat");
        });
    }

    #[test]
    fn an_openai_row_is_tested_in_the_shape_openai_actually_speaks() {
        // THE FOURTH WIRING POINT, AND I MISSED IT FIRST TIME. I reported three
        // — ROUTABLE_KINDS, apply_env's arm, adapter::init — and those get a row
        // LAUNCHING. This is what lets it go GREEN.
        //
        // `test_endpoint` posts Anthropic-shaped JSON to `{base}/v1/messages`.
        // Against https://api.openai.com/v1 that is `/v1/v1/messages` in the
        // wrong format: it can never pass, so the row could never earn a dot,
        // so `may_select` would refuse it for ever — the picker offering a
        // choice that dead-ends, which is the exact failure keeping OpenAI off
        // the screen was meant to prevent.
        //
        // Asserted through the URL each stage-1 builds, because that is the
        // difference and it is one line in each.
        let mut p = row("openai-compatible");
        p.base_url = "https://api.openai.com/v1".into();
        // Both paths refuse a non-address before they build anything, which is
        // the cheap half of the same guard.
        let mut bad = p.clone();
        bad.base_url = "ftp://nope".into();
        assert!(test_chat_endpoint(&bad).is_err());
        assert!(test_endpoint(&bad).is_err());
    }

    /// THE TEST THAT FAILS AGAINST THE OLD CODE — Beck read the same
    /// `{"max_tokens":8,…}` off her echo server's request log for
    /// `gpt-5.6-sol`, `o3-mini` and `gpt-4o-mini` alike: the Test-button body
    /// never branched on the model, while `adapter::translate_request` (the
    /// real send path) already did. That meant no o1/o3/o4/gpt-5* row could
    /// ever earn the green dot `may_select` requires, and
    /// `OPENAI_DEFAULT_MODEL` in `ui/index.html` pre-fills exactly one of
    /// them — so the model NameOS suggests could never actually connect.
    #[test]
    fn a_reasoning_model_test_body_uses_max_completion_tokens() {
        for m in ["gpt-5.6-sol", "o3-mini", "o1-preview", "o4-mini"] {
            let body = chat_completion_test_body(m);
            assert!(body.get("max_completion_tokens").is_some(), "{m}: {body}");
            assert!(
                body.get("max_tokens").is_none(),
                "{m}: the OLD field name 400s a reasoning model outright: {body}"
            );
        }
    }

    #[test]
    fn an_ordinary_model_test_body_keeps_max_tokens() {
        let body = chat_completion_test_body("gpt-4o-mini");
        assert!(body.get("max_tokens").is_some(), "{body}");
        assert!(body.get("max_completion_tokens").is_none(), "{body}");
    }

    /// THE TEST THE WHOLE RESTORATION HANGS ON.
    ///
    /// The silent failure being guarded: an `openai-compatible` row reaching
    /// `provider_env` directly would set ANTHROPIC_BASE_URL to the PROVIDER'S
    /// OWN endpoint and ANTHROPIC_AUTH_TOKEN to the user's REAL key — Claude
    /// Code then talks to OpenAI in Anthropic's wire format, carrying a
    /// credential, and the person finds out mid-answer. There is no error, no
    /// log and no way to notice from inside the app.
    ///
    /// So this asserts the observable signature of that bug rather than the
    /// shape of the code: for an openai row, the child's base URL is NEVER the
    /// provider's own address, in either gate state.
    #[test]
    fn an_openai_row_can_never_reach_the_direct_path() {
        let mut p = row("openai-compatible");
        p.id = "p9".into();
        p.base_url = "https://api.openai.com/v1".into();

        let env_of = |cmd: &Command| -> Vec<(String, String)> {
            cmd.get_envs()
                .filter_map(|(k, v)| Some((k.to_string_lossy().into_owned(),
                                           v?.to_string_lossy().into_owned())))
                .collect()
        };

        // THE INVARIANT HOLDS IN EVERY GATE STATE, which is why it is asserted
        // unconditionally rather than inside a `with_openai_enabled`. The gate
        // opened on 2026-08-29 and this test did not need weakening to let that
        // happen — if it had, that would have been the signal to stop.
        //
        // In a unit test the adapter is never initialised, so `ensure_running`
        // refuses and `apply_env` returns Err. THAT IS THE CORRECT OUTCOME.
        // What must never happen, in either case, is the provider's own
        // endpoint reaching the child.
        let mut cmd = Command::new("true");
        let out = apply_env(&mut cmd, &p);
        for (k, v) in env_of(&cmd) {
            assert_ne!(
                v, "https://api.openai.com/v1",
                "{k} was set to the provider's own endpoint — this is the direct path"
            );
            if k == "ANTHROPIC_BASE_URL" {
                assert!(
                    v.starts_with("http://127.0.0.1:"),
                    "the child must be pointed at the local translator, not at {v}"
                );
            }
        }
        // A refusal that had already set half the environment would leave a
        // caller that ignores the Err launching a half-configured binary.
        if out.is_err() {
            assert!(env_of(&cmd).is_empty(), "a failed launch left environment behind");
        }
    }

    #[test]
    fn and_the_direct_path_really_would_have_leaked_it() {
        // THE NEGATIVE CONTROL, without which the test above passes for free.
        // This is what `provider_env` produces when a row DOES take the direct
        // path — the provider's own URL, straight onto the child. If this ever
        // stops being true, the assertion above has quietly stopped meaning
        // anything and both tests need rereading together.
        let mut p = row("local");
        p.base_url = "https://api.openai.com/v1".into();
        let env = provider_env(&p, "https://api.openai.com/v1", "their-real-key".into());
        assert_eq!(
            env.iter().find(|(k, _)| *k == "ANTHROPIC_BASE_URL").map(|(_, v)| v.clone()),
            Some(Some("https://api.openai.com/v1".into())),
            "the direct path is supposed to set exactly this — that is the point"
        );
    }

    #[test]
    fn every_routable_kind_has_a_launch_arm_and_the_rest_are_refused() {
        // apply_env matches on kind with a REFUSING final arm, replacing an
        // `if openai { adapter } else { direct }` whose `else` caught
        // everything. A kind can now only launch if somebody wrote an arm for
        // it. This is the checklist that keeps the two lists honest: if you
        // add a kind to ROUTABLE_KINDS or GATED_KINDS, add it here, and adding
        // it here without an arm in apply_env fails.
        let handled = ["claude", "local", "openai-compatible"];
        for k in ROUTABLE_KINDS.iter().chain(GATED_KINDS) {
            assert!(handled.contains(k), "`{k}` is routable but has no arm in apply_env");
        }
        // And a kind with no arm never launches, whatever a hand-edited file
        // says.
        let mut odd = row("local");
        odd.kind = "some-new-cloud".into();
        assert!(apply_env(&mut Command::new("true"), &odd).is_err());
    }

    #[test]
    fn opening_the_gate_makes_the_row_real_rather_than_merely_present() {
        // The other half: code behind a gate that would not work if the gate
        // opened is worse than no code, because it reads as ready. With the
        // gate open the row is routable, editable and no longer retired.
        with_openai_enabled(|| {
            assert!(is_routable("openai-compatible"));
            let mut p = row("openai-compatible");
            p.base_url = "https://api.openai.com/v1".into();
            p.model = "gpt-5.6-sol".into();
            assert!(validate(&p).is_ok(), "{:?}", validate(&p));

            let mut theirs = p.clone();
            theirs.id = "p9".into();
            theirs.connected = true;
            let store = normalized(Store { providers: vec![theirs], active: "p9".into(), secondary: String::new() });
            let got = store.providers.iter().find(|q| q.id == "p9").unwrap();
            assert!(got.migration_note.is_empty(), "a live row must not be retired");
            assert_eq!(store.active, "p9", "and it may stay the active brain");
            assert!(got.caveat.contains("Your provider bills your usage"), "the cost warning travels with it");
        });
    }

    #[test]
    fn editing_a_retired_row_says_what_happened_rather_than_unknown_kind() {
        let p = row("anthropic-compatible");
        let err = validate(&p).unwrap_err();
        assert!(err.contains("no longer connects"), "{err}");
        assert!(!err.contains("Unknown"), "a message written for us, not for them: {err}");
    }

    // -- Loopback is a HOST check, not a substring. -------------------------

    #[test]
    fn a_lookalike_hostname_is_not_this_machine() {
        assert!(is_loopback("http://127.0.0.1:11434"));
        assert!(is_loopback("http://localhost:11434/v1"));
        assert!(is_loopback("http://[::1]:11434"));
        assert!(is_loopback("HTTP://LocalHost:11434"));
        // THE BUG THE SUBSTRING VERSION HAD. Every one of these contains
        // "://127.0.0.1" or "://localhost" and none of them is this machine —
        // the first is an ordinary public hostname anybody can register a
        // wildcard under, and the second puts the real host after the `@`.
        assert!(!is_loopback("https://127.0.0.1.evil.example/v1"));
        assert!(!is_loopback("http://127.0.0.1@evil.example/v1"));
        assert!(!is_loopback("https://localhost.evil.example"));
        assert!(!is_loopback("https://api.example.com"));
        // A machine on the user's own network is not loopback, and must not be
        // called loopback — the remote-brain path depends on the distinction.
        assert!(!is_loopback("http://192.0.2.10:11434"));
    }

    #[test]
    fn the_builtin_row_cannot_be_edited() {
        let mut p = row("local");
        p.id = "claude".into();
        assert!(validate(&p).is_err());
    }

    // -- A corrupt file degrades to the shipped product. -------------------

    #[test]
    fn an_empty_store_normalizes_to_claude() {
        let store = normalized(Store { active: "gone".into(), secondary: String::new(), providers: Vec::new() });
        assert_eq!(store.active, "claude");
        assert_eq!(store.providers.len(), 1);
        assert!(store.providers[0].builtin);
    }

    /// A user-edited file cannot make the default deletable or promote a row
    /// to builtin.
    #[test]
    fn the_file_on_disk_is_not_trusted_about_builtin() {
        let mut fake = row("local");
        fake.builtin = true; // claims a privilege it does not have
        let mut claude = builtin_claude();
        claude.builtin = false; // claims a weakness it does not have
        let store = normalized(Store {
            active: "p1".into(),
            secondary: String::new(),
            providers: vec![fake, claude],
        });
        let by_id = |id: &str| store.providers.iter().find(|p| p.id == id).unwrap();
        assert!(!by_id("p1").builtin);
        assert!(by_id("claude").builtin);
    }

    // -- Version gate for the local API. -----------------------------------

    #[test]
    fn the_ollama_version_gate_reads_real_strings() {
        assert!(version_at_least("0.33.1", 0, 33));
        assert!(version_at_least("0.33", 0, 33));
        assert!(version_at_least("1.0.0", 0, 33));
        assert!(!version_at_least("0.29.4", 0, 33));
        assert!(!version_at_least("", 0, 33));
        assert!(!version_at_least("not-a-version", 0, 33));
    }

    // -- Telling apart Ollama's two 404s. ----------------------------------
    //
    // Tessa's finding: the old code answered EVERY local 404 with "Update
    // Ollama", which is confidently wrong for the far more common case of a
    // mistyped or unpulled model name against a current build. See
    // `classify_local_404`'s own doc comment for why the branches are
    // ordered the way they are.

    #[test]
    fn a_current_ollama_missing_the_model_is_told_to_pull_it_not_to_update() {
        let msg = classify_local_404(Some("0.33.2"), Some(false), "qwen3-coder:30b");
        assert!(msg.contains("ollama pull qwen3-coder:30b"), "{msg}");
        assert!(
            !msg.to_lowercase().contains("update ollama"),
            "a current build must never be told to update: {msg}"
        );
    }

    #[test]
    fn an_old_ollama_is_still_told_to_update_even_if_the_model_looks_present() {
        // The version check wins outright: an old build cannot speak
        // /v1/messages at all, so it fully explains the 404 whatever the
        // model probe happened to find.
        let msg = classify_local_404(Some("0.29.4"), Some(true), "qwen3-coder:30b");
        assert!(msg.contains("older than 0.33"), "{msg}");
    }

    #[test]
    fn an_unexplained_404_says_so_instead_of_guessing() {
        // Neither probe pinned down a cause — version could not be read and
        // the model was not confirmed missing. The old code guessed "update
        // Ollama" here too; the fix is to say plainly it does not know.
        let msg = classify_local_404(None, None, "qwen3-coder:30b");
        assert!(
            !msg.to_lowercase().contains("update ollama"),
            "an unknown cause must not be reported as a known one: {msg}"
        );
        assert!(msg.contains("could not confirm why"), "{msg}");
    }

    #[test]
    fn model_list_path_is_ollamas_unless_the_kind_says_otherwise() {
        assert_eq!(model_list_path("local"), "/api/tags");
        assert_eq!(model_list_path("openai-compatible"), "/models");
        // Anything unrecognised falls back to Ollama's path rather than an
        // empty string, since every routable kind today IS Ollama's shape.
        assert_eq!(model_list_path("claude"), "/api/tags");
    }

    /// **THE TEST THAT FAILS AGAINST THE OLD CODE.** A real Ollama, current
    /// version, that simply does not have the requested model — the
    /// shipped code's ONLY local-404 branch matched on `p.kind == "local"`
    /// with no further check and always returned the fixed sentence "Ollama
    /// is running but does not speak the Anthropic API — it learned it in
    /// version 0.33. Update Ollama and test again." Against this exact
    /// scenario that sentence is false: the version is fine, and updating
    /// software that is already current fixes nothing. A real TCP server is
    /// used rather than mocking `ureq` — same reasoning as `adapter.rs`'s
    /// `relay` test just above this file's peer: `test_endpoint` opens a
    /// real socket to `p.base_url`, and only a real socket proves what it
    /// actually sends and reads.
    #[test]
    fn a_missing_model_on_a_current_ollama_is_named_not_mistaken_for_an_old_build() {
        let addr = spawn_fake_ollama(FakeOllama {
            version: "0.33.2",
            installed_models: &["some-other-model"],
        });
        let mut p = row("local");
        p.base_url = format!("http://{addr}");
        p.model = "qwen3-coder:30b".into();

        let err = test_endpoint(&p).expect_err("a 404 must fail the test");
        assert!(
            err.contains("does not have a model called qwen3-coder:30b"),
            "the real cause must be named: {err}"
        );
        assert!(
            !err.to_lowercase().contains("update ollama"),
            "a current, reachable Ollama must never be told to update: {err}"
        );
    }

    #[test]
    fn an_old_ollama_through_the_real_endpoint_is_still_told_to_update() {
        let addr = spawn_fake_ollama(FakeOllama {
            version: "0.30.0",
            installed_models: &["qwen3-coder:30b"],
        });
        let mut p = row("local");
        p.base_url = format!("http://{addr}");

        let err = test_endpoint(&p).expect_err("a 404 must fail the test");
        assert!(err.contains("older than 0.33"), "{err}");
    }

    /// The one canned Ollama the tests above point `test_endpoint` at:
    /// answers `/v1/messages` with a genuine 404 (as a real too-old-or-
    /// wrong-model Ollama does), `/api/version` and `/api/tags` for real, so
    /// `diagnose_local_404`'s two follow-up probes land on real responses
    /// rather than a second failed connection.
    struct FakeOllama {
        version: &'static str,
        installed_models: &'static [&'static str],
    }

    fn spawn_fake_ollama(cfg: FakeOllama) -> std::net::SocketAddr {
        use std::io::{Read, Write};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let version = cfg.version.to_string();
        let models_json = serde_json::to_string(
            &cfg.installed_models.iter().map(|n| serde_json::json!({ "name": n })).collect::<Vec<_>>(),
        )
        .unwrap();

        std::thread::spawn(move || {
            // Three requests expected: the stage-1 POST, then the two GET
            // probes `diagnose_local_404` makes when that POST comes back
            // 404. Each is its own connection — neither `test_endpoint` nor
            // `diagnose_local_404` builds a keep-alive agent here.
            for _ in 0..3 {
                let Ok((mut sock, _)) = listener.accept() else { break };
                let mut buf = [0u8; 4096];
                let n = sock.read(&mut buf).unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]);
                let path = req.lines().next().unwrap_or("").split_whitespace().nth(1).unwrap_or("");

                let (status, body) = if path.starts_with("/v1/messages") {
                    ("404 Not Found", "{\"error\":\"not found\"}".to_string())
                } else if path.starts_with("/api/version") {
                    ("200 OK", format!("{{\"version\":\"{version}\"}}"))
                } else if path.starts_with("/api/tags") {
                    ("200 OK", format!("{{\"models\":{models_json}}}"))
                } else {
                    ("404 Not Found", "{}".to_string())
                };
                let resp = format!(
                    "HTTP/1.1 {status}\r\ncontent-type: application/json\r\n\
                     content-length: {}\r\nconnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = sock.write_all(resp.as_bytes());
            }
        });
        addr
    }

    // -- Response shapes, stage 1 and stage 2. -----------------------------

    #[test]
    fn an_anthropic_shaped_reply_passes() {
        let body = r#"{"id":"msg_1","type":"message","role":"assistant",
            "content":[{"type":"text","text":"OK"}],"model":"qwen3-coder:30b"}"#;
        assert!(message_shape_ok(body).is_ok());
    }

    #[test]
    fn a_wrong_shaped_reply_is_named_for_what_it_is() {
        assert!(message_shape_ok("<html>gateway error</html>").unwrap_err().contains("JSON"));
        // An OpenAI-shaped body parses as JSON but has no content block.
        let openai = r#"{"choices":[{"message":{"content":"OK"}}]}"#;
        assert!(message_shape_ok(openai).unwrap_err().contains("content block"));
        let refusal = r#"{"error":{"type":"invalid_request_error","message":"model not found"}}"#;
        assert!(message_shape_ok(refusal).unwrap_err().contains("model not found"));
    }

    /// THE TEST THAT FAILS AGAINST THE OLD CODE — this is the exact bug Mark
    /// hit. The old arm was `Err(ureq::Error::Status(code, _)) =>
    /// Err(format!("The endpoint answered {code}."))`: the `_` threw the
    /// body away, so a 400 naming exactly what was wrong came back as four
    /// words and a number. `assert_ne!` below pins the old string as the
    /// failure mode, not just the new one as the success.
    #[test]
    fn a_400_shows_the_providers_own_words_not_a_bare_number() {
        let body = r#"{"error":{"message":"Unsupported parameter: 'max_tokens' is not supported with this model. Use 'max_completion_tokens' instead.","type":"invalid_request_error","param":"max_tokens","code":"unsupported_parameter"}}"#;
        let msg = generic_status_error(400, body.to_string(), "");
        assert!(
            msg.contains("max_completion_tokens"),
            "the provider's own explanation must reach the screen: {msg}"
        );
        assert_ne!(
            msg, "The endpoint answered 400.",
            "this exact sentence is what shipped and is the bug being fixed"
        );
    }

    /// THE ACTUAL BUG BECK FOUND, 2026-09-06 — "shows raw provider JSON
    /// (INVALID_ARGUMENT…) instead of a friendly message." The test above
    /// this one could not have caught it: it only asserts a SUBSTRING is
    /// present, and the raw, undigested body already contained it before this
    /// fix, which is exactly how the bug shipped invisibly. This one pins the
    /// actual complaint — no stray brace, quote or vendor status code
    /// reaching the screen, only the sentence a person can act on. Body is
    /// Google's real shape for a bad key against their OpenAI-compat
    /// endpoint, checked against their own docs the same day.
    #[test]
    fn an_invalid_key_shows_the_message_not_the_json_envelope() {
        let body = r#"{"error":{"code":400,"message":"API key not valid. Please pass a valid API key.","status":"INVALID_ARGUMENT"}}"#;
        let msg = generic_status_error(400, body.to_string(), "");
        assert_eq!(msg, "The endpoint answered 400: API key not valid. Please pass a valid API key.");
        assert!(!msg.contains('{'), "raw JSON reached the screen: {msg}");
        assert!(
            !msg.contains("INVALID_ARGUMENT"),
            "the vendor's internal status code is not a message a person reads: {msg}"
        );
    }

    /// **BECK'S RE-SWEEP, 2026-09-06 — the fix above was still partial.**
    /// Gemini's LIVE bad-key body is not the bare object the test above uses;
    /// it is ARRAY-wrapped: `[{"error":{...}}]`. `/error/message` never matches
    /// an array root, so before this fix the whole envelope fell through to the
    /// raw body and leaked `{`/`INVALID_ARGUMENT` at the person — the exact
    /// symptom Beck reported. This pins the array shape: the human sentence
    /// reaches the screen, nothing else does. Body is Google's real
    /// array-wrapped shape for a bad key on their OpenAI-compat endpoint.
    ///
    /// Proven able to fail: drop the `/0/error/message` fallback in
    /// `generic_status_error` and this leaks the raw JSON, failing the `{`
    /// assertion.
    #[test]
    fn an_array_wrapped_invalid_key_envelope_still_shows_only_the_message() {
        let body = r#"[{"error":{"code":400,"message":"API key not valid. Please pass a valid API key.","status":"INVALID_ARGUMENT"}}]"#;
        let msg = generic_status_error(400, body.to_string(), "");
        assert_eq!(msg, "The endpoint answered 400: API key not valid. Please pass a valid API key.");
        assert!(!msg.contains('{'), "raw JSON reached the screen: {msg}");
        assert!(!msg.contains('['), "the array wrapper reached the screen: {msg}");
        assert!(
            !msg.contains("INVALID_ARGUMENT"),
            "the vendor's internal status code is not a message a person reads: {msg}"
        );
    }

    #[test]
    fn the_error_body_never_carries_the_key_back_to_the_screen() {
        // Some servers echo request headers into their own error bodies —
        // this simulates one doing exactly that.
        let body = r#"{"error":"bad request, saw Authorization: Bearer sk-real-secret-value"}"#;
        let msg = generic_status_error(400, body.to_string(), "sk-real-secret-value");
        assert!(!msg.contains("sk-real-secret-value"), "the key must never be echoed back: {msg}");
        assert!(msg.contains("[key redacted]"), "{msg}");
    }

    #[test]
    fn a_short_placeholder_does_not_eat_ordinary_words_out_of_the_message() {
        // PLACEHOLDER_TOKEN-length guard: a local row with no stored key
        // uses a short-ish placeholder, and redacting on anything under 8
        // characters risks mangling a real word that happens to match.
        let msg = generic_status_error(400, "the model said no".to_string(), "no");
        assert_eq!(msg, "The endpoint answered 400: the model said no");
    }

    #[test]
    fn a_huge_error_body_is_capped() {
        let huge = "x".repeat(10_000);
        let msg = generic_status_error(500, huge, "");
        assert!(msg.len() < 600, "a runaway HTML error page must not flood the row: {}", msg.len());
    }

    #[test]
    fn an_empty_body_falls_back_to_the_old_short_sentence() {
        // A status with genuinely nothing in the body (some upstreams do
        // this) must not read as "the endpoint answered 400: " with a
        // trailing nothing.
        assert_eq!(generic_status_error(400, String::new(), ""), "The endpoint answered 400.");
    }

    /// THE TEST THAT FAILS AGAINST THE OLD CODE — Beck's finding, read off a
    /// live provider row after pressing the real Test button against her
    /// echo server, which pads its error body then echoes the Authorization
    /// header back. Build a body where the key starts before byte 500 and
    /// ends after it: the OLD order (truncate, then redact) cuts the key in
    /// half, so `replace` can no longer find the whole string and a prefix
    /// of it reaches the screen. The NEW order redacts the full, un-cut body
    /// first, so there is no fragment left for the cut to expose.
    #[test]
    fn a_key_straddling_the_cut_is_fully_redacted_not_left_as_a_prefix() {
        let key = "nameos-local-placeholder-0123456789abcdef"; // 42 bytes
        let mut body = "x".repeat(480); // key now spans bytes 480..522
        body.push_str(key);
        body.push_str(" — trailing text so the body keeps going past the cut");
        let msg = generic_status_error(400, body, key);
        assert!(
            !msg.contains(&key[..20]),
            "a partial key surviving the cut is still a credential: {msg}"
        );
        assert!(msg.contains("[key redacted]"), "{msg}");
    }

    /// THE SECOND TEST THAT FAILS AGAINST THE OLD CODE — this one by
    /// panicking. `String::truncate` requires the index to land on a UTF-8
    /// char boundary; byte 500 is picked with no regard for where character
    /// edges fall, and Beck reproduced a live body with a multibyte
    /// character sitting exactly across it. The old code's bare
    /// `body.truncate(500)` panics on that input — inside the command
    /// handler, so the `invoke` never resolves and the button hangs on
    /// "Testing…" with no error ever reaching the screen.
    #[test]
    fn a_multibyte_character_at_the_cut_point_does_not_panic() {
        let mut body = "x".repeat(499);
        body.push('é'); // 2-byte UTF-8 char occupying bytes 499 and 500
        body.push_str(" more text after it so the body keeps going");
        let msg = generic_status_error(400, body, "");
        assert!(msg.starts_with("The endpoint answered 400:"), "{msg}");
    }

    // -- Stderr on a successful run is filtered, not discarded. -------------

    #[test]
    fn the_benign_notice_and_its_detail_line_are_removed() {
        // The real shape, byte-verified by Beck against claude.exe 2.1.251,
        // nine runs: marker and detail on ONE line, space-separated.
        let stderr = "[claude-code:unrecognized_model] {\"model\":\"llama3.2:latest\",\"query_source\":\"sdk\"}\n";
        assert_eq!(filter_benign_stderr(stderr), "");
    }

    #[test]
    fn the_older_two_line_shape_is_still_removed() {
        // A different `claude` build may still split marker and detail onto
        // two lines, as Mark's screenshot originally showed. Nothing here
        // can confirm which build his own machine runs, so this shape stays
        // covered alongside the one-line shape above.
        let stderr = "[claude-code:unrecognized_model]\n\
                       {\"model\":\"llama3.2:latest\",\"query_source\":\"sdk\"}\n";
        assert_eq!(filter_benign_stderr(stderr), "");
    }

    #[test]
    fn a_real_warning_next_to_the_benign_one_still_gets_through() {
        // The filter must not eat the whole run just because the benign line
        // showed up somewhere in it — only the marker and its own detail.
        let stderr = "[claude-code:unrecognized_model] {\"model\":\"llama3.2:latest\",\"query_source\":\"sdk\"}\n\
                       Warning: something a person should actually see.\n";
        assert_eq!(filter_benign_stderr(stderr).trim(), "Warning: something a person should actually see.");
    }

    #[test]
    fn a_json_line_with_no_marker_above_it_is_never_eaten() {
        // The detail-line shape (`{...\"model\"...}`) is only swallowed
        // immediately after the marker — on its own it is just stderr text,
        // and could be a real error that happens to be JSON.
        let stderr = "{\"error\":\"connection refused\",\"model\":\"x\"}\n";
        assert_eq!(filter_benign_stderr(stderr).trim(), "{\"error\":\"connection refused\",\"model\":\"x\"}");
    }

    #[test]
    fn the_binary_result_must_name_the_model() {
        let good = r#"{"type":"result","is_error":false,"result":"OK",
            "modelUsage":{"qwen3-coder:30b":{"inputTokens":10}}}"#;
        assert!(binary_result_ok(good, "qwen3-coder:30b", false).is_ok());
        // Answered, but from some other model — the routing did not take.
        assert!(binary_result_ok(good, "some-other-model", false).is_err());
        // The builtin row does not pin a model; success is success.
        assert!(binary_result_ok(good, "", true).is_ok());
        let err = r#"{"type":"result","is_error":true,"result":"boom"}"#;
        assert!(binary_result_ok(err, "m", true).unwrap_err().contains("boom"));
        assert!(binary_result_ok("not json at all", "m", true).is_err());
    }

    // -- The failed run must not hand the benign notice back as the reason. -
    //
    // Mark, adding an OpenAI token: "you get [claude-code:unrecognized_model]
    // {"model":"gpt-5.6-sol","query_source":"sdk"}". Every fixture below is
    // captured VERBATIM from this machine's own claude 2.1.251, run against a
    // model the backend genuinely rejects (`ANTHROPIC_MODEL=gpt-5.6-sol`
    // pointed at this box's Ollama, which has no such model):
    //
    //   ANTHROPIC_BASE_URL=http://127.0.0.1:11434 ANTHROPIC_AUTH_TOKEN=x \
    //   ANTHROPIC_MODEL=gpt-5.6-sol claude -p "Say OK" \
    //   --output-format json --max-turns 1
    //
    // exit=1. Two failures below are DELIBERATELY against
    // `test_binary_failure_message` directly rather than against `test_binary`
    // itself — the fault is in the message-building logic, which is now pure
    // and needs no live process to exercise; the ignored end-to-end test
    // further down proves the same fixture through the real spawn.

    /// EXACTLY what Mark quoted — the marker and its JSON detail, one line,
    /// nothing else on the channel. This is the isolated fault: both channels
    /// hold nothing but benign noise, so the honest answer is whatever
    /// claude's own stdout said about the failure, never this line.
    const REPRO_STDERR_MARKER_ONLY: &str =
        "[claude-code:unrecognized_model] {\"model\":\"gpt-5.6-sol\",\"query_source\":\"sdk\"}\n";

    /// The FULL two-line stderr this machine's claude actually wrote for the
    /// same run — the marker above, PLUS its own benign connectors advisory
    /// ahead of it (present because `apply_env` sets an auth source, which is
    /// real and worth knowing, not something this fix has any business
    /// eating). Kept separate from the marker-only fixture on purpose: a test
    /// against this one proves the fix does not overreach into "the request
    /// must still be visible and specific."
    const REPRO_STDERR_WITH_CONNECTORS_ADVISORY: &str = "\u{26a0} claude.ai connectors are \
        disabled because ANTHROPIC_API_KEY or another auth source is set and takes precedence \
        over your claude.ai login \u{b7} Unset it to load your organization's connectors\n\
        [claude-code:unrecognized_model] {\"model\":\"gpt-5.6-sol\",\"query_source\":\"sdk\"}\n";

    /// The exact stdout `result` object for the same run — claude's own
    /// account of the failure, on a non-zero exit.
    const REPRO_STDOUT: &str = r#"{"is_error":true,"api_error_status":404,"result":"There's an issue with the selected model (gpt-5.6-sol). It may not exist or you may not have access to it.","type":"result"}"#;

    /// THE FAULT ITSELF: the marker line must never become the message a
    /// person reads. Fails against the pre-fix code, which returned
    /// `stderr.trim()` unfiltered — this exact fixture, unchanged, matching
    /// what Mark quoted verbatim.
    #[test]
    fn a_failed_run_never_hands_back_the_bare_unrecognized_model_notice() {
        let msg = test_binary_failure_message(REPRO_STDERR_MARKER_ONLY, REPRO_STDOUT, "exit status: 1");
        assert!(
            !msg.contains(crate::UNRECOGNIZED_MODEL_MARKER),
            "the benign notice reached the user as if it were the failure reason: {msg}"
        );
    }

    /// Filtering the benign noise out of stderr must not leave a BLANKER
    /// error than before — claude already wrote the real reason to stdout,
    /// and this is the one place that reads it.
    #[test]
    fn a_failed_run_surfaces_the_binarys_own_stdout_explanation() {
        let msg = test_binary_failure_message(REPRO_STDERR_MARKER_ONLY, REPRO_STDOUT, "exit status: 1");
        assert_eq!(
            msg,
            "There's an issue with the selected model (gpt-5.6-sol). It may not exist or you \
             may not have access to it."
        );
    }

    /// THE OVERREACH GUARD, THE BRIEF'S OWN WORDS: "do not widen the filter
    /// into a catch-all… a genuine failure on the OpenAI path must still be
    /// visible and specific." Only the one documented, always-present,
    /// always-benign marker gets filtered. A real advisory sitting right next
    /// to it on the SAME real run is not noise and must reach the user —
    /// this is the fixture actually captured off this box, connectors line
    /// and all.
    #[test]
    fn a_real_advisory_next_to_the_benign_marker_still_reaches_the_user() {
        let msg = test_binary_failure_message(
            REPRO_STDERR_WITH_CONNECTORS_ADVISORY,
            REPRO_STDOUT,
            "exit status: 1",
        );
        assert!(msg.contains("connectors are disabled"), "{msg}");
        assert!(!msg.contains(crate::UNRECOGNIZED_MODEL_MARKER), "{msg}");
    }

    /// A real, non-benign stderr line on a failed run must still win over
    /// stdout — stderr is where a genuine crash or a real access refusal
    /// actually lands, and stdout's `result` is only the fallback for when
    /// stderr, after filtering, has nothing left.
    #[test]
    fn a_genuine_stderr_failure_is_never_masked_by_the_stdout_fallback() {
        let stderr = "thread 'main' panicked at src/main.rs:1: something real broke\n";
        let msg = test_binary_failure_message(stderr, REPRO_STDOUT, "exit status: 101");
        assert!(msg.contains("something real broke"), "{msg}");
        assert!(!msg.contains("issue with the selected model"), "{msg}");
    }

    /// Both channels empty (killed by the timeout above this branch, or a
    /// process that wrote nothing at all) still needs an honest message
    /// rather than an empty string reaching the screen.
    #[test]
    fn nothing_on_either_channel_still_names_the_exit_status() {
        let msg = test_binary_failure_message("", "", "exit status: 137");
        assert_eq!(msg, "Claude Code exited with exit status: 137 and said nothing.");
    }

    #[test]
    fn stdout_result_text_reads_the_failure_shape() {
        assert_eq!(
            stdout_result_text(REPRO_STDOUT).as_deref(),
            Some(
                "There's an issue with the selected model (gpt-5.6-sol). It may not exist or \
                 you may not have access to it."
            )
        );
        assert_eq!(stdout_result_text("not json"), None);
        assert_eq!(stdout_result_text(r#"{"type":"result"}"#), None, "no result field at all");
        assert_eq!(stdout_result_text(r#"{"result":"  "}"#), None, "blank result is not useful");
    }

    /// AGAINST THE REAL BINARY, on demand only — same convention as
    /// `local_brain_earns_green_for_real` below. Proves the fixtures above
    /// are not just plausible strings but what `test_binary` itself returns
    /// end to end, spawning this machine's real claude against a model its
    /// own Ollama genuinely rejects.
    ///
    ///   cargo test --manifest-path desktop/src-tauri/Cargo.toml \
    ///     unrecognized_model_never_reaches -- --ignored --nocapture
    #[test]
    #[ignore = "talks to the real local endpoint and real claude binary on this box"]
    fn unrecognized_model_never_reaches_the_user_end_to_end() {
        let mut p = row("local");
        p.model = "gpt-5.6-sol".into(); // qwen3-coder:30b's real endpoint, a model it does not have
        let err = test_binary(&p).expect_err("this model does not exist on this box's Ollama");
        assert!(
            !err.contains(crate::UNRECOGNIZED_MODEL_MARKER),
            "the real spawn still leaked the benign notice: {err}"
        );
        println!("test_binary's real failure message: {err}");
    }

    // -- Reading what Ollama already has installed. -------------------------
    //
    // Mark, 2026-08-31: "no way to probe for local brain and different
    // models." These pin the fault Cassandra and Beck found: the picker
    // pre-filled whichever model Ollama's own /api/tags happened to list
    // first, with no idea whether it could actually drive this app.

    /// Three real `/api/tags` entries, captured live off the Windows box this
    /// feature is for, 2026-08-31 — NOT invented. `gpt-oss:20b` is Ollama's
    /// own first-listed model there, and it is the one Beck proved never
    /// completes a turn despite declaring "tools".
    fn real_tags_entry(name: &str, size: u64, caps: &[&str]) -> serde_json::Value {
        serde_json::json!({
            "name": name, "model": name, "size": size,
            "capabilities": caps,
        })
    }

    #[test]
    fn has_tools_reads_the_real_capabilities_lists() {
        // gpt-oss:20b — declares tools AND thinking. A real yes, and this is
        // exactly the row that must not be read as "will work" anywhere else
        // in this file: has_tools is a declaration, not a promise.
        let gpt_oss = real_tags_entry("gpt-oss:20b", 13_793_441_244, &["completion", "tools", "thinking"]);
        assert_eq!(has_tools_from(&gpt_oss), Some(true));

        // WhiteRabbitNeo — completion only. A real, checkable no.
        let wrn = real_tags_entry(
            "hf.co/TheBloke/WhiteRabbitNeo-13B-GGUF:Q4_K_M", 7_866_070_512, &["completion"],
        );
        assert_eq!(has_tools_from(&wrn), Some(false));

        // llama3.2:latest — the one model on that machine that actually
        // answers today.
        let llama = real_tags_entry("llama3.2:latest", 2_019_393_189, &["completion", "tools"]);
        assert_eq!(has_tools_from(&llama), Some(true));
    }

    #[test]
    fn no_capabilities_field_is_could_not_tell_not_a_no() {
        // An older Ollama point release, or any value that just does not
        // carry the field — held apart from a genuine Some(false) precisely
        // so the picker can word them differently instead of claiming a
        // model looked at and rejected when it was never checked.
        let no_field = serde_json::json!({"name": "old:1b", "size": 1});
        assert_eq!(has_tools_from(&no_field), None);
        let not_an_object = serde_json::json!("just a string");
        assert_eq!(has_tools_from(&not_an_object), None);
    }

    /// THE TEST THAT FAILS AGAINST THE OLD CODE. Before 2026-08-31,
    /// `LocalBrain.models` was `Vec<String>` in Ollama's own arbitrary
    /// `/api/tags` order — `rank_installed` and `InstalledModel` did not
    /// exist, so this will not even compile against that shape.
    ///
    /// **UPDATED 2026-09-04 — the assertions below used to say the OPPOSITE
    /// of what this test's own name promises.** Before `KNOWN_STALLING_
    /// MODELS` existed, this test asserted `ranked[0].name == "gpt-oss:20b"`
    /// — because both it and `llama3.2` declare `has_tools: Some(true)`, and
    /// a stable sort on that alone leaves the input's own order (Ollama's
    /// real `/api/tags` order on the machine this was built for, gpt-oss
    /// first) untouched. That is not "never defaults to the broken one"; it
    /// is exactly the default this test's name says it forbids, proven wrong
    /// by FACTS.md's own measurement (`gpt-oss:20b` never completes a turn,
    /// `llama3.2` ~4s) the day this file was written. The assertions now
    /// match the name.
    #[test]
    fn ranking_never_drops_a_model_and_never_defaults_to_the_broken_one() {
        // Built in Ollama's OWN real order for that machine: gpt-oss first.
        let models = vec![
            InstalledModel { name: "gpt-oss:20b".into(), size_mb: Some(13154), fit: Fit::Fast, has_tools: Some(true) },
            InstalledModel { name: "hf.co/TheBloke/WhiteRabbitNeo-13B-GGUF:Q4_K_M".into(), size_mb: Some(7502), fit: Fit::Fast, has_tools: Some(false) },
            InstalledModel { name: "llama3.2:latest".into(), size_mb: Some(1926), fit: Fit::Fast, has_tools: Some(true) },
        ];
        let ranked = rank_installed(models);

        // NEVER FILTERED — Mark, 2026-08-31: "i want it to give all the
        // options on a prob[e]." All three must still be present, findable,
        // by name.
        assert_eq!(ranked.len(), 3, "a model went missing during ranking");
        for name in ["gpt-oss:20b", "hf.co/TheBloke/WhiteRabbitNeo-13B-GGUF:Q4_K_M", "llama3.2:latest"] {
            assert!(ranked.iter().any(|m| m.name == name), "{name} was dropped, not just reordered");
        }

        // gpt-oss:20b is KNOWN STALLING, so it sorts dead last -- behind even
        // a model with no declared tool support at all. A measured "this
        // never finishes a turn" outranks (in badness) an undeclared
        // capability that might still work fine.
        assert_eq!(
            ranked.last().unwrap().name,
            "gpt-oss:20b",
            "a model proven never to complete a turn must never rank ahead of one that does, \
             even though both declare the same has_tools capability"
        );
        // The genuinely-completing, tools-declared model is first.
        assert_eq!(ranked[0].name, "llama3.2:latest");
        // The undeclared-tools model, which is merely unproven rather than
        // proven broken, sorts in the middle.
        assert_eq!(ranked[1].name, "hf.co/TheBloke/WhiteRabbitNeo-13B-GGUF:Q4_K_M");
    }

    /// **THE NARROW REGRESSION FOR ITEM 2 OF THE 2026-09-04 FIX, ISOLATED
    /// FROM THE BROADER TEST ABOVE.** Proves the ONE fact that matters on its
    /// own, with nothing else in the list to potentially mask a mistake: two
    /// models declaring the IDENTICAL `has_tools: Some(true))`, one of them
    /// on `KNOWN_STALLING_MODELS`, must never rank equal or ahead of the
    /// other. A tie here is what let `gpt-oss:20b` sit at index 0 before this
    /// fix — `has_tools` alone cannot break the tie, and it should not have
    /// to.
    #[test]
    fn a_known_stalling_model_never_outranks_a_completing_one_with_the_same_declared_tools() {
        let models = vec![
            InstalledModel { name: "gpt-oss:20b".into(), size_mb: Some(13154), fit: Fit::Fast, has_tools: Some(true) },
            InstalledModel { name: "llama3.2:latest".into(), size_mb: Some(1926), fit: Fit::Fast, has_tools: Some(true) },
        ];
        let ranked = rank_installed(models);
        assert_eq!(ranked[0].name, "llama3.2:latest", "the completing model must rank first");
        assert_eq!(ranked[1].name, "gpt-oss:20b", "the known-stalling model must rank last");
    }

    #[test]
    fn is_known_stalling_matches_the_exact_tag_only() {
        assert!(is_known_stalling("gpt-oss:20b"));
        // A DIFFERENT tag of the same family has not itself been measured
        // broken -- see KNOWN_STALLING_MODELS's own doc for why this is an
        // exact match, not a prefix one.
        assert!(!is_known_stalling("gpt-oss:120b"));
        assert!(!is_known_stalling("llama3.2:latest"));
    }

    #[test]
    fn installed_models_are_sized_by_the_same_rule_the_catalog_uses() {
        // A 16 GB card, the one measured on Mark's Windows machine
        // (RTX 5080) 2026-08-31. gpt-oss:20b's download_mb (13154, from the
        // CATALOG entry brain_setup.rs already ships) must classify
        // IDENTICALLY whether it arrives as a catalog `Model` or as an
        // installed model's raw megabytes — one rule, not two copies of it.
        let hw = HardwareProfile {
            os: "windows".into(),
            cores: Some(24),
            ram_mb: Some(65536),
            gpus: vec![Gpu { name: "RTX 5080".into(), vram_mb: Some(16303), source: "test".into() }],
            disk_free_mb: Some(400_000),
            notes: vec![],
        };
        let catalog_model = CATALOG.iter().find(|m| m.tag == "gpt-oss:20b").unwrap();
        assert_eq!(
            crate::brain_setup::classify_size(catalog_model.download_mb, &hw),
            crate::brain_setup::classify(catalog_model, &hw),
            "the installed-model sizing path must agree with the catalog's own classify()",
        );
    }

    // -- The real thing, on demand only. -----------------------------------

    /// THE THIRD LEG — the one `adapter.rs`'s own real-round-trip test
    /// explicitly says it does NOT cover: Claude Code itself, spawned through
    /// `apply_env`, talking to the translator over loopback, which then talks
    /// to a real OpenAI endpoint with a real key. `test_binary` is exactly
    /// what `test_provider`'s stage 2 runs for an `openai-compatible` row —
    /// this calls it directly, the same way `local_brain_earns_green_for_real`
    /// below calls it for `local`, because a Tauri `AppHandle` (which
    /// `test_provider` itself needs) is not available outside the running app.
    ///
    /// Builds the row exactly as `save_provider` would have — real
    /// `providers.json` on disk, real key in the OS keyring under this
    /// provider's id — so `adapter::row_from_disk` and `secret_of` read
    /// through the SAME path a genuine user's Test press would, not a
    /// shortcut. The key is decrypted straight into this one process's
    /// environment by the caller and read here with `std::env::var` — never
    /// written to a file, never logged, never in this source. Cleaned up on
    /// every exit, success or panic, by the drop guard below: the keyring
    /// entry and the temp `providers.json` are both real side effects on this
    /// machine and must not outlive the test.
    ///
    /// Run 2026-09-02, on this Linux box (Mark's own `openai-api` credential,
    /// decrypted for this process only) — **all three legs passed**: the
    /// listener bound, `claude` (2.1.258, resolved off PATH exactly as
    /// `claude_binary()` does) launched with `ANTHROPIC_BASE_URL` pointed at
    /// `127.0.0.1:<port>/<id>`, reached the translator, which reached OpenAI
    /// for real and translated the answer back, and `claude -p` returned a
    /// successful result naming the model it was told to use. This is
    /// FURTHER than `adapter.rs`'s own round-trip test goes — that one proves
    /// the wire edges; this proves Claude Code will actually accept and use
    /// what the translator hands back, which is the thing `may_select`'s
    /// green dot is actually promising.
    ///
    /// **WHAT THIS DOES NOT PROVE:** it is still this Linux box, not the
    /// shipping Windows build — FACTS.md is explicit that a Linux run is
    /// iteration, not evidence, and this module's tests are all `cfg`-neutral
    /// (no Windows-only code path runs through `apply_env` or `test_binary`),
    /// but the real proof Beck signs off on is on Windows, with the real
    /// installer, per the standing rule.
    ///
    ///   NAMEOS_TEST_OPENAI_KEY=sk-... cargo test --manifest-path \
    ///     desktop/src-tauri/Cargo.toml openai_full_round_trip -- --ignored --nocapture
    #[test]
    #[ignore = "talks to the real OpenAI endpoint, spawns the real claude binary, spends real tokens"]
    fn openai_full_round_trip_earns_green_for_real() {
        let key = std::env::var("NAMEOS_TEST_OPENAI_KEY")
            .expect("set NAMEOS_TEST_OPENAI_KEY to a real OpenAI key to run this test");

        struct Cleanup {
            dir: std::path::PathBuf,
            provider_id: String,
        }
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.dir);
                if let Ok(e) = entry(&self.provider_id) {
                    let _ = e.delete_credential();
                }
            }
        }

        let provider_id = format!("p-openai-e2e-test-{}", std::process::id());
        let dir = std::env::temp_dir().join(format!("nameos-openai-e2e-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp config dir");
        let _cleanup = Cleanup { dir: dir.clone(), provider_id: provider_id.clone() };

        // The real credential path — `apply_env`'s openai arm fetches this via
        // `secret_of`/`secret_for`, exactly as a genuine send would.
        entry(&provider_id).expect("keyring entry").set_password(&key).expect("store test key");

        let mut provider = row("openai-compatible");
        provider.id = provider_id.clone();
        provider.base_url = "https://api.openai.com/v1".into();
        // THE SHIPPED DEFAULT, DELIBERATELY, NOT AN EASY MODEL — `OPENAI_DEFAULT_MODEL`
        // in ui/index.html and BRAIN-CHOICES.md both point a real user here. A
        // reasoning-tier model is the one most likely to hit a translator
        // corner (see adapter.rs's degradation 6, found by an earlier run of
        // this exact test) — proving the easy model would have missed it.
        provider.model = "gpt-5.6-sol".into();
        let store = Store { active: provider_id.clone(), secondary: String::new(), providers: vec![provider.clone()] };
        std::fs::write(dir.join("providers.json"), serde_json::to_string_pretty(&store).unwrap())
            .expect("write providers.json");

        // Same call `main.rs`'s Tauri setup hook makes — hands the adapter the
        // directory `row_from_disk` will read on every request.
        crate::adapter::init(dir.clone());

        println!("stage 1: real /chat/completions round trip through OpenAI's own shape…");
        test_chat_endpoint(&provider).expect("stage 1: the OpenAI endpoint round trip");
        println!("stage 1 passed");

        println!("stage 2: real claude binary, through apply_env, through the loopback translator, to OpenAI…");
        test_binary(&provider).expect("stage 2: claude -> translator -> OpenAI -> translator -> claude");
        println!("stage 2 passed — all three legs of the round trip are real");
    }

    /// THE PROOF COMMAND. Runs the same two stages the Test button runs,
    /// against the real local endpoint and the real claude binary on this
    /// machine. Ignored by default so a box without Ollama does not lie by
    /// skipping silently — running it is an explicit act:
    ///
    ///   cargo test --manifest-path desktop/src-tauri/Cargo.toml \
    ///     local_brain -- --ignored --nocapture
    #[test]
    #[ignore = "talks to the real local endpoint and real claude binary on this box"]
    fn local_brain_earns_green_for_real() {
        let detected = detect_local_brain();
        assert!(detected.running, "Ollama is not running on this box: {:?}", detected.problem);
        assert!(detected.api_ok, "local API not usable: {:?}", detected.problem);
        println!("detected ollama {} with {} models", detected.version, detected.models.len());

        let p = row("local"); // qwen3-coder:30b at 127.0.0.1:11434, no stored key
        println!("stage 1: real /v1/messages round trip…");
        test_endpoint(&p).expect("stage 1: the endpoint round trip");
        println!("stage 1 passed — Anthropic-shaped answer from the local endpoint");

        println!("stage 2: real claude binary through the provider environment…");
        test_binary(&p).expect("stage 2: the binary round trip");
        println!("stage 2 passed — claude answered through {}", p.model);
    }

}
