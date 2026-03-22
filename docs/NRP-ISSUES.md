# NRP v0.1 — Open Issues (Grok review)

## Must fix for v0.1
1. Session scoping: one session per connection (simplest)
2. document_epoch: only on full document navigation, not innerHTML
3. ActionResult struct (outcome kind + payload) instead of heavy enum
4. Interact.type returns value_after + selection
5. Wait.forStable: AND logic, define network_idle as 0 pending for N ms
6. SemanticTree: split properties vs html_metadata

## Fix for v0.2
7. SemanticTree.find: add tag, actions_contains, limit, match mode
8. Observe: global sequence_id, overflow=dropped_until_seq
9. Runtime.awaitPromise command
10. releaseObjectGroup
11. execution_context_id
12. Page.getInfo: page_state enum (Idle|Loading|Interactive|Settled)
13. SemanticTree: backend_node_id for debug correlation
14. Network: request_id scope, redirect chains, stage semantics

## Deferred to v0.3+
- Network interception (fulfill/fail/continue)
- WebSocket transport
- Binary body handling
- Advanced Runtime object model
- Multi-frame support
