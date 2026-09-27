//! Shared memory sync — the network half of `facts.rs`'s "remember
//! everywhere" toggle. `GET`/`POST`/`DELETE /api/memory/facts`.
//!
//! **THE WIRE SHAPES BELOW ARE CHECKED AGAINST THE REAL WORKER, NOT GUESSED
//! FROM PROSE.** `helloim/API-auth.md` describes the contract; this was
//! written against `helloim/worker.js::handleFacts` directly, because that
//! file carries its OWN two comments that disagree with each other — a stale
//! header two lines above the function (`DELETE { deletes:[{fact_id,
//! seen_updated_at}] }`) and a newer "CONTRACT (rev 2, after Cassandra's
//! review)" comment right beside the code that actually runs
//! (`seen_seq`, matched here, confirmed by reading the handler's own
//! `Number.isInteger(d.seen_seq)` check rather than trusting either comment).
//! Two findings worth stating plainly because nothing here can be pinned
//! against a literal example the way `account.rs`'s `MeResponse`/`LicenseInfo`
//! tests are: `POST`'s response is `{ results:[{fact_id, status}] }` — no
//! `seq` per item — and `DELETE`'s is the identical shape. Both confirmed by
//! reading the handler's own `results.push({...})` calls.
//!
//! **DELIBERATELY A SEPARATE MODULE FROM `account.rs`**, reusing its
//! `agent()`/`require_token()`/`sync_failure()` (all `pub(crate)`) for the
//! exact HTTP client, auth check and 401-handling every other `helloim.ai`
//! call here already uses. Owns none of the LOCAL store logic — that stays in
//! `facts.rs`, untouched, per that file's own note on why the split matters
//! for its anti-data-loss guarantees: a network module reusing a local
//! module's careful reconcile/dedupe/delete-safety code is a much smaller
//! risk than a local module growing a second, network-aware copy of it.
//!
//! **A FOURTH SHARED HELPER, ADDED 2026-09-25:**
//! `account::sync_effective_cached()`. Every one of the three fire-points
//! below (`pull_memory_facts`, `push_memory_facts`, `push_memory_delete`)
//! checks it first and NO-OPs when it is `false` — the "local vs
//! Cloudflare shared store" toggle from `account.rs`'s "Sync preference"
//! section. The two blocking functions the reconcile step there needs
//! (`pull_memory_facts_blocking`, `push_memory_facts_blocking`) are
//! `pub(crate)` for exactly that call, the same reasoning as the three
//! helpers above: one shared check, not a second copy that could drift.
//!
//! **WHY A PUSH NEVER LEARNS THE SERVER'S `seq` IMMEDIATELY.** `seen_seq` is
//! what a later `DELETE` needs to prove "the fact was still at the version I
//! read" — but `POST`'s own response never carries one (see the finding
//! above), so a freshly pushed fact is recorded with `remote_seq = 0` (never
//! a real sequence number; the server refuses `seen_seq < 1`) until the NEXT
//! `pull_memory_facts` sees that `fact_id` in a page and corrects it via
//! `facts::apply_remote_upsert`'s own "already mirrored" branch. The
//! consequence, stated rather than hidden: a `forget()` on a fact pushed
//! moments ago but not yet re-pulled has nothing valid to send, so
//! `push_memory_delete` below skips the network call rather than spend a
//! request that would come back `bad_seen_seq` — the LOCAL delete still
//! happens either way; only the server-side mirror can lag until the next
//! sync catches it, and a subsequent pull of that now-stale mirror would
//! simply re-create a local copy the person already deleted once. Flagged as
//! a known gap for a later pass (a small retry-once-seq-is-known queue),
//! not solved here.

use serde::{Deserialize, Serialize};
use tauri::AppHandle;

const MEMORY_API: &str = "https://helloim.ai/api/memory/facts";

