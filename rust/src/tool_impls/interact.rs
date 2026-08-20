//! Interaction coverage beyond a plain click: keys, hover, drag, native
//! controls, shadow DOM and iframes, blocking dialogs, device emulation.
//!
//! Split into [`input`] (acting on an element: keys, hover, clicks, controls, drag) and
//! [`frames`] (the page's structure and environment: frames, dialogs, emulation).
pub mod frames;
pub mod input;

pub use frames::{DialogTool, EmulateTool, ListFramesTool, PierceTool};
pub use input::{ClickVariantTool, DragTool, HoverTool, PressTool, SetControlTool};
