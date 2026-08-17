//! What kind of action a tool performs.
//!
//! The default matters more than the taxonomy. An unrecognised tool classifies as `Script`,
//! the most restricted class, so forgetting to classify a newly added tool fails closed. The
//! alternative — an `Unknown` class that is permitted — means every new tool ships
//! unrestricted until someone notices.

//! Central policy engine: every tool call is classified and evaluated before it runs.
//!
//! Before this existed, security lived in scattered point-checks — an SSRF guard in
//! `reach`, an https requirement in `login`, an upload-root check in `reach`. Each is
//! correct, but there was no single place that could answer "is this call allowed?",
//! so every new tool had to remember to re-implement the relevant guard.
//!
//! The engine sits in the MCP dispatch path (see `mcp::handle_tools_call`) between
//! argument validation and execution. It does not replace the point-checks — defence
//! in depth — it decides whether the call happens at all.
//!
//! Design constraints worth stating, because they shaped the defaults:
//!
//! - **A denial must be legible.** A model that gets an opaque failure retries; one
//!   that is told which rule fired and what would satisfy it can adapt or ask.
//! - **`ask` is only useful if someone can answer.** Most MCP clients do not
//!   implement elicitation, so a profile that asks for confirmation on ordinary
//!   actions would simply be a profile that fails. Confirmation is therefore
//!   reserved for the `Safe` profile's elevated classes, and returns a structured
//!   `requires_confirmation` the caller can act on rather than a dead end.
//! - **The default profile does not break existing setups.** `Developer` enforces
//!   the domain lists and logs elevated actions but does not gate them. Users who
//!   want gating opt into `Safe`; unattended agents should use `Autonomous`, which
//!   demands an explicit allowlist.

/// What a tool call actually does, for policy purposes.
///
/// Grouped by consequence rather than by implementation: `js` sits alone because
/// arbitrary script in the page context can impersonate every other class, and
/// `Replay` is elevated because a recorded playbook can contain any mutation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ActionClass {
    /// Observation only: no page state changes, nothing leaves the machine.
    Read,
    /// Moves the browser somewhere, including tab management.
    Navigate,
    /// Mutates page state — clicks, typing, form submission.
    Interact,
    /// Outbound query to a third-party search provider.
    Search,
    /// Server-side fetch or file write (`browse`, `download`).
    Fetch,
    /// Reads a local file and hands it to a page.
    Upload,
    /// Touches credentials or session material.
    Auth,
    /// Arbitrary script execution, or replay of a recorded mutation sequence.
    Script,
}

impl ActionClass {
    /// Human label used in denial and confirmation messages.
    pub fn label(self) -> &'static str {
        match self {
            ActionClass::Read => "read",
            ActionClass::Navigate => "navigate",
            ActionClass::Interact => "interact",
            ActionClass::Search => "search",
            ActionClass::Fetch => "fetch",
            ActionClass::Upload => "upload",
            ActionClass::Auth => "auth",
            ActionClass::Script => "script",
        }
    }

    /// Classes whose blast radius reaches past the current page: local files,
    /// credentials, arbitrary code. These are what `Safe` gates.
    pub fn is_elevated(self) -> bool {
        matches!(
            self,
            ActionClass::Upload | ActionClass::Auth | ActionClass::Script | ActionClass::Fetch
        )
    }
}

/// Classify a tool by name.
///
/// An unrecognised name resolves to `Script`, the most restrictive class. A tool
/// added without being classified here therefore becomes *more* guarded, not less —
/// the failure mode of forgetting is a denial, never a silent bypass.
pub fn classify(tool: &str) -> ActionClass {
    match tool {
        "status" | "read" | "screenshot" | "find" | "list_tabs" | "page_info" | "console_logs"
        | "network_log" | "metrics" | "debug" | "analyze" | "extract" | "extract_table"
        | "session_info" | "wait" | "observe" | "perf_trace" | "computed_style" | "har_export"
        | "trace_bundle" | "profile_mode" | "list_frames" | "bridge_status" | "cpu_profile"
        | "heap_stats" | "source_map" => ActionClass::Read,

        "navigate" | "new_tab" | "switch_tab" | "close_tab" | "scroll" | "paginate" => {
            ActionClass::Navigate
        }

        "click" | "type" | "fill" | "form_fill" | "submit" | "find_and_click"
        | "dismiss_overlay" | "press" | "hover" | "click_variant" | "set_control" | "drag" => {
            ActionClass::Interact
        }

        "search" | "search_images" | "search_videos" | "search_twitter_videos" => {
            ActionClass::Search
        }

        "browse" | "download" => ActionClass::Fetch,

        // Reads a local file, so it belongs with upload rather than with the read-only
        // debugging tools.
        "har_import" => ActionClass::Upload,

        "upload" => ActionClass::Upload,

        // Composites inherit the worst class of what they do internally.
        "login_flow" => ActionClass::Auth,
        "extract_paginated" => ActionClass::Interact,
        "pierce" | "dialog" | "emulate" => ActionClass::Interact,

        // Raw CDP into the user's real browser is the most powerful thing here.
        "bridge_cdp" => ActionClass::Script,

        "login" | "save_cookies" | "restore_cookies" | "save_session" | "revoke_session" => {
            ActionClass::Auth
        }

        // `js` is arbitrary code. `replay` re-dispatches recorded steps, so it
        // inherits the worst class its playbook could contain. Recording controls
        // themselves change what gets persisted, which can capture credentials.
        "js" | "replay" | "record_task" | "stop_recording" => ActionClass::Script,

        _ => ActionClass::Script,
    }
}
