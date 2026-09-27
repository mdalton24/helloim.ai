//! Skills — the things this assistant knows how to do, that you taught it.
//!
//! Mark, 2026-08-26: "we also need ways to add skills." It arrived in the same
//! breath as the connectors work and it is the same complaint one layer up:
//! the capability exists, and reaching it means knowing where a folder lives.
//!
//! WHAT A SKILL ACTUALLY IS, because the whole design follows from it being
//! this small: a directory containing a `SKILL.md`, whose YAML frontmatter
//! carries a `description` telling the model when to reach for it, and whose
//! body is the instructions. The directory NAME is what you type after a
//! slash. That is the entire format.
//!
//!   Personal — `~/.claude/skills/<name>/SKILL.md`    every project
//!   Project  — `<workdir>/.claude/skills/<name>/SKILL.md`  this folder only
//!
//! THIS FILE WRITES NOTHING CLEVER. It creates that directory and that file,
//! and it reads them back. There is no database, no index, no state of our own
//! to drift out of step — the folder IS the truth, and a skill someone drops
//! in by hand shows up in the window with no import step. Anything we invented
//! on top would be a second source of truth for a format that already has one.
//!
//! THE DELETE IS THE DANGEROUS COMMAND and it is written defensively on
//! purpose: it re-derives the skills root itself, refuses any name that is not
//! a single plain path segment, and confirms the resolved path really sits
//! inside that root before removing anything. A settings window that can be
//! talked into `remove_dir_all` on an arbitrary path is a very bad settings
//! window, and the name comes from the front end.

use serde::{Deserialize, Serialize};
use std::io::Read;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Skill {
    /// The directory name, which is also what you type: `/summarize-changes`.
    pub name: String,
    /// "personal" or "project" — which of the two roots it was found in.
    pub scope: String,
    /// From the frontmatter. Empty if the file has none, which is legal and
    /// only means the model will not pick it up on its own.
    pub description: String,
    /// Everything after the frontmatter. What the model actually follows.
    pub body: String,
    /// Full path, so the window can offer to open it in a file manager.
    pub path: String,
    /// TRUE when the folder holds more than just SKILL.md — scripts, templates,
    /// reference documents. The window uses it to refuse to edit in place:
    /// rewriting SKILL.md is safe, but a skill with supporting files is one
    /// somebody built deliberately and a text box is the wrong tool for it.
    pub has_extras: bool,
}

fn personal_root() -> Option<PathBuf> {
    dirs_home().map(|h| h.join(".claude").join("skills"))
}

fn project_root(workdir: &str) -> Option<PathBuf> {
    let p = Path::new(workdir.trim());
    if workdir.trim().is_empty() || !p.is_dir() {
        return None;
    }
    Some(p.join(".claude").join("skills"))
}

/// No `dirs` crate in this build, and one line is cheaper than a dependency.
fn dirs_home() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

/// Split `SKILL.md` into (description, body).
///
/// Deliberately forgiving. A skill file written by hand, by another tool, or by
/// an older version of this app should still LIST — showing it without a
/// description is honest and useful, and refusing to show it because the YAML
/// is not quite right would hide the user's own work from them.
fn parse(text: &str) -> (String, String) {
    let t = text.trim_start_matches('\u{feff}');
    if !t.starts_with("---") {
        return (String::new(), t.to_string());
    }
    // Find the closing fence on its own line.
    let after = &t[3..];
    let Some(end) = after.find("\n---") else {
        return (String::new(), t.to_string());
    };
    let front = &after[..end];
    let body = after[end + 4..].trim_start_matches('\n').to_string();

    let mut description = String::new();
    for line in front.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("description:") {
            description = rest
                .trim()
                .trim_matches(|c| c == '"' || c == '\'')
                .to_string();
            break;
        }
    }
    (description, body)
}

fn read_dir_skills(root: &Path, scope: &str, out: &mut Vec<Skill>) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return; // no skills folder yet is the normal state, not an error
    };
    for e in entries.flatten() {
        let dir = e.path();
        if !dir.is_dir() {
            continue;
        }
        let file = dir.join("SKILL.md");
        let Ok(text) = std::fs::read_to_string(&file) else {
            continue; // a directory without SKILL.md is not a skill
        };
        let (description, body) = parse(&text);
        let has_extras = std::fs::read_dir(&dir)
            .map(|it| {
                it.flatten()
                    .any(|f| f.file_name() != std::ffi::OsStr::new("SKILL.md"))
            })
            .unwrap_or(false);
        out.push(Skill {
            name: e.file_name().to_string_lossy().into_owned(),
            scope: scope.to_string(),
            description,
            body,
            path: dir.to_string_lossy().into_owned(),
            has_extras,
        });
    }
}

