//! The Windows-only spawn behind the opt-in `Bash` tool — decision B, items 3
//! and 4 of the room-approved plan. `tools::bash_tool` reaches exactly one
//! function here, `run`, and only when `#[cfg(windows)]`; nowhere else in
//! this crate calls anything in this file.
//!
//! ## What decision A already covers, and what this adds
//!
//! Decision A (`tools.rs`'s own header, and `Provider::allow_shell`) is the
//! GATE: `Bash` is never offered to the model at all unless the row has
//! opted in. This file is what runs ONCE THAT GATE IS OPEN — it does not
//! reconsider whether the shell should run, `bash_tool` has already settled
//! that before this is ever called. What is added here is a SECOND, narrower
//! boundary for the case the gate exists to make survivable: the folder was
//! reviewed and trusted, the shell is on, and the command asked for is
//! whatever the model wrote. Two things still have to be true even then —
//! Stop has to actually be able to end the whole thing, not just the shell
//! wrapping it, and a command should not be able to reach outside the
//! trusted folder by WRITING there, even though it can read anywhere the
//! signed-in user can.
//!
//! ## The mechanism, in the order it runs
//!
//! 1. **A write-restricted token, derived from this process's OWN token** —
//!    `CreateRestrictedToken` with the `WRITE_RESTRICTED` flag (`0x8`). A
//!    write-restricted token still passes ordinary access checks for READS
//!    and EXECUTES on the enabled SIDs alone; a WRITE additionally has to
//!    pass a second check against the token's list of "restricting SIDs" —
//!    see [Restricted Tokens, MS Learn][rt]: *"the system performs two
//!    access checks: one using the token's enabled SIDs, and another using
//!    the list of restricting SIDs. Access is granted only if both access
//!    checks allow the requested access rights."* Because this is a
//!    restricted version of the CALLER's OWN token, `CreateProcessAsUser`
//!    does not need `SE_ASSIGNPRIMARYTOKEN_NAME` (a privilege an ordinary
//!    desktop app does not hold) — [CreateProcessAsUserW's own remarks][cpau]
//!    say so explicitly, which is what makes this runnable from a normal
//!    Tauri app rather than a service.
//! 2. **The restricting SID is this process's own LOGON SID**
//!    (`S-1-5-5-X-Y`), read off the current token's `TokenGroups` by finding
//!    the entry carrying `SE_GROUP_LOGON_ID` — the pattern MS's own
//!    "Restricted Tokens" page names directly: grant access to the logon SID
//!    on specific resources rather than minting a fresh identity for it.
//!    **Rejected: allocating a brand-new synthetic SID per run**, the way
//!    the closest public prior art (OpenAI's Codex Windows sandbox, see the
//!    citation below) does. The logon SID is already unique to this signed-
//!    in session, needs no `AllocateAndInitializeSid` bookkeeping to
//!    generate or free, and is exactly the case Microsoft's own docs
//!    describe — a smaller, better-trodden piece of API surface for the same
//!    guarantee. What is given up: two restricted shells started in the SAME
//!    logon session share a restricting SID, which does not matter here,
//!    because the thing being separated is "this session's trusted folder"
//!    from "everywhere else on disk," not one shell from another.
//! 3. **The trusted folder, and ONLY the trusted folder, gets an explicit
//!    ALLOW write ACE for that logon SID**, inherited onto everything
//!    created under it afterward (`CONTAINER_INHERIT_ACE |
//!    OBJECT_INHERIT_ACE`). `GetNamedSecurityInfoW` reads the folder's
//!    existing DACL, `SetEntriesInAclW` merges in the one new entry,
//!    `SetNamedSecurityInfoW` writes it back — the existing DACL is never
//!    replaced wholesale, only extended, so nothing the person or Windows
//!    already granted on that folder is touched. Nowhere else on the disk
//!    carries this ACE, so the SECOND access check above fails everywhere
//!    else by default — a deny that costs no other folder's ACL a single
//!    edit, because it was never granted there in the first place.
//! 4. **`cmd.exe /C <command>` is spawned under that token with
//!    `CreateProcessAsUserW`, suspended (`CREATE_SUSPENDED`), inside a Job
//!    Object carrying `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`, then resumed.**
//!    Spawning suspended and assigning to the Job Object before a single
//!    instruction of `cmd.exe` runs closes the race a plain
//!    `std::process::Command` + `AssignProcessToJobObject` pair cannot: that
//!    path never has the child's own thread handle to resume (Rust's
//!    `std::process::Child` deliberately does not expose it), so anything the
//!    command spawns in the gap between "the process exists" and "we finished
//!    assigning it" could theoretically start outside the job. Calling the
//!    Win32 functions directly, this file DOES hold that handle, so there is
//!    no gap: nothing in the command's own tree runs before it is already a
//!    member.
//! 5. **Stop and the timeout both end with `TerminateJobObject`, not
//!    `child.kill()`.** That is the actual fix for the bug this file exists
//!    to close: `tools.rs`'s own Unix/`std::process` path is honest that
//!    killing the direct child does not reach a build tool's own children —
//!    `npm`, `node`, a compiler — and on Windows that is not a rare edge
//!    case, it is the common one. A Job Object with
//!    `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` set terminates every process still
//!    in it — the whole tree `cmd.exe` spawned, unless something in that
//!    tree deliberately broke away — in one call.
//!
//! [rt]: https://learn.microsoft.com/en-us/windows/win32/secauthz/restricted-tokens
//! [cpau]: https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-createprocessasuserw
//!
//! Read alongside the OpenAI Codex Windows sandbox write-up (`openai.com/
//! index/building-codex-windows-sandbox`) and the Microsoft primary docs for
//! `CreateRestrictedToken`, `CreateProcessAsUserW`, `SetEntriesInAclW` and
//! the Job Object APIs — Codex's own public description confirms the same
//! shape is shipped, real, sandbox software (`CreateRestrictedToken` +
//! `WRITE_RESTRICTED` + a restricting SID + a write-allow ACE stamped on the
//! writable folder + `CreateProcessAsUserW`); Microsoft's pages are what this
//! file's actual flag values, struct layouts and function signatures were
//! checked against, field by field, in the vendored `windows-sys` crate
//! source rather than recalled.
//!
//! ## What this does NOT claim, said as plainly as `tools.rs`'s own header
//!
//! - **READS ARE NOT CONFINED.** A write-restricted token is exactly that —
//!   restricted for writes. The enabled SIDs alone govern reads, which is
//!   the same access the signed-in user already has everywhere. A command
//!   run through this path can read any file that user can read; it can only
//!   WRITE inside the trusted folder.
//! - **NETWORK IS NOT CONFINED AT ALL.** Nothing here touches Windows
//!   Filtering Platform, a firewall rule, or any per-process network
//!   boundary. A restricted shell can still make an outbound connection.
//!   Combined with the point above: a command that reads a secret file
//!   elsewhere on disk and sends it out over the network is NOT stopped by
//!   this mechanism — see the sweep in the top-level report for where that
//!   residual risk was checked and what else was and was not found to carry
//!   it.
//! - **ONLY THE DIRECT PROCESS TREE, THE SAME SCOPE `tools.rs`'S OWN DOC
//!   ALREADY STATES FOR THE TIMEOUT PATH.** A grandchild that deliberately
//!   breaks away from the job (`CREATE_BREAKAWAY_FROM_JOB`, if the job
//!   allows it — this one does not set `JOB_OBJECT_LIMIT_BREAKAWAY_OK`, so an
//!   ordinary child cannot opt out) is the only case this does not reach; an
//!   ordinary `npm`/`node`/compiler tree cannot opt out on its own.
//! - **NOT RUN. NOT EVEN ONCE, ON A REAL WINDOWS BOX.** Everything above is
//!   checked against the real Windows SDK headers with `cargo xwin check
//!   --target x86_64-pc-windows-msvc` from this Linux machine — every
//!   function name, every struct field, every constant value in this file is
//!   what the actual `windows-sys` crate (generated from Microsoft's own SDK
//!   metadata) says they are, not what memory says they are. That proves the
//!   code TYPE-CHECKS against the real API surface. It proves nothing about
//!   what happens when `CreateRestrictedToken` is actually called, whether
//!   the logon SID is found the way the docs describe on a real token,
//!   whether the ACE actually lands and actually blocks a write, whether the
//!   job actually kills the whole tree, or whether any of this behaves the
//!   same across a home edition, a domain-joined machine, or a machine with
//!   third-party AV hooking process creation. **Beck's own pass on a real
//!   Windows box is required before this ships**, and it needs to prove,
//!   specifically:
//!   1. A restricted-shell `Bash` call can create/edit/delete a file INSIDE
//!      the trusted folder (the ACE actually grants what it is meant to).
//!   2. The SAME call, attempting to write OUTSIDE the trusted folder (a
//!      neighbouring folder, `%TEMP%`, the user's own Documents root),
//!      is REFUSED by Windows itself — not by this code, by the OS access
//!      check — and that refusal reaches the model as an honest tool error
//!      rather than a hang or a panic.
//!   3. The same call CAN still read a file outside the trusted folder (the
//!      "reads are not confined" claim above, proven rather than asserted).
//!   4. A command that spawns real children (`npm install`, a multi-process
//!      build) is ENTIRELY gone from Task Manager / Process Explorer after
//!      Stop is pressed mid-run, not just its `cmd.exe` parent.
//!   5. The same, for the timeout path, not only Stop.
//!   6. Output (stdout and stderr, interleaved and capped the same way the
//!      Unix path already is) still reaches the model correctly.
//!   7. A normal, quick, well-behaved command (`dir`, `git status`) still
//!      runs and returns promptly — this is a security boundary, not a
//!      reason for every ordinary call to get slower or flakier.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, SetHandleInformation, GENERIC_READ, HANDLE, HANDLE_FLAG_INHERIT,
    INVALID_HANDLE_VALUE, WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use windows_sys::Win32::Security::Authorization::{
    BuildTrusteeWithSidW, SetEntriesInAclW, SetNamedSecurityInfoW, EXPLICIT_ACCESS_W, SE_FILE_OBJECT,
};
use windows_sys::Win32::Security::{
    CopySid, CreateRestrictedToken, GetLengthSid, GetTokenInformation, SID_AND_ATTRIBUTES,
    TOKEN_ASSIGN_PRIMARY, TOKEN_DUPLICATE, TOKEN_GROUPS, TOKEN_IMPERSONATE, TOKEN_QUERY,
    WRITE_RESTRICTED,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, DELETE, FILE_ATTRIBUTE_NORMAL, FILE_GENERIC_EXECUTE, FILE_GENERIC_READ,
    FILE_GENERIC_WRITE, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
    SetInformationJobObject, TerminateJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};
