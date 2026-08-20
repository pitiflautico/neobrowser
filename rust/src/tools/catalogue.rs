//! Which tools exist, which are advertised, and how one is looked up.
//!
//! Not every tool is advertised by default. A model handed sixty-seven tools chooses worse
//! than one handed the twenty-six it usually needs, so the rest are opt-in — the capability
//! is there without the cognitive cost of listing it.

use std::sync::Arc;

use serde_json::Value;

use super::ctx::Tool;

/// Which tools an MCP client sees.
///
/// The full set is 55 tools, and every one of them costs schema in the model's
/// context on every single request — before it has done anything. Most sessions use
/// eight or nine. `Core` advertises the ones that cover ordinary work; `Full` keeps
/// everything for scripted callers and expert use.
///
/// Deliberately a *filter over one registry*, not a second façade layer with its own
/// names. A parallel `browser_*` API mapping onto these tools would mean two
/// surfaces to keep in step and two places for behaviour to drift — and the names
/// would collide with Playwright MCP's for no benefit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Toolset {
    /// The default: enough to navigate, observe, act, extract and debug.
    Core,
    /// Everything registered.
    Full,
}

impl Toolset {
    pub fn from_env() -> Self {
        match std::env::var("NEOBROWSER_TOOLSET")
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase()
            .as_str()
        {
            "full" | "all" => Toolset::Full,
            // Anything unrecognised falls back to the default rather than erroring:
            // a typo should not leave a client with no tools at all.
            _ => Toolset::Core,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Toolset::Core => "core",
            Toolset::Full => "full",
        }
    }
}

/// The core set. Chosen by what a session actually needs end to end, not by
/// category: observe/act/verify, plus the escape hatches (`js`) and the debugging
/// entry points a developer reaches for first.
pub const CORE_TOOLS: &[&str] = &[
    "status",
    "navigate",
    "observe",
    "read",
    "find",
    "click",
    "type",
    "fill",
    "form_fill",
    "submit",
    "press",
    "screenshot",
    "extract",
    "search",
    "upload",
    "download",
    "js",
    "wait",
    "new_tab",
    "list_tabs",
    "switch_tab",
    "close_tab",
    "console_logs",
    "network_log",
    "perf_trace",
    "session_info",
];

/// The set of tools this server exposes. Grows per phase; `tools/list` only ever
/// advertises registered (i.e. genuinely working) tools.
#[derive(Default)]
pub struct Registry {
    tools: Vec<Arc<dyn Tool>>,
}

impl Registry {
    pub fn new() -> Self {
        Self { tools: Vec::new() }
    }

    pub fn register(&mut self, tool: Arc<dyn Tool>) {
        self.tools.push(tool);
    }

    pub fn get(&self, name: &str) -> Option<&Arc<dyn Tool>> {
        self.tools.iter().find(|t| t.spec().name == name)
    }

    pub fn descriptors(&self) -> Vec<Value> {
        self.tools.iter().map(|t| t.spec().descriptor()).collect()
    }

    /// Descriptors for `tools/list`, filtered by the active toolset.
    ///
    /// Filtering the *advertisement* only: a tool outside the core set is still
    /// callable if a client knows its name. Hiding a tool from the catalogue reduces
    /// context; refusing to run it would break scripted callers for no security gain,
    /// since the policy engine is what decides what is allowed.
    pub fn descriptors_for(&self, set: Toolset) -> Vec<Value> {
        self.tools
            .iter()
            .filter(|t| match set {
                Toolset::Full => true,
                Toolset::Core => CORE_TOOLS.contains(&t.spec().name),
            })
            .map(|t| t.spec().descriptor())
            .collect()
    }

    pub fn len(&self) -> usize {
        self.tools.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }
}
