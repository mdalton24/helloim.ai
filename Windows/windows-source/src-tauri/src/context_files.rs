//! Context files — a local document the person adds to a working folder so
//! the assistant can read it, the same way it already reads everything else
//! sitting in a trusted folder.
//!
//! **WHY THIS IS A COPY, NOT A NEW STORE.** The product's whole pitch is
//! "point it at a folder" — `engine/native/tools.rs`'s `read`/`glob`/`grep`
//! already walk anything under `workdir`, confined by `folder_trust::confine`
//! the same way `write`/`edit` are. A file picked from somewhere else on the
//! person's disk is not readable by that machinery until it is somewhere
//! `workdir` actually contains — so the whole job here is getting the bytes
//! from wherever the person found them to somewhere already covered, and
//! nothing more. No new index, no new read path, no new tool: the model sees
//! this file exactly the way it sees every other file already in the folder.
//!
//! **`workdir/context/`, VISIBLE, NOT `.helloim/`.** Everything Tier 2 and
//! Tier 3 write (`memory.rs`, `facts.rs`) goes under the dot-directory on
//! purpose — those are the app's own bookkeeping, mirrored to markdown only
//! so a curious person *can* read it. A context file is different: the person
//! chose it, on purpose, specifically to be read, so it belongs in the part
//! of the folder they already look at, not hidden beside files that were
//! never meant to be Finder/Explorer material.
//!
//! **NOT GATED ON FOLDER TRUST, THE SAME CALL `facts.rs`/`memory.rs` ALREADY
//! MADE.** `profile.rs::mirror_into` refuses to write into an untrusted
//! folder because `CLAUDE.md` is read automatically, with no prompt, by a
//! headless `claude -p` process — see `folder_trust.rs`'s own header. A
//! context file carries no such automatic-execution risk: it is inert bytes
//! that only ever reach the model because a turn WITH TOOLS chose to `read`
//! it, the identical path every other file already in the folder sits behind.
//! Gating the copy itself would not remove any capability the model does not
//! already have the moment tools are offered against this folder — it would
//! only add a failure mode to a feature specified as simple. If that
//! assumption about tool-gating ever stops holding, this call needs
//! revisiting alongside it, not in isolation.
//!
//! **THE SOURCE PATH IS DELIBERATELY *NOT* RUN THROUGH `folder_trust::confine`
//! — the destination is.** Confining the source would defeat the one thing
//! this command exists to do, which is reach outside the trusted folder to
//! bring something in. `confine` is used exactly once, on the write side, so
//! the copy can only ever land inside `workdir/context/` however the source
//! path or the picked filename is spelled.
//!
//! **GUARDRAILS.** A conservative plain-text extension allow-list — exactly
//! the one already shipped for "In your own words" → "Choose files"
//! (`ui/index.html`'s `#ownFileInput`, `accept=".txt,.md,.markdown,.eml"`),
//! reused rather than invented, so the app is not teaching the person two
//! different rules about what a "text file" is. And a size ceiling
//! (`MAX_CONTEXT_BYTES`) generous enough for a real document while staying
//! bounded — `read_tool`'s own per-call cap (`OUTPUT_CAP_CHARS`, 20,000
//! characters) means the model reads any of this in slices regardless; the
//! ceiling here is about disk and the copy operation, not about what one
//! `read` call returns.

use std::path::{Path, PathBuf};

use serde::Serialize;

/// The subfolder every context file lands in, directly under the working
/// folder — see the module doc for why this is visible rather than under
/// `.helloim/`.
const CONTEXT_DIR: &str = "context";

/// Matched case-insensitively against the source file's own extension.
/// Identical to `ui/index.html`'s `#ownFileInput` accept list — see the
/// module doc on why that is reused rather than a second policy invented
/// here.
const ALLOWED_EXTENSIONS: &[&str] = &["txt", "md", "markdown", "eml"];

/// 2 MiB. Roomy for a real document in plain text — hundreds of pages — while
/// staying small enough that adding a few context files is never how
/// somebody's disk or their folder listing gets slow.
pub const MAX_CONTEXT_BYTES: u64 = 2 * 1024 * 1024;