use windows_sys::Win32::System::Pipes::CreatePipe;
use windows_sys::Win32::System::Threading::{
    CreateProcessAsUserW, GetCurrentProcess, GetExitCodeProcess, OpenProcessToken, ResumeThread,
    WaitForSingleObject, CREATE_NO_WINDOW, CREATE_SUSPENDED, PROCESS_INFORMATION, STARTF_USESTDHANDLES,
    STARTUPINFOW,
};

use super::tools::{cap, ToolResult, OUTPUT_CAP_CHARS};

/// `SE_GROUP_LOGON_ID` — the token-group attribute marking "this SID
/// identifies the logon session," not exported by `windows-sys` (it carries
/// no function signature to generate a binding from, unlike everything else
/// imported above). Value confirmed against Microsoft's own "SID Attributes
/// in an Access Token" documentation rather than recalled: `0xC0000000`.
const SE_GROUP_LOGON_ID: u32 = 0xC0000000;

/// How often the wait loop below checks Stop and the deadline — same cadence
/// `tools.rs`'s own Unix poll loop uses, for the same reason: frequent enough
/// that Stop feels immediate, cheap enough that it costs nothing while a
/// command is genuinely still working.
const POLL_MS: u32 = 40;

/// A `HANDLE` this file owns and must close exactly once. `CloseHandle` on
/// drop, always — the alternative, closing by hand at every early-return
/// `?`, is exactly the shape that leaks a handle the one time a new error
/// path gets added later and somebody forgets the cleanup line to go with
/// it. `Option` rather than a null-check on drop: a handle already moved out
/// (`into_raw`, for the two cases where ownership genuinely transfers to a
/// child process across `CreateProcessAsUserW`) must not be closed twice.
struct Owned(Option<HANDLE>);

