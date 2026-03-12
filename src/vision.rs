//! Vision AI — intelligent page perception for AI agents.
//!
//! Not just "here's the text on the page" but:
//! 1. What TYPE of page is this? (login, search, chat, article, form, dashboard)
//! 2. What ACTIONS are available? (submit form, click link, type in search, send message)
//! 3. What CONTENT matters? (main content, not nav/footer/ads noise)
//! 4. What's the current STATE? (logged in? loading? error? success?)
//!
//! This is what makes NeoBrowser fundamentally different from Playwright/Puppeteer.

use crate::semantic;
use markup5ever_rcdom::Handle;

/// Classified view of a page — what an AI needs to understand and act.
#[derive(Debug)]
pub struct PageView {
    pub url: String,
    pub title: String,
    pub page_type: PageType,
    pub state: PageState,
    pub content: Vec<String>,       // Main content (noise filtered)
    pub actions: Vec<Action>,       // Available actions
    pub summary: String,            // One-line description
}

#[derive(Debug, Clone, PartialEq)]
pub enum PageType {
    Login,
    Search,
    SearchResults,
    Article,
    Chat,
    Form,
    Dashboard,
    List,
    Profile,
    Error,
    Unknown,
}

impl std::fmt::Display for PageType {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            PageType::Login => write!(f, "login"),
            PageType::Search => write!(f, "search"),
            PageType::SearchResults => write!(f, "search-results"),
            PageType::Article => write!(f, "article"),
            PageType::Chat => write!(f, "chat"),
            PageType::Form => write!(f, "form"),
            PageType::Dashboard => write!(f, "dashboard"),
            PageType::List => write!(f, "list"),
            PageType::Profile => write!(f, "profile"),
            PageType::Error => write!(f, "error"),
            PageType::Unknown => write!(f, "unknown"),
        }
    }
}

#[derive(Debug, Clone)]
pub enum PageState {
    Ready,
    Loading,
    LoggedIn,
    LoggedOut,
    Error(String),
}

impl std::fmt::Display for PageState {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            PageState::Ready => write!(f, "ready"),
            PageState::Loading => write!(f, "loading"),
            PageState::LoggedIn => write!(f, "logged-in"),
            PageState::LoggedOut => write!(f, "logged-out"),
            PageState::Error(e) => write!(f, "error: {e}"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Action {
    pub kind: ActionKind,
    pub label: String,
    pub target: String, // What to pass to click/focus
}

#[derive(Debug, Clone)]
pub enum ActionKind {
    Click,
    Type,
    Submit,
    Navigate,
    Search,
    SendMessage,
    Login,
    Scroll,
}

impl std::fmt::Display for ActionKind {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            ActionKind::Click => write!(f, "click"),
            ActionKind::Type => write!(f, "type"),
            ActionKind::Submit => write!(f, "submit"),
            ActionKind::Navigate => write!(f, "navigate"),
            ActionKind::Search => write!(f, "search"),
            ActionKind::SendMessage => write!(f, "send_message"),
            ActionKind::Login => write!(f, "login"),
            ActionKind::Scroll => write!(f, "scroll"),
        }
    }
}

impl std::fmt::Display for Action {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{}: {} → {}", self.kind, self.label, self.target)
    }
}

/// Analyze a DOM tree and produce an AI-ready page view.
pub fn analyze(document: &Handle, url: &str, title: &str) -> PageView {
    // 1. Extract raw semantic lines
    let mut all_lines = Vec::new();
    semantic::walk(document, 0, &mut all_lines);

    // 2. Get stats
    let mut stats = semantic::PageStats::new();
    semantic::count_nodes(document, &mut stats);

    // 3. Classify page type
    let page_type = classify_page(&all_lines, &stats, url, title);

    // 4. Detect state
    let state = detect_state(&all_lines, url);

    // 5. Extract actions
    let actions = extract_actions(&all_lines, &page_type);

    // 6. Filter content (remove noise)
    let content = filter_content(&all_lines, &page_type);

    // 7. Generate summary
    let summary = summarize(&page_type, &state, &stats, title, url);

    PageView {
        url: url.to_string(),
        title: title.to_string(),
        page_type,
        state,
        content,
        actions,
        summary,
    }
}

// ─── Page classification ───

