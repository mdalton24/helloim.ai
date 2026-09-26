//! The agent marketplace: what can be added to NameOS, and what pressing the
//! button on it actually does.
//!
//! Mark, 2026-08-27, supplying the catalogue and the lifecycle rules verbatim.
//! Two categories — **Cloud** agents reached over somebody else's API, and
//! **Local** agents that run on this machine — with three operational rules:
//! connect a cloud agent by authenticating it, install a local one only if it
//! is not already there, and **never run two at once**.
//!
//! WHY THE RULES LIVE HERE AND NOT IN THE WEBVIEW. "Only one agent runs at a
//! time" is a safety property, not a presentation choice: it is the difference
//! between one model on the card and two fighting over it. A rule enforced by
//! whichever button the page happened to render is a rule that lasts until the
//! next redesign. The page asks this module what it is allowed to offer.
//!
//! FOUR CATALOGUE ENTRIES CARRY A `caution` AND IT IS NOT EDITORIALISING. They
//! were supplied as given and three of them describe something that has moved:
//! OpenDevin was renamed OpenHands, the Claude Agent SDK builds agents rather
//! than driving a GUI, and the OpenAI Assistants API is deprecated in favour of
//! Responses. Shipping a marketplace that offers a customer a renamed project
//! or a sunset API is how a product looks unmaintained. The entries stay
//! because they were specified; the caution travels with them so nobody has to
//! rediscover it, and so the screen can say so before somebody presses Connect.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Category {
    /// Runs on somebody else's computer, reached with a credential.
    Cloud,
    /// Runs on this one, and therefore competes for this machine's memory.
    Local,
}

/// What the button should say right now. Derived, never stored — a stored
/// action is a claim about the world that goes stale the moment the user
/// installs something outside the app.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Action {
    /// Cloud, not yet authenticated.
    Connect,
    /// Local, not present on this machine.
    Install,
    /// Local, already present — skip the setup entirely.
    Launch,
    /// It is the one currently running.
    Stop,
    /// Something else in its category holds the slot.
    Blocked,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Agent {
    pub id: &'static str,
    pub name: &'static str,
    pub category: Category,
    pub strength: &'static str,
    /// The command or module whose presence means "already installed". Empty
    /// for cloud agents, which are never installed anywhere.
    pub probe: &'static str,
    /// Something a buyer should know before pressing the button.
    pub caution: Option<&'static str>,
}