impl Owned {
    /// `h` must be a value this code actually owns, never a borrowed or
    /// pseudo-handle (`GetCurrentProcess()`'s return value, for instance,
    /// which this file never wraps here for exactly that reason).
    fn new(h: HANDLE) -> Option<Self> {
        if h.is_null() || h == INVALID_HANDLE_VALUE {
            None
        } else {
            Some(Owned(Some(h)))
        }
    }

    fn raw(&self) -> HANDLE {
        self.0.expect("Owned::raw called after into_raw")
    }
}

impl Drop for Owned {
    fn drop(&mut self) {
        if let Some(h) = self.0.take() {
            unsafe {
                CloseHandle(h);
            }
        }
    }
}

fn last_error(doing: &str) -> String {
    let code = unsafe { GetLastError() };
    format!("Windows refused while {doing} (error {code}).")
}

/// UTF-16, null-terminated — the shape every `…W` Win32 call in this file
/// wants for a string argument.
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// The current process's own LOGON SID, copied into a buffer this function
/// owns — see this file's own header for why the logon SID rather than a
/// freshly minted one. Returns the raw bytes of the SID (a `PSID` is a
/// pointer into memory whose layout is opaque to callers; `Vec<u8>` is
/// enough to keep it alive and hand a pointer back into it later).
fn own_logon_sid() -> Result<Vec<u8>, String> {
    let mut proc_token: HANDLE = std::ptr::null_mut();
    let ok = unsafe {
        OpenProcessToken(
            GetCurrentProcess(),
            TOKEN_QUERY,
            &mut proc_token,
        )
    };
    if ok == 0 {
        return Err(last_error("opening this process's own security token"));
    }
    let proc_token = Owned::new(proc_token).ok_or_else(|| {
        "opening this process's own security token returned a handle that was not real".to_string()
    })?;

    // First call: ask how big the buffer needs to be. This is documented
    // Win32 usage for `GetTokenInformation` -- it fails on purpose with the
    // real length rather than accepting a guessed one.
    let mut needed: u32 = 0;
    unsafe {
        GetTokenInformation(proc_token.raw(), windows_sys::Win32::Security::TokenGroups, std::ptr::null_mut(), 0, &mut needed);
    }
    if needed == 0 {
        return Err(last_error("sizing this process's own group list"));
    }
    let mut buf = vec![0u8; needed as usize];
    let ok = unsafe {
        GetTokenInformation(
            proc_token.raw(),
            windows_sys::Win32::Security::TokenGroups,
            buf.as_mut_ptr() as *mut core::ffi::c_void,
            needed,
            &mut needed,
        )
    };
    if ok == 0 {
        return Err(last_error("reading this process's own group list"));
    }

    // `TOKEN_GROUPS.Groups` is declared as a one-element array; the real
    // buffer `GetTokenInformation` filled is `GroupCount` entries long,
    // stored contiguously starting at that same field -- the classic Win32
    // "variable-length struct" shape. Reading it as a slice would be
    // undefined (the declared length lies); walking it by pointer arithmetic
    // off the field's own address, the way every C sample for this call
    // does, is what is actually being asked of the buffer's layout.
    let groups = buf.as_ptr() as *const TOKEN_GROUPS;
    let count = unsafe { (*groups).GroupCount } as usize;
    let first = unsafe { std::ptr::addr_of!((*groups).Groups) } as *const SID_AND_ATTRIBUTES;

    for i in 0..count {
        let entry = unsafe { &*first.add(i) };
        if entry.Attributes & SE_GROUP_LOGON_ID == SE_GROUP_LOGON_ID {
            let len = unsafe { GetLengthSid(entry.Sid) };
            if len == 0 {
                return Err(last_error("reading the length of this session's logon SID"));
            }
            let mut sid_buf = vec![0u8; len as usize];
            let ok =
                unsafe { CopySid(len, sid_buf.as_mut_ptr() as *mut core::ffi::c_void, entry.Sid) };
            if ok == 0 {
                return Err(last_error("copying this session's logon SID"));
            }
            return Ok(sid_buf);
        }
    }
    Err("this process's security token carries no logon SID -- decision B's write-\
         confined shell has nothing to restrict access to, so it refuses rather than \
         running unconfined."
        .to_string())
}