/// Longest a display name may be once cleaned. A filename is a label, not
/// prose; `facts::MAX_FACT` (600) is the right order of magnitude for a
/// sentence and this is not one.
const MAX_NAME: usize = 200;

/// What the window gets back, so it can show what just landed without a
/// second round trip to re-list the folder.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextFile {
    /// The file's real path on disk, inside `workdir/context/`. Unlike the
    /// error strings below, this is a SUCCESS value the window explicitly
    /// asked for — same reasoning `save_voice_card`/`save_profile` already
    /// give for returning the `CLAUDE.md` path they wrote: someone who just
    /// added a file is entitled to know where it went.
    pub path: String,
    /// The cleaned display name — never the raw OS filename verbatim. See
    /// `memory::clean`'s own doc on why a name the person merely *found*,
    /// rather than typed, still goes through the same sanitizer as anything
    /// else this app puts in front of someone.
    pub name: String,
}

/// Is this extension one of the ones `ownFileInput` already offers? `None`
/// (no extension at all) is refused the same as an extension not on the list
/// — there is nothing in `ALLOWED_EXTENSIONS` for `confine` to match against
/// otherwise, and a bare extensionless name is exactly as likely to be a
/// binary as a text file.
fn extension_allowed(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| ALLOWED_EXTENSIONS.iter().any(|a| a.eq_ignore_ascii_case(e)))
        .unwrap_or(false)
}

/// A destination filename that will not collide with one already in
/// `context/`. `notes.txt`, then `notes (2).txt`, `notes (3).txt` — the same
/// shape a file manager already uses, so it reads as ordinary rather than as
/// this app inventing its own convention. Bounded at 1000 attempts: past that
/// something else is wrong and the honest answer is an error, not a longer
/// loop.
fn unique_target(dir: &Path, cleaned: &str) -> Result<PathBuf, String> {
    let path = Path::new(cleaned);
    let stem = path.file_stem().and_then(|s| s.to_str()).filter(|s| !s.is_empty()).unwrap_or("context-file");
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");

    for n in 1..=1000u32 {
        let name = if n == 1 {
            if ext.is_empty() { stem.to_string() } else { format!("{stem}.{ext}") }
        } else if ext.is_empty() {
            format!("{stem} ({n})")
        } else {
            format!("{stem} ({n}).{ext}")
        };
        let candidate = dir.join(&name);
        if !candidate.exists() {
            return Ok(candidate);
        }
    }
    Err("Could not find a free name for that file — the context folder has too many files with this name already.".into())
}

