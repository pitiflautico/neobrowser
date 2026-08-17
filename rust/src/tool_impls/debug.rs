//! Debugging and performance: console, network, Web Vitals, CPU and heap,
//! computed styles, source maps, HAR, evidence bundles.
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
