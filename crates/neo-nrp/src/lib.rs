//! neo-nrp — NeoRender Protocol v0.1 implementation.
//!
//! Core protocol layer providing:
//! - **Types**: Wire format (NrpRequest, NrpResponse, Target, ActionResult, SemanticNode)
//! - **SemanticTree**: DOM-derived navigable tree with heuristic roles and names
//! - **Resolve**: Target resolution (text/role/label/css -> node_id)
//! - **Dispatcher**: JSON-RPC command routing to BrowserEngine
//!
//! See `docs/PDR-NRP.md` for the full protocol specification.

pub mod dispatcher;
pub mod resolve;
pub mod semantic_tree;
pub mod types;

pub use dispatcher::NrpDispatcher;
pub use resolve::resolve_target;
pub use semantic_tree::build_semantic_tree;
pub use types::*;