/// A write-restricted token derived from this process's own token, whose
/// only restricting SID is `logon_sid` -- see this file's own header for
/// what `WRITE_RESTRICTED` actually changes about how writes are checked.
fn restricted_token(logon_sid: &[u8]) -> Result<Owned, String> {
    let mut proc_token: HANDLE = std::ptr::null_mut();
    let ok = unsafe {
        OpenProcessToken(
            GetCurrentProcess(),
            TOKEN_DUPLICATE | TOKEN_QUERY | TOKEN_ASSIGN_PRIMARY | TOKEN_IMPERSONATE,
            &mut proc_token,
        )
    };
    if ok == 0 {
        return Err(last_error("opening this process's own token to restrict a copy of it"));
    }
    let proc_token = Owned::new(proc_token).ok_or_else(|| {
        "opening this process's own token returned a handle that was not real".to_string()
    })?;

    let sid_and_attrs = SID_AND_ATTRIBUTES {
        // `logon_sid` outlives this call (it is borrowed for the duration of
        // `CreateRestrictedToken`, which reads it and does not retain the
        // pointer afterward), and the cast drops `const`-ness only because
        // the windows-sys binding declares `Sid: PSID` (mutable) even where
        // the API in question only reads it -- the same shape `SID_AND_
        // ATTRIBUTES` always has in this API family.
        Sid: logon_sid.as_ptr() as *mut core::ffi::c_void,
        // "The Attributes member... must be zero" for a restricting SID --
        // CreateRestrictedToken's own documented contract, not a default
        // being left unset.
        Attributes: 0,
    };

    let mut new_token: HANDLE = std::ptr::null_mut();
    let ok = unsafe {
        CreateRestrictedToken(
            proc_token.raw(),
            WRITE_RESTRICTED,
            0,
            std::ptr::null(),
            0,
            std::ptr::null(),
            1,
            &sid_and_attrs,
            &mut new_token,
        )
    };
    if ok == 0 {
        return Err(last_error("creating the write-restricted token"));
    }
    Owned::new(new_token)
        .ok_or_else(|| "creating the write-restricted token returned a handle that was not real".to_string())
}

