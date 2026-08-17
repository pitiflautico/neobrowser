//! Tools for inspecting and discarding session state.
//!
//! `revoke_session` is the one that has to be right: it once reported success while leaving
//! the sealed cookie file behind, because its target list named only the legacy path. A
//! revoke that reports success without revoking is worse than one that fails.

//! Credentials and session state. Everything here is `ActionClass::Auth`.
//!
//! Split out of a single 2700-line `tool_impls.rs`: at 67 tools that file had
//! stopped being navigable, and a reviewer could not tell which tools a change
//! touched.

use async_trait::async_trait;
use serde_json::{json, Map, Value};

use crate::page;
use crate::sessions;
use crate::tools::{ParamSpec, ParamType, Tool, ToolCtx, ToolError, ToolOutput, ToolSpec};

use super::super::{arg_f64, arg_str};

pub struct SaveSessionTool;

#[async_trait]
impl Tool for SaveSessionTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec { name: "save_session", description: "Full session save: cookies + localStorage → ~/.neobrowser/sessions/. Persists authenticated state across restarts.", params: vec![] }
    }
    async fn call(
        &self,
        ctx: &ToolCtx,
        _args: &Map<String, Value>,
    ) -> Result<ToolOutput, ToolError> {
        let tab = ctx.browser.tab().await?;
        Ok(ToolOutput::text(sessions::save_session(&tab).await?))
    }
}

pub struct SessionInfoTool;

#[async_trait]
impl Tool for SessionInfoTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "session_info",
            description:
                "Show session persistence state: last save time, cookie count, domains, file paths.",
            params: vec![],
        }
    }
    async fn call(
        &self,
        _ctx: &ToolCtx,
        _args: &Map<String, Value>,
    ) -> Result<ToolOutput, ToolError> {
        Ok(ToolOutput::text(sessions::session_info()))
    }
}

// --- login ---------------------------------------------------------------------

// --- revoke_session ------------------------------------------------------------

pub struct RevokeSessionTool;

#[async_trait]
impl Tool for RevokeSessionTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "revoke_session",
            description: "Destroy this profile's stored session material (cookie vault, localStorage, manifest). Overwrites before unlinking and then verifies the files are gone, reporting anything it could not remove instead of assuming success.",
            params: vec![],
        }
    }
    async fn call(
        &self,
        _ctx: &ToolCtx,
        _args: &Map<String, Value>,
    ) -> Result<ToolOutput, ToolError> {
        Ok(ToolOutput::text(sessions::revoke_session()?.to_string()))
    }
}

pub struct ProfileModeTool;

#[async_trait]
impl Tool for ProfileModeTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "profile_mode",
            description: "Report which of the three session modes is active — isolated (ephemeral, no credentials), agent (persistent NeoBrowser profile you log into once), or attached (driving a Chrome you already have open) — plus what each implies for your credentials.",
            params: vec![],
        }
    }
    async fn call(
        &self,
        ctx: &ToolCtx,
        _args: &Map<String, Value>,
    ) -> Result<ToolOutput, ToolError> {
        Ok(ToolOutput::text(
            crate::sessions::profile_mode_report(&ctx.browser).await,
        ))
    }
}

// --- B3 interaction coverage + E1 devtools -------------------------------------

// --- C4 composite actions + D1 profile modes -----------------------------------

pub struct LoginFlowTool;

#[async_trait]
impl Tool for LoginFlowTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "login_flow",
            description: "Navigate to a login page, fill credentials, submit, and verify the result — in one call instead of five. Reports `needs_human` when it lands on an MFA or captcha step rather than claiming success. The internal steps are returned, so nothing is hidden.",
            params: vec![
                ParamSpec::new("url", ParamType::String, "https:// login page URL").required(),
                ParamSpec::new("email", ParamType::String, "Username or email").required(),
                ParamSpec::new("password", ParamType::String, "Password (never logged, never traced)").required(),
                ParamSpec::new("budget_s", ParamType::Number, "Total seconds for the whole flow (default 30)"),
            ],
        }
    }
    async fn call(
        &self,
        ctx: &ToolCtx,
        args: &Map<String, Value>,
    ) -> Result<ToolOutput, ToolError> {
        use crate::action::{ActionStatus, Budget};

        let url = arg_str(args, "url")
            .ok_or_else(|| ToolError::Argument("login_flow: url must be a string".into()))?;
        let email = arg_str(args, "email")
            .ok_or_else(|| ToolError::Argument("login_flow: email must be a string".into()))?;
        let password = arg_str(args, "password")
            .ok_or_else(|| ToolError::Argument("login_flow: password must be a string".into()))?;
        let budget = Budget::from_secs(arg_f64(args, "budget_s", 30.0));

        let tab = ctx.browser.tab().await?;
        let before = crate::action::observe(&tab).await;
        let mut steps: Vec<Value> = Vec::new();

        // The composite is a sequence of the same verified primitives, so a failure
        // mid-flow reports which step and why — the reason for keeping the trace
        // visible rather than collapsing it into one boolean.
        let navigated = page::navigate_budgeted(&tab, url, &budget).await?;
        steps.push(json!({ "step": "navigate", "ok": navigated }));

        let login_result = sessions::login(&tab, url, email, password).await?;
        let parsed: Value = serde_json::from_str(&login_result).unwrap_or(Value::Null);
        steps.push(json!({ "step": "submit_credentials", "result": parsed.clone() }));

        // A wall after submitting is the interesting case: an MFA prompt or captcha
        // means the credentials were probably fine and a person has to continue. That
        // is `needs_human`, not failure and certainly not success.
        let wall = crate::walls::detect(&tab).await;
        let after = crate::action::observe(&tab).await;
        let logged_in = parsed.get("ok").and_then(Value::as_bool).unwrap_or(false);

        let status = match (&wall, logged_in) {
            (Some(w), _) => w.action_status(),
            (None, true) => ActionStatus::Succeeded,
            (None, false) => ActionStatus::Failed,
        };
        let mut result = crate::action::ActionResult::new("login_flow", status)
            .with_detail(format!("login flow against {url}"));
        result.before = before;
        result.after = after;
        result.changes = crate::action::detect_changes(&result.before, &result.after);
        if let Some(w) = wall {
            result = result.warn(format!("{}: {}", w.as_str(), w.hint()));
        }
        let mut out = result.to_json();
        out["steps"] = json!(steps);
        Ok(ToolOutput::text(out.to_string()))
    }
}
