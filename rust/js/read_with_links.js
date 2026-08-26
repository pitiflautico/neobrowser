(function() {
  const el = document.querySelector(__SELECTOR__);
  if (!el) {
    return '';
  }
  const walker = document.createTreeWalker(el, NodeFilter.SHOW_TEXT, null, false);
  const parts = [];
  let node;
  while (node = walker.nextNode()) {
    const text = node.textContent.trim();
    if (text) parts.push(text);
  }
  const links = Array.from(el.querySelectorAll('a[href]'))
    .map(a => `[${a.innerText.trim()}](${a.href})`)
    .filter(s => s.length > 4);
  return parts.join(' ') + (links.length ? '\n\nLinks:\n' + links.join('\n') : '');
})()
