//! The safe agency layer's three actions — v1, `SAFE-AGENCY-SPEC.md`, the
//! Mastermind room's own amendments folded in 2026-09-04. Everything a
//! connected brain may do here without a real shell: open one web page,
//! launch one of four named apps, open one of a handful of named Settings
//! pages. Gated behind `Provider::allow_agency`, a SEPARATE flag from
//! `allow_shell` — see that field's own doc for why the two are not one gate.
//!
//! ## The one rule every function in this file holds
//!
//! **A STRING THE MODEL WROTE NEVER REACHES A COMMAND LINE, NOT ONCE, NOT
//! EVEN INSIDE A CONSTANT WE CONTROL.** `LaunchApp` and `OpenSettingsPage`
//! each take a single enum value; what actually gets spawned is looked up
//! from a `match`/const table baked in at compile time, and the model's own
//! string never appears anywhere in the spawned process's argv — the JSON
//! schema's `enum` is a MODEL HINT ONLY and is never trusted as the gate (see
//! `SAFE-AGENCY-SPEC.md`'s own line on this). `OpenUrl` is the one action
//! that does carry a free string, and it is never put on a command line
//! either: everything here spawns through `connectors::open_in_browser`,
//! which passes its target through an ENVIRONMENT VARIABLE to
//! `Start-Process`, the same injection-safe primitive Cassandra hardened
//! 2026-08-28 (see that function's own header for the `&`/`|`/`^` command-
//! injection bug it replaced). No new spawn path is written here.
//!
//! ## `OpenUrl`'s network-target guard — room amendment 1, mandatory
//!
//! The room's unanimous objection to the first draft of this design was a
//! confused-deputy hole: nothing stopped a hijacked brain from calling
//! `OpenUrl` on `http://127.0.0.1:<port>/admin` or `http://192.168.1.1/`,
//! handing whatever is listening there the browser's own existing cookies
//! and single-sign-on session. `refuse_unsafe_target` closes that — a REAL
//! URL parse (`providers::host_of`, already hardened once against a
//! substring-match bug, reused here rather than a second parser written from
//! scratch) followed by a real IP-range/hostname check, refusing loopback,
//! the three RFC 1918 private ranges, link-local (including the IPv4-mapped
//! AND the older IPv4-compatible IPv6 forms of all of the above), CGNAT
//! (100.64.0.0/10), "this network" (0.0.0.0/8), IETF protocol assignments
//! (192.0.0.0/24), benchmarking (198.18.0.0/15), reserved space and the
//! limited broadcast address (240.0.0.0/4 — added 2026-09-04, see
//! `is_unsafe_ipv4`'s own doc), and anything that looks like an intranet
//! name rather than a public host (`.local`, or no dot at all).
//!
//! **THIS RUNS UNCONDITIONALLY, ON EVERY CALL, REGARDLESS OF WHAT THE PERSON
//! ANSWERED AT THE CONFIRM PROMPT.** The per-call confirm
//! (`engine::native::mod::drive`'s own `OpenUrl` special-case, and
//! `TurnSink::confirm`) is a UI-level "does the person want this", asked
//! BEFORE dispatch even runs; this guard is the actual enforcement, and a
//! "yes" from a person shown a plausible-looking link does not relax it —
//! the whole point of the guard is that the person cannot be expected to
//! notice a URL is actually the router.
//!
//! **TWO ESCAPES CASSANDRA PROVED AGAINST THE FIRST VERSION OF THIS GUARD,
//! 2026-09-04 — both closed here, against the round-27 design already
//! agreed, not a re-decision.** `SAFE-AGENCY-SPEC.md`'s "CASSANDRA ESCAPES"
//! section has her own write-up; this is the fix.
//!
//! 1. **Ambiguous numeric encodings were not canonicalised before judging.**
//!    `http://127.1/`, `http://0177.0.0.1/` (octal), `http://0x7f.0.0.1/`
//!    (hex), and a trailing dot (`http://192.168.1.1./`,
//!    `http://169.254.169.254./`) all reached the LOOSE hostname rules
//!    below rather than the IP-range check, because std's own `IpAddr`
//!    parser — already correctly hardened against octal/hex ambiguity —
//!    refuses every one of those strings, and the old code treated any
//!    refusal as "must be an ordinary hostname." It is not: it is a numeral
//!    written in a form this parser does not accept but a browser or OS
//!    resolver might read differently. `looks_like_a_numeric_ip_attempt`
//!    catches that shape and refuses OUTRIGHT rather than trying to compute
//!    what it "really" means — the fix is not a looser IP parser, std's is
//!    already the right one, it is refusing on ambiguity instead of falling
//!    through past it. The trailing-dot case is simpler: a single trailing
//!    dot is ordinary, legal DNS/FQDN syntax and is stripped before any
//!    other check runs, so `192.168.1.1.` is judged as exactly what it
//!    names.
//! 2. **DNS resolution was never implemented, a locked round-27
//!    requirement.** A public-LOOKING hostname whose own DNS record points
//!    at a private/loopback/link-local address reached no check at all.
//!    `refuse_if_resolves_privately` closes it — see that function's own
//!    doc for the fail-closed timeout reasoning.
//!
//! ## What could NOT be verified from this box, said plainly
//!
//! **`LaunchApp`'s four targets are executable NAMES resolved through
//! Windows' own PATH/app-execution-alias search, not full paths, on purpose
//! — but which concrete program each name actually launches on a real
//! Windows 11 machine (the classic Win32 app, or a packaged Store app behind
//! an app execution alias) was NOT verified here.** `notepad.exe`,
//! `calc.exe`, `explorer.exe` and `taskmgr.exe` are documented app-execution-
//! alias names (Microsoft's own "App execution aliases" feature), and this
//! machine has no Windows kernel to actually run `Start-Process -FilePath
//! calc.exe` against. Beck's own runtime pass needs to prove all four
//! actually launch something recognisable, per `SAFE-AGENCY-SPEC.md`'s
//! build-order note.
//!
//! **The seven Settings URIs were checked against Microsoft's own live
//! reference page** (`learn.microsoft.com/en-us/windows/apps/develop/launch/
//! launch-settings`, fetched 2026-09-04, not recalled from memory) and each
//! one is quoted verbatim from that page's own table — but, same as
//! `LaunchApp`, actually opening one was not verified on a real Windows box.

use serde_json::{json, Value};

use super::tools::ToolResult;
use super::ToolDef;

/// `("app" enum value the model may send", "the exact command spawned")`.
/// **CLOSED VOCABULARY, NO NORMALISATION — SAFE-AGENCY-SPEC.md, explicit.**
/// Matched with `==` against the model's raw string; no case-folding, no
/// trimming beyond what `str_arg` already does, no "close enough". A
/// homoglyph, a path, or `cmd.exe` itself simply do not equal any entry here
/// and fall through to the one generic refusal — see `launch_app`'s own doc
/// for why that refusal is deliberately uninformative.
///
/// These four are Windows' own documented "app execution alias" names — see
/// this file's own header for what was and was not verified about them.
#[cfg(windows)]
const APPS: &[(&str, &str)] = &[
    ("Notepad", "notepad.exe"),
    ("Calculator", "calc.exe"),
    ("File Explorer", "explorer.exe"),
    ("Task Manager", "taskmgr.exe"),
];