/// Stamps an ALLOW-write ACE for `logon_sid`, inherited onto everything
/// created under it later, onto `folder`'s EXISTING DACL -- never replacing
/// it. Idempotent: called once per `Bash` call rather than once per process
/// lifetime, because `SetEntriesInAclW` merges an entry for a Trustee it
/// already has rather than duplicating it, so re-stamping an already-
/// stamped folder is a wasted round trip, not a growing ACL.
fn grant_write_on_trusted_folder(folder: &Path, logon_sid: &[u8]) -> Result<(), String> {
    let folder_w = wide(&folder.to_string_lossy());

    let mut old_dacl: *mut windows_sys::Win32::Security::ACL = std::ptr::null_mut();
    let mut sd: windows_sys::Win32::Security::PSECURITY_DESCRIPTOR = std::ptr::null_mut();
    let status = unsafe {
        windows_sys::Win32::Security::Authorization::GetNamedSecurityInfoW(
            folder_w.as_ptr(),
            SE_FILE_OBJECT,
            windows_sys::Win32::Security::DACL_SECURITY_INFORMATION,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut old_dacl,
            std::ptr::null_mut(),
            &mut sd,
        )
    };
    if status != 0 {
        return Err(format!(
            "reading the trusted folder's own permissions failed (error {status}); the shell was \
             not started, because it would otherwise run with no confirmed write boundary."
        ));
    }
    // `sd` (and the `old_dacl` pointer INTO it -- not a separate allocation,
    // per `GetNamedSecurityInfoW`'s own contract) is freed exactly once,
    // whichever way this function returns from here.
    struct FreeSd(windows_sys::Win32::Security::PSECURITY_DESCRIPTOR);
    impl Drop for FreeSd {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe {
                    windows_sys::Win32::Foundation::LocalFree(self.0);
                }
            }
        }
    }
    let _free_sd = FreeSd(sd);

    let mut trustee: windows_sys::Win32::Security::Authorization::TRUSTEE_W =
        unsafe { std::mem::zeroed() };
    unsafe {
        BuildTrusteeWithSidW(&mut trustee, logon_sid.as_ptr() as *mut core::ffi::c_void);
    }

    let entry = EXPLICIT_ACCESS_W {
        grfAccessPermissions: FILE_GENERIC_READ | FILE_GENERIC_WRITE | FILE_GENERIC_EXECUTE | DELETE,
        grfAccessMode: windows_sys::Win32::Security::Authorization::GRANT_ACCESS,
        grfInheritance: windows_sys::Win32::Security::CONTAINER_INHERIT_ACE
            | windows_sys::Win32::Security::OBJECT_INHERIT_ACE,
        Trustee: trustee,
    };

    let mut new_dacl: *mut windows_sys::Win32::Security::ACL = std::ptr::null_mut();
    let status = unsafe { SetEntriesInAclW(1, &entry, old_dacl, &mut new_dacl) };
    if status != 0 {
        return Err(format!(
            "building the trusted folder's new permission list failed (error {status}); the \
             shell was not started."
        ));
    }
    struct FreeAcl(*mut windows_sys::Win32::Security::ACL);
    impl Drop for FreeAcl {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe {
                    windows_sys::Win32::Foundation::LocalFree(self.0 as *mut core::ffi::c_void);
                }
            }
        }
    }
    let _free_new = FreeAcl(new_dacl);

    let status = unsafe {
        SetNamedSecurityInfoW(
            folder_w.as_ptr(),
            SE_FILE_OBJECT,
            windows_sys::Win32::Security::DACL_SECURITY_INFORMATION,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            new_dacl,
            std::ptr::null_mut(),
        )
    };
    if status != 0 {
        return Err(format!(
            "writing the trusted folder's new permissions failed (error {status}); the shell \
             was not started."
        ));
    }
    Ok(())
}