fn classify_page(
    lines: &[String],
    stats: &semantic::PageStats,
    url: &str,
    title: &str,
) -> PageType {
    let text = lines.join("\n").to_lowercase();
    let url_lower = url.to_lowercase();
    let title_lower = title.to_lowercase();

    // Login page: has password field + login/signin button
    if text.contains("[textbox:") && text.contains("type=password")
        || (text.contains("sign in") || text.contains("log in") || text.contains("iniciar sesión"))
            && stats.textboxes >= 2
    {
        return PageType::Login;
    }

    // Chat: chatgpt, gemini, claude, or has send/message patterns
    if url_lower.contains("chat") || url_lower.contains("gemini")
        || text.contains("send message") || text.contains("enviar mensaje")
        || text.contains("enter a prompt") || text.contains("escribe un mensaje")
    {
        return PageType::Chat;
    }

    // Search results: has "results" or many article-like items
    if (text.contains("results") || text.contains("resultados"))
        && stats.links > 20
    {
        return PageType::SearchResults;
    }

    // Search page: prominent search box, few other elements
    if url_lower.contains("google.com") && !url_lower.contains("search")
        || url_lower.contains("bing.com") && !url_lower.contains("search")
        || (stats.textboxes >= 1 && stats.links < 15 && stats.headings < 3)
    {
        return PageType::Search;
    }

    // Article: long content, few forms, has headings
    if stats.headings >= 3 && stats.forms <= 1 && lines.len() > 50 {
        return PageType::Article;
    }

    // Profile: social media profile patterns
    if url_lower.contains("/in/") || url_lower.contains("/profile")
        || url_lower.contains("/@")
        || title_lower.contains("profile")
    {
        return PageType::Profile;
    }

    // Form: multiple inputs
    if stats.textboxes >= 3 && stats.forms >= 1 {
        return PageType::Form;
    }

    // Dashboard: lots of buttons, sections
    if stats.buttons > 10 && stats.headings > 5 {
        return PageType::Dashboard;
    }

    // List: many links, repeating structure
    if stats.links > 30 {
        return PageType::List;
    }

    // Error page
    if text.contains("404") && text.contains("not found")
        || text.contains("500") && text.contains("error")
        || text.contains("403") && text.contains("forbidden")
    {
        return PageType::Error;
    }

    PageType::Unknown
}

// ─── State detection ───

fn detect_state(lines: &[String], url: &str) -> PageState {
    let text = lines.join("\n").to_lowercase();

    // Loading indicators
    if text.contains("loading") || text.contains("cargando")
        || text.contains("please wait") || text.contains("espere")
    {
        return PageState::Loading;
    }

    // Error indicators
    if text.contains("something went wrong") || text.contains("error occurred")
        || text.contains("try again") || text.contains("inténtalo de nuevo")
    {
        return PageState::Error("Page error detected".into());
    }

    // Logged in indicators
    if text.contains("sign out") || text.contains("log out") || text.contains("cerrar sesión")
        || text.contains("my account") || text.contains("mi cuenta")
        || text.contains("profile") || text.contains("perfil")
    {
        return PageState::LoggedIn;
    }

    // Logged out indicators
    if text.contains("sign in") || text.contains("log in") || text.contains("iniciar sesión")
        || text.contains("create account") || text.contains("registrarse")
    {
        return PageState::LoggedOut;
    }

    PageState::Ready
}

// ─── Action extraction ───

fn extract_actions(lines: &[String], page_type: &PageType) -> Vec<Action> {
    let mut actions = Vec::new();

    for line in lines {
        let trimmed = line.trim();

        // Textbox → type action
        if trimmed.starts_with("[textbox:") {
            let label = trimmed
                .strip_prefix("[textbox: ")
                .and_then(|s| s.split(']').next())
                .unwrap_or("")
                .to_string();

            let kind = if label.to_lowercase().contains("search")
                || label.to_lowercase().contains("buscar")
            {
                ActionKind::Search
            } else if label.to_lowercase().contains("prompt")
                || label.to_lowercase().contains("message")
                || label.to_lowercase().contains("mensaje")
            {
                ActionKind::SendMessage
            } else if label.to_lowercase().contains("password")
                || label.to_lowercase().contains("contraseña")
            {
                ActionKind::Login
            } else {
                ActionKind::Type
            };

            actions.push(Action {
                kind,
                label: if label.is_empty() {
                    "text input".into()
                } else {
                    label.clone()
                },
                target: label,
            });
        }

        // Button → click action
        if trimmed.starts_with("[button:") {
            let label = trimmed
                .strip_prefix("[button: ")
                .and_then(|s| s.strip_suffix(']'))
                .unwrap_or("")
                .to_string();

            if !label.is_empty() {
                let kind = if label.to_lowercase().contains("send")
                    || label.to_lowercase().contains("enviar")
                    || label.to_lowercase().contains("submit")
                {
                    ActionKind::Submit
                } else if label.to_lowercase().contains("search")
                    || label.to_lowercase().contains("buscar")
                {
                    ActionKind::Search
                } else if label.to_lowercase().contains("sign in")
                    || label.to_lowercase().contains("log in")
                    || label.to_lowercase().contains("iniciar")
                {
                    ActionKind::Login
                } else {
                    ActionKind::Click
                };

                actions.push(Action {
                    kind,
                    label: label.clone(),
                    target: label,
                });
            }
        }

        // Important links → navigate
        if trimmed.starts_with("[link:") && trimmed.contains("](") {
            let label = trimmed
                .strip_prefix("[link: ")
                .and_then(|s| s.split("](").next())
                .unwrap_or("")
                .to_string();

            if !label.is_empty() && label.len() > 2 {
                actions.push(Action {
                    kind: ActionKind::Navigate,
                    label: label.clone(),
                    target: label,
                });
            }
        }
    }

    // Always suggest scroll if page has content
    if lines.len() > 30 {
        actions.push(Action {
            kind: ActionKind::Scroll,
            label: "scroll down for more".into(),
            target: "down".into(),
        });
    }

    // Limit — too many actions is noise
    if actions.len() > 20 {
        // Keep non-navigate actions + first 10 navigate
        let mut kept = Vec::new();
        let mut nav_count = 0;
        for a in actions {
            match a.kind {
                ActionKind::Navigate => {
                    if nav_count < 10 {
                        kept.push(a);
                        nav_count += 1;
                    }
                }
                _ => kept.push(a),
            }
        }
        return kept;
    }

    actions
}