/// **THE LINUX PORT'S OWN VERSION OF `APPS`, 2026-09-25.** Windows gets away
/// with one fixed executable name per app because "app execution aliases"
/// are a single, OS-wide, documented naming scheme (see this file's own
/// header). Linux has no equivalent: every desktop environment ships its own
/// name for the "same" utility — GNOME's calculator is `gnome-calculator`,
/// KDE's is `kcalc`, and so on — so a single fixed name would work on
/// whichever desktop it happened to be tested against and error on every
/// other one. That is precisely "an action that always errors" once it
/// leaves this box for a real user's machine, which nobody here runs GNOME.
///
/// Each entry is instead an ORDERED LIST of candidates, most-common-desktop
/// first (checked against current package names, 2026-09-25 — see the
/// sources on `spawn_first_available`'s own doc for the two that were
/// verified rather than assumed: GNOME's editor rename and KDE Plasma 6's
/// system-monitor rename). `launch_app` tries each in turn and uses the
/// first one that actually spawns.
///
/// **STILL NO SHELL, SAME GUARANTEE AS WINDOWS' OWN TABLE, FOR A DIFFERENT
/// REASON.** Windows needs `Start-Process`/PowerShell specifically because
/// `cmd.exe`'s own argument decoding is unsafe (see this file's own header
/// and `connectors::open_in_browser`'s). `std::process::Command::new` on
/// Linux never invokes `/bin/sh` at all — there is no command line for a
/// shell to misparse in the first place, so a bare candidate name straight
/// out of this compile-time table is already exactly as safe as it can be,
/// with no indirection needed to make it so.
#[cfg(target_os = "linux")]
const LINUX_APPS: &[(&str, &[&str])] = &[
    ("Notepad", &["gnome-text-editor", "gedit", "kate", "xed", "mousepad"]),
    ("Calculator", &["gnome-calculator", "kcalc", "galculator", "qalculate-gtk"]),
    ("File Explorer", &["nautilus", "dolphin", "nemo", "pcmanfm", "thunar"]),
    ("Task Manager", &["gnome-system-monitor", "plasma-systemmonitor", "ksysguard", "xfce4-taskmanager", "lxtask"]),
];

/// The four names the model may actually send, for whichever platform this
/// binary was built for — kept as a function rather than a second constant
/// so `definitions()`'s schema `enum` and `launch_app`'s own matching can
/// never quietly drift apart onto two different lists.
#[cfg(windows)]
fn launch_app_names() -> Vec<&'static str> {
    APPS.iter().map(|(name, _)| *name).collect()
}
#[cfg(target_os = "linux")]
fn launch_app_names() -> Vec<&'static str> {
    LINUX_APPS.iter().map(|(name, _)| *name).collect()
}

/// `("page" enum value the model may send", "the exact ms-settings: URI
/// opened")`. Same closed-vocabulary contract as `APPS` above. Every URI is
/// quoted verbatim from Microsoft's own `ms-settings:` reference — see this
/// file's own header. **WINDOWS ONLY — see `definitions()`'s own comment for
/// why Linux gets a clean v1 disable rather than a guessed-at table.**
#[cfg(windows)]
const SETTINGS_PAGES: &[(&str, &str)] = &[
    ("Microphone privacy", "ms-settings:privacy-microphone"),
    ("Camera privacy", "ms-settings:privacy-webcam"),
    ("Bluetooth", "ms-settings:bluetooth"),
    ("Wi-Fi", "ms-settings:network-wifi"),
    ("Display", "ms-settings:display"),
    ("Sound", "ms-settings:sound"),
    ("Windows Update", "ms-settings:windowsupdate"),
];

/// The three tools this module offers, in `tools::definitions`'s
/// strict-compatible shape — see that type's own doc for why every property
/// is listed in `required` even where it reads as optional (none of these
/// three have an optional property at all, so every `required` list here is
/// simply every property).
pub(crate) fn definitions() -> Vec<ToolDef> {
    // `mut` is only exercised on the branch below that pushes
    // `OpenSettingsPage` onto this -- real and needed on Windows, silenced
    // rather than removed everywhere else so the same source reads
    // correctly regardless of which `#[cfg]` branch a given build takes.
    #[cfg_attr(not(windows), allow(unused_mut))]
    let mut defs = vec![
        ToolDef {
            name: "OpenUrl".into(),
            description: "Open one http:// or https:// web page in the user's own, already \
                signed-in browser. The user is asked to confirm before it opens. Refused for any \
                address on this machine's own network -- loopback, a private range, a link-local \
                address, or an intranet-looking hostname -- so this can never be used to reach \
                the router, this machine's own services, or anything else on the local network.".into(),
            strict: true,
            parameters: json!({
                "type": "object",
                "properties": {
                    "url": {
                        "type": "string",
                        "description": "A full http:// or https:// address on the open web."
                    }
                },
                "required": ["url"],
                "additionalProperties": false
            }),
        },
        ToolDef {
            name: "LaunchApp".into(),
            description: "Launch one of a small, fixed set of ordinary apps. No path and no \
                arguments can be given -- anything outside the exact list below is refused.".into(),
            strict: true,
            parameters: json!({
                "type": "object",
                "properties": {
                    "app": {
                        "type": "string",
                        "enum": launch_app_names(),
                        "description": "Which app to launch, spelled exactly as in this list."
                    }
                },
                "required": ["app"],
                "additionalProperties": false
            }),
        },
    ];
    // **A CLEAN v1 DISABLE, NOT A TOOL THAT ALWAYS ERRORS — the Linux port,
    // 2026-09-25.** Windows' `ms-settings:` URIs are a single, OS-wide,
    // documented scheme (this file's own header cites Microsoft's own
    // reference for it). Linux has nothing that plays the same role: GNOME,
    // KDE, XFCE and the rest each own their own settings app with their own
    // deep-link convention, and there is no cross-desktop table that would
    // be honest the way `LaunchApp`'s curated candidate lists are -- a
    // "curated" settings table would really mean picking one desktop's
    // scheme and quietly failing on every other. Rather than offer a tool
    // that refuses every single call it could ever receive (exactly what
    // this file's own header, and `SAFE-AGENCY-SPEC.md`, warn a shipped
    // action must never do), the tool is simply not offered on this
    // platform at all. `open_settings_page` itself still exists for every
    // target below -- `tools.rs`'s dispatch match calls it by name
    // unconditionally -- and gives an honest refusal if it is ever reached
    // anyway (a stale tool schema cached from a session that started before
    // a hot-reload, say).
    #[cfg(windows)]
    defs.push(ToolDef {
        name: "OpenSettingsPage".into(),
        description: "Open one of a small, fixed set of Windows Settings pages -- useful \
            when the user needs to change a permission (like the microphone) or a device \
            setting themselves. Anything outside the exact list below is refused.".into(),
        strict: true,
        parameters: json!({
            "type": "object",
            "properties": {
                "page": {
                    "type": "string",
                    "enum": [
                        "Microphone privacy", "Camera privacy", "Bluetooth", "Wi-Fi",
                        "Display", "Sound", "Windows Update"
                    ],
                    "description": "Which Settings page to open, spelled exactly as in this \
                        list."
                }
            },
            "required": ["page"],
            "additionalProperties": false
        }),
    });
    defs
}

/// **NOT TRIMMED — DELIBERATELY, UNLIKE `tools.rs`'s OWN FILE-PATH HELPERS.**
/// `LaunchApp`/`OpenSettingsPage` are closed-vocabulary, no-normalisation
/// matches (`SAFE-AGENCY-SPEC.md`, explicit): `"Notepad "` with a trailing
/// space must NOT silently become `"Notepad"` and match. `open_url` below
/// trims its own `url` value explicitly, at its own call site, because a
/// URL is not a closed-vocabulary enum and incidental whitespace around one
/// is not a security-relevant distinction the way it is for the other two.
fn str_arg<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key).and_then(|v| v.as_str()).filter(|s| !s.is_empty())
}

/// The message shown at the per-call confirm prompt for `OpenUrl`, and the
/// gate `engine::native::mod::drive` uses to decide whether to dispatch at
/// all — see that function's own comment for why this lives here rather than
/// in `mod.rs` itself (it is the one place that already knows how to read
/// `OpenUrl`'s own argument shape). Returns `None` on malformed/missing
/// arguments so the caller falls through to the ordinary dispatch path and
/// gets the ordinary "url (string) is required" error, rather than this
/// function inventing a second wording for the same failure.
pub(crate) fn confirm_prompt_for_open_url(args_json: &str) -> Option<String> {
    let args: Value = serde_json::from_str(args_json).ok()?;
    let url = str_arg(&args, "url")?.trim();
    if url.is_empty() {
        return None;
    }
    Some(format!("Open {url} in your browser?"))
}

// ---------------------------------------------------------------------------
// OpenUrl
// ---------------------------------------------------------------------------

fn has_scheme(url: &str, scheme: &str) -> bool {
    url.get(..scheme.len()).map(|s| s.eq_ignore_ascii_case(scheme)).unwrap_or(false)
}