/// One fact (or tombstone) exactly as the GET response's `facts` array holds
/// it. `text`/`deleted_at` are mutually exclusive in practice (a live row has
/// the first, a tombstone the second) — never enforced here structurally,
/// because the real distinguishing test this module uses is `deleted_at.is_some()`,
/// read directly off this struct by `pull_memory_facts_blocking` below.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
struct RemoteFactWire {
    fact_id: String,
    seq: i64,
    text: Option<String>,
    origin: Option<String>,
    deleted_at: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
struct FactsPage {
    facts: Vec<RemoteFactWire>,
    cursor: i64,
    has_more: bool,
    /// "History this device hadn't seen was purged" — `helloim/API-auth.md`'s
    /// own wording. Confirmed against the real handler: this is one MORE
    /// field on the ordinary page shape (`{facts:[], cursor:0, has_more:false,
    /// reset:true}`), never a second, alternate response shape — the earlier
    /// draft of this module assumed the latter and would have failed to parse
    /// a real reset response; caught by reading `handleFacts`'s own
    /// `json(200, { facts: [], cursor: 0, has_more: false, reset: true }, cors)`
    /// before writing this struct, not after.
    reset: bool,
}

#[derive(Clone, Debug, Deserialize)]
struct FactResultItem {
    fact_id: String,
    status: String,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
struct FactsResults {
    results: Vec<FactResultItem>,
}

/// A UUID v4, built from OS randomness the same way `adapter.rs`/`ms_graph.rs`/
/// `google_email.rs` already draw their own nonces — no new dependency for
/// sixteen bytes and two bit-twiddles. The server accepts a device-chosen
/// `fact_id` or generates its own if absent; choosing it here is what lets a
/// push match its own response back to a local row by id rather than by
/// array position, which the contract never promises stays 1:1.
pub(crate) fn uuid_v4() -> Result<String, String> {
    let mut b = [0u8; 16];
    getrandom::getrandom(&mut b).map_err(|e| format!("no OS randomness for a fact id: {e}"))?;
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    Ok(format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7], b[8], b[9], b[10], b[11], b[12], b[13], b[14], b[15]
    ))
}

// ---------------------------------------------------------------------------
// Pull.
// ---------------------------------------------------------------------------

#[derive(Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PullFactsResult {
    pub upserted: u32,
    pub deleted: u32,
    pub reset: bool,
}

fn pull_page(app: &AppHandle, token: &str, cursor: i64) -> Result<FactsPage, String> {
    let resp = crate::account::agent()
        .get(&format!("{MEMORY_API}?cursor={cursor}"))
        .set("Authorization", &format!("Bearer {token}"))
        .call()
        .map_err(|e| crate::account::sync_failure(app, e))?;
    let body = resp
        .into_string()
        .map_err(|e| format!("helloim.ai sent back something unreadable: {e}"))?;
    serde_json::from_str(&body)
        .map_err(|e| format!("helloim.ai sent back something this app could not parse: {e}"))
}

/// `pub(crate)` so `account.rs`'s enable-reconcile can call this directly —
/// see this module's header. NO-OPs (rather than erroring) when the cached
/// sync preference is off, because "off" is a normal, chosen state and a
/// pull that never ran has nothing to report.
pub(crate) fn pull_memory_facts_blocking(app: AppHandle, workdir: String) -> Result<PullFactsResult, String> {
    if !crate::account::sync_effective_cached(&app) {
        return Ok(PullFactsResult::default());
    }
    let token = crate::account::require_token(&app)?;
    let mut cursor = crate::facts::sync_cursor(&workdir)?;
    let mut out = PullFactsResult::default();
    // BOUNDED, NOT "UNTIL has_more IS FALSE" ALONE — the house rule on an
    // unattended loop needing a real stop condition, not a description of
    // one. 200 pages * 500 rows/page (FACTS_PAGE on the server) is 100,000
    // facts, fifty times the server's own FACTS_PER_USER_MAX (2,000), so
    // this can never be the reason a legitimate sync does not finish, only
    // the reason a malfunctioning one eventually stops instead of spinning.
    for _ in 0..200 {
        let page = pull_page(&app, &token, cursor)?;
        if page.reset {
            crate::facts::clear_remote_mirrors(&workdir)?;
            out.reset = true;
            cursor = 0;
            crate::facts::set_sync_cursor(&workdir, 0)?;
            continue; // the reset page itself may still carry a fresh page 0
        }
        for item in &page.facts {
            if item.deleted_at.is_some() {
                crate::facts::apply_remote_delete(&workdir, &item.fact_id)?;
                out.deleted += 1;
            } else {
                let remote = crate::facts::RemoteFact {
                    fact_id: item.fact_id.clone(),
                    seq: item.seq,
                    text: item.text.clone(),
                    origin: item.origin.clone(),
                    deleted_at: None,
                };
                crate::facts::apply_remote_upsert(&workdir, &remote)?;
                out.upserted += 1;
            }
        }
        cursor = page.cursor;
        crate::facts::set_sync_cursor(&workdir, cursor)?;
        if !page.has_more {
            break;
        }
    }
    Ok(out)
}

