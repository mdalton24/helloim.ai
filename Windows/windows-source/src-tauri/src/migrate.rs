//! The one-time migration that retires the "api" connector kind.
//!
//! **THE FAULT THIS FIXES:** an api-kind connector stored a real key in the
//! OS credential store, tested it with a genuine call, and showed an honestly
//! earned green — and then nothing ever used it. `connectors::launch_config`
//! is the only consumer of the connector list and it always skipped api rows.
//! Stored, proven, displayed, never used: a green light on a row that does
//! nothing. API providers belong in providers.rs, where they select the brain
//! and the key genuinely powers something. This module moves them there.
//!
//! **THE CENTRAL DECISION — THE SECRET IS NEVER TOUCHED.** The keyring entry
//! is keyed `(service, account)` where the account is the row's id, so the
//! migrated provider row simply KEEPS the connector's `c`-prefixed id. The
//! safest operation on a customer's credential is the one you never perform:
//! there is no read, no copy, no delete, and therefore no window in which a
//! `set_password` succeeded and a delete failed. "The secret survived" is an
//! identity assertion — same id, same entry — not a promise about a copy,
//! which is a different and much stronger kind of guarantee. The prefix
//! exception this creates is recorded beside `KEYRING_SERVICE` in
//! providers.rs; the property that actually matters, non-collision, survives
//! because connector ids are never reused and the connector row is removed in
//! the same pass that creates the provider row.
//!
//! **GREEN IS NEVER CARRIED ACROSS, and that is the point, not a detail.**
//! The connector's green attested "this key can list models". A provider's
//! green attests "this endpoint can be the brain, through the real binary" —
//! strictly stronger, and not yet earned. Reproducing the old green here
//! would commit the exact fault being fixed, one layer along. Migrated rows
//! arrive untested (amber: key stored, never answered) with a
//! `migration_note` telling the person where the row went and what to do.
//!
//! **THIS MODULE IS UNCHANGED BY THE 2026-08-28 CLOUD REMOVAL, AND THAT IS A
//! DECISION RATHER THAN AN OVERSIGHT — read it before "fixing" the fact that
//! `provider_row_for` still produces `openai-compatible` and
//! `anthropic-compatible`, which `providers.rs` no longer routes.** Those rows
//! land in the store and `normalized()` RETIRES them there: no green, not
//! selectable, not launchable, and a sentence saying what happened and that
//! the key is untouched.
//!
//! **One path, one message, one place.** A person can arrive at a retired
//! cloud brain two ways — a provider row written by yesterday's build, or an
//! api connector migrating for the first time today — and routing both through
//! the same retirement means one explanation to write and one to keep true.
//! Freezing api rows here instead would have been a second message, in a
//! second screen, saying a slightly different thing.
//!
//! It also means this module needs NO work when the kinds come back: the
//! mapping and the credential-preserving id are already right, and they were
//! derived from what each file's test actually called rather than guessed.
//!
//! **WRITE ORDER AND THE CRASH CASE.** providers.json is written FIRST, then
//! connectors.json. A crash between the two leaves both rows sharing an id;
//! the next launch re-runs, finds the provider already present, skips
//! creating a duplicate, and finishes removing the connector row. Idempotent
//! by shape — the trigger is the presence of api rows, not a version flag,
//! because a flag is a thing that can be wrong.

use crate::connectors::Connector;
use crate::providers::{Provider, Store};
use std::path::Path;

/// Run at app setup, before either state's first read of its file. Best
/// effort by design: a failure here leaves the old files in place and the
/// next launch simply tries again — the one thing it can never do is drop a
/// row or touch a credential, because neither operation exists in this
/// module.
pub fn run(app: &tauri::AppHandle) {
    let (Ok(cpath), Ok(ppath)) = (
        crate::connectors::config_path(app),
        crate::providers::config_path(app),
    ) else {
        return;
    };
    run_at(&cpath, &ppath);
}