/// **HAND-ROLLED PAST THE FIRST FOUR, FOR A DIFFERENT REASON THAN
/// `is_unsafe_ipv6` BELOW.** `is_shared`/`is_reserved`/`is_benchmarking`/
/// `is_broadcast` all exist on `Ipv4Addr` and would name these ranges more
/// readably than octet math does -- but checked against the current nightly
/// docs, 2026-09-04, all four sit behind the unstable `ip` feature
/// (rust-lang/rust#27709) and have never been stable on ANY channel. That is
/// not an MSRV gap this crate could close by bumping `rust-version`; it is
/// "wait for these to stabilise at all," which is not a wait a security gate
/// gets to take. Checked against the ranges directly instead, same approach
/// as `is_unsafe_ipv6`.
///
/// **THE SIX RANGES CASSANDRA PROVED STILL REACHABLE, 2026-09-04 — a browser
/// or a piece of network gear can be listening on every one of these, and
/// none of them is "the open web":**
fn is_unsafe_ipv4(v4: &std::net::Ipv4Addr) -> bool {
    if v4.is_loopback() || v4.is_private() || v4.is_link_local() || v4.is_unspecified() {
        return true;
    }
    let o = v4.octets();
    // 100.64.0.0/10 -- Shared Address Space / carrier-grade NAT (RFC 6598).
    // /10 fixes the top two bits of the second octet at 01, i.e. 64..=127.
    if o[0] == 100 && (o[1] & 0xc0) == 0x40 {
        return true;
    }
    // 0.0.0.0/8 -- "this network" (RFC 791 s3.2). 0.0.0.0 itself is already
    // caught by `is_unspecified` above; this is the rest of the /8 that
    // is_unspecified does not reach, e.g. 0.1.2.3.
    if o[0] == 0 {
        return true;
    }
    // 192.0.0.0/24 -- IETF Protocol Assignments (RFC 6890 s2.1).
    if o[0] == 192 && o[1] == 0 && o[2] == 0 {
        return true;
    }
    // 198.18.0.0/15 -- benchmarking (RFC 2544 s12). /15 fixes every bit of
    // the second octet except the lowest one, so both 18 and 19 are in range.
    if o[0] == 198 && (o[1] & 0xfe) == 18 {
        return true;
    }
    // 240.0.0.0/4 -- reserved for future use (RFC 1112 s4). /4 fixes the top
    // four bits of the first octet, i.e. everything from 240 through 255 --
    // which also covers 255.255.255.255, the limited broadcast address, so
    // that one needs no separate branch of its own.
    if (o[0] & 0xf0) == 0xf0 {
        return true;
    }
    false
}

/// Hand-rolled from the address's own 16-bit segments rather than reaching
/// for `Ipv6Addr::is_unicast_link_local`/`to_ipv4_mapped` — both real,
/// stable methods, but this crate's declared MSRV (`Cargo.toml`,
/// `rust-version = "1.77"`) is older than this file's author could confirm
/// either stabilised by, and a security gate is the wrong place to guess.
/// Checked against the address ranges directly (RFC 4291 §2.5.6 for
/// fe80::/10, RFC 4193 for fc00::/7) rather than against a method name.
fn is_unsafe_ipv6(v6: &std::net::Ipv6Addr) -> bool {
    let seg = v6.segments();
    if seg == [0, 0, 0, 0, 0, 0, 0, 1] {
        return true; // ::1, loopback
    }
    if seg == [0, 0, 0, 0, 0, 0, 0, 0] {
        return true; // ::, unspecified
    }
    if seg[0] & 0xffc0 == 0xfe80 {
        return true; // fe80::/10, link-local
    }
    if seg[0] & 0xfe00 == 0xfc00 {
        return true; // fc00::/7, unique local
    }
    // **BOTH IPv4-EMBEDDING FORMS, ONE CHECK — Cassandra's proven bypass,
    // 2026-09-04: `[::7f00:1]` (127.0.0.1 written as the deprecated but
    // still perfectly valid RFC 4291 §2.5.5.1 "IPv4-compatible" form)
    // walked straight past the ORIGINAL version of this function, which
    // only unwrapped the `::ffff:x.x.x.x` "IPv4-mapped" form (§2.5.5.2).**
    // The two forms differ only in whether segment 5 is 0xffff (mapped) or
    // 0 (compatible); both put a real IPv4 target in the low 32 bits with
    // the high 96 bits otherwise zero. `::1` and `::` are already excluded
    // by the two explicit checks above -- both would ALSO match this
    // shape (decoding as v4 0.0.0.1 and 0.0.0.0), and `::1` in particular
    // must not fall through to here: 0.0.0.1 is not itself flagged unsafe
    // by `is_unsafe_ipv4` (it is not loopback, private, link-local or
    // unspecified), so the dedicated `::1` check above is load-bearing,
    // not redundant with this one.
    if seg[0..5] == [0, 0, 0, 0, 0] && (seg[5] == 0 || seg[5] == 0xffff) {
        let v4 = std::net::Ipv4Addr::new(
            (seg[6] >> 8) as u8,
            seg[6] as u8,
            (seg[7] >> 8) as u8,
            seg[7] as u8,
        );
        return is_unsafe_ipv4(&v4);
    }
    false
}

/// **CASSANDRA'S PROVEN BYPASS #1 — anything that LOOKS like an attempt to
/// write a numeric IP address, in an encoding std's own `IpAddr` parser
/// refuses: octal (`"0177.0.0.1"`), hex (`"0x7f.0.0.1"`), or a short form
/// (`"127.1"`, `"127.0.1"`).** Verified by running std's parser rather than
/// assumed: it already refuses all four of those outright — `Err(invalid IP
/// address syntax)` — which is the CORRECT, already-hardened answer to "is
/// this a real dotted-quad." The bug this function exists to close was
/// never in that parser; it was in what the caller did with a refusal,
/// which used to be "fall through to the loose hostname rules." This is the
/// second half of that fix: catch the shape BEFORE it reaches those rules
/// and refuse it outright instead.
///
/// A bare, dot-free numeral (`"2130706433"`) is already caught separately
/// by the single-label intranet-name rule below -- so this only needs to
/// reach the MULTI-label numeric shapes that rule does not.
///
/// **REFUSED OUTRIGHT, NEVER RE-INTERPRETED.** The fix Cassandra's own
/// write-up asks for is "reject", not "compute what a browser would make of
/// it and judge that instead" -- an ambiguous numeral is exactly the shape a
/// security guard must refuse rather than guess at, because different
/// systems (a browser, an OS resolver, `curl`) are not guaranteed to agree
/// on what it means.
///
/// **EVERY LABEL, NOT ANY ONE OF THEM — Cassandra's second finding on this
/// same guard, 2026-09-04, a functional regression rather than a hole.**
/// `host.split('.').any(...)` refused `0.gravatar.com`,
/// `2.android.pool.ntp.org`, `3.basecamp.com`, `123.example.com` and
/// `0x1a.example.com` — every one an ordinary, legitimately numeric-leading
/// SUBDOMAIN label sitting beside perfectly normal alphabetic ones. A single
/// numeric or hex-marked label among alphabetic ones is not an attempt to
/// write an IP address at all; only a host where EVERY label reads that way
/// is. This is also the stricter and MORE defensible reading of the rule: a
/// real TLD is never purely numeric (ICANN does not allow one), so an
/// "every label numeric-or-0x" host can never be a genuine multi-label DNS
/// name in the first place -- `.all(...)` loses no real coverage against
/// the six proven bypasses (`127.1`, `0177.0.0.1`, `0x7f.0.0.1` are caught
/// here; the trailing-dot and v4-compatible-v6 cases are caught earlier, as
/// real IP literals, and never reach this function at all).
fn looks_like_a_numeric_ip_attempt(host: &str) -> bool {
    let mut labels = host.split('.').peekable();
    labels.peek().is_some()
        && labels.all(|label| {
            !label.is_empty()
                && (label.bytes().all(|b| b.is_ascii_digit()) || label.to_ascii_lowercase().starts_with("0x"))
        })
}

/// How long `refuse_if_resolves_privately` waits for a name to resolve
/// before giving up. See that function's own doc for why a timeout is
/// treated exactly like a bad answer rather than like "could not check."
const DNS_RESOLVE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);

