//! Tools for signing in: the interactive login, and persisting the cookies it produced.
//!
//! `login` reports needing a human rather than guessing when it cannot tell whether the sign-in
//! finished, because a page still showing the form might be mid-validation, mid-2FA, or simply
//! rejected — and reporting success on any of those sends an agent onward as if authenticated.

//! Credentials and session state. Everything here is `ActionClass::Auth`.
//!
//! Split out of a single 2700-line `tool_impls.rs`: at 67 tools that file had
//! stopped being navigable, and a reviewer could not tell which tools a change
//! touched.

use async_trait::async_trait;
use serde_json::{Map, Value};

use crate::sessions;
use crate::tools::{ParamSpec, ParamType, Tool, ToolCtx, ToolError, ToolOutput, ToolSpec};

use super::super::arg_str;

// --- login ---------------------------------------------------------------------

pub struct LoginTool;

#[async_trait]
impl Tool for LoginTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "login",
            description: "Navigate an https login page, fill email + password, submit, and report honest success (a lingering password field means it failed).",
            params: vec![
                ParamSpec::new("url", ParamType::String, "Login page URL (must be https)").required(),
                ParamSpec::new("email", ParamType::String, "Email or username").required(),
                ParamSpec::new("password", ParamType::String, "Password").required(),
            ],
        }
    }
    async fn call(
        &self,
        ctx: &ToolCtx,
        args: &Map<String, Value>,
    ) -> Result<ToolOutput, ToolError> {
        let url = arg_str(args, "url")
            .ok_or_else(|| ToolError::Argument("login: url must be a string".into()))?;
        let email = arg_str(args, "email")
            .ok_or_else(|| ToolError::Argument("login: email must be a string".into()))?;
        let password = arg_str(args, "password")
            .ok_or_else(|| ToolError::Argument("login: password must be a string".into()))?;
        let tab = ctx.browser.tab().await?;
        Ok(ToolOutput::text(
            sessions::login(&tab, url, email, password).await?,
        ))
    }
}

// --- browse --------------------------------------------------------------------

// --- save_cookies / restore_cookies -------------------------------------------

pub struct SaveCookiesTool;

#[async_trait]
impl Tool for SaveCookiesTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec { name: "save_cookies", description: "Save the current session's cookies to ~/.neobrowser/cookies/{profile}.json (0600 perms).", params: vec![] }
    }
    async fn call(
        &self,
        ctx: &ToolCtx,
        _args: &Map<String, Value>,
    ) -> Result<ToolOutput, ToolError> {
        let tab = ctx.browser.tab().await?;
        let n = sessions::save_cookies(&tab).await?;
        Ok(ToolOutput::text(format!("Saved {n} cookies")))
    }
}

pub struct RestoreCookiesTool;

#[async_trait]
impl Tool for RestoreCookiesTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "restore_cookies",
            description:
                "Inject saved cookies from disk into the current tab. Returns count restored.",
            params: vec![],
        }
    }
    async fn call(
        &self,
        ctx: &ToolCtx,
        _args: &Map<String, Value>,
    ) -> Result<ToolOutput, ToolError> {
        let tab = ctx.browser.tab().await?;
        let n = sessions::restore_cookies(&tab).await?;
        Ok(ToolOutput::text(format!("Restored {n} cookies")))
    }
}

// --- save_session / session_info ----------------------------------------------

// --- save_session / session_info ----------------------------------------------