/// The file-level half, split from `run` so the on-demand keyring test below
/// can drive it against a temp directory without a Tauri app.
fn run_at(cpath: &Path, ppath: &Path) {
    let connectors: Vec<Connector> = std::fs::read_to_string(cpath)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default();
    // The trigger. No api rows — the common case forever after — costs one
    // file read and no writes.
    if !connectors.iter().any(|c| c.kind == "api") {
        return;
    }
    let store: Store = std::fs::read_to_string(ppath)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_else(|| Store { active: "claude".into(), secondary: String::new(), providers: Vec::new() });

    let (kept, store) = split(connectors, store);

    // providers.json FIRST — see the module doc for why this order is the
    // safe one. If it cannot be written, connectors.json is left alone too,
    // and the next launch retries the whole thing.
    let Ok(pjson) = serde_json::to_string_pretty(&store) else { return };
    if std::fs::write(ppath, pjson).is_err() {
        return;
    }
    if let Ok(cjson) = serde_json::to_string_pretty(&kept) {
        let _ = std::fs::write(cpath, cjson);
    }
}

/// The pure core: every decision, no keyring, no filesystem, no app — which
/// is what makes the decisions testable one by one.
fn split(connectors: Vec<Connector>, mut store: Store) -> (Vec<Connector>, Store) {
    let mut kept = Vec::new();
    for mut c in connectors {
        if c.kind != "api" {
            // MCP rows pass through byte-identical. This migration retires a
            // dead branch; it does not refactor a living one.
            kept.push(c);
            continue;
        }
        match provider_row_for(&c) {
            Some(p) => {
                // The crash-rerun case: the provider may already exist from a
                // launch that died between the two writes. Never duplicate —
                // and either way the connector row is dropped, which is the
                // half the crashed run left unfinished.
                if !store.providers.iter().any(|q| q.id == p.id) {
                    store.providers.push(p);
                }
            }
            None => {
                // A provider string this code cannot map — a hand-edited
                // file. Mark's call, taken verbatim into behaviour: better a
                // row visible and unusable with an explanation than a lost
                // credential. The row stays, its green does not: the green
                // meant "the key works", and a frozen row must not keep
                // wearing it.
                c.connected = false;
                c.checked_at = String::new();
                c.last_error = "API providers have moved to the AI components menu. This one \
                     couldn't be moved automatically, so it stays here and its key is \
                     still safely stored. Add it in the AI components menu yourself, or remove \
                     this row."
                    .into();
                kept.push(c);
            }
        }
    }
    (kept, store)
}

/// The mapping, derived from what each file's test actually called — not
/// guessed. The connector test hit `{base}/v1/models`; the provider kinds
/// call `{base}/v1/messages` (anthropic-shaped, same base convention) or
/// `{base}/chat/completions` (openai-shaped, so the equivalent base is the
/// connector's plus `/v1`).
fn provider_row_for(c: &Connector) -> Option<Provider> {
    let (kind, base_url, custom) = match c.provider.as_str() {
        "anthropic" => {
            let base = c.base_url.trim().trim_end_matches('/');
            let base = if base.is_empty() { "https://api.anthropic.com" } else { base };
            ("anthropic-compatible", base.to_string(), false)
        }
        // "custom" maps to openai-compatible deliberately: `test_api` always
        // sent custom rows `Authorization: Bearer` against `/v1/models` — the
        // OpenAI-shape assumption was baked in the day such a row went green.
        // The note says so, and the Test button is what settles it.
        "openai" => ("openai-compatible", openai_base(&c.base_url), false),
        "custom" => ("openai-compatible", openai_base(&c.base_url), true),
        _ => return None,
    };

    let mut note = String::from(
        "Moved here from Apps — API providers now power the brain from this menu. ",
    );
    if c.model.trim().is_empty() {
        note.push_str("Pick the model it should run, then press Test to use it.");
    } else {
        note.push_str("Press Test to use it.");
    }
    if custom {
        note.push_str(
            " It was a custom connector, set up here as an OpenAI-style endpoint; \
             if that is wrong, the test will say so.",
        );
    }

    Some(Provider {
        // THE ID IS THE POINTER TO THE STORED KEY. Keeping it IS the
        // migration of the credential — see the module doc.
        id: c.id.clone(),
        kind: kind.into(),
        name: c.name.clone(),
        base_url,
        model: c.model.trim().to_string(),
        builtin: false,
        disconnected: false,
        // Never carried across. See the module doc.
        connected: false,
        checked_at: String::new(),
        last_error: String::new(),
        has_secret: false, // re-derived from the OS store on every read
        caveat: String::new(), // derived from kind on every read
        migration_note: note,
        // A migrated row is off by default like every other -- see
        // `Provider::allow_shell`'s own doc. Nothing about an old "api"
        // connector implies consent to a shell on the machine it points at.
        allow_shell: false,
        // Same reasoning, for the safe agency layer -- see
        // `Provider::allow_agency`'s own doc. Nothing about an old "api"
        // connector implies consent to open URLs or launch apps either.
        allow_agency: false,
    })
}