/// **PURE — the actual security judgment, split out from the real DNS
/// lookup below on purpose, the same "decision here, plumbing there" split
/// this house already uses throughout (`providers.rs`'s own `provider_env`/
/// `apply_env` split is the model for this).** Given a hostname and the set
/// of addresses it resolved to, refuse if ANY of them is
/// loopback/private/link-local/unspecified — a name is only as safe as its
/// least safe answer, because that is the one a browser's own resolver may
/// pick. An EMPTY list is refused too: "resolved to nothing" is not
/// "resolved to something safe."
fn judge_resolved_ips(host: &str, ips: &[std::net::IpAddr]) -> Result<(), String> {
    if ips.is_empty() {
        return Err(format!(
            "OpenUrl could not resolve {host} to a real address, so it was refused."
        ));
    }
    for ip in ips {
        let unsafe_ip = match ip {
            std::net::IpAddr::V4(v4) => is_unsafe_ipv4(v4),
            std::net::IpAddr::V6(v6) => is_unsafe_ipv6(v6),
        };
        if unsafe_ip {
            return Err(format!(
                "OpenUrl refuses {host} -- it resolves to an address on this machine's own                  network ({ip}), which is exactly what this guard exists to stop even when the                  name itself looks public."
            ));
        }
    }
    Ok(())
}

/// **THE LOCKED ROUND-27 REQUIREMENT — DNS resolution, Cassandra's second
/// proven gap, 2026-09-04: a public-LOOKING hostname whose own DNS record
/// points at a private/loopback/link-local address reached no check at
/// all.** A hostname that passed every earlier check in
/// `refuse_unsafe_target` (a real scheme, not localhost/.local/single-label,
/// not an ambiguous numeral) is resolved for real, on a background thread
/// with a hard `DNS_RESOLVE_TIMEOUT`, and refused if the judgment above
/// refuses any address it comes back with.
///
/// **FAILS CLOSED ON A TIMEOUT OR A RESOLVE ERROR, DELIBERATELY, AND THAT IS
/// A ONE-LINE TRADEOFF WORTH STATING: over-blocking costs one retry,
/// under-blocking costs the browser's own cookies.** This tool is opt-in
/// (`Provider::allow_agency`) and, for `OpenUrl` specifically, confirmed per
/// call (`TurnSink::confirm`) — a person who hits a slow or failed lookup
/// presses the same button again a moment later. A name this function
/// cannot verify is a name it must not wave through.
///
/// The lookup thread is deliberately NOT joined on a timeout — there is no
/// cooperative way to cancel a blocking OS resolver call (`getaddrinfo` and
/// its equivalents give the caller no cancellation handle), so this
/// function returns within its own stated budget regardless of how long the
/// OS takes, at the cost of a background thread that outlives the call by
/// however long the real resolution eventually takes. Bounded by how often
/// this tool is actually invoked, not by anything unbounded.
fn refuse_if_resolves_privately(host: &str) -> Result<(), String> {
    let target = format!("{host}:0");
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        use std::net::ToSocketAddrs;
        let result = target
            .to_socket_addrs()
            .map(|addrs| addrs.map(|sa| sa.ip()).collect::<Vec<_>>());
        // The receiver may already be gone (this function timed out and
        // returned) -- that is not a failure worth reporting, there is
        // nobody left to tell.
        let _ = tx.send(result);
    });
    match rx.recv_timeout(DNS_RESOLVE_TIMEOUT) {
        Ok(Ok(ips)) => judge_resolved_ips(host, &ips),
        Ok(Err(e)) => Err(format!("OpenUrl could not resolve {host} ({e}), so it was refused.")),
        Err(_) => Err(format!(
            "OpenUrl could not resolve {host} in time, so it was refused rather than guessed at."
        )),
    }
}

/// **THE NETWORK-TARGET GUARD — SAFE-AGENCY-SPEC.md room amendment 1.** See
/// this file's own header for the full reasoning. `Ok(())` means safe to
/// proceed; `Err` carries the refusal text the model (and, through it, the
/// person) sees.
fn refuse_unsafe_target(url: &str) -> Result<(), String> {
    let trimmed = url.trim();
    if !has_scheme(trimmed, "http://") && !has_scheme(trimmed, "https://") {
        // Catches javascript:, data:, file://, ms-settings:, a bare string
        // with no scheme at all, and a scheme spelled in any case -- the
        // check is case-insensitive so `HTTP://` cannot walk around it.
        return Err(
            "OpenUrl only opens http:// or https:// links -- that address was refused.".into(),
        );
    }
    let raw_host = match crate::providers::host_of(trimmed) {
        Some(h) if !h.is_empty() => h,
        _ => return Err("That does not look like a real web address.".into()),
    };

    // **CANONICALISE BEFORE JUDGING — Cassandra's proven bypass #1,
    // 2026-09-04.** A single trailing dot is ordinary, legal DNS/FQDN
    // syntax (`192.168.1.1.` names the exact same host as `192.168.1.1`),
    // and stripping it once, here, before anything else runs, is what lets
    // every check below see the address it is actually judging rather than
    // a string one character longer than the one on its own allow/deny
    // list. Only ONE dot is stripped -- more than one is not valid FQDN
    // syntax to begin with, and accepting it as if it were would be
    // loosening a rule rather than following it.
    let host = raw_host.strip_suffix('.').unwrap_or(&raw_host);

    // A bracketed IPv6 literal (`host_of` keeps the brackets -- see its own
    // doc) is checked as an IP; std's parser does not accept the brackets,
    // so they are stripped first, here, rather than taught to `host_of`,
    // which has its own reasons for keeping them (see that function's doc).
    let ip_text = host.strip_prefix('[').and_then(|s| s.strip_suffix(']')).unwrap_or(host);
    if let Ok(ip) = ip_text.parse::<std::net::IpAddr>() {
        let unsafe_ip = match &ip {
            std::net::IpAddr::V4(v4) => is_unsafe_ipv4(v4),
            std::net::IpAddr::V6(v6) => is_unsafe_ipv6(v6),
        };
        return if unsafe_ip {
            Err(format!(
                "OpenUrl refuses to open an address on this machine's own network ({host}) -- \
                 that would hand a hijacked call the browser's existing logins for whatever is \
                 listening there."
            ))
        } else {
            Ok(())
        };
    }
    // **CASSANDRA'S PROVEN BYPASS #1.** `ip_text` did not parse as a real
    // IP literal by std's own (already correctly hardened) rules -- but it
    // may still LOOK like an attempt to write one: octal (`0177.0.0.1`),
    // hex (`0x7f.0.0.1`), or a short form (`127.1`). Refused outright: see
    // `looks_like_a_numeric_ip_attempt`'s own doc for why this is a
    // refusal, never a second, looser parser.
    if looks_like_a_numeric_ip_attempt(host) {
        return Err(format!(
            "OpenUrl refuses an address written as a number ({host}) -- an ambiguous numeral is \
             refused outright rather than guessed at, because different systems can read one \
             differently."
        ));
    }

    // Not a numeral at all -- an ordinary-looking hostname. `localhost`,
    // anything ending `.local` (mDNS/Bonjour's own convention for a machine
    // on this network), and any single-label name with no dot at all (the
    // shape of an intranet host -- `router`, `printer`, `nas` -- rather
    // than a public domain) are all refused the same way an IP-range hit
    // is, for the same reason.
    let lower = host.to_ascii_lowercase();
    if lower == "localhost" || lower.ends_with(".local") || !lower.contains('.') {
        return Err(format!(
            "OpenUrl refuses to open what looks like a machine or intranet name ({host}) rather \
             than an address on the open web."
        ));
    }

    // **CASSANDRA'S PROVEN BYPASS #2, THE LOCKED ROUND-27 REQUIREMENT.** The
    // name itself looks public; what it actually resolves to is the last
    // word. See `refuse_if_resolves_privately`'s own doc for the fail-closed
    // timeout reasoning.
    refuse_if_resolves_privately(host)
}

pub(crate) fn open_url(args: &Value) -> ToolResult {
    let url = match str_arg(args, "url").map(str::trim).filter(|s| !s.is_empty()) {
        Some(u) => u,
        None => return ToolResult { output: "url (string) is required.".into(), is_error: true },
    };
    if let Err(reason) = refuse_unsafe_target(url) {
        return ToolResult { output: reason, is_error: true };
    }
    // `connectors::open_url` is the SAME command the Connections view's own
    // "finish this connection" button calls -- reused rather than a second
    // copy of its scheme check and its call into `open_in_browser`. It
    // enforces http(s)-or-the-one-microphone-URI; the network-target guard
    // above has already run by the time this is reached, so the two checks
    // are additive, not a race to see which one applies.
    match crate::connectors::open_url(url.to_string()) {
        Ok(()) => ToolResult { output: format!("Opened {url}"), is_error: false },
        Err(e) => ToolResult { output: e, is_error: true },
    }
}

