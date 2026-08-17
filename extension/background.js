// NeoBrowser Bridge — service worker.
//
// Design constraints that shaped this:
//
// * A Manifest V3 service worker is terminated whenever it is idle, so no state may
//   live in a module variable and no long-lived socket may be relied upon. Shared-tab
//   state goes in chrome.storage.session; the transport is a poll.
// * Only tabs the user has explicitly shared are ever attached. The set of shared tab
//   ids is the single authority, checked on every command, so a stale or forged tab id
//   from the agent side cannot reach a tab the user did not offer.
// * The debugger is attached per shared tab. Chrome shows its own banner for the whole
//   duration, which means sharing cannot be made invisible to the user.

const DEFAULT_PORT = 9333;
const POLL_MS = 500;

async function getPort() {
  const { port } = await chrome.storage.session.get('port');
  return port || DEFAULT_PORT;
}

async function getShared() {
  const { shared } = await chrome.storage.session.get('shared');
  return Array.isArray(shared) ? shared : [];
}

async function setShared(ids) {
  await chrome.storage.session.set({ shared: ids });
  // The badge is a second, always-visible indicator of how many tabs are exposed.
  await chrome.action.setBadgeText({ text: ids.length ? String(ids.length) : '' });
  await chrome.action.setBadgeBackgroundColor({ color: '#d93025' });
}

async function share(tabId) {
  const ids = await getShared();
  if (ids.includes(tabId)) return { ok: true, already: true };
  try {
    await chrome.debugger.attach({ tabId }, '1.3');
  } catch (e) {
    return { ok: false, error: String(e.message || e) };
  }
  ids.push(tabId);
  await setShared(ids);
  return { ok: true };
}

async function unshare(tabId) {
  const ids = (await getShared()).filter((id) => id !== tabId);
  try {
    await chrome.debugger.detach({ tabId });
  } catch (e) {
    // Already detached (tab closed, or Chrome dropped it). Removing it from the
    // shared set is still the correct outcome, so this is not an error.
  }
  await setShared(ids);
  return { ok: true };
}

// A tab that goes away must leave the shared set, or the count would drift upward and
// stop meaning anything.
chrome.tabs.onRemoved.addListener(async (tabId) => {
  const ids = await getShared();
  if (ids.includes(tabId)) await setShared(ids.filter((id) => id !== tabId));
});

// If the user detaches via Chrome's own banner, honour it immediately rather than
// leaving the extension believing it still has access.
chrome.debugger.onDetach.addListener(async (source) => {
  if (!source.tabId) return;
  const ids = await getShared();
  if (ids.includes(source.tabId)) await setShared(ids.filter((id) => id !== source.tabId));
});

/// Execute one queued command, refusing anything outside the shared set.
async function execute(cmd) {
  const ids = await getShared();
  const tabId = cmd.tabId;
  if (!ids.includes(tabId)) {
    // The authorization check. The agent proposes; this decides.
    return {
      id: cmd.id,
      error: `tab ${tabId} is not shared. The user must share it from the NeoBrowser Bridge popup.`,
    };
  }
  try {
    const result = await chrome.debugger.sendCommand(
      { tabId },
      cmd.method,
      cmd.params || {}
    );
    return { id: cmd.id, result };
  } catch (e) {
    return { id: cmd.id, error: String(e.message || e) };
  }
}

/// One poll cycle: fetch queued commands, run them, post results back.
async function poll() {
  const port = await getPort();
  const ids = await getShared();
  let batch;
  try {
    const res = await fetch(`http://127.0.0.1:${port}/bridge`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ shared_tabs: ids }),
    });
    if (!res.ok) return;
    batch = await res.json();
  } catch (e) {
    // The agent is not running. Silence is correct: the next alarm retries, and
    // logging every failed poll would fill the console while idle.
    return;
  }
  const commands = Array.isArray(batch && batch.commands) ? batch.commands : [];
  if (!commands.length) return;

  const results = [];
  for (const cmd of commands) results.push(await execute(cmd));
  try {
    await fetch(`http://127.0.0.1:${port}/bridge/results`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ results }),
    });
  } catch (e) {
    // Results lost; the agent times out the command rather than hanging, so dropping
    // them is recoverable.
  }
}

// chrome.alarms has a one-minute floor, which is far too slow for interactive driving.
// A self-rescheduling timeout inside a kept-alive worker is the supported way to poll
// faster; the worker is only kept alive while tabs are actually shared.
async function loop() {
  await poll();
  const ids = await getShared();
  if (ids.length) setTimeout(loop, POLL_MS);
}

chrome.runtime.onMessage.addListener((msg, _sender, respond) => {
  (async () => {
    if (msg.type === 'share') {
      const r = await share(msg.tabId);
      loop();
      respond(r);
    } else if (msg.type === 'unshare') {
      respond(await unshare(msg.tabId));
    } else if (msg.type === 'status') {
      respond({ shared: await getShared(), port: await getPort() });
    } else if (msg.type === 'set_port') {
      await chrome.storage.session.set({ port: Number(msg.port) || DEFAULT_PORT });
      respond({ ok: true });
    } else {
      respond({ ok: false, error: 'unknown message type' });
    }
  })();
  return true; // async respond
});