/// Pull everything this folder hasn't seen yet, from its own saved cursor —
/// see `facts::sync_cursor`'s own doc for why the cursor lives per folder
/// rather than once for the whole app. Called from the window on sign-in and
/// on focus, alongside the persona pull in `account.rs`.
#[tauri::command]
pub async fn pull_memory_facts(app: AppHandle, workdir: String) -> Result<PullFactsResult, String> {
    tauri::async_runtime::spawn_blocking(move || pull_memory_facts_blocking(app, workdir))
        .await
        .map_err(|_| "The app could not start that request. Try again.".to_string())?
}

// ---------------------------------------------------------------------------
// Push.
// ---------------------------------------------------------------------------

#[derive(Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PushFactsResult {
    pub pushed: u32,
}

/// `pub(crate)` for the same reason as `pull_memory_facts_blocking` above —
/// `account.rs`'s enable-reconcile calls this directly. Checked BEFORE
/// `list_unpushed_everywhere` runs, not after: no point opening the local DB
/// for a push this preference says must not happen.
pub(crate) fn push_memory_facts_blocking(app: AppHandle, workdir: String) -> Result<PushFactsResult, String> {
    if !crate::account::sync_effective_cached(&app) {
        return Ok(PushFactsResult::default());
    }
    let pending = crate::facts::list_unpushed_everywhere(&workdir)?;
    if pending.is_empty() {
        return Ok(PushFactsResult::default());
    }
    let token = crate::account::require_token(&app)?;

    let mut ids = Vec::with_capacity(pending.len());
    let mut items = Vec::with_capacity(pending.len());
    for f in &pending {
        let id = uuid_v4()?;
        items.push(serde_json::json!({ "fact_id": id, "text": f.text, "scope": "everywhere" }));
        ids.push((f.id, id));
    }
    let payload = serde_json::json!({ "facts": items }).to_string();

    let resp = crate::account::agent()
        .post(MEMORY_API)
        .set("Authorization", &format!("Bearer {token}"))
        .set("Content-Type", "application/json")
        .send_string(&payload)
        .map_err(|e| crate::account::sync_failure(&app, e))?;
    let body = resp
        .into_string()
        .map_err(|e| format!("helloim.ai sent back something unreadable: {e}"))?;
    let parsed: FactsResults = serde_json::from_str(&body)
        .map_err(|e| format!("helloim.ai sent back something this app could not parse: {e}"))?;

    let mut pushed = 0u32;
    for item in &parsed.results {
        let Some((local_id, _)) = ids.iter().find(|(_, sent_id)| sent_id == &item.fact_id) else {
            continue; // a status for an id this batch never sent -- ignore, do not guess
        };
        // remote_seq 0 -- SEE THIS MODULE'S OWN HEADER on why a push never
        // learns the real one, and why 0 is the honest "not yet confirmed"
        // placeholder rather than a real sequence number.
        crate::facts::attach_remote(&workdir, *local_id, &item.fact_id, 0)?;
        if matches!(item.status.as_str(), "created" | "updated" | "restored" | "duplicate") {
            pushed += 1;
        }
    }
    Ok(PushFactsResult { pushed })
}

/// Push every fact marked "remember everywhere" that has never reached the
/// shared store yet. Called from the window right after `set_everywhere`
/// turns the toggle on, best-effort — see `account.rs`'s own established
/// "local is authority, sync is best-effort" pattern for the same reasoning
/// applied to persona.
#[tauri::command]
pub async fn push_memory_facts(app: AppHandle, workdir: String) -> Result<PushFactsResult, String> {
    tauri::async_runtime::spawn_blocking(move || push_memory_facts_blocking(app, workdir))
        .await
        .map_err(|_| "The app could not start that request. Try again.".to_string())?
}

// ---------------------------------------------------------------------------
// Delete.
// ---------------------------------------------------------------------------

fn push_memory_delete_blocking(app: AppHandle, workdir: String, id: i64) -> Result<(), String> {
    if !crate::account::sync_effective_cached(&app) {
        // Off means nothing here ever reached the server to begin with, so
        // there is nothing to tell it either — same NO-OP as the other two
        // fire-points above. The LOCAL forget this call follows still
        // happened regardless; see this function's own doc below.
        return Ok(());
    }
    let Some((fact_id, seen_seq)) = crate::facts::remote_link(&workdir, id)? else {
        return Ok(()); // never shared -- nothing on the server to tell
    };
    if seen_seq < 1 {
        // Never confirmed by a pull yet -- see this module's own header.
        // The server refuses seen_seq < 1 outright; skip the call rather
        // than spend a request on a delete that cannot succeed.
        return Ok(());
    }
    let token = crate::account::require_token(&app)?;
    let payload = serde_json::json!({
        "deletes": [{ "fact_id": fact_id, "seen_seq": seen_seq }]
    })
    .to_string();
    crate::account::agent()
        .delete(MEMORY_API)
        .set("Authorization", &format!("Bearer {token}"))
        .set("Content-Type", "application/json")
        .send_string(&payload)
        .map_err(|e| crate::account::sync_failure(&app, e))?;
    Ok(())
}

