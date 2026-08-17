//! The browsing tools — the ones an agent reaches for first.
//!
//! Grouped the same way `ops/` is, so the tool layer and the operation layer have the same
//! shape: [`nav`] arrives and looks, [`locate`] finds, [`act`] clicks and types,
//! [`introspect`] asks the page about itself, [`forms`] fills and submits, [`target`]
//! reaches an obstructed element, and [`harvest`] gets data out.

pub mod act;
pub mod forms;
pub mod harvest;
pub mod introspect;
pub mod locate;
pub mod nav;
pub mod target;

pub use act::{ClickTool, TypeTool};
pub use forms::{FillTool, FormFillTool, SubmitTool};
pub use harvest::{ExtractTableTool, ExtractTool, PaginateTool};
pub use introspect::{AnalyzeTool, JsTool, PageInfoTool};
pub use locate::{FindTool, ObserveTool};
pub use nav::{NavigateTool, ReadTool, ScreenshotTool, StatusTool};
pub use target::{DismissOverlayTool, FindAndClickTool, ScrollTool, WaitTool};
