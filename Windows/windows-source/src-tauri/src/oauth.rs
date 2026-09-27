//! Signing in to a hosted connector, the way an end user expects.
//!
//! Mark, 2026-08-26: *"we are setting this up for end users not us.. so we need
//! to think like that."* He said it after I proudly reported that his Gmail was
//! already connected — true, and true only because HE has connectors set up on
//! his own Claude account. A person who installs this tomorrow has none, and
//! telling them to go and configure things on a website is not a product.
//!
//! **WHAT AN END USER SHOULD GET**, and what he described watching a competitor
//! do: press Connect, the browser opens, they sign in to their own Notion or
//! Slack account, and it works. No API key to find in a settings page nobody
//! can navigate, no subscription to a middleman, no developer account.
//!
//! **WHY THAT IS POSSIBLE WITHOUT REGISTERING NAMEOS ANYWHERE.** These servers
//! publish `/.well-known/oauth-authorization-server` with a
//! `registration_endpoint` — dynamic client registration. The app registers
//! itself at the moment somebody presses Connect and gets a client id back.
//! Checked against Notion, Linear, Canva and Slack before this file existed;
//! all four advertise it.
//!
//! **THE PIECES, and each one is here because leaving it out breaks something
//! specific:**
//!
//! - **PKCE.** The redirect lands on a loopback port that any other program on
//!   the machine could also have been listening on. Without the code verifier,
//!   an authorization code seen by anything else is usable. With it, it is not.
//! - **`state`, checked on return.** A browser can be sent to our callback by
//!   any page on the internet. Without this we would happily exchange a code
//!   that somebody else's site handed us.
//! - **One-shot loopback listener on a port the OS picks.** A fixed port is one
//!   more thing that can already be in use, and a listener that outlives the
//!   flow is a hole left open for no reason.
//! - **A timeout.** Somebody closes the tab and walks away; a thread waiting
//!   forever on a socket is a leak that never announces itself.
//!
//! The token goes into the OS credential store through `connectors.rs`, which
//! already refuses to write a secret to disk. Nothing here changes that rule.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::{BufRead, BufReader, Write};
use std::net::{IpAddr, TcpListener, ToSocketAddrs};
use std::time::Duration;

/// How long to wait for somebody to finish signing in before giving up. Long
/// enough to find a password, short enough that an abandoned attempt does not
/// hold a socket open for the rest of the session.
const WAIT: Duration = Duration::from_secs(300);
const NET: Duration = Duration::from_secs(20);
/// How many redirects we will follow by hand before giving up. Each hop is
/// re-validated; this only bounds a redirect loop.
const MAX_REDIRECTS: usize = 5;

#[derive(Debug, Deserialize)]
struct Meta {
    authorization_endpoint: String,
    token_endpoint: String,
    #[serde(default)]
    registration_endpoint: String,
}

#[derive(Debug, Serialize)]
pub struct Signed {
    /// The bearer token to store. Never returned to the front end — see the
    /// caller in `connectors.rs`.
    ///
    /// **`#[serde(skip)]` ON EVERY SECRET FIELD.** Nothing serializes `Signed`
    /// to the webview today, but a future command that returns it must not be
    /// the thing that leaks a token. The skip makes that leak impossible rather
    /// than merely absent — the value is not in the serialized form at all.
    #[serde(skip)]
    pub access_token: String,
    // Returned for the caller to persist, but `connectors.rs` does not store it
    // yet (the connector token-refresh flow is a tracked follow-up), so it reads
    // as dead until then. `allow` rather than deletion keeps the return contract
    // whole so wiring persistence later does not reopen this file.
    #[serde(skip)]
    #[allow(dead_code)]
    pub refresh_token: String,
    /// WHO they signed in as, when the service says so. Mark, 2026-08-26: "if
    /// it is connected, it needs to show what username it is connected with".
    /// He is right that a green dot with no name is half an answer — on a
    /// machine with two Slack accounts it is no answer at all.
    /// Best effort by design: not every token response carries it, and an
    /// invented name would be worse than a blank. Empty means we do not know,
    /// and the window says nothing rather than guessing.
    pub account: String,
    /// THE REFRESH MATERIAL, returned so the caller can persist it. None of the
    /// three below is ever sent to the front end — `connectors.rs` stores them
    /// in the OS credential store, exactly as it does `access_token`.
    ///
    /// `client_secret` is non-empty ONLY for a confidential client: some
    /// authorization servers (Monday) issue a per-install secret at dynamic
    /// registration regardless of our asking for a public one, and their token
    /// endpoint then requires it. A public client leaves this blank and nothing
    /// changes. `client_id` and `token_endpoint` travel with it because a
    /// refresh needs all three plus the refresh token, and storing half of that
    /// set is storing none of it.
    pub client_id: String,
    #[serde(skip)]
    #[allow(dead_code)] // persisted by the tracked refresh follow-up; see refresh_token.
    pub client_secret: String,
    pub token_endpoint: String,
}

fn b64url(bytes: &[u8]) -> String {
    // base64url without padding, by hand. One small function against a
    // dependency whose only job here is thirty lines of table lookup.
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        if chunk.len() > 1 { out.push(T[(n >> 6) as usize & 63] as char); }
        if chunk.len() > 2 { out.push(T[n as usize & 63] as char); }
    }
    out
}

/// Enough entropy for a verifier and a state value, without pulling in a
/// random-number crate: the OS gives us this directly.
fn nonce() -> String {
    let mut buf = [0u8; 32];
    getrandom(&mut buf);
    b64url(&buf)
}

#[cfg(unix)]
fn getrandom(buf: &mut [u8]) {
    use std::io::Read;
    // /dev/urandom is the OS's own generator; nothing here is rolling its own.
    let mut f = std::fs::File::open("/dev/urandom").expect("no /dev/urandom");
    f.read_exact(buf).expect("could not read random bytes");
}

#[cfg(windows)]
fn getrandom(buf: &mut [u8]) {
    #[link(name = "bcrypt")]
    extern "system" {
        fn BCryptGenRandom(h: *mut core::ffi::c_void, p: *mut u8, c: u32, f: u32) -> i32;
    }
    // BCRYPT_USE_SYSTEM_PREFERRED_RNG — the platform's own CSPRNG.
    let ok = unsafe { BCryptGenRandom(std::ptr::null_mut(), buf.as_mut_ptr(), buf.len() as u32, 2) };
    assert!(ok == 0, "BCryptGenRandom failed");
}

