//! Credentials and session state. Everything here is `ActionClass::Auth`.
//!
//! Split out of a single 2700-line `tool_impls.rs`: at 67 tools that file had
//! stopped being navigable, and a reviewer could not tell which tools a change
//! touched.
//!
//! Split into [`credentials`] (signing in and persisting the result) and [`state`]
//! (inspecting and discarding session state).

pub mod credentials;
pub mod state;

pub use credentials::{LoginTool, RestoreCookiesTool, SaveCookiesTool};
pub use state::{
    LoginFlowTool, ProfileModeTool, RevokeSessionTool, SaveSessionTool, SessionInfoTool,
};
