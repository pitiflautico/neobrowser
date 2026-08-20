//! Credentials and session state. Everything here is `ActionClass::Auth`.
//!
//! Split into [`credentials`] (signing in and persisting the result) and [`state`]
//! (inspecting and discarding session state).
pub mod credentials;
pub mod state;

pub use credentials::{LoginTool, RestoreCookiesTool, SaveCookiesTool};
pub use state::{
    LoginFlowTool, ProfileModeTool, RevokeSessionTool, SaveSessionTool, SessionInfoTool,
};
