//! Integration tests for the MCP memory server — REAL PROCESSES, not
//! in-process calls. The unit tests in `src/mcp.rs` prove the protocol logic;
//! these prove the two things only a real process can prove: that the shipped
//! binary actually serves MCP over stdio when spawned the way Claude Code
//! spawns it, and that TWO processes writing one store cannot corrupt the
//! mirror or the stamp — the cross-process-lock obligation slice 2's header
//! recorded and slice 3 was required to close first.
//!
//! Linux only in practice: no Windows box has ever run these.
//! Documented-not-tested there, same as the rest of the crate.

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use serde_json::{json, Value};
use sha2::{Digest, Sha256};

struct Server {
    child: Child,
    stdin: ChildStdin,
    out: BufReader<ChildStdout>,
    next_id: i64,
}

impl Server {
    /// Spawn the real binary exactly as Claude Code would.
    fn spawn(workdir: &str, scope: Option<&str>) -> Server {
        let mut args = vec!["mcp-memory".to_string(), "--workdir".into(), workdir.to_string()];
        if let Some(s) = scope {
            args.push("--scope".into());
            args.push(s.into());
        }
        let mut child = Command::new(env!("CARGO_BIN_EXE_remembrancer"))
            .args(&args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("could not spawn the mcp-memory subcommand");
        let stdin = child.stdin.take().unwrap();
        let out = BufReader::new(child.stdout.take().unwrap());
        Server { child, stdin, out, next_id: 0 }
    }

    fn send_raw(&mut self, msg: &Value) {
        writeln!(self.stdin, "{msg}").unwrap();
        self.stdin.flush().unwrap();
    }

    fn read_response(&mut self) -> Value {
        let mut line = String::new();
        self.out.read_line(&mut line).unwrap();
        assert!(!line.trim().is_empty(), "server closed stdout unexpectedly");
        serde_json::from_str(&line).expect("server wrote a non-JSON line to stdout")
    }

    /// One request, one response.
    fn call(&mut self, method: &str, params: Value) -> Value {
        self.next_id += 1;
        self.send_raw(&json!({
            "jsonrpc": "2.0", "id": self.next_id, "method": method, "params": params
        }));
        self.read_response()
    }

    fn handshake(&mut self) {
        let r = self.call("initialize", json!({
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": { "name": "test", "version": "0" }
        }));
        assert_eq!(r["result"]["protocolVersion"], "2025-06-18", "handshake failed: {r}");
        self.send_raw(&json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }));
    }

    fn save(&mut self, text: &str) -> Value {
        self.call("tools/call", json!({
            "name": "memory_save", "arguments": { "text": text }
        }))
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn tmp(n: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!("helloim-mcp-it-{}-{}", n, std::process::id()));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).unwrap();
    p
}

fn md_path(d: &Path) -> PathBuf {
    d.join(".helloim/memory/facts.md")
}

fn db_path(d: &Path) -> PathBuf {
    d.join(".helloim/memory/memory.db")
}