/// Copy a file the person picked (via the existing `pick_files` dialog)
/// into `workdir/context/`, so it is readable by the model's ordinary
/// `read`/`glob`/`grep` tools on the next turn — no different from any other
/// file already in the folder.
///
/// **Who may call:** the window, over Tauri IPC, exactly like every other
/// command in this app — no HTTP route, and there must never be one.
/// **Malformed input:** an empty or missing `workdir`, a `source_path` that
/// does not exist, is a directory, exceeds `MAX_CONTEXT_BYTES`, or does not
/// carry one of `ALLOWED_EXTENSIONS` are all refused with a plain error
/// naming the reason. **What leaks:** never the working folder's real path —
/// same rule `facts.rs`/`memory.rs` already state for their own write
/// errors, because a path is a disclosure about the person's machine. The
/// source path DOES appear in a "could not read" error, because it was the
/// caller's own argument a moment before, not a secret this command learned.
#[tauri::command]
pub fn add_context_file(workdir: String, source_path: String) -> Result<ContextFile, String> {
    let root = Path::new(workdir.trim());
    if workdir.trim().is_empty() || !root.is_dir() {
        return Err("No working folder to add this file to.".into());
    }

    let source = Path::new(source_path.trim());
    let meta = std::fs::metadata(source)
        .map_err(|e| format!("could not read {}: {e}", source_path.trim()))?;
    if !meta.is_file() {
        return Err(format!("{} is not a file.", source_path.trim()));
    }
    if meta.len() > MAX_CONTEXT_BYTES {
        return Err(format!(
            "That file is too big to add as context ({} MB — the limit is {} MB).",
            meta.len() / (1024 * 1024),
            MAX_CONTEXT_BYTES / (1024 * 1024)
        ));
    }
    if !extension_allowed(source) {
        return Err(format!(
            "helloim.ai can only add plain-text files as context right now ({}).",
            ALLOWED_EXTENSIONS.iter().map(|e| format!(".{e}")).collect::<Vec<_>>().join(", ")
        ));
    }

    let raw_name = source
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| "That file has no usable name.".to_string())?;
    // THE SAME SANITIZER THE BRIDGE AND THE FACTS STORE ALREADY USE — see
    // `memory::clean`'s own doc, extended for this caller. A filename is
    // untrusted-shaped text by the identical argument `profile.rs`'s
    // "Read from a folder" note makes: the person pointed at it, they did
    // not necessarily write it.
    let cleaned = crate::memory::clean(raw_name, MAX_NAME);
    if cleaned.trim().is_empty() {
        return Err("That file's name did not leave anything usable once cleaned.".into());
    }

    let dir = root.join(CONTEXT_DIR);
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("could not create the context folder: {e}"))?;
    let dest = unique_target(&dir, &cleaned)?;

    // A COPY, NEVER A MOVE — the source is the person's own file, sitting
    // wherever they keep it, and this command has no business deleting it.
    std::fs::copy(source, &dest)
        .map_err(|e| format!("could not add {}: {e}", source_path.trim()))?;

    let display = dest
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(&cleaned)
        .to_string();
    Ok(ContextFile { path: dest.to_string_lossy().into_owned(), name: display })
}

/// One row of the "Files" group in the Memory tab — everything it needs to
/// render without a second round trip.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextFileEntry {
    /// The cleaned name `add_context_file` stored it under — also what
    /// `remove_context_file` takes back, so the window never has to remember
    /// a separate id for a plain file listing.
    pub name: String,
    pub bytes: u64,
    /// Unix seconds, off the file's own mtime — there is no separate
    /// "added at" record kept anywhere, so the filesystem's own clock is the
    /// only one there is, the same choice `main.rs::ConversationSummary`
    /// already made for the identical reason.
    pub modified_at: u64,
}

fn modified_at(meta: &std::fs::Metadata) -> u64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// List what is in `workdir/context/`, newest first — the Files group Iris
/// designed.
///
/// **Forgiving on error, `Vec` not `Result`, the same contract `list_facts`
/// already carries for the identical reason: this feeds a listing panel.** No
/// folder, no `context/` subfolder yet, or a folder that cannot be read are
/// all "nothing to show", never an error dialog over an empty list. A single
/// unreadable entry inside an otherwise-good listing is skipped rather than
/// failing the whole call — one bad `stat` must not hide every other file.
///
/// **Filtered to `ALLOWED_EXTENSIONS`, same as `add_context_file`.** This
/// folder is visible in the person's own file browser, so it can end up
/// holding something this app did not put there (a `.DS_Store`, a file
/// dragged in by hand). Only what this app would itself have accepted is
/// listed — not to hide the other files (they are not touched or removed),
/// but so the panel keeps its one promise: everything shown here is
/// something the model can actually read as text.
///
/// **`symlink_metadata` (lstat), NEVER `Path::is_file`/`fs::metadata` —
/// Cassandra's review, before this ever shipped.** Those two FOLLOW a
/// symlink: a hand-planted or synced link named `notes.txt` pointing at
/// `../CLAUDE.md` reads as an ordinary `.txt` file under the old check,
/// because it asked "what does this resolve to", not "what is this". It
/// listed correctly-shaped and (worse) it is exactly what `remove_context_file`
/// below used to act on. `symlink_metadata` answers about the DIRECTORY
/// ENTRY itself — a symlink's own type is "symlink", never "file" — so a
/// planted link is skipped here rather than shown as a safe, ordinary row.
#[tauri::command]
pub fn list_context_files(workdir: String) -> Vec<ContextFileEntry> {
    let dir = Path::new(workdir.trim()).join(CONTEXT_DIR);
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut out: Vec<ContextFileEntry> = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let path = entry.path();
            let meta = std::fs::symlink_metadata(&path).ok()?;
            if !meta.is_file() || !extension_allowed(&path) {
                return None;
            }
            let name = path.file_name()?.to_str()?.to_string();
            Some(ContextFileEntry { name, bytes: meta.len(), modified_at: modified_at(&meta) })
        })
        .collect();
    out.sort_by(|a, b| b.modified_at.cmp(&a.modified_at));
    out
}