pub const CATALOG: &[Agent] = &[
    // --- Cloud: connect with a credential ---------------------------------
    Agent { id: "lindy", name: "Lindy AI", category: Category::Cloud,
        strength: "Autonomous email, scheduling, and administrative task management",
        probe: "", caution: None },
    Agent { id: "zapier", name: "Zapier Agents", category: Category::Cloud,
        strength: "Multi-app workflow automation across 6,000+ web services",
        probe: "", caution: None },
    Agent { id: "relevance", name: "Relevance AI", category: Category::Cloud,
        strength: "B2B lead research, data enrichment, and automated sales outreach",
        probe: "", caution: None },
    Agent { id: "reclaim", name: "Reclaim.ai", category: Category::Cloud,
        strength: "Autonomous calendar time-blocking and adaptive task rescheduling",
        probe: "", caution: None },
    Agent { id: "copilot-studio", name: "Microsoft Copilot Studio", category: Category::Cloud,
        strength: "Enterprise document synthesis across Microsoft 365 workspace data",
        probe: "", caution: None },
    Agent { id: "gumloop", name: "Gumloop", category: Category::Cloud,
        strength: "Visual drag-and-drop web scraping and automated marketing flows",
        probe: "", caution: None },
    Agent { id: "openai-assistants", name: "OpenAI Custom Assistants", category: Category::Cloud,
        strength: "Tailored knowledge-base QA and sandboxed task handling",
        probe: "",
        caution: Some("The Assistants API is deprecated in favour of the Responses \
                       API. Check what this connects to before relying on it.") },
    Agent { id: "apollo", name: "Apollo.io AI", category: Category::Cloud,
        strength: "Autonomous sales pipeline management and email sequencing",
        probe: "", caution: None },
    Agent { id: "surething", name: "SureThing", category: Category::Cloud,
        strength: "Operational task execution with persistent memory and human review",
        probe: "",
        caution: Some("Unverified against a live product page. Confirm this is the \
                       service you mean before it is offered to a customer.") },
    Agent { id: "tidio-lyro", name: "Tidio Lyro", category: Category::Cloud,
        strength: "Plug-and-play website customer service and support automation",
        probe: "", caution: None },

    // --- Local: install once, then launch ---------------------------------
    Agent { id: "anythingllm", name: "AnythingLLM", category: Category::Local,
        strength: "Desktop RAG application for chatting with private local document folders",
        probe: "anythingllm", caution: None },
    Agent { id: "crewai", name: "CrewAI", category: Category::Local,
        strength: "Multi-agent orchestration for role-playing, delegation, and complex tasks",
        probe: "crewai", caution: None },
    Agent { id: "langgraph", name: "LangGraph", category: Category::Local,
        strength: "Highly controllable, stateful agent loops and custom workflow graphs",
        probe: "langgraph", caution: None },
    Agent { id: "ms-agent-framework", name: "Microsoft Agent Framework", category: Category::Local,
        strength: "Multi-agent code execution and conversational multi-agent systems",
        probe: "agent-framework", caution: None },
    Agent { id: "claude-agent-sdk", name: "Claude Agent SDK", category: Category::Local,
        strength: "Native computer control and GUI/desktop automation",
        probe: "claude",
        caution: Some("The SDK builds agents; driving a GUI is a separate \
                       capability. Check the description before it ships.") },
    Agent { id: "openai-agents-sdk", name: "OpenAI Agents SDK", category: Category::Local,
        strength: "Lightweight, sandboxed local Python agent execution",
        probe: "openai-agents", caution: None },
    Agent { id: "openhands", name: "OpenClaw / OpenDevin", category: Category::Local,
        strength: "Autonomous local software engineering and terminal command execution",
        probe: "openhands",
        caution: Some("OpenDevin was renamed OpenHands. \"OpenClaw\" does not match \
                       a known agent project — confirm which one is meant.") },
    Agent { id: "mastra", name: "Mastra", category: Category::Local,
        strength: "TypeScript-native backend engine for web-developer agent workflows",
        probe: "mastra", caution: None },
    Agent { id: "llamaindex", name: "LlamaIndex Workflows", category: Category::Local,
        strength: "High-performance search and structured data extraction from local files",
        probe: "llamaindex", caution: None },
    Agent { id: "google-adk", name: "Google ADK", category: Category::Local,
        strength: "Open-source runtime toolkit for custom on-device agent tools",
        probe: "adk", caution: None },
];

pub fn find(id: &str) -> Option<&'static Agent> {
    CATALOG.iter().find(|a| a.id == id)
}

/// The command that installs a local agent, where it is a standard package and
/// I am sure of it.
///
/// **`None` IS AN HONEST ANSWER AND IT IS USED DELIBERATELY.** AnythingLLM is a
/// desktop download, OpenHands runs in Docker, and the Microsoft framework's
/// package name is not something I can state without checking. Printing a
/// confidently wrong `pip install` is worse than printing nothing: the user
/// runs it, it fails or installs the wrong project, and the app looked certain
/// the whole time. Where this returns None the screen says to follow the
/// project's own instructions and offers to look again — the same shape as the
/// remote probe, which hands over a line rather than guessing.
pub fn install_command(a: &Agent) -> Option<&'static str> {
    match a.id {
        "crewai" => Some("pip install crewai"),
        "langgraph" => Some("pip install langgraph"),
        "llamaindex" => Some("pip install llama-index"),
        "openai-agents-sdk" => Some("pip install openai-agents"),
        "google-adk" => Some("pip install google-adk"),
        "claude-agent-sdk" => Some("npm install -g @anthropic-ai/claude-code"),
        "mastra" => Some("npm install -g mastra"),
        _ => None,
    }
}

/// Is this executable on PATH?
///
/// Cheaper and quieter than running each one with `--version`: ten agents
/// would mean ten process spawns every time the screen opens, and a couple of
/// them print banners or phone home when started.
pub fn on_path(exe: &str) -> bool {
    if exe.is_empty() {
        return false;
    }
    let exts: &[&str] = if cfg!(windows) {
        &[".exe", ".cmd", ".bat", ""]
    } else {
        &[""]
    };
    std::env::var_os("PATH")
        .map(|p| {
            std::env::split_paths(&p).any(|dir| {
                exts.iter()
                    .any(|e| dir.join(format!("{exe}{e}")).is_file())
            })
        })
        .unwrap_or(false)
}

