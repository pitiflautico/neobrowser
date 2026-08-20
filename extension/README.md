# NeoBrowser Bridge

A minimal Chrome extension that lets a **local** NeoBrowser agent drive tabs you
explicitly share with it.

## Why this exists

The alternative ways to give an agent your session both have real costs:

- **Copying cookies** out of your Chrome profile creates a duplicate session with a
  different fingerprint. Providers reasonably treat that as a possible account
  takeover, which is why Google, LinkedIn and Microsoft identity cookies are excluded
  from that path entirely.
- **`--remote-debugging-port`** on your everyday browser exposes *every* tab, with no
  per-tab consent and no revocation, to anything that can reach the port.

The bridge is the middle path: your real browser, your real session, no clone — and
one tab at a time, with consent you grant and can withdraw.

## Security model

Read this before installing; the whole value is in these properties.

1. **Nothing is shared by default.** The agent sees no tab until you open the popup
   and click *Share* on one.
2. **Per-tab, not per-profile.** Sharing a tab grants access to that tab. Your other
   tabs, cookies and history stay out of reach.
3. **Revocable, and visibly so.** *Stop sharing* detaches immediately. Chrome also
   shows its own "being debugged" banner the whole time a tab is attached, so a
   shared tab can never be silently shared.
4. **Localhost only.** `host_permissions` is `http://127.0.0.1/*`. The extension
   cannot talk to a remote server even if asked to.
5. **No credential export.** The bridge forwards CDP commands for the shared tab. It
   never reads the cookie jar and never writes session material to disk.

What it does **not** protect against: an agent you have granted a tab can act as you
*in that tab*. Share the tab you want automated, not the one with your bank open.

## Install

Chrome does not allow an unpacked extension to be installed from a URL, so this is a
deliberate manual step:

1. `chrome://extensions` → enable **Developer mode**
2. **Load unpacked** → select this `extension/` directory
3. Start the agent side with `NEOBROWSER_BRIDGE_PORT=9333 neobrowser serve`
4. Click the extension icon, then **Share** on the tab you want driven

## Protocol

The extension polls `http://127.0.0.1:<port>/bridge` for queued CDP commands and posts
results back. Polling rather than a socket, because a service worker is terminated
when idle and a long-lived connection would be killed with it — a poll that fails
simply retries on the next wake.