/// An inheritable pipe: `read` is un-inherited immediately (this process
/// keeps it, the child must not), `write` stays inheritable for the child to
/// receive as its stdout or stderr handle.
struct Pipe {
    read: Owned,
    write: Owned,
}

fn inheritable_pipe(doing: &str) -> Result<Pipe, String> {
    let sa = windows_sys::Win32::Security::SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<windows_sys::Win32::Security::SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: std::ptr::null_mut(),
        bInheritHandle: 1,
    };
    let mut r: HANDLE = std::ptr::null_mut();
    let mut w: HANDLE = std::ptr::null_mut();
    // `CreatePipe` takes `lppipeattributes` by `*const`, so a shared
    // reference is all it needs -- `&sa` coerces the same way `&startup`
    // does below for `CreateProcessAsUserW`'s own `*const` parameters.
    let ok = unsafe { CreatePipe(&mut r, &mut w, &sa, 0) };
    if ok == 0 {
        return Err(last_error(doing));
    }
    let read = Owned::new(r).ok_or_else(|| format!("{doing} produced a read handle that was not real"))?;
    let write =
        Owned::new(w).ok_or_else(|| format!("{doing} produced a write handle that was not real"))?;
    // The PARENT's copy of the read end must not be inherited by the child --
    // if it were, the child would hold open its own read end of its own
    // output pipe, and the pipe would never signal EOF once the child exits,
    // because a second writer (the child's inherited copy) is still open.
    unsafe {
        SetHandleInformation(read.raw(), HANDLE_FLAG_INHERIT, 0);
    }
    Ok(Pipe { read, write })
}

/// A `NUL`-backed, inheritable handle for the child's stdin. `STARTF_
/// USESTDHANDLES` requires all three standard handles to be valid --
/// [CreateProcessAsUserW's own docs][cpau] are explicit that an invalid one
/// here is undefined rather than merely ignored -- and a command run from a
/// tool call has no keyboard behind it to read from regardless.
///
/// [cpau]: https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-createprocessasuserw
fn nul_stdin() -> Result<Owned, String> {
    let sa = windows_sys::Win32::Security::SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<windows_sys::Win32::Security::SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: std::ptr::null_mut(),
        bInheritHandle: 1,
    };
    let name = wide("NUL");
    let h = unsafe {
        CreateFileW(
            name.as_ptr(),
            GENERIC_READ,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            &sa,
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            std::ptr::null_mut(),
        )
    };
    if h == INVALID_HANDLE_VALUE {
        return Err(last_error("opening NUL for the command's unused standard input"));
    }
    Owned::new(h).ok_or_else(|| "opening NUL returned a handle that was not real".to_string())
}

/// How the run actually ended -- the same three shapes `tools.rs`'s own
/// `bash_tool` distinguishes on Unix, named separately here rather than
/// shared across the module boundary because the two implementations no
/// longer share a spawn/wait loop to hang a common enum off of.
enum Ending {
    Finished(u32),
    TimedOut,
    Cancelled,
}