/// Every endpoint in the discovery document comes off the network, and the
/// document is served by whoever the user is trying to connect to.
///
/// **THE SCAR: `authorization_endpoint` WAS THE ONE VALUE IN THIS FILE THAT WAS
/// NEVER PASSED THROUGH `urlencode`.** Cassandra, 2026-08-28. It is pasted
/// straight into the URL built below and then handed to `open()`, which on
/// Windows went to `cmd.exe` — so a hosted connector could answer this request
/// with `"authorization_endpoint": "https://ok.example/a&<command>&"` and get
/// it run when the user pressed Sign in. `connectors.rs` no longer gives cmd a
/// command line to parse, and this is the other half: the string is checked at
/// the point it enters the program, so no future caller inherits a raw one.
///
/// **WHAT IS ENFORCED, AND WHERE IT COMES FROM.** RFC 6749 §3.1
/// (<https://www.rfc-editor.org/rfc/rfc6749#section-3.1>):
///
/// > "the authorization server MUST require the use of TLS"
/// > "The endpoint URI MAY include an 'application/x-www-form-urlencoded'
/// > formatted query component ... which MUST be retained when adding
/// > additional query parameters."
/// > "The endpoint URI MUST NOT include a fragment component."
///
/// So: https only, no fragment, an existing query is kept (see the join
/// below), and none of the characters that RFC 3986 does not permit unescaped
/// in a URI — which is the same set that makes a string dangerous to hand to
/// any shell, so one rule buys both.
fn checked_endpoint(raw: &str, what: &str) -> Result<String, String> {
    let url = raw.trim();
    // A URL long enough to be a problem is not a URL anybody meant to publish.
    if url.is_empty() || url.len() > 2048 {
        return Err(format!("Its sign-in description gave an unusable {what}."));
    }
    // RFC 6749 §3.1: TLS is required. This also refuses `file:`, `ms-settings:`
    // and anything else a scheme-blind check would have let through.
    if !url.starts_with("https://") {
        return Err(format!(
            "Its {what} is not an https address, so we stopped rather than sending \
             your sign-in over it."
        ));
    }
    // Something has to be there after the scheme.
    let rest = &url["https://".len()..];
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    if host.is_empty() {
        return Err(format!("Its sign-in description gave an unusable {what}."));
    }
    // RFC 6749 §3.1: no fragment. Ours would be lost behind it anyway.
    if url.contains('#') {
        return Err(format!("Its {what} carries a fragment, which is not allowed here."));
    }
    /* Characters RFC 3986 does not allow unescaped in a URI. `&` is NOT in this
       list and must not be — it is legal in a query and every OAuth URL has
       one. The defence against `&` is that the URL never reaches a command-line
       parser (connectors.rs), not that we ban a legal character and break real
       services. */
    const FORBIDDEN: &[char] =
        &[' ', '"', '\'', '<', '>', '\\', '^', '`', '{', '}', '|', '\t', '\n', '\r'];
    if url.chars().any(|c| c.is_control() || FORBIDDEN.contains(&c)) {
        return Err(format!(
            "Its {what} contains characters that are not valid in a web address, so it \
             was refused."
        ));
    }
    Ok(url.to_string())
}

/// The host of an https URL, lowercased, or None if there is not one. Used to
/// bind every network-supplied address in discovery back to the server we
/// actually meant to connect to.
fn host_of(url: &str) -> Option<String> {
    let after = url.trim().split("://").nth(1)?;
    let host = after.split(['/', '?', '#']).next()?;
    (!host.is_empty()).then(|| host.to_ascii_lowercase())
}

/// TRUST BOUNDARY. `b` is an address that came off the network; it is accepted
/// only if it lives on the same host as `a`, the connector we are driving.
///
/// This is the check that stops a hostile server pointing discovery at somebody
/// else's site — see `accept_resource_metadata` and `auth_server_from_prm` for
/// exactly which network-supplied values are held to it and why.
fn same_host(a: &str, b: &str, what: &str) -> Result<(), String> {
    match (host_of(a), host_of(b)) {
        (Some(x), Some(y)) if x == y => Ok(()),
        _ => Err(format!(
            "Its {what} pointed at a different site than the one being connected to, so it was \
             refused."
        )),
    }
}

/// Is this a resolved address we refuse to send a discovery request to?
///
/// Covers loopback, RFC1918 private space, link-local (incl. `169.254.0.0/16`,
/// the cloud-metadata range), the unspecified address, IPv6 ULA and IPv6
/// link-local, and EVERY transition form that carries an embedded IPv4 —
/// IPv4-mapped (`::ffff:a.b.c.d`), NAT64 (`64:ff9b::a.b.c.d`) and the deprecated
/// IPv4-compatible (`::a.b.c.d`) — each checked as its v4 address so a literal
/// that ROUTES to an internal v4 cannot read as public. Legitimate authorization
/// servers are public hostnames (`www.dropbox.com`, `github.com`), so nothing
/// real is caught here.
fn is_denied_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_loopback()       // 127.0.0.0/8
                || v4.is_private()  // 10/8, 172.16/12, 192.168/16
                || v4.is_link_local() // 169.254.0.0/16 (cloud metadata)
                || v4.is_unspecified() // 0.0.0.0
                || v4.is_broadcast()
        }
        IpAddr::V6(v6) => {
            let seg = v6.segments();
            // Any embedded-IPv4 transition form is checked as its v4 address, so
            // a literal that routes to an internal v4 cannot read as public.
            let embed = |hi: u16, lo: u16| {
                std::net::Ipv4Addr::new((hi >> 8) as u8, hi as u8, (lo >> 8) as u8, lo as u8)
            };
            // ::ffff:a.b.c.d — e.g. ::ffff:127.0.0.1.
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_denied_ip(IpAddr::V4(v4));
            }
            let is_nat64 = seg[0] == 0x0064
                && seg[1] == 0xff9b
                && seg[2..6].iter().all(|&s| s == 0);
            // ::a.b.c.d (deprecated), EXCLUDING ::1 and :: — those are
            // loopback/unspecified, handled below, and must not be re-read as an
            // embedded 0.0.0.1 / 0.0.0.0.
            let is_v4compat = seg[0..6].iter().all(|&s| s == 0)
                && !v6.is_loopback()
                && !v6.is_unspecified();
            if is_nat64 || is_v4compat {
                return is_denied_ip(IpAddr::V4(embed(seg[6], seg[7])));
            }
            let seg0 = seg[0];
            v6.is_loopback()                 // ::1
                || v6.is_unspecified()       // ::
                || (seg0 & 0xffc0) == 0xfe80 // fe80::/10 link-local
                || (seg0 & 0xfe00) == 0xfc00 // fc00::/7 unique-local
        }
    }
}

/// TRUST BOUNDARY — SSRF. Refuse a URL we are about to FETCH if it resolves to,
/// or is literally, an internal address. `checked_endpoint` already forces
/// https (which blocks `http://127.0.0.1` and `file:`), but https loopback,
/// RFC1918 hosts and `169.254.169.254` would otherwise sail through — and the
/// authorization server in a protected-resource document is deliberately NOT
/// bound to the connector's host, so a hostile server a user added could aim it
/// at cloud metadata or a service on the user's own network.
///
/// A literal IP is checked directly; a name is blocked outright if it is
/// `localhost`/`*.localhost`/`*.local`, otherwise resolved and every address it
/// maps to is checked. DNS REBIND IS OUT OF SCOPE: this resolves once and ureq
/// resolves again to connect, so a name that answers differently between the two
/// could still slip an internal address through. Closing that needs a resolver
/// that hands the checked IP straight to the connection, which this HTTP client
/// does not expose; the literal-IP and name checks are what stop the common
/// case a connector document can trigger.
fn deny_internal(url: &str, what: &str) -> Result<(), String> {
    let host = host_of(url).ok_or_else(|| format!("Its {what} had no host to check."))?;
    // Separate an IPv6 literal in brackets, or strip a numeric :port.
    let bare = if let Some(rest) = host.strip_prefix('[') {
        rest.split(']').next().unwrap_or(&rest).to_string()
    } else {
        match host.rsplit_once(':') {
            Some((h, p)) if !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()) => h.to_string(),
            _ => host.clone(),
        }
    };
    let refuse = || {
        Err(format!(
            "Its {what} pointed at an internal address, which is not a place we will send a \
             sign-in request."
        ))
    };
    if bare == "localhost" || bare.ends_with(".localhost") || bare.ends_with(".local") {
        return refuse();
    }
    if let Ok(ip) = bare.parse::<IpAddr>() {
        return if is_denied_ip(ip) { refuse() } else { Ok(()) };
    }
    // A name: resolve and refuse if ANY address it maps to is internal.
    match (bare.as_str(), 443u16).to_socket_addrs() {
        Ok(addrs) => {
            let mut saw_one = false;
            for a in addrs {
                saw_one = true;
                if is_denied_ip(a.ip()) {
                    return refuse();
                }
            }
            if !saw_one {
                return Err(format!("Its {what} did not resolve to any address."));
            }
            Ok(())
        }
        Err(_) => Err(format!("Its {what} could not be resolved, so it was refused.")),
    }
}

