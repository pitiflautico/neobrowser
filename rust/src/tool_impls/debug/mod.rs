//! Debugging and performance: console, network, Web Vitals, CPU and heap,
//! computed styles, source maps, HAR, evidence bundles.
//!
//! Split out of a single 2700-line `tool_impls.rs`: at 67 tools that file had
//! stopped being navigable, and a reviewer could not tell which tools a change
//! touched.
//!
//! Split into [`runtime`] (what the page is doing now), [`perf`] (why it is slow and what it
//! holds) and [`sources`] (mapping back to source, and exporting a trace).

pub mod perf;
pub mod runtime;
pub mod sources;

pub use perf::{
    ComputedStyleTool, CpuProfileTool, HarExportTool, HarImportTool, HeapStatsTool, PerfTraceTool,
};
pub use runtime::{ConsoleLogsTool, DebugTool, MetricsTool, NetworkLogTool};
pub use sources::{SourceMapTool, TraceBundleTool};