// ---------------------------------------------------------------------------
// LaunchApp
// ---------------------------------------------------------------------------

pub(crate) fn launch_app(args: &Value) -> ToolResult {
    let requested = match str_arg(args, "app") {
        Some(a) => a,
        None => return ToolResult { output: "app (string) is required.".into(), is_error: true },
    };

    #[cfg(windows)]
    {
        let Some((_, target)) = APPS.iter().find(|(name, _)| *name == requested) else {
            // **DELIBERATELY GENERIC — SAFE-AGENCY-SPEC.md: "refusal msg must
            // not leak which apps exist."** No echo of the full list, no
            // "did you mean", no different wording for a path vs a
            // homoglyph vs `cmd.exe` itself -- every non-match gets this one
            // sentence, so a fuzzing attempt learns nothing from the shape
            // of the refusal that the tool schema's own `enum` did not
            // already tell it.
            return ToolResult {
                output: "That is not one of the apps this can launch.".into(),
                is_error: true,
            };
        };
        return match crate::connectors::open_in_browser(target) {
            Ok(()) => ToolResult { output: format!("Launched {requested}"), is_error: false },
            Err(e) => ToolResult { output: format!("Could not launch {requested}: {e}"), is_error: true },
        };
    }

    #[cfg(target_os = "linux")]
    {
        let Some((_, candidates)) = LINUX_APPS.iter().find(|(name, _)| *name == requested) else {
            // Same deliberately-generic refusal as the Windows branch above.
            return ToolResult {
                output: "That is not one of the apps this can launch.".into(),
                is_error: true,
            };
        };
        return match spawn_first_available(candidates) {
            Ok(()) => ToolResult { output: format!("Launched {requested}"), is_error: false },
            Err(e) => ToolResult { output: format!("Could not launch {requested}: {e}"), is_error: true },
        };
    }

    #[allow(unreachable_code)]
    ToolResult { output: "Launching apps is not available on this platform.".into(), is_error: true }
}

/// **THE LINUX SPAWN PRIMITIVE — no `open_in_browser`, deliberately.** That
/// function's whole job on Linux is handing a string to `xdg-open`, which
/// resolves a FILE or a URL to its default handler; it has no way to launch
/// an arbitrary named PROGRAM the way Windows' `Start-Process -FilePath
/// <name>` does (Windows resolves a bare name through PATH/App Paths/app
/// execution aliases, which is the whole mechanism `APPS`'s table leans on).
/// `Command::new(candidate).spawn()` is the direct Linux equivalent of that
/// same resolution — PATH search, no shell in between — so this is not a
/// second, less-safe spawn path, it is the same "no shell, ever" guarantee
/// applied the way this platform actually offers it.
///
/// **CANDIDATE NAMES VERIFIED AGAINST CURRENT SOURCES, 2026-09-25, NOT
/// CARRIED FROM MEMORY — the two most likely to have drifted:** GNOME's
/// default editor is `gnome-text-editor`, not `gedit`, since GNOME 42
/// (2022) — <https://gitlab.gnome.org/GNOME/gnome-text-editor>, kept `gedit`
/// as a fallback for the desktops that never migrated. KDE Plasma 6 renamed
/// its process monitor to `plasma-systemmonitor`; `ksysguard` is kept as a
/// fallback for Plasma 5 machines —
/// <https://archlinux.org/packages/extra/x86_64/plasma-systemmonitor/>.
#[cfg(target_os = "linux")]
fn spawn_first_available(candidates: &[&str]) -> Result<(), String> {
    // A candidate that is not installed fails to spawn with `ENOENT` -- the
    // ordinary, expected way most of these are skipped on any one real
    // machine (nobody has all five desktop environments' utilities
    // installed at once). Only the LAST failure is worth reporting; the
    // earlier ones are not errors, they are "not this desktop."
    let mut last_err: Option<String> = None;
    for &name in candidates {
        match std::process::Command::new(name).spawn() {
            Ok(_) => return Ok(()),
            Err(e) => last_err = Some(format!("{name}: {e}")),
        }
    }
    Err(match last_err {
        Some(detail) => format!(
            "none of the {} candidate programs for this app are installed on this machine \
             (last tried: {detail})",
            candidates.len()
        ),
        None => "no candidate programs are configured for this app".into(),
    })
}

// ---------------------------------------------------------------------------
// OpenSettingsPage
// ---------------------------------------------------------------------------