/// Resolve a `Location` header against the URL it came from. Handles absolute,
/// scheme-relative (`//host/…`), root-relative (`/path`) and plain-relative
/// forms. Scheme is not trusted from the header: an absolute `http://` target is
/// returned as-is and refused a moment later by `checked_endpoint`, so a
/// redirect cannot downgrade https to http.
fn resolve_location(base: &str, loc: &str) -> Option<String> {
    let loc = loc.trim();
    if loc.is_empty() {
        return None;
    }
    if loc.starts_with("https://") || loc.starts_with("http://") {
        return Some(loc.to_string());
    }
    let after = base.split("://").nth(1)?;
    let host = after.split(['/', '?', '#']).next()?;
    if let Some(rest) = loc.strip_prefix("//") {
        return Some(format!("https://{rest}"));
    }
    if loc.starts_with('/') {
        return Some(format!("https://{host}{loc}"));
    }
    // Relative to the base's directory.
    let path = after[host.len()..].split(['?', '#']).next().unwrap_or("");
    let dir = match path.rfind('/') {
        Some(i) => &path[..=i],
        None => "/",
    };
    Some(format!("https://{host}{dir}{loc}"))
}

/// The next URL to fetch after a 3xx, fully re-validated. This is the whole
/// point of following redirects by hand: `deny_internal` on the FINAL string is
/// worthless if the client silently follows a `Location` to an internal address,
/// so every hop is put back through the SAME two gates (https via
/// `checked_endpoint`, not-internal via `deny_internal`) before we go there.
fn next_hop(current: &str, location: Option<&str>) -> Result<String, String> {
    let loc = location.ok_or("A redirect during sign-in gave no destination.")?;
    let next = resolve_location(current, loc)
        .ok_or("A redirect during sign-in gave an unusable destination.")?;
    let next = checked_endpoint(&next, "sign-in address")?;
    deny_internal(&next, "sign-in address")?;
    Ok(next)
}

/// EVERY outbound request in discovery and sign-in goes through here.
///
/// **WHY NOT `ureq`'s OWN REDIRECT HANDLING.** ureq 2.12 follows up to five
/// redirects with no per-hop callback, so a hostile server on a real public host
/// passes the string check and then answers `302 Location:
/// https://169.254.169.254/…` (or a `127.0.0.1`/RFC1918 target) and the client
/// walks straight to the internal address. On a 307/308 from a POST it re-sends
/// the body — the auth code and client secret — to that address. So auto-follow
/// is turned OFF (`redirects(0)`) and each hop is followed by hand through
/// `next_hop`, which re-runs the full guard before any bytes go anywhere.
///
/// Returns the transport outcome of the FINAL hop for the caller to read: a
/// `4xx` stays an `Err(Status)` (so `initialize`'s 401 carries its
/// `WWW-Authenticate` header, and a 404 well-known reads as "not here"), while a
/// hop that fails the guard, a `Location` we cannot use, or a redirect loop is a
/// hard `Err(String)` that stops the whole flow.
fn guarded(
    mut url: String,
    post: bool,
    content_type: &str,
    accept: &str,
    body: &str,
) -> Result<Result<ureq::Response, ureq::Error>, String> {
    // Hop 0 is validated too: this is the defence-in-depth that also covers the
    // connector's own first fetches (the classic host-root GET and the modern
    // initialize POST), which used to reach the network unchecked.
    url = checked_endpoint(&url, "sign-in address")?;
    deny_internal(&url, "sign-in address")?;

    // Redirect limits live on the agent in ureq 2.x, not the request: this agent
    // will not follow anything, so every hop comes back to us for revalidation.
    let agent = ureq::builder().redirects(0).build();

    for _ in 0..=MAX_REDIRECTS {
        let mut req = if post { agent.post(&url) } else { agent.get(&url) };
        req = req.timeout(NET);
        if post {
            req = req.set("content-type", content_type);
        }
        if !accept.is_empty() {
            req = req.set("accept", accept);
        }
        let outcome = if post { req.send_string(body) } else { req.call() };

        match outcome {
            // With auto-follow off, a 3xx comes back as Ok — follow it by hand.
            Ok(resp) if (300..400).contains(&resp.status()) => {
                url = next_hop(&url, resp.header("location"))?;
                continue;
            }
            other => return Ok(other),
        }
    }
    Err("Sign-in redirected too many times.".into())
}

/// CHECKED IN ONE PLACE. Every `Meta` is built here, so a validated one is the
/// only kind that exists and nothing downstream has to remember.
/// `registration_endpoint` is optional — empty is a real answer meaning "set
/// this up by hand" — but a non-empty one is held to the same rule.
fn validate_meta(mut meta: Meta) -> Result<Meta, String> {
    meta.authorization_endpoint = checked_endpoint(&meta.authorization_endpoint, "sign-in address")?;
    meta.token_endpoint = checked_endpoint(&meta.token_endpoint, "token address")?;
    if !meta.registration_endpoint.trim().is_empty() {
        meta.registration_endpoint =
            checked_endpoint(&meta.registration_endpoint, "registration address")?;
    }
    Ok(meta)
}

/// Find the authorization-server metadata, two ways, host-root first.
///
/// **CLASSIC (RFC 8414 at the host root)** is what the working servers publish
/// and must keep working untouched. **MODERN (the MCP protected-resource flow)**
/// is the fallback for a server that does not publish at the host root and
/// instead tells us where to look in a 401 to an unauthenticated `initialize`
/// (GitHub, Dropbox, Box). The fallback runs ONLY when the classic location
/// yields nothing usable, so no working connector changes behaviour.
fn discover(url: &str) -> Result<Meta, String> {
    if let Some(meta) = discover_host_root(url)? {
        return Ok(meta);
    }
    discover_via_protected_resource(url)
}