/// The whole mechanism, from a validated `command`/`timeout` (validated by
/// `bash_tool` before this is ever reached -- this function trusts both) to
/// a finished `ToolResult`. See this file's own header for the sequence and
/// for what is and is not verified.
pub(crate) fn run(
    command: &str,
    workdir: &Path,
    timeout: Duration,
    cancelled: Option<&AtomicBool>,
) -> ToolResult {
    match try_run(command, workdir, timeout, cancelled) {
        Ok((ending, mut combined)) => {
            let (capped, truncated) = cap(combined.split_off(0));
            combined = capped;
            if truncated {
                combined.push_str(&format!("\n\n[output cut at {OUTPUT_CAP_CHARS} characters]"));
            }
            match ending {
                Ending::Cancelled => {
                    // Same reasoning as the Unix path's identical branch: the
                    // turn itself is already ending in `drive`'s Cancelled
                    // path and shows the person nothing, so this text exists
                    // for a caller inspecting `ToolResult` directly.
                    combined.push_str("\n\n[stopped: the turn was cancelled]");
                    ToolResult { output: combined, is_error: true }
                }
                Ending::TimedOut => {
                    combined.push_str(&format!(
                        "\n\n[the command was still running after {}s and was stopped]",
                        timeout.as_secs()
                    ));
                    ToolResult { output: combined, is_error: true }
                }
                Ending::Finished(0) => ToolResult { output: combined, is_error: false },
                Ending::Finished(code) => ToolResult {
                    output: format!("{combined}\n\n[exit code {code}]"),
                    is_error: true,
                },
            }
        }
        Err(message) => ToolResult { output: message, is_error: true },
    }
}

