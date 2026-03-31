# Session & Security

## What NeoBrowser copies from your Chrome profile

On startup, NeoBrowser reads (read-only) from your real Chrome profile and copies to an isolated Ghost Chrome instance:

| Data | Source | Method |
|---|---|---|
| Cookies | `Cookies` SQLite DB | WAL-safe read-only copy |
| localStorage | `Local Storage/leveldb/` | File copy |
| IndexedDB | `IndexedDB/` | File copy |
| SessionStorage | `Session Storage/` | File copy |

### What is NOT copied

- Passwords (never accessed)
- Autofill data (never accessed)
- Browsing history (never accessed)
- Bookmarks (never accessed)
- Extensions (never loaded)
- Saved payment methods (never accessed)

## Excluded domains

Google domains are excluded from cookie sync by default to prevent Google from detecting a duplicate session and logging out your real browser:

```
.google.com
.google.es
.googleapis.com
.gstatic.com
.youtube.com
.accounts.google.com
.gmail.com
```

## Which Chrome profile

NeoBrowser reads from the profile set in the `PROFILE` constant (default: `Profile 24`). On startup it logs:

```
[neo] Session sync from Profile 24: 5332 cookies kept, 398 Google excluded
```

To change: edit `PROFILE` in `neo-browser.py` or set the environment variable (future feature).

## Ghost Chrome isolation

Each MCP process gets its own Chrome instance:

- Profile directory: `~/.neorender/ghost-{pid}/`
- Separate from your real Chrome — no shared state after initial sync
- Cleaned up on exit (process kill + directory removal)
- PID tracked in `~/.neorender/pids.json` for cleanup

## Network behavior

- NeoBrowser makes HTTP requests from your machine's IP
- Chrome headless connects to websites as a normal browser
- No data is sent to NeoBrowser servers (there are none)
- All processing is local

## Recommendations

- Review which profile `PROFILE` points to
- Add sensitive domains to `EXCLUDED_DOMAINS` if needed
- The ghost profile is ephemeral — deleted when the MCP process exits
- Run `neo-browser.py doctor` to verify your setup