/// RFC 8414 at the host root.
///
/// `Ok(None)` means "nothing usable here, let the modern flow try": that covers
/// both a non-2xx response AND a body that does not parse as discovery metadata
/// (some hosts serve an unrelated page at that path). Escalating in those cases
/// is safe because the modern flow carries its own guards — host-binding and the
/// SSRF denylist. TWO cases are a hard error rather than a fall-through: a
/// document that parses but carries an invalid or hostile endpoint
/// (`validate_meta` → `Err`), and a fetch that fails the redirect/SSRF guard
/// (`guarded` → `Err(String)` via `?`). Neither is retried, so a hostile
/// classic-path response cannot silently escalate to a second attempt.
fn discover_host_root(url: &str) -> Result<Option<Meta>, String> {
    let base = {
        let u = url.trim_end_matches('/');
        let after = u.split("://").nth(1).unwrap_or(u);
        let host = after.split('/').next().unwrap_or(after);
        format!("https://{host}")
    };
    let doc = match guarded(
        format!("{base}/.well-known/oauth-authorization-server"),
        false,
        "",
        "",
        "",
    )? {
        Ok(r) => r.into_string().map_err(|e| e.to_string())?,
        // Any non-2xx (404/401/429/5xx) means the classic location did not
        // answer for us; the modern flow gets its turn.
        Err(_) => return Ok(None),
    };
    // Present but not a discovery document: also a "try the other way", not an
    // error — some hosts serve an unrelated page at that path.
    match serde_json::from_str::<Meta>(&doc) {
        Ok(meta) => validate_meta(meta).map(Some),
        Err(_) => Ok(None),
    }
}

/// The JSON-RPC `initialize` an MCP server answers, anonymously, with a 401 that
/// names its metadata. Kept in one place so the two callers cannot drift.
fn initialize_body() -> String {
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": { "name": "helloim.ai", "version": env!("CARGO_PKG_VERSION") }
        }
    })
    .to_string()
}

/// Pull `resource_metadata="…"` out of a `WWW-Authenticate` header value. The
/// header is a comma-separated `Bearer` challenge; we want the one parameter
/// that says where the protected-resource metadata lives.
fn resource_metadata_from_header(header: &str) -> Option<String> {
    let at = header.find("resource_metadata=")?;
    let rest = &header[at + "resource_metadata=".len()..];
    let rest = rest.trim_start();
    if let Some(stripped) = rest.strip_prefix('"') {
        stripped.split('"').next().map(str::to_string)
    } else {
        // Unquoted form: read up to the next comma or whitespace.
        rest.split([',', ' ', ';']).next().map(str::to_string)
    }
}

/// Decide whether a `WWW-Authenticate` challenge's metadata pointer may be
/// fetched, given the connector it came from. PURE so the wrong-call path is
/// unit-testable without a network.
///
/// **TWO GATES, BOTH REQUIRED.** `checked_endpoint` (https only — this also
/// refuses `file:` and `http://127.0.0.1`, so a server cannot aim discovery at
/// a local service) AND `same_host`: protected-resource metadata is fetched
/// ONLY from the host we are connecting to. Without the host gate a hostile
/// server could answer `initialize` with `resource_metadata="https://evil/…"`
/// and we would fetch an attacker-authored document naming an attacker's
/// authorization server — the whole point of the sign-in is that the browser
/// lands on the REAL service.
fn accept_resource_metadata(connector_url: &str, header: &str) -> Result<String, String> {
    let rm = resource_metadata_from_header(header)
        .ok_or("Its sign-in challenge did not say where to find its metadata.")?;
    let rm = checked_endpoint(&rm, "sign-in metadata address")?;
    same_host(connector_url, &rm, "sign-in metadata address")?;
    Ok(rm)
}

/// Read the authorization server out of a protected-resource metadata document,
/// bound to the connector it describes. PURE — the network fetch is the
/// caller's job — so the wrong-call path is unit-testable.
///
/// The `resource` field must name the same host we are connecting to: it is the
/// document asserting *what it is metadata for*, and a document claiming to
/// describe someone else is one we refuse rather than trust. The authorization
/// server itself is legitimately on a DIFFERENT host (Dropbox's is
/// `www.dropbox.com`, GitHub's is `github.com`), so it is held to
/// `checked_endpoint` (https) but not to `same_host`.
fn auth_server_from_prm(connector_url: &str, body: &str) -> Result<String, String> {
    let v: serde_json::Value = serde_json::from_str(body)
        .map_err(|e| format!("Its sign-in metadata could not be read: {e}"))?;
    let resource = v.get("resource").and_then(|r| r.as_str()).unwrap_or_default();
    same_host(connector_url, resource, "sign-in metadata")?;
    let issuer = v
        .get("authorization_servers")
        .and_then(|a| a.as_array())
        .and_then(|a| a.first())
        .and_then(|s| s.as_str())
        .ok_or("Its sign-in metadata did not name an authorization server.")?;
    checked_endpoint(issuer, "authorization server address")
}

/// Where an issuer publishes its RFC 8414 metadata, in the order to try. RFC
/// 8414 §3.1 inserts the well-known segment between host and path, so a
/// path-bearing issuer (`https://auth.monday.com/mcp`) is asked path-aware
/// first, then at the host root as a fallback.
fn as_metadata_candidates(issuer: &str) -> Vec<String> {
    let iss = issuer.trim_end_matches('/');
    let after = iss.split("://").nth(1).unwrap_or(iss);
    let (host, path) = match after.find('/') {
        Some(i) => (&after[..i], &after[i..]),
        None => (after, ""),
    };
    let mut out = Vec::new();
    if !path.is_empty() {
        out.push(format!("https://{host}/.well-known/oauth-authorization-server{path}"));
        out.push(format!("https://{host}/.well-known/openid-configuration{path}"));
    }
    out.push(format!("https://{host}/.well-known/oauth-authorization-server"));
    out.push(format!("https://{host}/.well-known/openid-configuration"));
    out
}

/// The modern MCP discovery chain, every network-supplied URL checked.
fn discover_via_protected_resource(url: &str) -> Result<Meta, String> {
    // 1. Unauthenticated initialize. A driveable server answers 401 with a
    //    `WWW-Authenticate` naming its metadata; a 2xx means there is nothing
    //    here to sign in to and nothing to discover.
    let header = match guarded(
        url.to_string(),
        true,
        "application/json",
        "application/json, text/event-stream",
        &initialize_body(),
    )? {
        Ok(_) => {
            return Err("This service does not offer a sign-in we can drive.".into());
        }
        Err(ureq::Error::Status(_, resp)) => resp
            .header("www-authenticate")
            .map(str::to_string)
            .ok_or("This service refused the request without saying how to sign in.")?,
        Err(ureq::Error::Transport(t)) => {
            return Err(format!("This service does not offer a sign-in we can drive: {t}"));
        }
    };

    // 2. The metadata pointer — checked and bound to this host, then fetched
    //    through the guard (which re-checks it and every redirect hop).
    let rm = accept_resource_metadata(url, &header)?;
    let prm = match guarded(rm, false, "", "", "")? {
        Ok(r) => r.into_string().map_err(|e| e.to_string())?,
        Err(e) => return Err(format!("Could not read its sign-in metadata: {e}")),
    };

    // 3. The authorization server it names — host-bound resource, then the
    //    guarded fetch below refuses it (and any redirect) if it is internal.
    //    This URL is deliberately allowed to be a DIFFERENT host, so the SSRF
    //    denylist inside `guarded` is what protects it, per hop.
    let issuer = auth_server_from_prm(url, &prm)?;

    // 4. That server's RFC 8414 metadata, path-aware then host-root. A candidate
    //    that simply is not there (404/transport) moves on to the next; a
    //    candidate that redirects to an internal address is a hard refusal and
    //    stops the flow via `?`.
    for candidate in as_metadata_candidates(&issuer) {
        if let Ok(resp) = guarded(candidate, false, "", "", "")? {
            if let Ok(doc) = resp.into_string() {
                if let Ok(meta) = serde_json::from_str::<Meta>(&doc) {
                    return validate_meta(meta);
                }
            }
        }
    }
    Err("Its authorization server did not publish a sign-in description we could read.".into())
}