/// A bare filename, and nothing that could ever mean "somewhere else" —
/// checked before `name` is joined onto any path at all.
///
/// Refuses any separator or a `..`/`.` component, so `dir.join(name)` below
/// can only ever land on a direct child of `context/` — never a step back
/// out to `workdir` itself or beyond it. This is what makes it safe for
/// `remove_context_file` to join `name` onto the context directory directly,
/// with no further path-confinement call needed on top.
fn plain_filename(name: &str) -> Result<&str, String> {
    let name = name.trim();
    if name.is_empty() || name == "." || name == ".." || name.contains('/') || name.contains('\\') {
        return Err("That is not a context file this app added.".into());
    }
    Ok(name)
}

/// Remove one file from `workdir/context/` — "Forget" on a Files row.
///
/// **IDEMPOTENT, THE SAME CONTRACT `facts::forget` ALREADY GIVES.** A `name`
/// that does not exist (already removed, or never existed) is a plain `Ok`,
/// not an error — "already forgotten" and "forgotten" are the same outcome
/// from the window's side, and a second click after a slow first one must
/// not surface a scary error over nothing having actually gone wrong.
///
/// **THE SCAR THIS FUNCTION USED TO CARRY — Cassandra's review, caught
/// before this ever shipped.** The first version resolved `name` through
/// `folder_trust::confine`, on the theory that reusing the exact same
/// symlink-aware resolution `read`/`write`/`edit` already trust was the safe,
/// proven-pattern move. It was backwards for a DELETE specifically:
/// `confine` exists to find out what a path REALLY POINTS AT (so a symlink
/// planted inside the trusted folder cannot be used to sneak a *write*
/// outside it), and it hands back that resolved target — which for a
/// hand-planted symlink `context/notes.txt -> ../CLAUDE.md` is
/// `workdir/CLAUDE.md`. Still inside `workdir`, so `confine` allowed it, and
/// `remove_file` on that path deleted the real `CLAUDE.md` — a person's
/// persona/instructions file, gone because they clicked Forget on what
/// LOOKED like an ordinary added text file. A delete has the opposite
/// question to ask: not "what does this resolve to", but "what IS this
/// entry" — so `confine` is the wrong tool here even though it is the right
/// one one function up.
///
/// **THE FIX: `symlink_metadata` (lstat) on the entry itself, never
/// followed, and a plain filename that cannot contain `..` (`plain_filename`
/// above) so there is nothing for a symlink's OWN path to redirect through
/// in the first place.** Anything that is not a regular file — a symlink
/// above all, but also a directory or anything stranger — is refused
/// outright rather than acted on; `add_context_file` only ever creates
/// plain regular files via `std::fs::copy`, so a non-regular entry here can
/// only be something else that arrived in this folder, and the safe answer
/// is to leave it alone, not to remove the link (which would still require
/// deciding it is "safe enough" to touch) or guess at what it means.
#[tauri::command]
pub fn remove_context_file(workdir: String, name: String) -> Result<(), String> {
    let root = Path::new(workdir.trim());
    if workdir.trim().is_empty() || !root.is_dir() {
        return Err("No working folder to remove this file from.".into());
    }
    let name = plain_filename(&name)?;
    let path = root.join(CONTEXT_DIR).join(name);

    match std::fs::symlink_metadata(&path) {
        Ok(meta) if meta.is_file() => {
            std::fs::remove_file(&path).map_err(|e| format!("could not remove {name}: {e}"))
        }
        // Present, but not a regular file -- a symlink above all. Refused,
        // never followed and never removed as a link either: see this
        // function's own doc on why "leave it alone" is the only answer
        // that cannot itself be turned into a way to touch something this
        // command was never meant to reach.
        Ok(_) => {
            Err(format!("\"{name}\" is not a context file this app added — refusing to remove it."))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("could not check {name}: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("nameos-ctx-{}-{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn it_copies_an_allowed_file_into_the_visible_context_folder() {
        let src_dir = tmp("src");
        let wd = tmp("wd");
        let src = src_dir.join("notes.txt");
        std::fs::write(&src, "The pricing page ships Friday.").unwrap();

        let out = add_context_file(
            wd.to_string_lossy().into_owned(),
            src.to_string_lossy().into_owned(),
        )
        .unwrap();

        assert_eq!(out.name, "notes.txt");
        assert!(Path::new(&out.path).starts_with(wd.join("context")));
        assert_eq!(std::fs::read_to_string(&out.path).unwrap(), "The pricing page ships Friday.");
        // The source is untouched — a copy, not a move.
        assert!(src.exists());

        let _ = std::fs::remove_dir_all(&src_dir);
        let _ = std::fs::remove_dir_all(&wd);
    }

    #[test]
    fn a_disallowed_extension_is_refused() {
        let src_dir = tmp("src2");
        let wd = tmp("wd2");
        let src = src_dir.join("photo.png");
        std::fs::write(&src, [0u8, 1, 2, 3]).unwrap();

        let err = add_context_file(
            wd.to_string_lossy().into_owned(),
            src.to_string_lossy().into_owned(),
        )
        .unwrap_err();
        assert!(err.contains(".txt"), "{err}");
        assert!(!wd.join("context").exists(), "nothing should have been created");

        let _ = std::fs::remove_dir_all(&src_dir);
        let _ = std::fs::remove_dir_all(&wd);
    }

    #[test]
    fn an_oversized_file_is_refused_before_it_is_copied() {
        let src_dir = tmp("src3");
        let wd = tmp("wd3");
        let src = src_dir.join("big.txt");
        // Sparse file, not actually written to disk full of bytes — proves the
        // guard reads metadata rather than reading the whole file to measure it.
        let f = std::fs::File::create(&src).unwrap();
        f.set_len(MAX_CONTEXT_BYTES + 1).unwrap();

        let err = add_context_file(
            wd.to_string_lossy().into_owned(),
            src.to_string_lossy().into_owned(),
        )
        .unwrap_err();
        assert!(err.contains("too big"), "{err}");
        assert!(!wd.join("context").join("big.txt").exists());

        let _ = std::fs::remove_dir_all(&src_dir);
        let _ = std::fs::remove_dir_all(&wd);
    }

    #[test]
    fn a_missing_source_is_a_named_error_not_a_panic() {
        let wd = tmp("wd4");
        let err = add_context_file(
            wd.to_string_lossy().into_owned(),
            "/definitely/not/a/real/file.txt".into(),
        )
        .unwrap_err();
        assert!(err.contains("could not read"), "{err}");
        let _ = std::fs::remove_dir_all(&wd);
    }

    #[test]
    fn a_missing_workdir_is_refused_without_touching_disk() {
        let src_dir = tmp("src5");
        let src = src_dir.join("notes.txt");
        std::fs::write(&src, "hello").unwrap();
        let err = add_context_file(
            "/definitely/not/a/real/folder".into(),
            src.to_string_lossy().into_owned(),
        )
        .unwrap_err();
        assert!(err.contains("working folder"), "{err}");
        let _ = std::fs::remove_dir_all(&src_dir);
    }

    #[test]
    fn a_name_collision_gets_a_distinct_file_rather_than_overwriting() {
        let src_dir = tmp("src6");
        let wd = tmp("wd6");
        let src = src_dir.join("notes.txt");
        std::fs::write(&src, "first").unwrap();

        let first = add_context_file(
            wd.to_string_lossy().into_owned(),
            src.to_string_lossy().into_owned(),
        )
        .unwrap();
        std::fs::write(&src, "second").unwrap();
        let second = add_context_file(
            wd.to_string_lossy().into_owned(),
            src.to_string_lossy().into_owned(),
        )
        .unwrap();

        assert_ne!(first.path, second.path);
        assert_eq!(second.name, "notes (2).txt");
        // Both files survive with the content each had at the moment it was
        // added — the first was never overwritten by the second.
        assert_eq!(std::fs::read_to_string(&first.path).unwrap(), "first");
        assert_eq!(std::fs::read_to_string(&second.path).unwrap(), "second");

        let _ = std::fs::remove_dir_all(&src_dir);
        let _ = std::fs::remove_dir_all(&wd);
    }

    #[test]
    fn a_hostile_filename_is_flattened_the_same_way_a_bridge_item_is() {
        let src_dir = tmp("src7");
        let wd = tmp("wd7");
        // A newline and a direction override cannot survive into the display
        // name or the stored filename — same guarantee `memory::clean` already
        // proves against a planted bridge file.
        let src = src_dir.join("evil\u{202E}.txt");
        std::fs::write(&src, "x").unwrap();

        let out = add_context_file(
            wd.to_string_lossy().into_owned(),
            src.to_string_lossy().into_owned(),
        )
        .unwrap();
        assert!(!out.name.contains('\u{202E}'), "{:?}", out.name);

        let _ = std::fs::remove_dir_all(&src_dir);
        let _ = std::fs::remove_dir_all(&wd);
    }

    // -- list_context_files ---------------------------------------------

    #[test]
    fn it_lists_what_was_added_newest_first() {
        let src_dir = tmp("src8");
        let wd = tmp("wd8");
        let a = src_dir.join("a.txt");
        let b = src_dir.join("b.md");
        std::fs::write(&a, "one").unwrap();
        std::fs::write(&b, "two").unwrap();

        add_context_file(wd.to_string_lossy().into_owned(), a.to_string_lossy().into_owned()).unwrap();
        // Force a distinct mtime ordering rather than trusting two writes in
        // the same instant to land on different seconds.
        std::thread::sleep(std::time::Duration::from_millis(1100));
        add_context_file(wd.to_string_lossy().into_owned(), b.to_string_lossy().into_owned()).unwrap();

        let listed = list_context_files(wd.to_string_lossy().into_owned());
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].name, "b.md", "newest should be first: {listed:?}");
        assert_eq!(listed[1].name, "a.txt");
        assert!(listed.iter().all(|f| f.bytes == 3));

        let _ = std::fs::remove_dir_all(&src_dir);
        let _ = std::fs::remove_dir_all(&wd);
    }

    #[test]
    fn an_empty_or_missing_context_folder_lists_nothing_not_an_error() {
        let wd = tmp("wd9");
        assert!(list_context_files(wd.to_string_lossy().into_owned()).is_empty());
        assert!(list_context_files("/definitely/not/a/real/folder".into()).is_empty());
        let _ = std::fs::remove_dir_all(&wd);
    }

    #[test]
    fn listing_skips_a_file_this_app_would_not_have_accepted() {
        let wd = tmp("wd10");
        std::fs::create_dir_all(wd.join("context")).unwrap();
        std::fs::write(wd.join("context").join(".DS_Store"), "junk").unwrap();
        std::fs::write(wd.join("context").join("real.txt"), "kept").unwrap();

        let listed = list_context_files(wd.to_string_lossy().into_owned());
        assert_eq!(listed.len(), 1, "{listed:?}");
        assert_eq!(listed[0].name, "real.txt");
        // Untouched, not deleted -- the filter is display-only.
        assert!(wd.join("context").join(".DS_Store").exists());

        let _ = std::fs::remove_dir_all(&wd);
    }

    // -- remove_context_file ----------------------------------------------

    #[test]
    fn it_removes_a_file_it_added() {
        let src_dir = tmp("src11");
        let wd = tmp("wd11");
        let src = src_dir.join("notes.txt");
        std::fs::write(&src, "x").unwrap();
        let added = add_context_file(
            wd.to_string_lossy().into_owned(),
            src.to_string_lossy().into_owned(),
        )
        .unwrap();
        assert!(Path::new(&added.path).exists());

        remove_context_file(wd.to_string_lossy().into_owned(), added.name.clone()).unwrap();
        assert!(!Path::new(&added.path).exists());

        let _ = std::fs::remove_dir_all(&src_dir);
        let _ = std::fs::remove_dir_all(&wd);
    }

    #[test]
    fn removing_a_name_that_is_already_gone_is_not_an_error() {
        let wd = tmp("wd12");
        std::fs::create_dir_all(wd.join("context")).unwrap();
        remove_context_file(wd.to_string_lossy().into_owned(), "never-existed.txt".into()).unwrap();
        let _ = std::fs::remove_dir_all(&wd);
    }

    #[test]
    fn removing_cannot_reach_outside_the_context_folder() {
        let wd = tmp("wd13");
        std::fs::create_dir_all(wd.join("context")).unwrap();
        std::fs::write(wd.join("CLAUDE.md"), "do not touch me").unwrap();

        // Neither spelling of "escape the folder" is accepted: `plain_filename`
        // refuses the separator/`..` shape outright, before `name` is ever
        // joined onto a path at all.
        for hostile in ["../CLAUDE.md", "..", "sub/dir.txt", "a/b"] {
            let err = remove_context_file(wd.to_string_lossy().into_owned(), hostile.into())
                .unwrap_err();
            assert!(err.contains("not a context file"), "{hostile}: {err}");
        }
        assert!(wd.join("CLAUDE.md").exists(), "the real file must survive every attempt");

        let _ = std::fs::remove_dir_all(&wd);
    }

    /// **CASSANDRA'S FINDING, TURNED INTO A REGRESSION TEST — caught before
    /// this ever shipped.** A symlink SPELLED like an ordinary added file
    /// (`plain_filename` cannot tell the two apart; a name is a name) must
    /// not let "Forget" reach through it and delete whatever it points at —
    /// here, the real `CLAUDE.md` sitting one level up, still inside
    /// `workdir`, so the old `folder_trust::confine`-based resolution
    /// considered it a perfectly ordinary in-folder target.
    #[test]
    fn a_symlink_in_context_cannot_let_forget_delete_its_target() {
        let wd = tmp("wd14");
        std::fs::create_dir_all(wd.join("context")).unwrap();
        let target = wd.join("CLAUDE.md");
        std::fs::write(&target, "the person's real instructions file").unwrap();
        let link = wd.join("context").join("notes.txt");
        std::os::unix::fs::symlink("../CLAUDE.md", &link).unwrap();

        let err = remove_context_file(wd.to_string_lossy().into_owned(), "notes.txt".into())
            .unwrap_err();
        assert!(err.contains("not a context file"), "{err}");

        // The real target is untouched, and so is the link itself -- refused
        // outright, nothing removed on either side of it.
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "the person's real instructions file");
        assert!(
            std::fs::symlink_metadata(&link).map(|m| m.file_type().is_symlink()).unwrap_or(false),
            "the symlink itself should still be exactly a symlink"
        );

        let _ = std::fs::remove_dir_all(&wd);
    }

    /// The listing side of the same finding: a planted symlink must not show
    /// up in the Files group looking like an ordinary added file, because
    /// that is the row a person would click Forget on next.
    #[test]
    fn listing_skips_a_symlink_even_with_an_allowed_extension() {
        let wd = tmp("wd15");
        std::fs::create_dir_all(wd.join("context")).unwrap();
        std::fs::write(wd.join("CLAUDE.md"), "real instructions").unwrap();
        std::os::unix::fs::symlink("../CLAUDE.md", wd.join("context").join("sneaky.txt")).unwrap();
        std::fs::write(wd.join("context").join("real.txt"), "an actual added file").unwrap();

        let listed = list_context_files(wd.to_string_lossy().into_owned());
        assert_eq!(listed.len(), 1, "{listed:?}");
        assert_eq!(listed[0].name, "real.txt");

        let _ = std::fs::remove_dir_all(&wd);
    }
}