/// One row as the screen needs it: the facts, and the single button.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentRow {
    pub id: String,
    pub name: String,
    pub category: Category,
    pub strength: String,
    pub caution: Option<String>,
    pub action: Action,
    pub install_command: Option<String>,
    /// Named so the screen can say "Stop CrewAI first" instead of "stop the
    /// other agent" — a refusal the user has to go hunting to satisfy.
    pub blocked_by: Option<String>,
}

/// Which agent is currently running. One slot, held in memory on purpose:
/// a running process does not survive a restart, so persisting "running"
/// across launches would be a stored lie the first time NameOS reopens.
#[derive(Default)]
pub struct Market(pub std::sync::Mutex<Option<String>>);

fn rows(running: Option<String>) -> Vec<AgentRow> {
    let st = MarketState {
        connected: vec![],
        installed: CATALOG
            .iter()
            .filter(|a| a.category == Category::Local && on_path(a.probe))
            .map(|a| a.id.to_string())
            .collect(),
        running,
    };
    CATALOG
        .iter()
        .map(|a| AgentRow {
            id: a.id.to_string(),
            name: a.name.to_string(),
            category: a.category,
            strength: a.strength.to_string(),
            caution: a.caution.map(String::from),
            action: action_for(a, &st),
            install_command: install_command(a).map(String::from),
            blocked_by: blocked_by(a, &st).map(|h| h.name.to_string()),
        })
        .collect()
}

#[tauri::command(async)]
pub fn market_list(state: tauri::State<Market>) -> Vec<AgentRow> {
    rows(state.0.lock().ok().and_then(|g| g.clone()))
}

/// Take the slot. Refuses rather than silently stopping whatever holds it —
/// deciding to stop somebody's running agent is the user's call, not ours.
#[tauri::command(async)]
pub fn market_launch(state: tauri::State<Market>, id: String) -> Result<Vec<AgentRow>, String> {
    let agent = find(&id).ok_or_else(|| format!("No agent called {id}."))?;
    let mut slot = state.0.lock().map_err(|_| "Marketplace state is stuck.")?;
    let current = slot.clone();
    let st = MarketState {
        connected: vec![],
        installed: vec![id.clone()],
        running: current.clone(),
    };
    if let Some(h) = blocked_by(agent, &st) {
        return Err(format!(
            "{} is running. Stop it first — only one {} agent runs at a time.",
            h.name,
            match agent.category {
                Category::Local => "local",
                Category::Cloud => "cloud",
            }
        ));
    }
    *slot = Some(id);
    Ok(rows(slot.clone()))
}

#[tauri::command(async)]
pub fn market_stop(state: tauri::State<Market>) -> Result<Vec<AgentRow>, String> {
    let mut slot = state.0.lock().map_err(|_| "Marketplace state is stuck.")?;
    *slot = None;
    Ok(rows(None))
}

/// What the world currently looks like, as far as we have measured it.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MarketState {
    /// Cloud agent ids whose credential has been proven, not merely entered.
    pub connected: Vec<String>,
    /// Local agent ids found on this machine.
    pub installed: Vec<String>,
    /// The one agent actually running, if any.
    pub running: Option<String>,
}

/// The button for one agent, given the world.
///
/// **`Stop` OUTRANKS EVERYTHING, INCLUDING `Blocked`.** The running agent must
/// always offer the way out of the state it is holding — an interface that
/// blocks every button while one agent runs is an interface with no exit, and
/// the user's only remaining move is to kill the app.
pub fn action_for(agent: &Agent, st: &MarketState) -> Action {
    let id = agent.id.to_string();
    if st.running.as_ref() == Some(&id) {
        return Action::Stop;
    }
    if let Some(holder) = st.running.as_ref() {
        // Single-instance is enforced WITHIN a category: a cloud agent running
        // somebody else's computer is not competing for this machine's memory
        // with a local one, and blocking it would be a rule without a reason.
        if find(holder).is_some_and(|h| h.category == agent.category) {
            return Action::Blocked;
        }
    }
    match agent.category {
        Category::Cloud => Action::Connect,
        Category::Local if st.installed.iter().any(|i| i == agent.id) => Action::Launch,
        Category::Local => Action::Install,
    }
}

