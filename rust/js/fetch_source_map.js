// Fetch a source map from the page's own origin.
//
// Fetched from inside the page rather than by the agent, so it inherits the page's
// credentials and CSP — a source map behind auth is reachable exactly when the page itself
// can reach it, and no wider.

(async function(){
  var url = __URL__;
  var text;
  try { text = await (await fetch(url)).text(); }
  catch (e) { return JSON.stringify({ ok: false, error: 'could not fetch the script: ' + e }); }
  var m = /[#@]\s*sourceMappingURL=(\S+)/.exec(text.slice(-4000));
  if (!m) return JSON.stringify({ ok: false, error: 'the script declares no sourceMappingURL' });
  var mapUrl = new URL(m[1], url).href;
  var map;
  try { map = await (await fetch(mapUrl)).json(); }
  catch (e) { return JSON.stringify({ ok: false, error: 'could not fetch the source map: ' + e }); }
  return JSON.stringify({
    ok: true,
    map_url: mapUrl,
    sources: map.sources || [],
    source_root: map.sourceRoot || '',
    mappings: map.mappings || '',
    has_content: !!(map.sourcesContent && map.sourcesContent.length),
  });
})()