// ─── Content filtering ───

fn filter_content(lines: &[String], page_type: &PageType) -> Vec<String> {
    let mut content = Vec::new();
    let mut in_nav = false;
    let mut in_footer = false;

    for line in lines {
        let trimmed = line.trim();

        // Skip navigation sections
        if trimmed == "--- nav ---" {
            in_nav = true;
            continue;
        }
        if trimmed == "[contentinfo]" {
            in_footer = true;
            continue;
        }
        // End of nav/footer when we hit main content markers
        if trimmed == "[main]" || trimmed == "[article]" {
            in_nav = false;
            in_footer = false;
        }

        if in_nav || in_footer {
            continue;
        }

        // Skip structural markers that don't add info
        if trimmed == "[banner]"
            || trimmed == "[section]"
            || trimmed == "[main]"
            || trimmed == "[table]"
            || trimmed == "[list]"
            || trimmed == "[listitem]"
        {
            continue;
        }

        // Keep everything else (headings, paragraphs, links, buttons, text)
        if !trimmed.is_empty() {
            content.push(line.clone());
        }
    }

    content
}

// ─── Summary generation ───

fn summarize(
    page_type: &PageType,
    state: &PageState,
    stats: &semantic::PageStats,
    title: &str,
    url: &str,
) -> String {
    let type_desc = match page_type {
        PageType::Login => "Login page",
        PageType::Search => "Search page",
        PageType::SearchResults => "Search results",
        PageType::Article => "Article/content page",
        PageType::Chat => "Chat/conversation",
        PageType::Form => "Form",
        PageType::Dashboard => "Dashboard",
        PageType::List => "List/directory page",
        PageType::Profile => "Profile page",
        PageType::Error => "Error page",
        PageType::Unknown => "Page",
    };

    let state_desc = match state {
        PageState::LoggedIn => " (logged in)",
        PageState::LoggedOut => " (not logged in)",
        PageState::Loading => " (loading...)",
        PageState::Error(e) => return format!("{type_desc}: {title} — ERROR: {e}"),
        PageState::Ready => "",
    };

    format!(
        "{type_desc}{state_desc}: {title} | {url} | {}L {}B {}F",
        stats.links, stats.buttons, stats.forms,
    )
}

/// Format a PageView for display.
pub fn format_view(view: &PageView, max_lines: usize) -> String {
    let mut out = Vec::new();

    // Header
    out.push(format!("=== {} ===", view.summary));
    out.push(format!("Type: {} | State: {}", view.page_type, view.state));

    // Actions
    if !view.actions.is_empty() {
        out.push(String::new());
        out.push("Actions:".into());
        for (i, action) in view.actions.iter().enumerate() {
            if i >= 15 {
                out.push(format!("  ... ({} more)", view.actions.len() - 15));
                break;
            }
            out.push(format!("  {}: {} → '{}'", action.kind, action.label, action.target));
        }
    }

    // Content
    out.push(String::new());
    out.push("Content:".into());
    for (i, line) in view.content.iter().enumerate() {
        if i >= max_lines {
            out.push(format!("  ... ({} more lines)", view.content.len() - max_lines));
            break;
        }
        out.push(line.clone());
    }

    out.join("\n")
}