/// Empty means OpenAI's own default. Anything else gets `/v1` appended unless
/// it already ends with it — the connector called `{base}/v1/models`, so
/// `{base}/v1` is the same server's OpenAI-convention base, a deterministic
/// transform rather than a guess. (A green connector's base never ended in
/// `/v1` — its test would have called `/v1/v1/models` and failed — but an
/// untested row may have, and doubling it would break a URL we were handed.)
fn openai_base(raw: &str) -> String {
    let t = raw.trim().trim_end_matches('/');
    if t.is_empty() {
        return "https://api.openai.com/v1".into();
    }
    if t.ends_with("/v1") {
        t.into()
    } else {
        format!("{t}/v1")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn api_row(id: &str, provider: &str) -> Connector {
        Connector {
            id: id.into(),
            kind: "api".into(),
            name: "My provider".into(),
            provider: provider.into(),
            base_url: String::new(),
            // A CURRENT MODEL, NOT A DEAD ONE. This is only a fixture — the real
            // migration carries over whatever the person already had (see
            // `model: c.model.trim()` above) and never invents a model — so no
            // user was ever handed this string. It said `gpt-4o` until
            // 2026-08-29, and a fixture naming a retired model is still worth
            // fixing: it is where the next reader looks to learn what we use.
            model: "gpt-5.6-sol".into(),
            command: String::new(),
            args: vec![],
            env_key: String::new(),
            account: String::new(),
            account_key: String::new(),
            url: String::new(),
            needs_token: false,
            header_key: String::new(),
            connected: true, // honestly earned FOR THE KEY — and must not travel
            checked_at: "1724700000".into(),
            last_error: String::new(),
            has_secret: true,
        }
    }

    fn mcp_row(id: &str) -> Connector {
        let mut c = api_row(id, "");
        c.kind = "mcp".into();
        c.command = "npx".into();
        c.args = vec!["-y".into(), "some-server".into()];
        c
    }

    fn empty_store() -> Store {
        Store { active: "claude".into(), secondary: String::new(), providers: Vec::new() }
    }

    // -- A row that migrates. ----------------------------------------------

    /// The identity assertion that replaces a promise about a copy: the
    /// provider row carries the SAME id, so the keyring entry — keyed on the
    /// id — is reachable without ever having been read, copied or deleted.
    #[test]
    fn a_migrated_row_keeps_its_id_which_is_the_credential() {
        let (kept, store) = split(vec![api_row("c123-9", "openai")], empty_store());
        assert!(kept.is_empty(), "the api row must leave connectors");
        let p = store.providers.iter().find(|p| p.id == "c123-9").expect("row migrated");
        assert_eq!(p.kind, "openai-compatible");
        assert_eq!(p.name, "My provider");
        assert_eq!(p.model, "gpt-5.6-sol");
        assert!(!p.migration_note.is_empty(), "the person must be told, not surprised");
    }

    /// THE GREEN NEVER TRAVELS. The connector's green attested "this key can
    /// list models"; a provider's green attests "this endpoint can be the
    /// brain, through the real binary" — strictly stronger, not yet earned.
    #[test]
    fn the_green_is_never_carried_across() {
        let mut c = api_row("c1-1", "anthropic");
        c.connected = true;
        let (_, store) = split(vec![c], empty_store());
        let p = &store.providers[0];
        assert!(!p.connected);
        assert!(p.checked_at.is_empty());
        assert!(!p.builtin);
    }

    #[test]
    fn an_anthropic_row_with_no_base_gets_anthropics_own() {
        let (_, store) = split(vec![api_row("c1-1", "anthropic")], empty_store());
        let p = &store.providers[0];
        assert_eq!(p.kind, "anthropic-compatible");
        assert_eq!(p.base_url, "https://api.anthropic.com");
    }

    /// The /v1 rule is a deterministic transform of what the connector's own
    /// test called, not a guess: `{base}/v1/models` worked means `{base}/v1`
    /// is the OpenAI-convention base.
    #[test]
    fn the_openai_base_rule_is_deterministic() {
        assert_eq!(openai_base(""), "https://api.openai.com/v1");
        assert_eq!(openai_base("https://api.example.com"), "https://api.example.com/v1");
        assert_eq!(openai_base("https://api.example.com/"), "https://api.example.com/v1");
        // Already ends in /v1 — never doubled.
        assert_eq!(openai_base("https://api.example.com/v1"), "https://api.example.com/v1");
        assert_eq!(openai_base("https://api.example.com/v1/"), "https://api.example.com/v1");
    }

    /// "custom" was always tested Bearer-against-/v1/models — the OpenAI
    /// assumption was already baked in, so that is where it lands, said so.
    #[test]
    fn a_custom_row_lands_openai_compatible_and_says_so() {
        let mut c = api_row("c1-1", "custom");
        c.base_url = "https://llm.internal.example".into();
        let (_, store) = split(vec![c], empty_store());
        let p = &store.providers[0];
        assert_eq!(p.kind, "openai-compatible");
        assert_eq!(p.base_url, "https://llm.internal.example/v1");
        assert!(p.migration_note.contains("custom"), "{}", p.migration_note);
    }

    /// A row with no model migrates — the key is the thing being preserved —
    /// and the note tells them the extra step.
    #[test]
    fn an_empty_model_migrates_and_the_note_names_the_step() {
        let mut c = api_row("c1-1", "openai");
        c.model = String::new();
        let (_, store) = split(vec![c], empty_store());
        let p = &store.providers[0];
        assert!(p.model.is_empty());
        assert!(p.migration_note.contains("Pick the model"), "{}", p.migration_note);
    }

    // -- A row that cannot. ------------------------------------------------

    /// Unknown provider string: stays put, keeps its key, loses its green,
    /// and explains itself in plain words. Visible and unusable beats lost.
    #[test]
    fn an_unmappable_row_stays_frozen_with_an_explanation() {
        let mut c = api_row("c1-1", "weirdco");
        c.connected = true;
        let (kept, store) = split(vec![c], empty_store());
        assert!(store.providers.is_empty());
        let row = &kept[0];
        assert_eq!(row.kind, "api", "the row survives");
        assert!(!row.connected, "a frozen row must not wear a green it no longer means");
        assert!(row.last_error.contains("AI components menu"), "{}", row.last_error);
        assert!(row.last_error.contains("still safely stored"), "{}", row.last_error);
    }

    // -- The shapes around the move. ----------------------------------------

    /// MCP rows pass through byte-identical — this is a removal of a dead
    /// branch, not a refactor of a working one.
    #[test]
    fn mcp_rows_are_untouched() {
        let m = mcp_row("cmcp-1");
        let (kept, store) = split(vec![m.clone(), api_row("capi-1", "openai")], empty_store());
        assert_eq!(kept.len(), 1);
        let survivor = &kept[0];
        assert_eq!(survivor.id, m.id);
        assert_eq!(survivor.kind, "mcp");
        assert_eq!(survivor.connected, m.connected, "an mcp row's earned state survives");
        assert_eq!(survivor.args, m.args);
        assert_eq!(store.providers.len(), 1);
    }

    /// The crash-rerun case: providers.json was written, connectors.json was
    /// not. The re-run must not duplicate the provider, and must finish
    /// removing the connector row.
    #[test]
    fn a_rerun_after_a_crash_finishes_without_duplicating() {
        let c = api_row("c1-1", "openai");
        let (_, store_after_first) = split(vec![c.clone()], empty_store());
        // Simulate the crash: connectors.json still holds the api row, while
        // the store already holds the migrated provider.
        let (kept, store) = split(vec![c], store_after_first);
        assert!(kept.is_empty(), "the removal is finished");
        assert_eq!(
            store.providers.iter().filter(|p| p.id == "c1-1").count(),
            1,
            "never a duplicate"
        );
    }

    #[test]
    fn the_active_selection_is_never_touched() {
        let mut store = empty_store();
        store.active = "claude".into();
        let (_, store) = split(vec![api_row("c1-1", "openai")], store);
        assert_eq!(store.active, "claude");
    }

    // -- The structural proof, in the default suite. ------------------------

    /// "NO SECRET IS DROPPED" IS NOT A PROMISE HERE — IT IS A STRUCTURAL
    /// IMPOSSIBILITY, and this test keeps it one. The migration keeps the
    /// row's id, and the id is the keyring account, so the credential is
    /// never read, copied or deleted BECAUSE THE CODE CONTAINS NO CALL THAT
    /// COULD. This asserts that fact against the source itself (the same
    /// pattern local-first-gate uses to prove `ask` is absent): everything
    /// above the test module must be free of credential-store operations.
    /// If this fails, someone has added a keyring call to the migration —
    /// which converts an identity assertion back into a promise about a
    /// copy, and needs the whole design re-argued, not just the test fixed.
    #[test]
    fn the_migration_source_contains_no_credential_store_call() {
        let source = include_str!("migrate.rs");
        let production = source
            .split("#[cfg(test)]")
            .next()
            .expect("the file has a body above the tests");
        // CODE lines only — the comments in this file explain the no-keyring
        // design and are allowed to name the operations it does not perform.
        let code = production
            .lines()
            .filter(|l| {
                let t = l.trim_start();
                !t.starts_with("//") && !t.starts_with("//!") && !t.starts_with("///")
            })
            .collect::<String>();
        for forbidden in ["keyring", "set_password", "get_password", "delete_credential", "secret_for"] {
            assert!(
                !code.contains(forbidden),
                "`{forbidden}` appeared in migrate.rs production code — the migration \
                 must not be able to touch the credential store at all"
            );
        }
    }

    // -- The real thing, on demand only. ------------------------------------

    /// THE NO-SECRET-DROPPED PROOF, against the real OS credential store: a
    /// key stored under the connector's id reads back identical after the
    /// migration, because the migration never touches the store at all —
    /// the id it keeps IS the pointer. On-demand in the house pattern:
    ///
    ///   cargo test --manifest-path desktop/src-tauri/Cargo.toml \
    ///     stored_key_survives -- --ignored --nocapture
    ///
    /// PRODUCTION-IDENTICAL ON PURPOSE: `Entry::new`, the default collection,
    /// the exact path `save_connector` used to store the key and the brain
    /// picker will use to read it. On Windows (Credential Manager) and macOS
    /// (Keychain) this needs nothing special. On Linux it needs an UNLOCKED
    /// login keyring — a normal desktop login has one; a headless session
    /// does not, and this test then fails in its own SETUP with
    /// `IsLocked`/`NoStorageAccess` before the migration ever runs. That is
    /// the environment refusing, not the code failing. The working headless
    /// recipe, PROVEN ON THIS BOX 2026-08-27 (it passed): a private bus with
    /// its own unlocked daemon AND its own keyring store — `XDG_DATA_HOME`
    /// is the part that is easy to miss, and without it the sandboxed daemon
    /// still reads the real user's locked keyring files and the throwaway
    /// password rightly fails against them:
    ///
    ///   XDG_DATA_HOME=$(mktemp -d) dbus-run-session -- sh -c \
    ///     'echo -n testpw | gnome-keyring-daemon --unlock --components=secrets; \
    ///      sleep 1; cargo test stored_key_survives -- --ignored --nocapture'
    ///
    /// (Nothing of the real user's keyring is involved in that sandbox.)
    #[test]
    #[ignore = "touches the real OS credential store on this box"]
    fn the_stored_key_survives_migration_untouched() {
        let id = format!("ctest-migrate-{}", std::process::id());
        let value = "the-exact-key-the-person-pasted";
        keyring::Entry::new(crate::connectors::KEYRING_SERVICE, &id)
            .expect("keyring")
            .set_password(value)
            .expect("store the test key");

        let dir = std::env::temp_dir().join(format!("nameos-migrate-proof-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let cpath = dir.join("connectors.json");
        let ppath = dir.join("providers.json");
        std::fs::write(&cpath, serde_json::to_string_pretty(&vec![api_row(&id, "openai")]).unwrap())
            .expect("write connectors.json");

        run_at(&cpath, &ppath);

        let store: Store =
            serde_json::from_str(&std::fs::read_to_string(&ppath).expect("providers.json written"))
                .expect("providers.json parses");
        assert!(store.providers.iter().any(|p| p.id == id), "row migrated under the same id");
        let kept: Vec<Connector> =
            serde_json::from_str(&std::fs::read_to_string(&cpath).unwrap()).unwrap();
        assert!(kept.iter().all(|c| c.id != id), "api row removed from connectors");

        let read_back = keyring::Entry::new(crate::connectors::KEYRING_SERVICE, &id)
            .expect("keyring")
            .get_password()
            .expect("the key is still there");
        assert_eq!(read_back, value, "byte-identical — nothing was moved, so nothing could drop");

        // Clean up the real store and the temp dir.
        let _ = keyring::Entry::new(crate::connectors::KEYRING_SERVICE, &id)
            .and_then(|e| e.delete_credential());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