/// What dynamic registration gives back. `secret` is empty for the public
/// clients most of these servers issue; non-empty when the server insists on a
/// confidential one (see the note in `Signed`).
struct Registered {
    client_id: String,
    client_secret: String,
}

/// Register this copy of the app and return its client credentials.
///
/// We ask for a public client (`token_endpoint_auth_method: "none"`), which is
/// what a distributed desktop app should be. Some servers honour that; some
/// (Monday) hand back a `client_secret` regardless — a per-install secret, not
/// one shared across the world's installs — and their token endpoint then
/// requires it. We take whichever they give and let `sign_in` use it the
/// matching way.
fn register(meta: &Meta, redirect: &str) -> Result<Registered, String> {
    if meta.registration_endpoint.trim().is_empty() {
        return Err("This service needs an account set up with them by hand.".into());
    }
    let body = serde_json::json!({
        "client_name": "helloim.ai",
        "redirect_uris": [redirect],
        "grant_types": ["authorization_code", "refresh_token"],
        "response_types": ["code"],
        "token_endpoint_auth_method": "none",
    });
    // SSRF: `guarded` vets the registration endpoint (and any redirect it
    // returns) at the point of use, regardless of which discovery path produced
    // it — a 307/308 here would otherwise re-POST our registration request to
    // wherever it pointed.
    let res = match guarded(
        meta.registration_endpoint.clone(),
        true,
        "application/json",
        "",
        &body.to_string(),
    )? {
        Ok(r) => r.into_string().map_err(|e| e.to_string())?,
        Err(e) => return Err(format!("Could not register with the service: {e}")),
    };
    let v: serde_json::Value = serde_json::from_str(&res)
        .map_err(|e| format!("The service's registration reply could not be read: {e}"))?;
    let client_id = v
        .get("client_id")
        .and_then(|c| c.as_str())
        .ok_or("The service registered us but did not say who we are.")?
        .to_string();
    let client_secret = v
        .get("client_secret")
        .and_then(|c| c.as_str())
        .unwrap_or_default()
        .to_string();
    Ok(Registered { client_id, client_secret })
}

/// Wait on the loopback redirect and return the `code`, having checked `state`.
fn await_code(listener: TcpListener, want_state: &str) -> Result<String, String> {
    listener
        .set_nonblocking(false)
        .map_err(|e| e.to_string())?;
    let deadline = std::time::Instant::now() + WAIT;

    for stream in listener.incoming() {
        if std::time::Instant::now() > deadline {
            return Err("Sign-in timed out.".into());
        }
        let Ok(mut stream) = stream else { continue };
        // A PER-SOCKET READ TIMEOUT. Without it, any local process can connect
        // to the loopback port and send nothing, and `read_line` blocks on that
        // one socket forever — past the overall `deadline`, which is only
        // checked between connections. `NET` bounds a single request line.
        let _ = stream.set_read_timeout(Some(NET));
        let mut line = String::new();
        if BufReader::new(&stream).read_line(&mut line).is_err() {
            continue;
        }
        // "GET /callback?code=…&state=… HTTP/1.1"
        let target = line.split_whitespace().nth(1).unwrap_or("");
        let query = target.split_once('?').map(|(_, q)| q).unwrap_or("");
        let mut code = String::new();
        let mut state = String::new();
        for pair in query.split('&') {
            let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
            let v = percent_decode(v);
            match k {
                "code" => code = v,
                "state" => state = v,
                _ => {}
            }
        }

        let ok = !code.is_empty() && state == want_state;
        // THE PAGE THEY LAND ON. They are staring at a browser tab; leaving it
        // blank makes a completed sign-in look like a failure.
        let msg = if ok {
            "<h2>Connected.</h2><p>You can close this tab and go back to helloim.ai.</p>"
        } else {
            "<h2>That did not work.</h2><p>Go back to helloim.ai and try again.</p>"
        };
        let _ = write!(
            stream,
            "HTTP/1.1 200 OK\r\ncontent-type: text/html; charset=utf-8\r\n\
             content-length: {}\r\nconnection: close\r\n\r\n{}",
            msg.len(),
            msg
        );
        let _ = stream.flush();

        if !ok {
            // A mismatched state is the attack this check exists for; say so
            // plainly rather than retrying and hoping.
            return Err(if code.is_empty() {
                "The service did not send a sign-in code back.".into()
            } else {
                "The sign-in came back from somewhere unexpected and was refused.".to_string()
            });
        }
        return Ok(code);
    }
    Err("Sign-in was closed before it finished.".into())
}