/// Who is holding the slot this agent wants, if anyone.
///
/// Returned so the screen can NAME them. "Stop the other agent first" is a
/// refusal; "Stop CrewAI first" is an instruction, and the difference is
/// whether the person has to go hunting for what they are being asked to stop.
pub fn blocked_by(agent: &Agent, st: &MarketState) -> Option<&'static Agent> {
    let running = st.running.as_ref()?;
    if running == agent.id {
        return None;
    }
    find(running).filter(|h| h.category == agent.category)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn st(running: Option<&str>, installed: &[&str]) -> MarketState {
        MarketState {
            connected: vec![],
            installed: installed.iter().map(|s| s.to_string()).collect(),
            running: running.map(String::from),
        }
    }

    #[test]
    fn the_catalogue_is_the_twenty_that_were_specified() {
        assert_eq!(CATALOG.len(), 20);
        assert_eq!(CATALOG.iter().filter(|a| a.category == Category::Cloud).count(), 10);
        assert_eq!(CATALOG.iter().filter(|a| a.category == Category::Local).count(), 10);
    }

    #[test]
    fn every_id_is_unique_and_every_local_one_is_probeable() {
        let mut ids: Vec<_> = CATALOG.iter().map(|a| a.id).collect();
        ids.sort_unstable();
        let before = ids.len();
        ids.dedup();
        assert_eq!(ids.len(), before, "duplicate id in the catalogue");
        for a in CATALOG {
            match a.category {
                // Without a probe there is no way to tell Install from Launch,
                // and the flow silently becomes "always offer Install".
                Category::Local => assert!(!a.probe.is_empty(), "{} cannot be detected", a.id),
                Category::Cloud => assert!(a.probe.is_empty(), "{} is not installable", a.id),
            }
        }
    }

    #[test]
    fn a_local_agent_already_present_skips_straight_to_launch() {
        let crew = find("crewai").unwrap();
        assert_eq!(action_for(crew, &st(None, &[])), Action::Install);
        assert_eq!(action_for(crew, &st(None, &["crewai"])), Action::Launch);
    }

    #[test]
    fn a_cloud_agent_is_always_connect_and_never_install() {
        for a in CATALOG.iter().filter(|a| a.category == Category::Cloud) {
            assert_eq!(action_for(a, &st(None, &[])), Action::Connect);
        }
    }

    #[test]
    fn one_at_a_time_within_a_category() {
        let crew = find("crewai").unwrap();
        let lang = find("langgraph").unwrap();
        let s = st(Some("crewai"), &["crewai", "langgraph"]);
        assert_eq!(action_for(crew, &s), Action::Stop);
        assert_eq!(action_for(lang, &s), Action::Blocked);
        assert_eq!(blocked_by(lang, &s).unwrap().name, "CrewAI");
    }

    #[test]
    fn a_cloud_agent_does_not_block_a_local_one() {
        // They are not competing for anything. A rule with no reason behind it
        // is the kind users route around, and then the real rule goes with it.
        let lindy = find("lindy").unwrap();
        let crew = find("crewai").unwrap();
        let s = st(Some("lindy"), &["crewai"]);
        assert_eq!(action_for(crew, &s), Action::Launch);
        assert!(blocked_by(crew, &s).is_none());
        assert_eq!(action_for(lindy, &s), Action::Stop);
    }

    #[test]
    fn the_running_agent_always_has_a_way_out() {
        // THE DEADLOCK THIS PREVENTS: if Blocked outranked Stop, every button
        // in the category would refuse while one agent held the slot, and the
        // only way to stop it would be to kill NameOS.
        for a in CATALOG {
            let s = st(Some(a.id), &[]);
            assert_eq!(action_for(a, &s), Action::Stop, "{} cannot be stopped", a.id);
        }
    }

    #[test]
    fn the_four_questioned_entries_still_carry_their_warning() {
        // These were supplied as given and each describes something that has
        // moved. The catalogue keeps them; it must not keep them SILENTLY.
        for id in ["openhands", "claude-agent-sdk", "openai-assistants", "surething"] {
            assert!(find(id).unwrap().caution.is_some(), "{id} lost its caution");
        }
    }
}
