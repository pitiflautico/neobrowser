//! Interaction coverage beyond a plain click: keys, hover, drag, native
//! controls, shadow DOM and iframes, blocking dialogs, device emulation.
//!
//! Split out of a single 2700-line `tool_impls.rs`: at 67 tools that file had
//! stopped being navigable, and a reviewer could not tell which tools a change
//! touched.
//!
//! Split into [`input`] (acting on an element: keys, hover, clicks, controls, drag) and
//! [`frames`] (the page's structure and environment: frames, dialogs, emulation).

pub mod frames;
pub mod input;

pub use frames::{DialogTool, EmulateTool, ListFramesTool, PierceTool};
pub use input::{ClickVariantTool, DragTool, HoverTool, PressTool, SetControlTool};