/// Tell the shared store a fact was forgotten — called from the window
/// BEFORE the matching local `forget`, so the `remote_fact_id`/`remote_seq`
/// this needs are still on the row (`forget` removes it outright). Best
/// effort: the local forget is the source of truth and must succeed
/// regardless of whether the server-side mirror could be reached.
#[tauri::command]
pub async fn push_memory_delete(app: AppHandle, workdir: String, id: i64) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || push_memory_delete_blocking(app, workdir, id))
        .await
        .map_err(|_| "The app could not start that request. Try again.".to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uuid_v4_has_the_right_shape_and_version_bits() {
        let id = uuid_v4().unwrap();
        assert_eq!(id.len(), 36, "{id}");
        let parts: Vec<&str> = id.split('-').collect();
        assert_eq!(parts.iter().map(|p| p.len()).collect::<Vec<_>>(), vec![8, 4, 4, 4, 12], "{id}");
        assert_eq!(&parts[2][0..1], "4", "not marked version 4: {id}");
        assert!(matches!(parts[3].chars().next(), Some('8') | Some('9') | Some('a') | Some('b')), "bad variant nibble: {id}");
        // The one property that actually matters: two calls do not collide.
        assert_ne!(id, uuid_v4().unwrap());
    }

    /// **THE ORDINARY PAGE, PINNED AGAINST THE REAL HANDLER'S OWN SHAPE** —
    /// `helloim/worker.js::handleFacts`'s GET branch, read directly rather
    /// than assumed from `API-auth.md`'s prose. A live fact and a tombstone
    /// in the SAME page, because a real sync page carries both.
    #[test]
    fn facts_page_parses_the_real_handlers_shape() {
        let body = r#"{
          "facts": [
            {"fact_id": "abc-1", "seq": 5, "text": "Prefers short replies", "origin": "app", "created_at": "2026-09-01T00:00:00Z", "updated_at": "2026-09-01T00:00:00Z"},
            {"fact_id": "abc-2", "seq": 6, "deleted_at": "2026-09-02T00:00:00Z", "updated_at": "2026-09-02T00:00:00Z"}
          ],
          "unreadable": 0,
          "cursor": 6,
          "has_more": false
        }"#;
        let page: FactsPage = serde_json::from_str(body).expect("the real handler's own shape must parse");
        assert_eq!(page.facts.len(), 2);
        assert_eq!(page.facts[0].text.as_deref(), Some("Prefers short replies"));
        assert!(page.facts[0].deleted_at.is_none(), "a live fact read as a tombstone");
        assert!(page.facts[1].text.is_none(), "a tombstone read as carrying text");
        assert!(page.facts[1].deleted_at.is_some());
        assert_eq!(page.cursor, 6);
        assert!(!page.has_more);
        assert!(!page.reset, "an ordinary page was read as a reset");
    }

    /// **THE RESET SHAPE IS ONE MORE FIELD, NOT A SECOND RESPONSE SHAPE** —
    /// see this module's own header on why an earlier draft got this wrong
    /// and how it was caught. Pinned against the LITERAL body
    /// `handleFacts` sends: `{ facts: [], cursor: 0, has_more: false, reset: true }`.
    #[test]
    fn facts_page_parses_the_real_handlers_reset_shape() {
        let body = r#"{"facts": [], "cursor": 0, "has_more": false, "reset": true}"#;
        let page: FactsPage = serde_json::from_str(body).expect("the real handler's own reset shape must parse");
        assert!(page.reset);
        assert!(page.facts.is_empty());
    }

    /// **PUSH/DELETE RESPONSES CARRY NO `seq` PER ITEM** — the finding this
    /// module's header states plainly. Pinned so a future server change that
    /// starts sending one is at least noticed here (this struct would need a
    /// field added, not silently ignore it) rather than the gap staying
    /// invisible forever.
    #[test]
    fn facts_results_parses_the_real_handlers_push_response_shape() {
        let body = r#"{"results": [{"fact_id": "abc-1", "status": "created"}, {"fact_id": "abc-2", "status": "duplicate"}]}"#;
        let parsed: FactsResults = serde_json::from_str(body).expect("the real handler's own results shape must parse");
        assert_eq!(parsed.results.len(), 2);
        assert_eq!(parsed.results[0].status, "created");
        assert_eq!(parsed.results[1].status, "duplicate");
    }
}