fn percent_decode(s: &str) -> String {
    let b = s.replace('+', " ");
    let bytes = b.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(v) = u8::from_str_radix(&b[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The whole flow. Blocking, and called from an async command so the window
/// keeps painting while somebody is off in their browser.
pub fn sign_in(url: &str, open: impl Fn(&str)) -> Result<Signed, String> {
    let meta = discover(url)?;

    // Port 0: the OS picks a free one. A fixed port is one more thing that can
    // already be taken, on a machine we know nothing about.
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let redirect = format!("http://127.0.0.1:{port}/callback");

    let reg = register(&meta, &redirect)?;
    let client_id = &reg.client_id;

    let verifier = nonce();
    let challenge = b64url(&Sha256::digest(verifier.as_bytes()));
    let state = nonce();

    /* `?` OR `&`, DECIDED BY WHAT THE ENDPOINT ALREADY HAS. RFC 6749 §3.1: the
       endpoint URI "MAY include an 'application/x-www-form-urlencoded'
       formatted query component ... which MUST be retained when adding
       additional query parameters." Hard-coding `?` produced a URL with two of
       them against any server that publishes one — malformed, and it would
       have failed with a message about neither end. Endpoints with a query are
       uncommon but legal, and this is a correctness fix as much as a tidy one. */
    let join = if meta.authorization_endpoint.contains('?') { '&' } else { '?' };
    let auth = format!(
        "{}{join}response_type=code&client_id={}&redirect_uri={}&state={}\
         &code_challenge={}&code_challenge_method=S256",
        meta.authorization_endpoint,
        urlencode(client_id),
        urlencode(&redirect),
        urlencode(&state),
        urlencode(&challenge),
    );
    open(&auth);

    let code = await_code(listener, &state)?;

    let mut form = format!(
        "grant_type=authorization_code&code={}&redirect_uri={}&client_id={}&code_verifier={}",
        urlencode(&code),
        urlencode(&redirect),
        urlencode(client_id),
        urlencode(&verifier),
    );
    /* CONFIDENTIAL CLIENT. When registration issued a secret, the token
       endpoint requires it (`client_secret_post`). PKCE still travels — the
       verifier is not dropped just because a secret exists — so the code is
       protected both ways. Over TLS, in the body, never in the URL. */
    if !reg.client_secret.is_empty() {
        form.push_str(&format!("&client_secret={}", urlencode(&reg.client_secret)));
    }
    // SSRF: the token endpoint is a network-supplied URL we POST the auth code
    // and client secret to, so `guarded` vets it and every redirect hop before
    // any bytes leave — a 307/308 to an internal address is the worst case here,
    // and it is refused rather than followed. The authorization endpoint is not
    // checked this way: it is handed to the user's OWN browser, not fetched by
    // this process.
    let res = match guarded(
        meta.token_endpoint.clone(),
        true,
        "application/x-www-form-urlencoded",
        "",
        &form,
    )? {
        Ok(r) => r.into_string().map_err(|e| e.to_string())?,
        Err(e) => return Err(format!("The service refused to finish the sign-in: {e}")),
    };

    let v: serde_json::Value =
        serde_json::from_str(&res).map_err(|e| format!("Its answer could not be read: {e}"))?;
    let access = v
        .get("access_token")
        .and_then(|t| t.as_str())
        .ok_or_else(|| "The service did not return a token.".to_string())?;
    // Services put the identity in different places and most put it nowhere.
    // These are the ones seen in the wild; anything else leaves it blank.
    let account = ["account", "email", "user_email", "username", "user"]
        .iter()
        .find_map(|k| v.get(*k).and_then(|x| x.as_str()))
        .map(str::to_string)
        .or_else(|| {
            v.get("authed_user")
                .and_then(|u| u.get("id"))
                .and_then(|x| x.as_str())
                .map(str::to_string)
        })
        .or_else(|| {
            // Slack returns the workspace under `team`, which is the thing a
            // person recognises even though it is not their username.
            v.get("team")
                .and_then(|t| t.get("name"))
                .and_then(|x| x.as_str())
                .map(str::to_string)
        })
        .unwrap_or_default();

    Ok(Signed {
        access_token: access.to_string(),
        refresh_token: v
            .get("refresh_token")
            .and_then(|t| t.as_str())
            .unwrap_or_default()
            .to_string(),
        account,
        client_id: reg.client_id,
        client_secret: reg.client_secret,
        token_endpoint: meta.token_endpoint,
    })
}

fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The challenge has to be the base64url of the SHA-256 of the verifier,
    /// unpadded. A padded or standard-alphabet value is rejected by the server
    /// with an error that says nothing about which of the two ends is wrong.
    #[test]
    fn pkce_challenge_matches_the_spec_example() {
        // RFC 7636 appendix B, the worked example every implementation is
        // checked against.
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        let challenge = b64url(&Sha256::digest(verifier.as_bytes()));
        assert_eq!(challenge, "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
    }

    #[test]
    fn b64url_has_no_padding_and_no_unsafe_characters() {
        for n in 1..40usize {
            let s = b64url(&vec![0xABu8; n]);
            assert!(!s.contains('='), "{n}: padded");
            assert!(!s.contains('+') && !s.contains('/'), "{n}: wrong alphabet");
        }
    }

    #[test]
    fn nonces_are_not_repeated() {
        let a = nonce();
        let b = nonce();
        assert_ne!(a, b);
        assert!(a.len() >= 43, "too short to be worth having");
    }

    /// The redirect carries values that arrive percent-encoded.
    #[test]
    fn it_decodes_what_a_browser_sends() {
        assert_eq!(percent_decode("a%2Fb%20c"), "a/b c");
        assert_eq!(percent_decode("plain"), "plain");
        assert_eq!(percent_decode("a+b"), "a b");
    }

    #[test]
    fn urlencode_escapes_everything_that_would_break_a_query() {
        assert_eq!(urlencode("a/b c&d=e"), "a%2Fb%20c%26d%3De");
        assert_eq!(urlencode("safe-._~"), "safe-._~");
    }

    /// The endpoint is the one value in this file that is NOT urlencoded — it
    /// cannot be, it is the address itself — so it is the one that has to be
    /// checked instead.
    #[test]
    fn a_hostile_authorization_endpoint_is_refused() {
        for bad in [
            // Characters RFC 3986 does not permit unescaped in a URI. They are
            // also every remaining shell metacharacter, which is the point.
            "https://ok.example/a|whoami",
            "https://ok.example/a^b",
            "https://ok.example/a b",
            "https://ok.example/a\"b",
            "https://ok.example/a`b`",
            "https://ok.example/a<b>",
            "https://ok.example/a\\b",
            "https://ok.example/a\nb",
            "https://ok.example/a\tb",
            "https://ok.example/a\0b",
            // RFC 6749 §3.1: no fragment.
            "https://ok.example/a#frag",
            // RFC 6749 §3.1: TLS is required. This is also what stops a
            // discovery document pointing us at a local handler instead of a
            // web address.
            "http://ok.example/a",
            "file:///etc/passwd",
            "ms-settings:privacy-microphone",
            "javascript:alert(1)",
            // Nothing to go to.
            "",
            "   ",
            "https://",
            "https:///path",
        ] {
            assert!(
                checked_endpoint(bad, "sign-in address").is_err(),
                "accepted a hostile endpoint: {bad:?}"
            );
        }
    }

    /// **`&` IS ACCEPTED HERE ON PURPOSE, AND THIS TEST EXISTS SO NOBODY
    /// "FIXES" THAT.** Cassandra's example payload was
    /// `https://ok.example/a&<command>&`, and it is a syntactically valid https
    /// URL: RFC 3986 lists `&` as a sub-delimiter, legal in a path and
    /// universal in a query — every OAuth URL we build contains four of them.
    ///
    /// So this validator cannot be what stops it, and pretending otherwise
    /// would put the defence in the wrong place while breaking real services.
    /// **What stops it is that the URL never reaches a command-line parser:**
    /// `connectors::open_in_browser` passes it to PowerShell in an environment
    /// variable, and its fallback is `explorer.exe`, which decodes arguments
    /// the standard way. The `cmd /C start` that made `&` dangerous is gone.
    ///
    /// If anyone ever puts a shell back in that path, this comment is the
    /// record that these strings were known to arrive here intact.
    #[test]
    fn ampersands_are_legal_and_are_defended_elsewhere() {
        for legal in [
            "https://ok.example/a?x=1&y=2",
            "https://ok.example/a&calc.exe&",
            "https://example.com/authorize?tenant=acme&mode=strict",
        ] {
            assert!(
                checked_endpoint(legal, "sign-in address").is_ok(),
                "refused a legal URL: {legal}"
            );
        }
    }

    /// Ordinary endpoints must pass untouched, whitespace-trimmed. A validator
    /// that rejects real services is one somebody deletes.
    #[test]
    fn real_endpoints_survive_validation() {
        for good in [
            "https://api.notion.com/v1/oauth/authorize",
            "https://linear.app/oauth/authorize",
            "https://slack.com/oauth/v2/authorize",
            "https://www.canva.com/api/oauth/authorize",
            "https://example.com/authorize?tenant=acme",
        ] {
            assert_eq!(
                checked_endpoint(good, "sign-in address").unwrap(),
                good,
                "rejected a real endpoint: {good}"
            );
        }
        assert_eq!(
            checked_endpoint("  https://ok.example/a  ", "x").unwrap(),
            "https://ok.example/a"
        );
    }

    /// RFC 6749 §3.1: an existing query "MUST be retained when adding
    /// additional query parameters". Hard-coding `?` produced two of them.
    #[test]
    fn a_query_already_on_the_endpoint_is_retained() {
        let with = "https://example.com/authorize?tenant=acme";
        let without = "https://example.com/authorize";
        let join = |e: &str| if e.contains('?') { '&' } else { '?' };
        assert_eq!(join(with), '&');
        assert_eq!(join(without), '?');
        assert_eq!(
            format!("{with}{}response_type=code", join(with)),
            "https://example.com/authorize?tenant=acme&response_type=code"
        );
        assert_eq!(
            format!("{without}{}response_type=code", join(without)),
            "https://example.com/authorize?response_type=code"
        );
    }

    // ---- The modern (protected-resource) discovery chain -----------------
    //
    // Every value the chain follows comes off the network from the server being
    // connected to. The tests below prove the WRONG-CALL path: a chain pointed
    // somewhere other than the connector's own host is REFUSED, before any
    // browser is ever opened. The happy path is proven too, so the guard cannot
    // be satisfied by simply refusing everything.

    #[test]
    fn host_is_extracted_and_lowercased() {
        assert_eq!(host_of("https://MCP.Dropbox.com/mcp").as_deref(), Some("mcp.dropbox.com"));
        assert_eq!(host_of("https://api.box.com/").as_deref(), Some("api.box.com"));
        assert_eq!(host_of("https://host").as_deref(), Some("host"));
        assert_eq!(host_of("not a url"), None);
        assert_eq!(host_of("https://"), None);
    }

    #[test]
    fn same_host_holds_only_within_one_host() {
        assert!(same_host("https://mcp.dropbox.com/mcp", "https://mcp.dropbox.com/.well-known/x", "m").is_ok());
        assert!(same_host("https://mcp.dropbox.com/mcp", "https://evil.example/.well-known/x", "m").is_err());
        // Case-insensitive host match; a different subdomain is a different host.
        assert!(same_host("https://MCP.dropbox.com/a", "https://mcp.dropbox.com/b", "m").is_ok());
        assert!(same_host("https://mcp.dropbox.com/a", "https://auth.dropbox.com/b", "m").is_err());
    }

    #[test]
    fn resource_metadata_is_read_out_of_the_challenge() {
        // Real shapes seen in the wild: Dropbox (bare), Monday (realm first),
        // GitHub (error params first).
        assert_eq!(
            resource_metadata_from_header(
                r#"Bearer resource_metadata="https://mcp.dropbox.com/.well-known/oauth-protected-resource/mcp", error="invalid_token""#
            ).as_deref(),
            Some("https://mcp.dropbox.com/.well-known/oauth-protected-resource/mcp")
        );
        assert_eq!(
            resource_metadata_from_header(
                r#"Bearer realm="OAuth", resource_metadata="https://mcp.monday.com/.well-known/oauth-protected-resource/sse", error="invalid_token""#
            ).as_deref(),
            Some("https://mcp.monday.com/.well-known/oauth-protected-resource/sse")
        );
        assert_eq!(
            resource_metadata_from_header(
                r#"Bearer error="invalid_request", resource_metadata=https://api.githubcopilot.com/.well-known/oauth-protected-resource/mcp/"#
            ).as_deref(),
            Some("https://api.githubcopilot.com/.well-known/oauth-protected-resource/mcp/")
        );
        // No pointer at all is a real answer meaning "cannot drive this".
        assert_eq!(resource_metadata_from_header("Bearer realm=\"x\""), None);
    }

    #[test]
    fn metadata_pointer_must_be_https_and_same_host() {
        let conn = "https://mcp.dropbox.com/mcp";
        // Happy path: same host, https.
        assert_eq!(
            accept_resource_metadata(
                conn,
                r#"Bearer resource_metadata="https://mcp.dropbox.com/.well-known/oauth-protected-resource/mcp""#
            ).unwrap(),
            "https://mcp.dropbox.com/.well-known/oauth-protected-resource/mcp"
        );
        // WRONG CALL: pointed at another site — refused before any fetch.
        assert!(accept_resource_metadata(
            conn,
            r#"Bearer resource_metadata="https://evil.example/.well-known/oauth-protected-resource/mcp""#
        ).is_err());
        // WRONG CALL: not https (checked_endpoint) — refused. This is also what
        // stops a pointer at a local service, e.g. http://127.0.0.1.
        assert!(accept_resource_metadata(
            conn,
            r#"Bearer resource_metadata="http://mcp.dropbox.com/.well-known/x""#
        ).is_err());
        assert!(accept_resource_metadata(
            conn,
            r#"Bearer resource_metadata="file:///etc/passwd""#
        ).is_err());
        // WRONG CALL: no pointer.
        assert!(accept_resource_metadata(conn, "Bearer realm=\"x\"").is_err());
    }

    #[test]
    fn authorization_server_is_read_but_bound_to_the_resource() {
        let conn = "https://mcp.dropbox.com/mcp";
        // Happy path: resource is our host, AS is legitimately a different host.
        let good = r#"{"resource":"https://mcp.dropbox.com/mcp","authorization_servers":["https://www.dropbox.com"]}"#;
        assert_eq!(auth_server_from_prm(conn, good).unwrap(), "https://www.dropbox.com");
        // GitHub's real shape: AS on github.com, resource on the connector host.
        let gh = r#"{"resource":"https://api.githubcopilot.com/mcp/","authorization_servers":["https://github.com/login/oauth"]}"#;
        assert_eq!(
            auth_server_from_prm("https://api.githubcopilot.com/mcp/", gh).unwrap(),
            "https://github.com/login/oauth"
        );
        // WRONG CALL: a document claiming to describe some OTHER resource — its
        // authorization server is not one we will send the user's browser to.
        let swapped = r#"{"resource":"https://evil.example/mcp","authorization_servers":["https://evil.example/oauth"]}"#;
        assert!(auth_server_from_prm(conn, swapped).is_err());
        // WRONG CALL: right resource, but the AS is not https.
        let bad_as = r#"{"resource":"https://mcp.dropbox.com/mcp","authorization_servers":["http://127.0.0.1/oauth"]}"#;
        assert!(auth_server_from_prm(conn, bad_as).is_err());
        // WRONG CALL: no authorization server named.
        let none = r#"{"resource":"https://mcp.dropbox.com/mcp"}"#;
        assert!(auth_server_from_prm(conn, none).is_err());
    }

    #[test]
    fn as_metadata_is_tried_path_aware_first() {
        // RFC 8414 §3.1: the well-known segment goes between host and path.
        let c = as_metadata_candidates("https://auth.monday.com/mcp");
        assert_eq!(c[0], "https://auth.monday.com/.well-known/oauth-authorization-server/mcp");
        assert!(c.contains(&"https://auth.monday.com/.well-known/oauth-authorization-server".to_string()));
        // No path: only the host-root forms.
        let d = as_metadata_candidates("https://www.dropbox.com");
        assert_eq!(d[0], "https://www.dropbox.com/.well-known/oauth-authorization-server");
    }

    // ---- SSRF: internal fetch targets are refused ------------------------
    //
    // These are the wrong-call cases for the denylist. They use literal IPs and
    // localhost/.local names so no DNS is touched — the guard's short-circuits
    // decide them, which keeps the test hermetic.

    #[test]
    fn internal_addresses_are_refused_as_fetch_targets() {
        for bad in [
            // The two Cassandra named explicitly: https loopback (which passes
            // the https-only endpoint check) and the cloud-metadata address.
            "https://127.0.0.1/oauth",
            "https://169.254.169.254",
            "https://169.254.169.254/latest/meta-data/",
            // The rest of the ranges.
            "https://10.99.99.5/oauth",
            "https://172.16.9.9/oauth",
            "https://192.168.1.1/oauth",
            "https://0.0.0.0/oauth",
            "https://[::1]/oauth",
            "https://[fe80::1]/oauth",
            "https://[fc00::1]/oauth",
            "https://[::ffff:127.0.0.1]/oauth", // IPv4-mapped loopback
            "https://localhost/oauth",
            "https://service.localhost/oauth",
            "https://printer.local/oauth",
        ] {
            assert!(
                deny_internal(bad, "authorization server address").is_err(),
                "allowed an internal fetch target: {bad}"
            );
        }
        // A public literal IP is allowed — proves the guard is not refuse-all.
        assert!(deny_internal("https://8.8.8.8/oauth", "x").is_ok());
    }

    #[test]
    fn denied_ip_ranges_are_exactly_the_internal_ones() {
        use std::net::{Ipv4Addr, Ipv6Addr};
        for ip in [
            IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
            IpAddr::V4(Ipv4Addr::new(10, 1, 2, 3)),
            IpAddr::V4(Ipv4Addr::new(172, 16, 0, 1)),
            IpAddr::V4(Ipv4Addr::new(192, 168, 0, 1)),
            IpAddr::V4(Ipv4Addr::new(169, 254, 169, 254)),
            IpAddr::V4(Ipv4Addr::UNSPECIFIED),
            IpAddr::V6(Ipv6Addr::LOCALHOST),
            IpAddr::V6(Ipv6Addr::UNSPECIFIED),
        ] {
            assert!(is_denied_ip(ip), "should deny {ip}");
        }
        for ip in [
            IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8)),
            IpAddr::V4(Ipv4Addr::new(140, 82, 112, 3)), // github.com
        ] {
            assert!(!is_denied_ip(ip), "should allow {ip}");
        }
    }

    #[test]
    fn nat64_and_ipv4_compatible_embedded_addresses_are_denied() {
        for s in [
            "64:ff9b::7f00:1",    // NAT64 -> 127.0.0.1
            "64:ff9b::a9fe:a9fe", // NAT64 -> 169.254.169.254 (cloud metadata)
            "::7f00:1",           // deprecated IPv4-compatible ::127.0.0.1
            "::1",                // loopback must NOT be re-read as embedded 0.0.0.1
        ] {
            let ip: IpAddr = s.parse().unwrap();
            assert!(is_denied_ip(ip), "should deny {s}");
        }
        // A public v4 inside NAT64 stays allowed — not a blanket deny on the
        // prefix. 64:ff9b::808:808 embeds 8.8.8.8.
        let ok: IpAddr = "64:ff9b::808:808".parse().unwrap();
        assert!(!is_denied_ip(ok), "should allow public NAT64 8.8.8.8");
    }

    // ---- SSRF via redirect: every hop is re-validated ---------------------
    //
    // The denylist on the initial string is worthless if the client silently
    // follows a `Location` to an internal address, so these prove the per-hop
    // guard: a redirect target that is internal is REFUSED, a public one is
    // allowed, and an https->http downgrade is refused.

    #[test]
    fn next_hop_refuses_redirects_to_internal_targets() {
        let base = "https://mcp.evil.example/mcp";
        for bad in [
            "https://169.254.169.254/latest/meta-data/", // cloud metadata
            "https://127.0.0.1/oauth",
            "https://10.99.99.5/x",
            "https://[::1]/x",
            "http://mcp.evil.example/x", // https->http downgrade
        ] {
            assert!(next_hop(base, Some(bad)).is_err(), "followed a bad redirect: {bad}");
        }
        assert!(next_hop(base, None).is_err());
        // Public redirects ARE followed, so the guard is not refuse-all. Literal
        // public IPs keep the assertion off DNS: root-relative resolves against
        // the base host, absolute to another public host.
        assert_eq!(
            next_hop("https://8.8.8.8/mcp", Some("/oauth/authorize")).unwrap(),
            "https://8.8.8.8/oauth/authorize"
        );
        assert_eq!(
            next_hop("https://8.8.8.8/mcp", Some("https://8.8.4.4/x")).unwrap(),
            "https://8.8.4.4/x"
        );
    }

    #[test]
    fn resolve_location_handles_the_common_forms() {
        let b = "https://h.example/a/b?q=1";
        assert_eq!(resolve_location(b, "https://other/x").as_deref(), Some("https://other/x"));
        assert_eq!(resolve_location(b, "//cdn/x").as_deref(), Some("https://cdn/x"));
        assert_eq!(resolve_location(b, "/x").as_deref(), Some("https://h.example/x"));
        assert_eq!(resolve_location(b, "c").as_deref(), Some("https://h.example/a/c"));
        assert_eq!(resolve_location(b, ""), None);
    }

    /// Stand up a real local HTTP redirector and prove the redirect to a denied
    /// IP is refused, not followed. Covers a 302 (GET) and a 307 (which would
    /// re-send a POST body). `guarded` itself requires https at hop 0, so it
    /// cannot be pointed at a plain-http test server; this drives the same
    /// redirect decision (`next_hop`) against a genuine `Location` off the wire,
    /// which is the part that closes the hole. Fails against the pre-fix code,
    /// which had no per-hop revalidation and let the client auto-follow.
    #[test]
    fn a_local_redirector_to_a_denied_ip_is_refused() {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        use std::thread;

        for (status, target) in [
            ("302 Found", "https://169.254.169.254/latest/meta-data/"),
            ("307 Temporary Redirect", "https://127.0.0.1/token"),
        ] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let port = listener.local_addr().unwrap().port();
            let target_owned = target.to_string();
            let handle = thread::spawn(move || {
                if let Ok((mut s, _)) = listener.accept() {
                    let _ = s.read(&mut [0u8; 1024]);
                    let resp = format!(
                        "HTTP/1.1 {status}\r\nLocation: {target_owned}\r\nContent-Length: 0\r\n\
                         Connection: close\r\n\r\n"
                    );
                    let _ = s.write_all(resp.as_bytes());
                    let _ = s.flush();
                }
            });

            let url = format!("http://127.0.0.1:{port}/");
            let agent = ureq::builder().redirects(0).build();
            let resp = agent.get(&url).timeout(NET).call().unwrap();
            assert!((300..400).contains(&resp.status()));
            assert!(
                next_hop(&url, resp.header("location")).is_err(),
                "guard followed a {status} redirect to {target}"
            );
            let _ = handle.join();
        }
    }
}
