//! The app's own voice — the floor under every reply.
//!
//! Mark, 2026-08-26, looking at a first conversation: "very dry and not
//! friendly." He was right, and the cause was not the wording of anything we
//! had written. It was that we had written NOTHING. The only channel to the
//! model was `profile.rs`'s CLAUDE.md block, and that block only exists once
//! the person has typed something into the form. Somebody who names their
//! assistant and starts talking gets a bare model with no instructions, which
//! answers in the register of a support ticket.
//!
//! THE SCAR IS THE REPLY ITSELF. Asked "how's it going?", it opened by
//! explaining that it does not carry anything between sessions and had no
//! context on the folder. Every word true, and a terrible thing to lead with:
//! the product introduced itself by what it cannot do, to someone who had just
//! given it a name. **A first impression spent on a disclaimer is gone.**
//!
//! WHY THIS IS NOT IN CLAUDE.md, which was the obvious place and is wrong:
//!
//! 1. **CLAUDE.md is the USER'S file.** `profile.rs` goes to real lengths to
//!    touch only the bytes between its own markers, because that file may be
//!    full of instructions somebody cares about. The app's own voice does not
//!    belong in a document the user owns and edits.
//! 2. **It gets stripped.** Clearing the profile removes the block by design —
//!    which would take the voice with it, so emptying a form would silently
//!    make the assistant stiff again.
//! 3. **It does not exist yet on the very first run**, which is the one turn
//!    that matters most here.
//!
//! `--append-system-prompt` has none of those problems: it is ours, it is on
//! every turn, and it is present before the user has typed a thing.
//!
//! IT IS A FLOOR, NOT A CEILING, and the last paragraph of the prompt says so
//! out loud. Someone who writes "blunt, no small talk" into the personality box
//! has told us what they want and must get it. This removes stiffness from the
//! default; it does not install a character over the top of somebody's choice.

/// What gets appended to the system prompt on every turn.
///
/// Written as prose rather than a bullet list on purpose — a model reading
/// instructions about how to sound copies the shape of what it reads, and a
/// list of terse directives is itself the dry register we are trying to leave.
pub const VOICE: &str = "\
You are the assistant inside helloim.ai — a desktop app someone runs on their own \
machine and gives their own name to. Here is how you sound.

Talk like a person, not a service. Warm, plain, unhurried, contractions and \
all. The register is someone sitting next to them who is glad to be there, \
not a support agent working a ticket and not a manual.

Never open a reply with what you cannot do. No disclaimers about memory, \
context, sessions or your own limits as an opening move — nobody asked, and it \
spends the one moment you had on the thing you are worst at. If you are \
missing something you genuinely need, ask a short question for it instead. \
Asking is friendly. Announcing a gap is not.

The first time you meet someone, be genuinely pleased about it. They just \
named you and opened a window to see what you are — so say hello like it \
matters, with real energy, in a line or two. Then get out of the way and ask \
what they are working on. If you have spoken before, be glad to see them again \
and keep it to a beat.

Lead with the answer. When short and thorough are both true, be short.

If someone asks how to connect an app, an account or their email: there is no \
Settings menu, so point them at the Apps button in the left rail. Most of what \
is listed there connects in one click. Email does not — Gmail is not one of \
those one-click apps, and offering it as a quick Connect would be a promise \
this app cannot keep today. Say so plainly rather than sending them looking \
for a menu that is not there.

If the working folder's CLAUDE.md gives you a name or a personality, that is \
the person describing their own assistant, and it outranks everything here \
about tone. This is the floor, not the ceiling.";

#[cfg(test)]
mod tests {
    use super::VOICE;

    /// The two failures from the screenshot, asserted directly. Both of these
    /// are the actual words the bad reply used, so a future edit that quietly
    /// drops the rule fails here rather than in front of a new user.
    #[test]
    fn it_forbids_opening_on_a_limitation() {
        let v = VOICE.to_lowercase();
        assert!(v.contains("never open a reply with what you cannot do"));
        assert!(v.contains("memory, context, sessions"));
    }

    #[test]
    fn it_asks_for_energy_on_a_first_meeting() {
        let v = VOICE.to_lowercase();
        assert!(v.contains("first time you meet someone"));
        assert!(v.contains("real energy"));
    }

    /// The user's own personality setting has to win, or this stops being a
    /// default and becomes an override of somebody's stated choice.
    #[test]
    fn the_users_own_personality_still_outranks_it() {
        assert!(VOICE.contains("outranks everything here about tone"));
        assert!(VOICE.contains("floor, not the ceiling"));
    }

    /// Wren traced a real reply ("Settings → Integrations → Gmail") back to
    /// this constant: it is the ONLY place the model learns the app's own
    /// name, and it still said "NameOS" after the product was renamed. The
    /// app name was real-but-stale; the nav path was invented outright. Both
    /// assertions guard the specific words that were wrong, not a paraphrase
    /// of them, so a future edit that quietly reintroduces either failure
    /// breaks here instead of in front of a user.
    #[test]
    fn the_app_name_is_current() {
        assert!(VOICE.contains("helloim.ai"));
        assert!(!VOICE.contains("NameOS"));
    }

    /// The other half of that same reply: there is no Settings menu, and
    /// Gmail was never a one-click connector. This does not need to name the
    /// hallucination it is replacing — only that the true nav (Apps, left
    /// rail) is in the prompt and that Gmail is called out as the exception
    /// rather than left implied by omission.
    #[test]
    fn it_names_the_real_nav_and_the_gmail_exception() {
        let v = VOICE.to_lowercase();
        assert!(v.contains("apps button in the left rail"));
        assert!(v.contains("no settings menu"));
        assert!(v.contains("gmail"));
    }

    /// A prompt passed as one argv entry — no shell, no quoting to get wrong.
    /// Newlines are fine; a stray NUL would truncate it at the exec boundary.
    #[test]
    fn it_survives_being_an_argv_entry() {
        assert!(!VOICE.contains('\0'));
        assert!(!VOICE.trim().is_empty());
    }
}
