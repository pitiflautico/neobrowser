// Popup: the consent surface. Every share starts here, with a click.

const send = (msg) => new Promise((r) => chrome.runtime.sendMessage(msg, r));

async function render() {
  const status = await send({ type: 'status' });
  const shared = new Set(status.shared || []);
  document.getElementById('port').value = status.port;

  const tabs = await chrome.tabs.query({ currentWindow: true });
  const host = document.getElementById('tabs');
  host.textContent = '';

  for (const tab of tabs) {
    const row = document.createElement('div');
    row.className = 'tab';

    const title = document.createElement('span');
    title.className = 'title' + (shared.has(tab.id) ? ' shared' : '');
    // textContent, never innerHTML: a page controls its own title, and injecting it as
    // markup would let any open tab run script in this popup.
    title.textContent = tab.title || tab.url || '(untitled)';
    title.title = tab.url || '';

    const button = document.createElement('button');
    button.textContent = shared.has(tab.id) ? 'Stop sharing' : 'Share';
    button.addEventListener('click', async () => {
      button.disabled = true;
      await send({ type: shared.has(tab.id) ? 'unshare' : 'share', tabId: tab.id });
      render();
    });

    row.append(title, button);
    host.append(row);
  }
}

document.getElementById('port').addEventListener('change', async (e) => {
  await send({ type: 'set_port', port: e.target.value });
});

render();