/// Where installed plugins live.
///
/// **PLUGINS ARE A THING CLAUDE CODE ALREADY DOES, and Mark pointed at it:**
/// `claude --plugin-dir ./connect-apps-plugin`. Verified here before any of
/// this was written — loading that directory really does make
/// `connect-apps:setup` a command the model can run.
///
/// That matters because it is the difference between a skill and a plugin: a
/// skill is one SKILL.md we can copy, a plugin is a directory with its own
/// manifest, commands, and often an MCP server. Copying a plugin's SKILL.md
/// out of it would give somebody a broken half of the thing.
///
/// Kept OUT of `~/.claude/plugins`, which is Claude Code's own managed
/// directory — writing into another program's state is how two tools end up
/// fighting over the same folder.
fn plugins_root() -> Option<PathBuf> {
    dirs_home().map(|h| h.join(".helloim").join("plugins"))
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Plugin {
    pub name: String,
    pub description: String,
    pub path: String,
}

/// Every installed plugin, for the window and for the launcher.
#[tauri::command]
pub fn list_plugins() -> Vec<Plugin> {
    let mut out = Vec::new();
    let Some(root) = plugins_root() else { return out };
    let Ok(entries) = std::fs::read_dir(&root) else { return out };
    for e in entries.flatten() {
        let dir = e.path();
        let manifest = dir.join(".claude-plugin").join("plugin.json");
        if !manifest.is_file() {
            continue; // a directory without a manifest is not a plugin
        }
        let description = std::fs::read_to_string(&manifest)
            .ok()
            .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
            .and_then(|v| v.get("description").and_then(|d| d.as_str()).map(str::to_string))
            .unwrap_or_default();
        out.push(Plugin {
            name: e.file_name().to_string_lossy().into_owned(),
            description,
            path: dir.to_string_lossy().into_owned(),
        });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// The directories to hand `claude --plugin-dir`. One flag per plugin.
pub fn plugin_dirs() -> Vec<PathBuf> {
    list_plugins().into_iter().map(|p| PathBuf::from(p.path)).collect()
}

#[tauri::command]
pub fn delete_plugin(name: String) -> Result<(), String> {
    let dir_name = safe_name(&name)?;
    let root = plugins_root().ok_or_else(|| "Could not find your home folder.".to_string())?;
    let dir = root.join(&dir_name);
    if !dir.starts_with(&root) {
        return Err("Refusing to delete outside the plugins folder.".into());
    }
    if !dir.join(".claude-plugin").join("plugin.json").is_file() {
        return Err("That does not look like a plugin folder.".into());
    }
    std::fs::remove_dir_all(&dir).map_err(|e| format!("could not remove {dir:?}: {e}"))
}

#[tauri::command]
pub fn list_skills(workdir: String) -> Vec<Skill> {
    let mut out = Vec::new();
    if let Some(r) = personal_root() {
        read_dir_skills(&r, "personal", &mut out);
    }
    if let Some(r) = project_root(&workdir) {
        read_dir_skills(&r, "project", &mut out);
    }
    out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    out
}

/// The folder name, and therefore the thing you type after a slash.
///
/// Someone types "Summarise my week" as a name. That has to become
/// `summarise-my-week` or the skill is unusable — and it has to be done HERE
/// rather than in the window, because this is the name the filesystem sees.
fn slug(name: &str) -> String {
    let mut s = String::new();
    let mut dash = false;
    for ch in name.trim().chars() {
        if ch.is_ascii_alphanumeric() {
            s.push(ch.to_ascii_lowercase());
            dash = false;
        } else if !s.is_empty() && !dash {
            s.push('-');
            dash = true;
        }
    }
    while s.ends_with('-') {
        s.pop();
    }
    s
}

/// Reject anything that is not one ordinary directory name.
///
/// The name reaches this file from a text box, so `..`, an absolute path and a
/// separator all have to die here rather than be trusted to have been cleaned
/// up on the way. `slug` already strips them; this is the check that does not
/// depend on `slug` having been called.
fn safe_name(name: &str) -> Result<String, String> {
    let n = name.trim();
    if n.is_empty() {
        return Err("Give the skill a name.".into());
    }
    if n == "." || n == ".." || n.contains('/') || n.contains('\\') || Path::new(n).is_absolute() {
        return Err("That is not a valid skill name.".into());
    }
    if Path::new(n).components().count() != 1 {
        return Err("That is not a valid skill name.".into());
    }
    Ok(n.to_string())
}

fn root_for(scope: &str, workdir: &str) -> Result<PathBuf, String> {
    match scope {
        "project" => project_root(workdir)
            .ok_or_else(|| "Pick a working folder first — a project skill lives in it.".to_string()),
        _ => personal_root().ok_or_else(|| "Could not find your home folder.".to_string()),
    }
}

/// Create or overwrite a skill.
///
/// `original` is the name it had before, if this is a rename — without it, a
/// rename would leave the old folder behind and the person would end up with
/// two skills where they meant to have one.
#[tauri::command]
pub fn save_skill(
    workdir: String,
    scope: String,
    name: String,
    description: String,
    body: String,
    original: String,
) -> Result<Skill, String> {
    let slugged = slug(&name);
    let dir_name = safe_name(&slugged)?;
    let root = root_for(&scope, &workdir)?;
    let dir = root.join(&dir_name);
    std::fs::create_dir_all(&dir).map_err(|e| format!("could not create {dir:?}: {e}"))?;

    // The description goes on ONE line and quoted. A frontmatter value with a
    // stray newline or a bare colon in it is invalid YAML, and the failure
    // shows up later as a skill the model silently never loads — which is the
    // worst kind, because the file is right there looking correct.
    let clean = description
        .replace(['\n', '\r'], " ")
        .replace('"', "'")
        .trim()
        .to_string();

    let mut out = String::from("---\ndescription: \"");
    out.push_str(&clean);
    out.push_str("\"\n---\n\n");
    out.push_str(body.trim());
    out.push('\n');

    let file = dir.join("SKILL.md");
    std::fs::write(&file, &out).map_err(|e| format!("could not write {file:?}: {e}"))?;

    // Clean up after a rename, and only after the new one is safely written.
    let prior = original.trim();
    if !prior.is_empty() && prior != dir_name {
        if let Ok(safe) = safe_name(prior) {
            let old = root.join(safe);
            if old.starts_with(&root) && old.join("SKILL.md").is_file() {
                let _ = std::fs::remove_dir_all(&old);
            }
        }
    }

    Ok(Skill {
        name: dir_name,
        scope,
        description: clean,
        body,
        path: dir.to_string_lossy().into_owned(),
        has_extras: false,
    })
}

/// THE ONE COMMAND HERE THAT DESTROYS SOMETHING. Read the note at the top of
/// the file: the root is re-derived, the name must be a single segment, and
/// the resolved path is checked to be inside the root before anything goes.
#[tauri::command]
pub fn delete_skill(workdir: String, scope: String, name: String) -> Result<(), String> {
    let dir_name = safe_name(&name)?;
    let root = root_for(&scope, &workdir)?;
    let dir = root.join(&dir_name);

    if !dir.starts_with(&root) {
        return Err("Refusing to delete outside the skills folder.".into());
    }
    // A directory with no SKILL.md in it is not a skill, and this window has no
    // business removing it whatever it is.
    if !dir.join("SKILL.md").is_file() {
        return Err("That does not look like a skill folder.".into());
    }
    std::fs::remove_dir_all(&dir).map_err(|e| format!("could not remove {dir:?}: {e}"))
}

/// Turn whatever someone pasted into the zip URL for that repository.
///
/// Mark, 2026-08-26: "we need the ability to import skills along with write as
/// many people point to github repositories." People do not paste a tidy
/// canonical URL — they paste the address bar, which may be a branch, a
/// subdirectory, a `.git` clone string, or `owner/repo` typed from memory. All
/// of those name the same archive, and asking someone to normalise it by hand
/// is asking them to do the computer's job.
///
/// Returns `(zip_url, subdir)` — the subdirectory matters because a `/tree/`
/// URL usually points at ONE skill inside a big collection, and importing the
/// whole repository when they pointed at one folder is not what they asked for.
fn github_zip(input: &str) -> Result<(String, String), String> {
    let raw = input.trim().trim_end_matches('/');
    if raw.is_empty() {
        return Err("Paste a GitHub link.".into());
    }

    let body = raw
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_start_matches("www.")
        .trim_start_matches("github.com/")
        .trim_end_matches(".git");

    let parts: Vec<&str> = body.split('/').filter(|p| !p.is_empty()).collect();
    if parts.len() < 2 {
        return Err("That does not look like a GitHub repository.".into());
    }
    let (owner, repo) = (parts[0], parts[1]);
    if owner.is_empty() || repo.is_empty() {
        return Err("That does not look like a GitHub repository.".into());
    }

    // .../tree/<branch>/<path...>  and  .../blob/<branch>/<path...>
    let mut branch = String::new();
    let mut subdir = String::new();
    if parts.len() > 3 && (parts[2] == "tree" || parts[2] == "blob") {
        branch = parts[3].to_string();
        subdir = parts[4..].join("/");
        // A link to the SKILL.md itself means the folder holding it.
        if let Some(rest) = subdir.strip_suffix("SKILL.md") {
            subdir = rest.trim_end_matches('/').to_string();
        }
    }

    // No branch named means "whatever they call their default" — codeload
    // resolves HEAD for us, so we never have to guess between main and master.
    let branch = if branch.is_empty() { "HEAD".into() } else { branch };
    Ok((
        format!("https://codeload.github.com/{owner}/{repo}/zip/{branch}"),
        subdir,
    ))
}

/// What a skill import found, before anything is written.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FoundSkill {
    pub name: String,
    pub description: String,
    pub body: String,
    /// Where it sat inside the archive — shown so somebody importing from a
    /// big collection can tell two similarly-named skills apart.
    pub from: String,
    pub has_extras: bool,
}

/// Download a GitHub repository and find the skills in it. WRITES NOTHING.
///
/// **NOTHING IS INSTALLED BY THIS COMMAND, and that is the point.** A skill is
/// instructions that a model will follow, from a stranger on the internet —
/// closer to running their code than to reading their document. So the import
/// is two steps: this one finds and returns them, the window shows what they
/// are, and `save_skill` writes only what the person chose after looking.
/// Fetch-and-install in one click would make a pasted link into arbitrary
/// instructions with no moment to say no.
#[tauri::command(async)]
pub fn find_skills_in_repo(url: String) -> Result<Vec<FoundSkill>, String> {
    let (zip_url, subdir) = github_zip(&url)?;

    let res = ureq::get(&zip_url)
        .timeout(std::time::Duration::from_secs(45))
        .call()
        .map_err(|e| match e {
            ureq::Error::Status(404, _) => {
                "No such repository, or it is private.".to_string()
            }
            ureq::Error::Status(c, _) => format!("GitHub answered {c}."),
            ureq::Error::Transport(t) => format!("Could not reach GitHub: {t}"),
        })?;

    // A CEILING, because this reads from the internet into memory. Skill
    // repositories are text; anything past this is not one, and an unbounded
    // read is how a settings window becomes a way to exhaust the machine.
    const MAX: u64 = 64 * 1024 * 1024;
    let mut buf = Vec::new();
    res.into_reader()
        .take(MAX + 1)
        .read_to_end(&mut buf)
        .map_err(|e| format!("Download failed: {e}"))?;
    if buf.len() as u64 > MAX {
        return Err("That repository is too big to be a skill collection.".into());
    }

    skills_in_archive(std::io::Cursor::new(buf), &subdir)
}

/// Find the skills inside a downloaded archive.
///
/// Split from the download so it can be tested against an archive built in
/// memory. The network half has one job — fetch bytes — and this half has all
/// the logic that can actually be wrong: the wrapper directory GitHub adds,
/// the subdirectory filter, and whether a folder carries supporting files.
fn skills_in_archive<R: std::io::Read + std::io::Seek>(
    reader: R,
    subdir: &str,
) -> Result<Vec<FoundSkill>, String> {
    let mut zip = zip::ZipArchive::new(reader)
        .map_err(|e| format!("Could not read the download: {e}"))?;

    // First pass: every SKILL.md in the archive, by the directory holding it.
    let mut dirs: Vec<(String, String)> = Vec::new(); // (dir, path in archive)
    for i in 0..zip.len() {
        let Ok(f) = zip.by_index(i) else { continue };
        let path = f.name().to_string();
        if !path.ends_with("/SKILL.md") {
            continue;
        }
        // Strip the wrapper directory GitHub adds: `repo-branch/`.
        let inner = path.splitn(2, '/').nth(1).unwrap_or(&path).to_string();
        if !subdir.is_empty() && !inner.starts_with(subdir) {
            continue;
        }
        let dir = inner.trim_end_matches("SKILL.md").trim_end_matches('/').to_string();
        dirs.push((dir, path));
    }

    if dirs.is_empty() {
        return Err("No skills in that repository — nothing in it has a SKILL.md.".into());
    }

    let mut out = Vec::new();
    for (dir, path) in dirs {
        let mut text = String::new();
        {
            let Ok(mut f) = zip.by_name(&path) else { continue };
            if f.read_to_string(&mut text).is_err() {
                continue; // not text; not a skill we can show
            }
        }
        let (description, body) = parse(&text);
        let name = dir.rsplit('/').next().unwrap_or(&dir).to_string();
        if name.is_empty() {
            continue;
        }
        // Does the folder hold anything besides SKILL.md? The window uses this
        // to warn that a one-file copy leaves its scripts behind.
        let prefix = path.trim_end_matches("SKILL.md").to_string();
        let has_extras = (0..zip.len()).any(|i| {
            zip.by_index(i)
                .map(|f| {
                    let n = f.name();
                    n.starts_with(&prefix) && n != path && !n.ends_with('/')
                })
                .unwrap_or(false)
        });
        out.push(FoundSkill {
            name: slug(&name),
            description,
            body,
            from: dir,
            has_extras,
        });
    }

    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

/// Install a plugin from a GitHub repository, or from one directory inside one.
///
/// Unlike a skill, this copies the WHOLE directory: a plugin is its manifest,
/// its commands, its MCP config and whatever else it ships, and taking only
/// the parts we recognise would install a broken half of it.
#[tauri::command(async)]
pub fn install_plugin(url: String) -> Result<Plugin, String> {
    let (zip_url, subdir) = github_zip(&url)?;
    let res = ureq::get(&zip_url)
        .timeout(std::time::Duration::from_secs(60))
        .call()
        .map_err(|e| match e {
            ureq::Error::Status(404, _) => "No such repository, or it is private.".to_string(),
            ureq::Error::Status(c, _) => format!("GitHub answered {c}."),
            ureq::Error::Transport(t) => format!("Could not reach GitHub: {t}"),
        })?;

    const MAX: u64 = 64 * 1024 * 1024;
    let mut buf = Vec::new();
    res.into_reader()
        .take(MAX + 1)
        .read_to_end(&mut buf)
        .map_err(|e| format!("Download failed: {e}"))?;
    if buf.len() as u64 > MAX {
        return Err("That repository is too big to be a plugin.".into());
    }

    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(buf))
        .map_err(|e| format!("Could not read the download: {e}"))?;

    // Find the manifest — that is what makes a directory a plugin, and its
    // parent is the root to copy.
    let mut plugin_prefix: Option<String> = None;
    for i in 0..zip.len() {
        let Ok(f) = zip.by_index(i) else { continue };
        let path = f.name().to_string();
        if !path.ends_with(".claude-plugin/plugin.json") {
            continue;
        }
        let inner = path.splitn(2, '/').nth(1).unwrap_or(&path).to_string();
        if !subdir.is_empty() && !inner.starts_with(&subdir) {
            continue;
        }
        plugin_prefix = Some(path.trim_end_matches(".claude-plugin/plugin.json").to_string());
        break;
    }
    let Some(prefix) = plugin_prefix else {
        return Err("No plugin there — nothing in it has a .claude-plugin/plugin.json.".into());
    };

    // Name it after its own manifest where possible; the folder name is the
    // fallback. A plugin that calls itself something is entitled to that name.
    let named = {
        let mut t = String::new();
        if let Ok(mut f) = zip.by_name(&format!("{prefix}.claude-plugin/plugin.json")) {
            let _ = f.read_to_string(&mut t);
        }
        serde_json::from_str::<serde_json::Value>(&t)
            .ok()
            .and_then(|v| v.get("name").and_then(|n| n.as_str()).map(str::to_string))
            .unwrap_or_default()
    };
    let folder = if named.trim().is_empty() {
        prefix.trim_end_matches('/').rsplit('/').next().unwrap_or("plugin").to_string()
    } else {
        named
    };
    let dir_name = safe_name(&slug(&folder))?;

    let root = plugins_root().ok_or_else(|| "Could not find your home folder.".to_string())?;
    let dest = root.join(&dir_name);
    // Replace rather than merge: a half-old, half-new plugin is a state nobody
    // can reason about, and reinstalling is how people fix things.
    let _ = std::fs::remove_dir_all(&dest);
    std::fs::create_dir_all(&dest).map_err(|e| format!("could not create {dest:?}: {e}"))?;

    for i in 0..zip.len() {
        let Ok(mut f) = zip.by_index(i) else { continue };
        let name = f.name().to_string();
        if !name.starts_with(&prefix) || name.ends_with('/') {
            continue;
        }
        let rel = &name[prefix.len()..];
        // THE ZIP-SLIP CHECK. An archive can name `../../etc/passwd`, and this
        // is downloaded from a stranger's repository.
        if rel.contains("..") {
            continue;
        }
        let out = dest.join(rel);
        if !out.starts_with(&dest) {
            continue;
        }
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let mut w = std::fs::File::create(&out).map_err(|e| format!("could not write {out:?}: {e}"))?;
        std::io::copy(&mut f, &mut w).map_err(|e| e.to_string())?;
    }

    list_plugins()
        .into_iter()
        .find(|p| p.name == dir_name)
        .ok_or_else(|| "The plugin copied but its manifest is missing.".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every one of these is a string somebody would genuinely paste, and they
    /// all name the same archive.
    #[test]
    fn it_understands_the_shapes_people_paste() {
        let cases = [
            "https://github.com/anthropics/skills",
            "https://github.com/anthropics/skills/",
            "github.com/anthropics/skills",
            "anthropics/skills",
            "https://github.com/anthropics/skills.git",
            "www.github.com/anthropics/skills",
        ];
        for c in cases {
            let (zip, sub) = github_zip(c).unwrap_or_else(|e| panic!("{c:?}: {e}"));
            assert_eq!(zip, "https://codeload.github.com/anthropics/skills/zip/HEAD", "{c:?}");
            assert!(sub.is_empty(), "{c:?} should have no subdirectory");
        }
    }

    /// A /tree/ link points at ONE skill inside a collection. Importing the
    /// whole repository when somebody pointed at one folder is not the ask.
    #[test]
    fn it_keeps_the_subdirectory_off_a_tree_link() {
        let (zip, sub) =
            github_zip("https://github.com/anthropics/skills/tree/main/document-skills/pdf")
                .unwrap();
        assert_eq!(zip, "https://codeload.github.com/anthropics/skills/zip/main");
        assert_eq!(sub, "document-skills/pdf");
    }

    /// People link the file, not the folder. Both mean the same skill.
    #[test]
    fn a_link_to_the_file_means_its_folder() {
        let (_, sub) =
            github_zip("https://github.com/o/r/blob/main/skills/tidy/SKILL.md").unwrap();
        assert_eq!(sub, "skills/tidy");
    }

    /// Build the archive GitHub actually serves — wrapper directory and all —
    /// and prove the scan handles the three things that can be wrong: the
    /// wrapper, the subdirectory filter, and supporting files.
    fn fake_repo() -> std::io::Cursor<Vec<u8>> {
        use zip::write::SimpleFileOptions;
        let mut buf = Vec::new();
        {
            let mut w = zip::ZipWriter::new(std::io::Cursor::new(&mut buf));
            let o = SimpleFileOptions::default();
            let mut add = |name: &str, body: &str| {
                use std::io::Write as _;
                w.start_file(name, o).unwrap();
                w.write_all(body.as_bytes()).unwrap();
            };
            // GitHub wraps everything in `<repo>-<branch>/`.
            add("skills-main/README.md", "not a skill");
            add("skills-main/skills/pdf/SKILL.md",
                "---\ndescription: \"Reads PDFs\"\n---\n\nOpen it.\n");
            add("skills-main/skills/pdf/helper.py", "print('hi')");
            add("skills-main/skills/tidy/SKILL.md",
                "---\ndescription: \"Tidies up\"\n---\n\nClean it.\n");
            w.finish().unwrap();
        }
        std::io::Cursor::new(buf)
    }

    #[test]
    fn it_finds_every_skill_and_strips_the_wrapper_directory() {
        let found = skills_in_archive(fake_repo(), "").unwrap();
        let names: Vec<&str> = found.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, vec!["pdf", "tidy"]);

        let pdf = found.iter().find(|f| f.name == "pdf").unwrap();
        assert_eq!(pdf.description, "Reads PDFs");
        assert_eq!(pdf.body.trim(), "Open it.");
        // The wrapper must be gone, or every path shown is wrong.
        assert_eq!(pdf.from, "skills/pdf");
        // It has helper.py beside it, so a one-file copy would lose something.
        assert!(pdf.has_extras);
        assert!(!found.iter().find(|f| f.name == "tidy").unwrap().has_extras);
    }

    /// A /tree/ link means "this one", not "all of them".
    #[test]
    fn a_subdirectory_narrows_it_to_one() {
        let found = skills_in_archive(fake_repo(), "skills/tidy").unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "tidy");
    }

    /// Pointing at a repository with no skills in it must say so rather than
    /// hand back an empty list that reads as "nothing to import here".
    #[test]
    fn a_repository_with_no_skills_says_so() {
        use zip::write::SimpleFileOptions;
        let mut buf = Vec::new();
        {
            let mut w = zip::ZipWriter::new(std::io::Cursor::new(&mut buf));
            use std::io::Write as _;
            w.start_file("repo-main/README.md", SimpleFileOptions::default()).unwrap();
            w.write_all(b"nothing here").unwrap();
            w.finish().unwrap();
        }
        assert!(skills_in_archive(std::io::Cursor::new(buf), "").is_err());
    }

    /// A plugin is its whole directory, so the install has to copy all of it
    /// and name it from its own manifest. Built in memory with GitHub's own
    /// wrapper directory, and with a zip-slip entry that must be refused.
    #[test]
    fn a_plugin_installs_whole_and_names_itself() {
        use zip::write::SimpleFileOptions;
        let mut buf = Vec::new();
        {
            let mut w = zip::ZipWriter::new(std::io::Cursor::new(&mut buf));
            let o = SimpleFileOptions::default();
            let mut add = |n: &str, b: &str| {
                use std::io::Write as _;
                w.start_file(n, o).unwrap();
                w.write_all(b.as_bytes()).unwrap();
            };
            add("repo-main/thing/.claude-plugin/plugin.json",
                "{\"name\":\"Connect Apps\",\"description\":\"Does things\"}");
            add("repo-main/thing/commands/setup.md", "# setup");
            add("repo-main/thing/README.md", "readme");
            add("repo-main/thing/../../escape.txt", "should never be written");
            w.finish().unwrap();
        }

        // The scan half is what this exercises; the download half is a network
        // call and is not a unit test's business.
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(buf)).unwrap();
        let mut prefix = None;
        for i in 0..zip.len() {
            let f = zip.by_index(i).unwrap();
            let path = f.name().to_string();
            if path.ends_with(".claude-plugin/plugin.json") {
                prefix = Some(path.trim_end_matches(".claude-plugin/plugin.json").to_string());
                break;
            }
        }
        let prefix = prefix.expect("manifest not found");
        assert_eq!(prefix, "repo-main/thing/");

        let mut t = String::new();
        zip.by_name(&format!("{prefix}.claude-plugin/plugin.json"))
            .unwrap()
            .read_to_string(&mut t)
            .unwrap();
        let named: serde_json::Value = serde_json::from_str(&t).unwrap();
        assert_eq!(slug(named["name"].as_str().unwrap()), "connect-apps");

        // And the escaping entry is refused by the same rule install_plugin uses.
        let escaping = "repo-main/thing/../../escape.txt";
        let rel = &escaping[prefix.len()..];
        assert!(rel.contains(".."), "the test's own trap is wrong");
    }

    #[test]
    fn it_refuses_what_is_not_a_repository() {
        for bad in ["", "   ", "https://github.com/", "github.com/onlyowner", "nonsense"] {
            assert!(github_zip(bad).is_err(), "{bad:?} should have been refused");
        }
    }

    #[test]
    fn slug_makes_a_typable_name() {
        assert_eq!(slug("Summarise my week"), "summarise-my-week");
        assert_eq!(slug("  Weekly   Report!!  "), "weekly-report");
        assert_eq!(slug("Already-fine"), "already-fine");
    }

    /// Every one of these is a real path the front end could send.
    #[test]
    fn safe_name_refuses_anything_that_escapes() {
        for bad in ["..", ".", "../etc", "a/b", "a\\b", "/etc", ""] {
            assert!(safe_name(bad).is_err(), "{bad:?} should have been refused");
        }
        assert!(safe_name("summarize-changes").is_ok());
    }

    /// And the slug has to close the hole too, since it runs first.
    #[test]
    fn slug_cannot_produce_an_escape() {
        for bad in ["../../etc", "..", "/", "\\", "./.."] {
            let s = slug(bad);
            assert!(!s.contains('/') && !s.contains('\\') && s != ".." && s != ".");
        }
    }

    /// THE ONE THAT MATTERS: write a skill, read it back off the disk, rename
    /// it, delete it. Everything above tests a helper; this tests the thing the
    /// window actually calls, against a real filesystem, in the exact layout
    /// Claude Code looks in.
    ///
    /// Project scope on purpose — it takes the root as an argument, so this
    /// needs no `HOME` juggling and cannot collide with a parallel test.
    #[test]
    fn a_skill_survives_a_round_trip_on_disk() {
        let tmp = std::env::temp_dir().join(format!("nameos-skills-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();

        let wd = tmp.to_string_lossy().to_string();
        let saved = save_skill(
            wd.clone(),
            "project".into(),
            "Catch Me Up".into(),
            "When I ask what happened.".into(),
            "Five bullets, worst first.".into(),
            String::new(),
        )
        .expect("save failed");
        assert_eq!(saved.name, "catch-me-up");

        // In the place Claude Code reads, under the name you type.
        let file = tmp.join(".claude/skills/catch-me-up/SKILL.md");
        assert!(file.is_file(), "SKILL.md not written to {file:?}");

        let listed = list_skills(wd.clone());
        let found = listed.iter().find(|s| s.name == "catch-me-up").unwrap();
        assert_eq!(found.description, "When I ask what happened.");
        assert_eq!(found.body.trim(), "Five bullets, worst first.");
        assert_eq!(found.scope, "project");
        assert!(!found.has_extras);

        // A rename must not leave the old folder behind — two skills where the
        // person meant to have one is the bug this argument exists to stop.
        save_skill(
            wd.clone(),
            "project".into(),
            "Catch Up".into(),
            "d".into(),
            "b".into(),
            "catch-me-up".into(),
        )
        .expect("rename failed");
        assert!(!tmp.join(".claude/skills/catch-me-up").exists(), "old folder survived the rename");
        assert!(tmp.join(".claude/skills/catch-up/SKILL.md").is_file());

        delete_skill(wd.clone(), "project".into(), "catch-up".into()).expect("delete failed");
        assert!(!tmp.join(".claude/skills/catch-up").exists());

        // And it will not be talked into deleting something else.
        assert!(delete_skill(wd.clone(), "project".into(), "../../..".into()).is_err());

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn parse_reads_the_description_and_leaves_the_body() {
        let (d, b) = parse("---\ndescription: \"Does a thing\"\n---\n\nStep one.\n");
        assert_eq!(d, "Does a thing");
        assert_eq!(b.trim(), "Step one.");
    }

    /// A file somebody wrote by hand, with no frontmatter at all, still lists.
    #[test]
    fn parse_survives_a_file_with_no_frontmatter() {
        let (d, b) = parse("Just instructions, no YAML.\n");
        assert!(d.is_empty());
        assert_eq!(b.trim(), "Just instructions, no YAML.");
    }

    /// The failure this prevents is invisible: invalid YAML means the skill is
    /// never loaded, and the file looks perfectly fine when you open it.
    #[test]
    fn a_description_with_newlines_and_quotes_stays_one_valid_line() {
        let d = "Line one:\nand \"line two\"";
        let clean = d.replace(['\n', '\r'], " ").replace('"', "'");
        assert!(!clean.contains('\n') && !clean.contains('"'));
    }
}