/// The setup-and-spawn body, split from `run` only so every early failure
/// can return through one `?`-friendly `Result` instead of duplicating the
/// cleanup at each one -- every `Owned`/`FreeSd`/`FreeAcl` guard above runs
/// on the way out of this function regardless of which `return` (or which
/// `?`) is taken, which is the entire reason those are RAII types rather
/// than handles closed by hand next to each error branch.
fn try_run(
    command: &str,
    workdir: &Path,
    timeout: Duration,
    cancelled: Option<&AtomicBool>,
) -> Result<(Ending, String), String> {
    let logon_sid = own_logon_sid()?;
    grant_write_on_trusted_folder(workdir, &logon_sid)?;
    let token = restricted_token(&logon_sid)?;

    let stdout = inheritable_pipe("opening a pipe for the command's output")?;
    let stderr = inheritable_pipe("opening a pipe for the command's error output")?;
    let stdin = nul_stdin()?;

    let comspec = std::env::var("ComSpec").unwrap_or_else(|_| r"C:\Windows\System32\cmd.exe".into());
    let app_w = wide(&comspec);
    // `argv[0]` is the quoted comspec path again, then `/C`, then the raw
    // command -- the same convention `std::process::Command::new("cmd.exe")
    // .arg("/C")` builds, made explicit here because `CreateProcessAsUserW`
    // takes one command-line string, not an argv array, and its own
    // "Security Remarks" warn specifically about an unquoted path here.
    let mut cmdline_w = wide(&format!("\"{comspec}\" /C {command}"));
    let dir_w = wide(&workdir.to_string_lossy());

    let mut startup: STARTUPINFOW = unsafe { std::mem::zeroed() };
    startup.cb = std::mem::size_of::<STARTUPINFOW>() as u32;
    startup.dwFlags = STARTF_USESTDHANDLES;
    startup.hStdInput = stdin.raw();
    startup.hStdOutput = stdout.write.raw();
    startup.hStdError = stderr.write.raw();

    let mut proc_info: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
    let ok = unsafe {
        CreateProcessAsUserW(
            token.raw(),
            app_w.as_ptr(),
            cmdline_w.as_mut_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            1, // bInheritHandles -- required for STARTF_USESTDHANDLES to work at all.
            CREATE_NO_WINDOW | CREATE_SUSPENDED,
            std::ptr::null(),
            dir_w.as_ptr(),
            &startup,
            &mut proc_info,
        )
    };
    if ok == 0 {
        return Err(last_error("starting the restricted shell"));
    }
    let process = Owned::new(proc_info.hProcess)
        .ok_or_else(|| "the restricted shell reported success but returned no process handle".to_string())?;
    let thread = Owned::new(proc_info.hThread)
        .ok_or_else(|| "the restricted shell reported success but returned no thread handle".to_string())?;

    // The child now exists, suspended, owning its own duplicate of the pipe
    // write ends and the NUL read end -- THIS process's copies of those three
    // must close now, both so the pipes signal EOF correctly once the child
    // exits (see `inheritable_pipe`'s own comment) and so nothing here leaks
    // a handle the child does not need us to keep.
    drop(stdout.write);
    drop(stderr.write);
    drop(stdin);

    let job = Owned::new(unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) })
        .ok_or_else(|| last_error("creating the job object that lets Stop reach the whole command tree"))?;
    let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
    limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    let ok = unsafe {
        SetInformationJobObject(
            job.raw(),
            JobObjectExtendedLimitInformation,
            &limits as *const _ as *const core::ffi::c_void,
            std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        )
    };
    if ok == 0 {
        return Err(last_error("configuring the job object to kill the whole command tree on close"));
    }
    // Assigned and only THEN resumed -- the command has not executed a
    // single instruction yet (`CREATE_SUSPENDED` above), so there is no
    // window in which anything it might spawn could exist outside the job.
    // See this file's own header for why this beats the plain
    // `std::process::Command` + `AssignProcessToJobObject` pairing the Unix
    // path's own doc already names as imperfect for the same reason.
    let ok = unsafe { AssignProcessToJobObject(job.raw(), process.raw()) };
    if ok == 0 {
        return Err(last_error("placing the restricted shell in its job object"));
    }
    let resumed = unsafe { ResumeThread(thread.raw()) };
    if resumed == u32::MAX {
        return Err(last_error("starting the restricted shell running"));
    }
    // The thread handle has done its one job; nothing below needs it again.
    drop(thread);

    // Reading both pipes on background threads, exactly the reason
    // `tools.rs`'s own Unix path gives for doing the same thing: a chatty
    // command can fill an OS pipe buffer and deadlock a caller that is
    // polling for exit status while holding the pipe unread. Wrapping the
    // raw `HANDLE` in a `std::fs::File` reuses that exact proven pattern
    // (`Read::read_to_string` on a background thread, joined through an
    // `mpsc` channel) instead of hand-rolling a second one for Windows.
    use std::os::windows::io::FromRawHandle;
    let out_handle = stdout.read.raw();
    let err_handle = stderr.read.raw();
    // Ownership of the raw handles moves into the `File`s now -- `Owned`
    // must not ALSO close them, so they are consumed here rather than kept
    // alive as `Owned` guards that would double-close on drop.
    std::mem::forget(stdout.read);
    std::mem::forget(stderr.read);
    let mut out_file = unsafe { std::fs::File::from_raw_handle(out_handle as std::os::windows::raw::HANDLE) };
    let mut err_file = unsafe { std::fs::File::from_raw_handle(err_handle as std::os::windows::raw::HANDLE) };
    let (out_tx, out_rx) = std::sync::mpsc::channel();
    let (err_tx, err_rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        use std::io::Read;
        let mut buf = String::new();
        let _ = out_file.read_to_string(&mut buf);
        let _ = out_tx.send(buf);
    });
    std::thread::spawn(move || {
        use std::io::Read;
        let mut buf = String::new();
        let _ = err_file.read_to_string(&mut buf);
        let _ = err_tx.send(buf);
    });

    let started = Instant::now();
    let ending = loop {
        let waited = unsafe { WaitForSingleObject(process.raw(), POLL_MS) };
        match waited {
            WAIT_OBJECT_0 => {
                let mut code: u32 = 0;
                if unsafe { GetExitCodeProcess(process.raw(), &mut code) } == 0 {
                    return Err(last_error("reading the restricted shell's own exit code"));
                }
                break Ending::Finished(code);
            }
            WAIT_TIMEOUT => {
                // Same order as the Unix path's own poll loop, and the same
                // reason: cancellation is checked first so "you pressed
                // Stop" is the truer message when both are true in the same
                // tick.
                if cancelled.map(|c| c.load(Ordering::SeqCst)).unwrap_or(false) {
                    unsafe {
                        TerminateJobObject(job.raw(), 1);
                    }
                    break Ending::Cancelled;
                }
                if started.elapsed() > timeout {
                    unsafe {
                        TerminateJobObject(job.raw(), 1);
                    }
                    break Ending::TimedOut;
                }
            }
            _ => return Err(last_error("waiting on the restricted shell")),
        }
    };

    let stdout_text = out_rx.recv_timeout(Duration::from_secs(5)).unwrap_or_default();
    let stderr_text = err_rx.recv_timeout(Duration::from_secs(5)).unwrap_or_default();
    let mut combined = String::new();
    if !stdout_text.is_empty() {
        combined.push_str(&stdout_text);
    }
    if !stderr_text.is_empty() {
        if !combined.is_empty() {
            combined.push('\n');
        }
        combined.push_str("[stderr]\n");
        combined.push_str(&stderr_text);
    }
    Ok((ending, combined))
}