/// The shipped binary, spawned the way Claude Code spawns it, speaks the
/// protocol end to end: handshake, list, call, and the store-unavailable
/// wording — all through real pipes.
#[test]
fn a_real_process_speaks_mcp_end_to_end() {
    let d = tmp("proto");
    let wd = d.to_string_lossy().to_string();
    let mut s = Server::spawn(&wd, None);
    s.handshake();

    let r = s.call("tools/list", json!({}));
    let tools = r["result"]["tools"].as_array().expect("no tools array");
    let mut names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    names.sort();
    assert_eq!(names, vec!["memory_save", "memory_search"]);

    let saved = s.save("The pricing page ships on Friday");
    assert_eq!(saved["result"]["isError"], false, "save failed: {saved}");

    let hits = s.call("tools/call", json!({
        "name": "memory_search", "arguments": { "query": "when does the pricing page ship" }
    }));
    let text = hits["result"]["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("ships on Friday"), "roundtrip lost the fact: {text}");
    assert!(text.contains("from the assistant"), "provenance missing over the wire: {text}");

    // Store unavailable, through a real process, in words a model can act on.
    let mut broken = Server::spawn("/definitely/not/here", None);
    broken.handshake();
    let e = broken.call("tools/call", json!({
        "name": "memory_search", "arguments": { "query": "anything" }
    }));
    assert_eq!(e["result"]["isError"], true);
    let etext = e["result"]["content"][0]["text"].as_str().unwrap();
    assert!(
        etext.contains("does not mean the user has no memories"),
        "unavailable collapsed into something else: {etext}"
    );

    let _ = std::fs::remove_dir_all(&d);
}

/// THE OBLIGATION. Two real processes, one store, pipelined writes from both
/// at once. If the cross-process lock (BEGIN IMMEDIATE around mutate + mirror
/// + stamp) does not hold, this shows up as a lost fact, a mangled mirror, or
/// a stamp describing a version of the file that does not exist — the exact
/// state the next reconcile would misread as a hand edit and delete from.
#[test]
fn two_real_processes_cannot_corrupt_the_store() {
    let d = tmp("twoproc");
    let wd = d.to_string_lossy().to_string();

    let mut a = Server::spawn(&wd, None);
    let mut b = Server::spawn(&wd, None);
    a.handshake();
    b.handshake();

    // Pipeline: queue all ten requests into EACH process before reading any
    // response, so both children genuinely contend for the store instead of
    // politely alternating.
    for i in 0..10 {
        a.next_id += 1;
        let id = a.next_id;
        a.send_raw(&json!({
            "jsonrpc": "2.0", "id": id, "method": "tools/call",
            "params": { "name": "memory_save",
                        "arguments": { "text": format!("process A fact number {i}") } }
        }));
        b.next_id += 1;
        let id = b.next_id;
        b.send_raw(&json!({
            "jsonrpc": "2.0", "id": id, "method": "tools/call",
            "params": { "name": "memory_save",
                        "arguments": { "text": format!("process B fact number {i}") } }
        }));
    }
    for _ in 0..10 {
        let ra = a.read_response();
        assert_eq!(ra["result"]["isError"], false, "process A save failed: {ra}");
        let rb = b.read_response();
        assert_eq!(rb["result"]["isError"], false, "process B save failed: {rb}");
    }
    drop(a);
    drop(b);

    // The database holds all twenty.
    let conn = rusqlite::Connection::open(db_path(&d)).unwrap();
    let count: i64 = conn.query_row("SELECT count(*) FROM facts", [], |r| r.get(0)).unwrap();
    assert_eq!(count, 20, "facts were lost across two processes");

    // The mirror holds all twenty, unmangled.
    let md = std::fs::read_to_string(md_path(&d)).unwrap();
    assert_eq!(md.matches("process A fact").count(), 10, "mirror lost A's writes:\n{md}");
    assert_eq!(md.matches("process B fact").count(), 10, "mirror lost B's writes:\n{md}");

    // And the stamp describes EXACTLY the file on disk — the invariant whose
    // violation would make the next reconcile delete facts.
    let stamp: String = conn
        .query_row("SELECT value FROM meta WHERE key='md_sha256'", [], |r| r.get(0))
        .unwrap();
    let hash: String = Sha256::digest(md.as_bytes()).iter().map(|x| format!("{x:02x}")).collect();
    assert_eq!(stamp, hash, "stamp and file describe different versions");

    let _ = std::fs::remove_dir_all(&d);
}

/// Scope is fixed at spawn and survives the process boundary: a server
/// spawned for one specialist cannot surface another's rows, and the global
/// server cannot either. (NameOS passes no --scope today — this proves the
/// mechanism, not the product wiring.)
#[test]
fn a_spawned_scope_holds_across_a_real_process() {
    let d = tmp("scope");
    let wd = d.to_string_lossy().to_string();

    // Create the store through the real server, then plant a code-scoped row
    // directly — the app writes scoped facts; the MCP server never does.
    {
        let mut s = Server::spawn(&wd, None);
        s.handshake();
        let r = s.save("Mark is in Texas");
        assert_eq!(r["result"]["isError"], false);
    }
    let conn = rusqlite::Connection::open(db_path(&d)).unwrap();
    conn.execute(
        "INSERT INTO facts(kind, text, scope, source, at, verified)
         VALUES('fact', 'The staging database lives on port 5433', 'code', 'setup',
                strftime('%s','now'), 0)",
        [],
    )
    .unwrap();
    drop(conn);

    for spawn_scope in [Some("creative"), None] {
        let mut s = Server::spawn(&wd, spawn_scope);
        s.handshake();
        let r = s.call("tools/call", json!({
            "name": "memory_search",
            "arguments": { "query": "staging database port Texas", "scope": "code" }
        }));
        let text = r["result"]["content"][0]["text"].as_str().unwrap();
        assert!(
            !text.contains("5433"),
            "scope {spawn_scope:?} reached another specialist's memory: {text}"
        );
        assert!(text.contains("Texas"), "global context went missing: {text}");
    }

    let _ = std::fs::remove_dir_all(&d);
}