pub(crate) fn open_settings_page(args: &Value) -> ToolResult {
    let requested = match str_arg(args, "page") {
        Some(p) => p,
        None => return ToolResult { output: "page (string) is required.".into(), is_error: true },
    };

    #[cfg(windows)]
    {
        let Some((_, uri)) = SETTINGS_PAGES.iter().find(|(name, _)| *name == requested) else {
            // Same reasoning as `launch_app`'s own refusal -- generic, no list
            // echoed back.
            return ToolResult {
                output: "That is not one of the Settings pages this can open.".into(),
                is_error: true,
            };
        };
        return match crate::connectors::open_in_browser(uri) {
            Ok(()) => ToolResult { output: format!("Opened Settings: {requested}"), is_error: false },
            Err(e) => {
                ToolResult { output: format!("Could not open Settings: {requested}: {e}"), is_error: true }
            }
        };
    }

    // **THE v1 DISABLE ITSELF — see `definitions()`'s own comment for why
    // this platform never offers the tool in the first place.** Kept as a
    // real function rather than removed so `tools.rs`'s unconditional
    // dispatch match still compiles for every target, and so a call that
    // somehow reaches here anyway (a stale cached schema) gets an honest
    // sentence instead of a panic or a silent no-op.
    #[allow(unreachable_code)]
    {
        let _ = requested;
        ToolResult {
            output: "Settings pages are not available in this build yet.".into(),
            is_error: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(json_str: &str) -> Value {
        serde_json::from_str(json_str).unwrap()
    }

    #[test]
    fn every_tool_definition_is_strict_compatible() {
        for t in definitions() {
            let props = t.parameters.get("properties").and_then(|p| p.as_object()).unwrap();
            let required: Vec<&str> = t.parameters["required"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_str().unwrap())
                .collect();
            for key in props.keys() {
                assert!(
                    required.contains(&key.as_str()),
                    "{}: {key} is a property but not required -- strict mode rejects that",
                    t.name
                );
            }
            assert_eq!(
                t.parameters["additionalProperties"],
                Value::Bool(false),
                "{}: additionalProperties must be false",
                t.name
            );
        }
    }

    // -- OpenUrl: the scheme gate -----------------------------------------

    #[test]
    fn open_url_refuses_javascript_data_file_and_ms_settings_schemes() {
        for url in [
            "javascript:alert(1)",
            "data:text/html,<script>alert(1)</script>",
            "file:///etc/passwd",
            "file://C:/Windows/System32/config/SAM",
            "ms-settings:privacy-microphone",
            "not-a-url-at-all",
            "",
        ] {
            let r = refuse_unsafe_target(url);
            assert!(r.is_err(), "{url:?} must be refused, got Ok");
        }
    }

    #[test]
    fn open_url_refusal_is_case_insensitive_on_the_scheme() {
        // A model writing `HTTP://` or `HtTpS://` must not walk around the
        // scheme check by casing alone. An IP LITERAL, not a hostname --
        // this test is about the SCHEME check, and a hostname would drag in
        // a real DNS lookup (`refuse_if_resolves_privately`) to prove a fact
        // that has nothing to do with DNS.
        assert!(refuse_unsafe_target("HTTP://93.184.216.34/").is_ok());
        assert!(refuse_unsafe_target("HtTpS://93.184.216.34/").is_ok());
        assert!(refuse_unsafe_target("JAVASCRIPT:alert(1)").is_err());
    }

    // -- OpenUrl: hostile strings that would matter if this ever reached a
    // -- command line, which it never does -- proven by construction (no
    // -- `Command`/`format!` building a shell string anywhere in this file),
    // -- and re-asserted here as the safety-test cases the design doc named.

    #[test]
    fn open_url_hostile_shell_metacharacters_never_change_the_scheme_decision() {
        // `&|^`$()` and a second `&calc.exe` are cmd.exe/PowerShell
        // metacharacters -- irrelevant to `refuse_unsafe_target`'s own job
        // (it never spawns anything), but the URL must still be judged
        // purely on its scheme and host, exactly as an ordinary URL would
        // be, with no special-casing that a crafted string could trip. An
        // IP LITERAL host, same reasoning as the case-insensitivity test
        // above -- this test is about the QUERY STRING, not DNS.
        let hostile = "https://93.184.216.34/x?a=1&b=`whoami`&c=$(calc)&d=^&calc.exe";
        assert!(refuse_unsafe_target(hostile).is_ok(), "a public https URL with a hostile QUERY \
            STRING is still a public https URL; the metacharacters live in the query, never on a \
            command line, so this must not be refused for that reason");
        // CRLF injection into the URL string itself.
        assert!(refuse_unsafe_target("https://93.184.216.34/\r\nSet-Cookie: x=1").is_ok());
    }

    // -- OpenUrl: the network-target guard, IPv4 -----------------------------

    #[test]
    fn open_url_refuses_loopback_private_link_local_and_unspecified_ipv4() {
        for host in [
            "127.0.0.1", "127.5.5.5", "10.99.99.1", "172.16.0.1", "172.31.255.255",
            "192.168.1.1", "169.254.1.1", "0.0.0.0",
        ] {
            let url = format!("http://{host}/");
            assert!(refuse_unsafe_target(&url).is_err(), "{url} must be refused");
        }
    }

    #[test]
    fn open_url_allows_ordinary_public_ipv4_literals() {
        // IP LITERALS ONLY -- an ordinary public HOSTNAME now also has to
        // survive `refuse_if_resolves_privately`'s real DNS lookup, which
        // this test deliberately does not exercise (see the `#[ignore]`d
        // `open_url_allows_a_real_public_hostname_after_dns_resolution`
        // below for that, real network and all). An IP literal never
        // reaches the resolver at all -- it is judged and returned from
        // inside the very first branch of `refuse_unsafe_target` -- so this
        // stays fast, offline, and deterministic.
        for url in ["http://93.184.216.34/", "https://8.8.8.8/"] {
            assert!(refuse_unsafe_target(url).is_ok(), "{url} must be allowed");
        }
    }

    #[test]
    fn open_url_does_not_refuse_the_boundary_just_outside_a_private_range() {
        // 172.15.x and 172.32.x are OUTSIDE 172.16/12 -- a range check that
        // is off by one octet either way is the classic way this kind of
        // guard silently over- or under-refuses.
        assert!(refuse_unsafe_target("http://172.15.255.255/").is_ok());
        assert!(refuse_unsafe_target("http://172.32.0.0/").is_ok());
    }

    // -- OpenUrl: the six IPv4 ranges Cassandra proved were still ALLOWED,
    // -- 2026-09-04, against the version of `is_unsafe_ipv4` that only ever
    // -- checked loopback/private/link-local/unspecified. Each of these is
    // -- an address a piece of network gear or a carrier can be listening on
    // -- that is not, in any sense, "the open web." ------------------------

    #[test]
    fn open_url_refuses_cgnat_this_network_protocol_benchmarking_and_reserved_ipv4() {
        for host in [
            "100.64.0.1",  // 100.64.0.0/10, CGNAT / Shared Address Space (RFC 6598)
            "0.1.2.3",     // 0.0.0.0/8, "this network" (RFC 791 s3.2)
            "192.0.0.1",   // 192.0.0.0/24, IETF Protocol Assignments (RFC 6890)
            "198.18.0.1",  // 198.18.0.0/15, benchmarking (RFC 2544)
            "240.0.0.1",   // 240.0.0.0/4, reserved (RFC 1112)
            "255.255.255.255", // limited broadcast -- inside 240.0.0.0/4 too
        ] {
            let url = format!("http://{host}/");
            assert!(refuse_unsafe_target(&url).is_err(), "{url} must be refused");
            assert!(is_unsafe_ipv4(&host.parse().unwrap()), "{host} must be flagged unsafe directly");
        }
    }

    #[test]
    fn open_url_does_not_refuse_the_boundaries_just_outside_the_six_new_ranges() {
        // Same reasoning as the 172.16/12 boundary test above -- a range
        // check that is off by one is the classic way this over-refuses.
        for host in [
            "100.63.255.255", // just below 100.64.0.0/10
            "100.128.0.0",    // just above it
            "192.0.1.0",      // just outside 192.0.0.0/24
            "198.17.255.255", // just below 198.18.0.0/15
            "198.20.0.0",     // just above it
            "239.255.255.255", // just below 240.0.0.0/4
        ] {
            assert!(!is_unsafe_ipv4(&host.parse().unwrap()), "{host} must NOT be flagged unsafe");
            assert!(refuse_unsafe_target(&format!("http://{host}/")).is_ok(), "{host} must be allowed");
        }
    }

    #[test]
    fn open_url_still_allows_ordinary_public_ipv4_after_the_six_new_ranges() {
        // The fix must not have widened anything -- a normal public address
        // still goes through untouched.
        assert!(refuse_unsafe_target("http://93.184.216.34/").is_ok());
    }

    // -- OpenUrl: Cassandra's proven bypass #1, canonicalisation
    // -- (2026-09-04) -- each of these returned Ok from the version of this
    // -- guard shipped one round earlier; every one must now refuse. -------

    #[test]
    fn open_url_refuses_a_trailing_dot_on_a_private_ipv4_address() {
        // http://192.168.1.1./ and http://169.254.169.254./ (the metadata
        // address) -- PROVEN able to pass as "not an IP literal, not
        // localhost, not .local, has a dot" before the trailing-dot strip
        // was added.
        for url in ["http://192.168.1.1./", "http://169.254.169.254./"] {
            assert!(refuse_unsafe_target(url).is_err(), "{url} must be refused");
        }
    }

    #[test]
    fn open_url_refuses_a_short_form_ipv4_address() {
        // http://127.1/ -- the classic BSD/curl shorthand for 127.0.0.1.
        // Two dot-separated, all-digit labels; std's own IpAddr parser
        // correctly refuses it as not a dotted-quad, and the bug was
        // treating that refusal as "must be a hostname."
        assert!(refuse_unsafe_target("http://127.1/").is_err());
        assert!(refuse_unsafe_target("http://127.0.1/").is_err());
    }

    #[test]
    fn open_url_refuses_an_octal_shaped_ipv4_octet() {
        // http://0177.0.0.1/ -- 0177 is 127 in octal (the classic
        // `inet_aton` ambiguity), and, separately, 177 in ordinary decimal.
        // Neither reading may win by default: refused outright.
        assert!(refuse_unsafe_target("http://0177.0.0.1/").is_err());
    }

    #[test]
    fn open_url_refuses_a_hex_shaped_ipv4_octet() {
        // http://0x7f.0.0.1/ -- 0x7f is 127 in hex.
        assert!(refuse_unsafe_target("http://0x7f.0.0.1/").is_err());
    }

    #[test]
    fn open_url_refuses_a_bare_decimal_ipv4_integer() {
        // http://2130706433/ -- the whole of 127.0.0.1 written as one
        // 32-bit decimal integer. No dot at all, so this is ALSO covered by
        // the single-label intranet-name rule -- proven here directly
        // rather than assumed from that rule's own test.
        assert!(refuse_unsafe_target("http://2130706433/").is_err());
    }

    // -- Cassandra's SECOND finding on this same guard, 2026-09-04: a
    // -- functional regression, not a hole -- `looks_like_a_numeric_ip_
    // -- attempt`'s `.any(...)` wrongly refused an ordinary hostname that
    // -- merely HAS a numeric-leading label somewhere in it. Fixed to
    // -- `.all(...)`; these are the proof, both directions. -----------------

    #[test]
    fn a_single_numeric_or_hex_leading_label_among_ordinary_ones_is_not_a_numeric_ip_attempt() {
        // PROVEN wrongly refused by the `.any(...)` version, 2026-09-04:
        // every one of these is an ordinary public hostname whose leading
        // label happens to be a digit or two, sitting beside perfectly
        // normal alphabetic labels -- `0.gravatar.com`'s "0" is a CDN shard
        // number, `2.android.pool.ntp.org`'s "2" is an NTP pool rotation
        // index, and so on. None of these is an attempt to write an IP
        // address; the classification function itself is what is being
        // proven here, on purpose, rather than the full `refuse_unsafe_
        // target` pipeline -- two of the five (`123.example.com`,
        // `0x1a.example.com`) do not actually resolve over real DNS
        // (verified against this box, 2026-09-04: `socket.gethostbyname`
        // raises for both), so asserting the END-TO-END function would
        // fail for a reason that has nothing to do with this bug. The other
        // three (`0.gravatar.com`, `2.android.pool.ntp.org`,
        // `3.basecamp.com`) DO resolve, and get their own real-DNS,
        // `#[ignore]`d, end-to-end proof below.
        for host in [
            "0.gravatar.com",
            "2.android.pool.ntp.org",
            "3.basecamp.com",
            "123.example.com",
            "0x1a.example.com",
        ] {
            assert!(
                !looks_like_a_numeric_ip_attempt(host),
                "{host} is an ordinary hostname, not a numeric IP encoding attempt"
            );
        }
    }

    #[test]
    fn a_host_where_every_label_is_numeric_or_hex_is_still_a_numeric_ip_attempt() {
        // The three of the six proven bypasses that actually reach this
        // function (the other three -- trailing-dot and the v4-compatible
        // IPv6 form -- are caught earlier, as real IP literals, and never
        // reach `looks_like_a_numeric_ip_attempt` at all). Re-asserted
        // directly against the classification function, not only against
        // the full pipeline (which is ALSO still proven, by the three
        // `open_url_refuses_a_*` tests immediately above this block).
        for host in ["127.1", "127.0.1", "0177.0.0.1", "0x7f.0.0.1"] {
            assert!(
                looks_like_a_numeric_ip_attempt(host),
                "{host} is every label numeric/0x -- must still be caught"
            );
        }
    }

    /// **REAL DNS, DELIBERATELY IGNORED BY DEFAULT — same reasoning as
    /// `open_url_allows_a_real_public_hostname_after_dns_resolution` above.**
    /// The three of Cassandra's five examples that actually resolve
    /// (verified against this box, 2026-09-04), proven through the WHOLE
    /// pipeline this time -- classification, hostname rules, and the real
    /// resolver -- not only the classification function alone.
    #[test]
    #[ignore = "resolves gravatar.com/ntp.org/basecamp.com over real DNS"]
    fn open_url_allows_real_hostnames_with_a_numeric_leading_label() {
        for url in [
            "https://0.gravatar.com/",
            "https://2.android.pool.ntp.org/",
            "https://3.basecamp.com/",
        ] {
            assert!(refuse_unsafe_target(url).is_ok(), "{url} must be allowed");
        }
    }

    #[test]
    fn open_url_refuses_the_v4_compatible_ipv6_form_of_loopback() {
        // http://[::7f00:1]/ -- 127.0.0.1 written as the deprecated but
        // still syntactically valid RFC 4291 IPv4-compatible IPv6 form.
        // PROVEN by Cassandra as "suspected" and confirmed here directly:
        // std's own parser accepts this string as ordinary IPv6 (it is
        // one), and the original `is_unsafe_ipv6` only ever unwrapped the
        // OTHER embedding form (`::ffff:x.x.x.x`).
        assert!(refuse_unsafe_target("http://[::7f00:1]/").is_err());
    }

    #[test]
    fn open_url_still_allows_an_ordinary_ipv6_address_with_a_zero_prefix() {
        // Not every address with a zero-heavy prefix is unsafe -- the
        // v4-compatible/mapped unwrap must judge the EMBEDDED address on its
        // own merits, not refuse every zero-prefixed v6 address on sight.
        // `::5db8:d822` decodes, by the IPv4-compatible reading, to
        // 93.184.216.34 -- the same public address used as the ordinary-IPv4
        // stand-in elsewhere in this file -- and must be allowed.
        //
        // **THIS USED TO ASSERT `::2` -- CHANGED 2026-09-04, alongside
        // `is_unsafe_ipv4` gaining the six new ranges above.** `::2` decodes
        // to 0.0.0.2, which is genuinely inside 0.0.0.0/8 ("this network",
        // RFC 791) and is therefore now, correctly, refused -- proven by its
        // own dedicated case in `open_url_refuses_cgnat_this_network_
        // protocol_benchmarking_and_reserved_ipv4` above. It was never a good
        // example of "an embedded address is fine on its own merits"; it
        // happened to read as fine only because the merits check used to be
        // narrower than it should have been. `::2` is asserted REFUSED,
        // directly, immediately below, so this fix does not quietly lose
        // that case.
        assert!(refuse_unsafe_target("http://[::5db8:d822]/").is_ok());
        assert!(refuse_unsafe_target("http://[::2]/").is_err());
    }

    // -- refuse_unsafe_target's remaining shape, still IP literal, still
    // -- offline -----------------------------------------------------------

    // -- OpenUrl: the network-target guard, IPv6 -----------------------------

    #[test]
    fn open_url_refuses_loopback_link_local_and_unique_local_ipv6() {
        for host in ["[::1]", "[fe80::1]", "[fc00::1]", "[fd12:3456:789a::1]"] {
            let url = format!("https://{host}/");
            assert!(refuse_unsafe_target(&url).is_err(), "{url} must be refused");
        }
    }

    #[test]
    fn open_url_refuses_ipv4_mapped_ipv6_loopback() {
        // ::ffff:127.0.0.1 -- the outer address family is v6, the real
        // target is loopback. A check that only looks at the v6 ranges and
        // never unwraps this form is exactly the hole this test exists to
        // catch.
        assert!(refuse_unsafe_target("https://[::ffff:127.0.0.1]/").is_err());
        assert!(refuse_unsafe_target("https://[::ffff:7f00:1]/").is_err()); // same address, hex form
    }

    #[test]
    fn open_url_allows_ordinary_public_ipv6() {
        assert!(refuse_unsafe_target("https://[2001:4860:4860::8888]/").is_ok());
    }

    // -- OpenUrl: the network-target guard, hostnames ------------------------

    #[test]
    fn open_url_refuses_localhost_dotlocal_and_single_label_intranet_names() {
        for url in [
            "http://localhost/", "http://LOCALHOST/", "http://printer.local/",
            "http://router/", "http://nas/", "http://fileserver/share",
        ] {
            assert!(refuse_unsafe_target(url).is_err(), "{url} must be refused");
        }
    }

    // -- OpenUrl: Cassandra's proven bypass #2, DNS resolution (2026-09-04),
    // -- the locked round-27 requirement `refuse_unsafe_target` never
    // -- implemented at all. `judge_resolved_ips` below is the PURE half --
    // -- fast, offline, deterministic -- and is where the actual security
    // -- judgment lives; `refuse_if_resolves_privately`'s own real-DNS half
    // -- is covered separately, `#[ignore]`d, by the two tests after it. --

    #[test]
    fn judge_resolved_ips_refuses_a_private_a_record() {
        let ips = [std::net::IpAddr::V4(std::net::Ipv4Addr::new(10, 0, 0, 5))];
        assert!(judge_resolved_ips("internal.example.com", &ips).is_err());
    }

    #[test]
    fn judge_resolved_ips_refuses_a_loopback_a_record() {
        let ips = [std::net::IpAddr::V4(std::net::Ipv4Addr::new(127, 0, 0, 1))];
        assert!(judge_resolved_ips("rebind.example.com", &ips).is_err());
    }

    #[test]
    fn judge_resolved_ips_refuses_a_link_local_aaaa_record() {
        let ips = [std::net::IpAddr::V6(std::net::Ipv6Addr::new(0xfe80, 0, 0, 0, 0, 0, 0, 1))];
        assert!(judge_resolved_ips("rebind6.example.com", &ips).is_err());
    }

    #[test]
    fn judge_resolved_ips_refuses_if_any_one_of_several_records_is_private() {
        // A name that resolves to BOTH a real public address and a private
        // one -- refused, because the private one is the one a browser's
        // own resolver might pick, and "mostly safe" is not safe.
        let ips = [
            std::net::IpAddr::V4(std::net::Ipv4Addr::new(93, 184, 216, 34)),
            std::net::IpAddr::V4(std::net::Ipv4Addr::new(192, 168, 1, 1)),
        ];
        assert!(judge_resolved_ips("mixed.example.com", &ips).is_err());
    }

    #[test]
    fn judge_resolved_ips_refuses_an_empty_answer() {
        // Resolved to nothing at all -- fail closed, same reasoning as a
        // timeout: "could not verify" is not "verified safe."
        assert!(judge_resolved_ips("nothing.example.com", &[]).is_err());
    }

    #[test]
    fn judge_resolved_ips_allows_an_ordinary_public_answer() {
        let ips = [
            std::net::IpAddr::V4(std::net::Ipv4Addr::new(93, 184, 216, 34)),
            std::net::IpAddr::V6(std::net::Ipv6Addr::new(0x2606, 0x2800, 0x220, 1, 0x248, 0x1893, 0x25c8, 0x1946)),
        ];
        assert!(judge_resolved_ips("real.example.com", &ips).is_ok());
    }

    /// **REAL DNS, DELIBERATELY IGNORED BY DEFAULT — same house pattern
    /// already used throughout this crate for anything that needs a live
    /// endpoint** (`providers.rs`'s own `#[ignore = "talks to the real ...
    /// endpoint"]` tests). `nip.io` is a real, public, third-party DNS
    /// service built for exactly this purpose: `<anything>.nip.io`
    /// resolves, over genuine DNS, to whatever IP address is embedded in
    /// the label -- confirmed against this box, 2026-09-04, `python3 -c
    /// "import socket; print(socket.gethostbyname('127.0.0.1.nip.io'))"`
    /// answered `127.0.0.1`. This is the ONE test in this file that proves
    /// `refuse_if_resolves_privately`'s actual resolver call and its
    /// timeout wiring, rather than only the pure judgment above it.
    #[test]
    #[ignore = "resolves 127.0.0.1.nip.io over real DNS"]
    fn open_url_refuses_a_public_looking_hostname_that_resolves_to_a_private_address() {
        assert!(refuse_unsafe_target("http://127.0.0.1.nip.io/").is_err());
    }

    /// **THE POSITIVE CASE, SAME REASONING, ALSO REAL DNS.** `example.com`
    /// is IANA's own permanently-reserved, stable test domain -- about as
    /// safe a real-network dependency as this house's own tests already
    /// accept for other endpoints. Without this test, every hostname case
    /// in this file would be a REFUSAL; this is the one proof that an
    /// ordinary public name actually gets through the resolve step, not
    /// only that a bad one is stopped by it.
    #[test]
    #[ignore = "resolves example.com over real DNS"]
    fn open_url_allows_a_real_public_hostname_after_dns_resolution() {
        assert!(refuse_unsafe_target("https://example.com/").is_ok());
    }

    // -- confirm_prompt_for_open_url ------------------------------------

    #[test]
    fn confirm_prompt_names_the_real_url() {
        let p = confirm_prompt_for_open_url(r#"{"url":"https://example.com/x"}"#).unwrap();
        assert!(p.contains("https://example.com/x"), "{p}");
    }

    #[test]
    fn confirm_prompt_is_none_on_malformed_or_missing_args() {
        assert!(confirm_prompt_for_open_url("not json").is_none());
        assert!(confirm_prompt_for_open_url(r#"{"url":""}"#).is_none());
        assert!(confirm_prompt_for_open_url(r#"{}"#).is_none());
    }

    // -- LaunchApp: closed vocabulary, no normalisation --------------------

    #[test]
    #[cfg(windows)]
    fn launch_app_accepts_only_the_exact_four_names() {
        for app in ["Notepad", "Calculator", "File Explorer", "Task Manager"] {
            assert!(APPS.iter().any(|(name, _)| *name == app), "{app} must be in the table");
        }
    }

    /// The Linux port's own table keeps the SAME four enum names as
    /// Windows -- only what each one launches differs (a list of
    /// candidates, tried in order, rather than one fixed executable) -- so
    /// the model-facing vocabulary is identical across both builds even
    /// though `LINUX_APPS`'s shape is not `APPS`'s.
    #[test]
    #[cfg(target_os = "linux")]
    fn launch_app_accepts_only_the_exact_four_names() {
        for app in ["Notepad", "Calculator", "File Explorer", "Task Manager"] {
            assert!(LINUX_APPS.iter().any(|(name, _)| *name == app), "{app} must be in the table");
        }
    }

    /// Every candidate list actually has at least one entry -- an app whose
    /// table row is `(name, &[])` would match `requested` successfully and
    /// then refuse with "no candidate programs are configured", which reads
    /// to a person exactly like a bug rather than like `launch_app`'s
    /// deliberate, generic "not one of the apps" refusal for an unknown
    /// name. This is a config-shape guarantee about the const table itself,
    /// not about any one machine's installed software.
    #[test]
    #[cfg(target_os = "linux")]
    fn every_linux_app_has_at_least_one_candidate() {
        for (name, candidates) in LINUX_APPS {
            assert!(!candidates.is_empty(), "{name} has no candidate programs listed");
        }
    }

    #[test]
    fn launch_app_refuses_cmd_exe_paths_traversal_and_homoglyphs() {
        let hostile = [
            "cmd.exe",
            "cmd",
            r"C:\Windows\System32\notepad.exe",
            "../../../Windows/System32/cmd.exe",
            "notepad.exe /c calc.exe",
            "notepad",         // close, but not the exact enum spelling
            "NOTEPAD",         // wrong case -- no normalisation
            "Notepad ",        // trailing content past what str_arg's trim allows through unmatched
            "Nоtepad",         // Cyrillic 'о' (U+043E) in place of Latin 'o' -- a homoglyph
            "",
        ];
        for app in hostile {
            let a = args(&format!(r#"{{"app":{app:?}}}"#));
            let r = launch_app(&a);
            assert!(r.is_error, "{app:?} must be refused, got: {}", r.output);
        }
    }

    #[test]
    fn launch_app_refusal_does_not_enumerate_the_real_app_list() {
        let a = args(r#"{"app":"cmd.exe"}"#);
        let r = launch_app(&a);
        for real in ["notepad.exe", "calc.exe", "explorer.exe", "taskmgr.exe"] {
            assert!(!r.output.contains(real), "refusal leaked a real target: {}", r.output);
        }
    }

    #[test]
    fn launch_app_requires_the_app_field() {
        let r = launch_app(&args(r#"{}"#));
        assert!(r.is_error);
    }

    // -- OpenSettingsPage: same closed-vocabulary contract -----------------

    #[test]
    #[cfg(windows)]
    fn open_settings_page_accepts_only_the_seven_named_pages() {
        assert_eq!(SETTINGS_PAGES.len(), 7);
        for (_, uri) in SETTINGS_PAGES {
            assert!(uri.starts_with("ms-settings:"), "{uri} is not an ms-settings: URI");
        }
    }

    /// **THE CLEAN v1 DISABLE, PROVEN FROM BOTH ENDS — the Linux port,
    /// 2026-09-25.** Not offered to the model at all (`definitions()`
    /// leaves it out entirely, same shape as `allow_agency` gating the other
    /// three tools off the list in `tools.rs`'s own test for that), AND the
    /// function itself refuses honestly if it is ever reached anyway.
    /// Both matter: the first is what stops the model from ever trying;
    /// the second is what happens to a stale cached schema from a build
    /// that still had it.
    #[test]
    #[cfg(target_os = "linux")]
    fn open_settings_page_is_a_clean_v1_disable_on_linux() {
        assert!(
            !definitions().iter().any(|t| t.name == "OpenSettingsPage"),
            "OpenSettingsPage must not be offered on Linux -- there is no honest cross-desktop \
             table for it, and offering it would mean every call refuses, exactly what a v1 \
             disable exists to avoid"
        );
        let r = open_settings_page(&args(r#"{"page":"Microphone privacy"}"#));
        assert!(r.is_error, "a direct call must still refuse honestly if it is ever reached: {}", r.output);
    }

    #[test]
    fn open_settings_page_refuses_anything_off_the_list() {
        for page in ["Privacy", "privacy-microphone", "ms-settings:privacy-microphone", "About", ""] {
            let a = args(&format!(r#"{{"page":{page:?}}}"#));
            let r = open_settings_page(&a);
            assert!(r.is_error, "{page:?} must be refused, got: {}", r.output);
        }
    }
}
