I'd like to submit **NeoBrowser** for inclusion in this awesome list.

**Repository:** https://github.com/pitiflautico/neobrowser  
**License:** MIT  
**Suggested category:** Browser Automation

**What it is**  
NeoBrowser is an MCP server that drives a real Google Chrome instance via the Chrome DevTools Protocol (CDP). Unlike generic headless automations, it lets AI agents browse using the user's real logged-in sessions, while keeping the renderer sandbox on and origin-scoped credentials.

**Key points**
- Single ~6 MB Rust binary, no Node/Python runtime required.
- Verified actions: observe → act → verify, with explicit state diffing.
- Real-profile cookie import, encrypted vault, audit trace, and anti-detection that is genuine rather than spoofed.
- Ships with `server.json` for one-click install in Claude Desktop, Cursor, Cline, etc.

**Suggested README entry**

```markdown
- [NeoBrowser](https://github.com/pitiflautico/neobrowser) - MCP server that drives a real Google Chrome via CDP. Supports real sessions, verified actions, encrypted vaults and sandboxed browsing. [MIT]
```

Thanks for maintaining the list!
