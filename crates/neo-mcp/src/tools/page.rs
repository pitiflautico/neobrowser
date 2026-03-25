//! `page` tool — inspect current page state without navigating or interacting.
//!
//! Actions:
//! - (default) Get page info: URL, title, page_id, WOM summary
//! - `screenshot`: text-based visual representation (text/html/outline)
//! - `analyze`: deep page analysis (SEO, forms, links, accessibility, tech)

use serde_json::Value;

use crate::state::McpState;
use crate::McpError;

use super::ToolDef;

/// Tool definition for `tools/list`.
pub(crate) fn definition() -> ToolDef {
    ToolDef {
        name: "page",
        description: "Inspect current page: state, text-based screenshot, or deep analysis. \
                       Actions: 'info' (default), 'screenshot', 'analyze'.",
        schema: serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["info", "screenshot", "analyze"],
                    "description": "What to do: 'info' (page state), 'screenshot' (text visual), 'analyze' (deep analysis). Default: info",
                    "default": "info"
                },
                "full": {
                    "type": "boolean",
                    "description": "Include complete WOM document (for action=info, default false)",
                    "default": false
                },
                "format": {
                    "type": "string",
                    "enum": ["text", "html", "outline"],
                    "description": "Screenshot format (for action=screenshot): 'text' (spatial layout), 'html' (simplified HTML), 'outline' (heading structure). Default: text",
                    "default": "text"
                }
            }
        }),
    }
}

/// Execute the `page` tool.
pub fn call(args: Value, state: &mut McpState) -> Result<Value, McpError> {
    let action = args
        .get("action")
        .and_then(|v| v.as_str())
        .unwrap_or("info");

    match action {
        "info" => call_info(&args, state),
        "screenshot" => call_screenshot(&args, state),
        "analyze" => call_analyze(state),
        other => Err(McpError::InvalidParams(format!(
            "unknown page action: {other}"
        ))),
    }
}

/// Original page info handler.
fn call_info(args: &Value, state: &mut McpState) -> Result<Value, McpError> {
    let full = args
        .get("full")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let url = state.engine.current_url().unwrap_or_default();
    let page_state = state.engine.page_state();
    let session_state = state.engine.session_state();
    let page_id = state.engine.page_id();

    let wom = state.engine.extract()?;

    let mut response = serde_json::json!({
        "url": url,
        "title": wom.title,
        "page_id": page_id,
        "page_state": page_state,
        "session_state": session_state,
        "page_type": wom.page_type,
        "summary": wom.summary,
        "node_count": wom.nodes.len(),
    });

    if full {
        response["wom"] = serde_json::to_value(&wom)?;
    }

    Ok(response)
}

/// Text-based visual representation of the page.
fn call_screenshot(args: &Value, state: &mut McpState) -> Result<Value, McpError> {
    let format = args
        .get("format")
        .and_then(|v| v.as_str())
        .unwrap_or("text");

    let js = match format {
        "text" => SCREENSHOT_TEXT_JS,
        "html" => SCREENSHOT_HTML_JS,
        "outline" => SCREENSHOT_OUTLINE_JS,
        _ => SCREENSHOT_TEXT_JS,
    };

    let raw = state.engine.eval(js)?;
    let url = state.engine.current_url().unwrap_or_default();

    Ok(serde_json::json!({
        "ok": true,
        "action": "screenshot",
        "format": format,
        "url": url,
        "content": raw,
    }))
}

/// Deep page analysis via eval.
fn call_analyze(state: &mut McpState) -> Result<Value, McpError> {
    let raw = state.engine.eval(ANALYZE_JS)?;
    let url = state.engine.current_url().unwrap_or_default();

    let analysis: Value = serde_json::from_str(&raw).unwrap_or_else(|_| {
        serde_json::json!({ "error": "failed to parse analysis", "raw": raw })
    });

    Ok(serde_json::json!({
        "ok": true,
        "action": "analyze",
        "url": url,
        "analysis": analysis,
    }))
}

// ── JavaScript snippets ─────────────────────────────────────────────

/// Text-format screenshot: spatial layout using landmarks.
const SCREENSHOT_TEXT_JS: &str = r#"(function(){
  var W=72;
  var SEP=Array(W+1).join('\u2500');
  var out=[];

  function trunc(s,max){ s=(s||'').trim(); return s.length>max?s.substring(0,max-1)+'\u2026':s; }
  function pad(s,w){ s=s||''; while(s.length<w) s+=' '; return s; }

  function renderEl(el,indent){
    var tag=(el.tagName||'').toLowerCase();
    var pre=Array(indent+1).join('  ');

    // Skip invisible
    if(tag==='script'||tag==='style'||tag==='noscript'||tag==='meta'||tag==='link') return;

    // Landmarks
    if(tag==='header'||el.getAttribute('role')==='banner'){
      out.push(pre+'[HEADER] '+trunc(el.textContent,W-indent*2-10));
      out.push(SEP);
      return;
    }
    if(tag==='footer'||el.getAttribute('role')==='contentinfo'){
      out.push(SEP);
      out.push(pre+'[FOOTER] '+trunc(el.textContent,W-indent*2-10));
      return;
    }
    if(tag==='nav'||el.getAttribute('role')==='navigation'){
      var links=el.querySelectorAll('a');
      var items=[];
      for(var i=0;i<links.length&&i<10;i++) items.push(trunc(links[i].textContent,20));
      out.push(pre+'[NAV] '+items.join(' | '));
      return;
    }
    if(tag==='main'||el.getAttribute('role')==='main'){
      out.push(pre+'[MAIN]');
      for(var c=el.firstElementChild;c;c=c.nextElementSibling) renderEl(c,indent+1);
      return;
    }
    if(tag==='aside'||el.getAttribute('role')==='complementary'){
      out.push(pre+'[ASIDE] '+trunc(el.textContent,W-indent*2-10));
      return;
    }

    // Headings
    if(tag.match(/^h[1-6]$/)){
      out.push(pre+'['+tag.toUpperCase()+'] '+trunc(el.textContent,W-indent*2-6));
      return;
    }

    // Forms
    if(tag==='form'){
      out.push(pre+'[FORM]'+(el.getAttribute('action')?' action='+el.getAttribute('action'):''));
      for(var c=el.firstElementChild;c;c=c.nextElementSibling) renderEl(c,indent+1);
      return;
    }

    // Inputs
    if(tag==='input'){
      var t=el.type||'text';
      var ph=el.getAttribute('placeholder')||el.getAttribute('name')||t;
      if(t==='submit'||t==='button') out.push(pre+'['+trunc(el.value||ph,30)+']');
      else if(t==='checkbox'||t==='radio') out.push(pre+(el.checked?'[x]':'[ ]')+' '+trunc(ph,30));
      else if(t==='hidden') return;
      else out.push(pre+trunc(ph,15)+': ['+pad(trunc(el.value||'',20),12)+']');
      return;
    }
    if(tag==='textarea'){
      var ph2=el.getAttribute('placeholder')||el.getAttribute('name')||'textarea';
      out.push(pre+trunc(ph2,15)+': ['+pad(trunc(el.value||'',40),20)+']');
      return;
    }
    if(tag==='select'){
      var sel=el.options&&el.selectedIndex>=0?el.options[el.selectedIndex].text:'';
      var nm=el.getAttribute('name')||'select';
      out.push(pre+trunc(nm,15)+': ['+trunc(sel||'--',20)+' v]');
      return;
    }

    // Buttons
    if(tag==='button'||el.getAttribute('role')==='button'){
      out.push(pre+'['+trunc(el.textContent,30)+']');
      return;
    }

    // Links
    if(tag==='a'){
      out.push(pre+'\u2192 '+trunc(el.textContent,W-indent*2-4));
      return;
    }

    // Images
    if(tag==='img'){
      out.push(pre+'[IMG: '+trunc(el.getAttribute('alt')||el.getAttribute('src')||'image',40)+']');
      return;
    }

    // Paragraphs
    if(tag==='p'){
      var txt=trunc(el.textContent,W-indent*2);
      if(txt) out.push(pre+txt);
      return;
    }

    // Lists
    if(tag==='ul'||tag==='ol'){
      for(var li=el.firstElementChild;li;li=li.nextElementSibling){
        if((li.tagName||'').toLowerCase()==='li'){
          out.push(pre+'  \u2022 '+trunc(li.textContent,W-indent*2-4));
        }
      }
      return;
    }

    // Tables
    if(tag==='table'){
      out.push(pre+'[TABLE]');
      var rows=el.querySelectorAll('tr');
      for(var r=0;r<rows.length&&r<10;r++){
        var cells=rows[r].querySelectorAll('td,th');
        var row=[];
        for(var c=0;c<cells.length&&c<8;c++) row.push(trunc(cells[c].textContent,15));
        out.push(pre+'  | '+row.join(' | ')+' |');
      }
      if(rows.length>10) out.push(pre+'  ... ('+rows.length+' rows total)');
      return;
    }

    // Dialog
    if(tag==='dialog'||el.getAttribute('role')==='dialog'){
      out.push(pre+'[DIALOG] '+trunc(el.textContent,W-indent*2-10));
      return;
    }

    // Generic containers: recurse
    if(tag==='div'||tag==='section'||tag==='article'||tag==='fieldset'||tag==='details'){
      // Only recurse if it has meaningful structure
      var children=el.children;
      if(children.length>0){
        if(tag==='article') out.push(pre+'[ARTICLE]');
        if(tag==='section') out.push(pre+'[SECTION]');
        for(var i=0;i<children.length;i++) renderEl(children[i],indent+(tag==='div'?0:1));
      } else {
        var t=trunc(el.textContent,W-indent*2);
        if(t) out.push(pre+t);
      }
      return;
    }
  }

  // Start from body
  var body=document.body;
  if(!body) return 'No page loaded';
  var title=document.title||'';
  if(title) out.push('=== '+title+' ===');
  out.push('');
  for(var c=body.firstElementChild;c;c=c.nextElementSibling) renderEl(c,0);

  return out.join('\n');
})()"#;

/// HTML-format screenshot: simplified cleaned HTML.
const SCREENSHOT_HTML_JS: &str = r#"(function(){
  var MAX=5000;
  function clean(el){
    var tag=(el.tagName||'').toLowerCase();
    if(tag==='script'||tag==='style'||tag==='noscript'||tag==='svg'||tag==='path') return '';
    var out='<'+tag;
    // Keep only semantic attributes
    var keep=['id','class','href','src','alt','type','name','placeholder','role','aria-label','action','method','value','for'];
    for(var i=0;i<keep.length;i++){
      var v=el.getAttribute(keep[i]);
      if(v){ v=v.substring(0,100); out+=' '+keep[i]+'="'+v.replace(/"/g,'&quot;')+'"'; }
    }
    // Self-closing
    if(tag==='img'||tag==='input'||tag==='br'||tag==='hr'||tag==='meta'||tag==='link') return out+'/>';
    out+='>';
    // Children
    if(el.children.length===0){
      var txt=(el.textContent||'').trim();
      if(txt) out+=txt.substring(0,200);
    } else {
      for(var c=el.firstElementChild;c;c=c.nextElementSibling){
        var child=clean(c);
        if(out.length+child.length>MAX){ out+='<!-- truncated -->'; break; }
        out+=child;
      }
    }
    out+='</'+tag+'>';
    return out;
  }
  var body=document.body;
  if(!body) return '<html><body>No page loaded</body></html>';
  var html='<!DOCTYPE html><html><head><title>'+(document.title||'')+'</title></head>';
  html+=clean(body);
  html+='</html>';
  if(html.length>MAX) html=html.substring(0,MAX)+'<!-- truncated at '+MAX+' chars -->';
  return html;
})()"#;

/// Outline-format screenshot: heading hierarchy like a table of contents.
const SCREENSHOT_OUTLINE_JS: &str = r#"(function(){
  var headings=document.querySelectorAll('h1,h2,h3,h4,h5,h6,[role="heading"]');
  var out=[];
  for(var i=0;i<headings.length;i++){
    var el=headings[i];
    var tag=(el.tagName||'').toLowerCase();
    var level=parseInt(tag.charAt(1))||parseInt(el.getAttribute('aria-level'))||1;
    var indent=Array(level).join('  ');
    var text=(el.textContent||'').trim().substring(0,100);
    if(text) out.push(indent+tag.toUpperCase()+': '+text);
  }
  if(out.length===0) return 'No headings found on page.';
  return out.join('\n');
})()"#;

/// Deep page analysis: SEO, forms, links, accessibility, tech detection, performance.
const ANALYZE_JS: &str = r#"(function(){
  var r={};

  // Page type detection
  var pt='unknown';
  var body=document.body;
  if(!body) return JSON.stringify({error:'no page loaded'});
  var html=(body.innerHTML||'').toLowerCase();
  var hasLogin=document.querySelector('input[type="password"]');
  var hasSearch=document.querySelector('input[type="search"]')||document.querySelector('[role="search"]');
  var hasResults=document.querySelectorAll('.search-result,.result,article').length>3;
  var hasProduct=document.querySelector('[itemtype*="Product"]')||document.querySelector('.product,.price,[data-price]');
  var hasCart=html.indexOf('cart')>=0||html.indexOf('checkout')>=0||document.querySelector('[class*="cart"],[class*="checkout"]');
  var hasError=document.querySelector('.error,.error-page,[class*="404"],[class*="error"]')||document.title.match(/404|error|not found/i);
  var hasArticle=document.querySelector('article')||document.querySelector('[itemtype*="Article"]');

  if(hasLogin) pt='login_form';
  else if(hasSearch&&hasResults) pt='search_results';
  else if(hasProduct) pt='product_page';
  else if(hasCart) pt='checkout';
  else if(hasError) pt='error_page';
  else if(hasArticle) pt='article';
  else if(document.querySelectorAll('a').length>20&&document.querySelectorAll('h1,h2,h3').length<=2) pt='landing_page';
  else pt='general';
  r.page_type=pt;

  // SEO
  var seo={};
  seo.title=document.title||'';
  var metaDesc=document.querySelector('meta[name="description"]');
  seo.meta_description=metaDesc?metaDesc.getAttribute('content')||'':'';
  var canonical=document.querySelector('link[rel="canonical"]');
  seo.canonical=canonical?canonical.getAttribute('href')||'':'';
  var ogImg=document.querySelector('meta[property="og:image"]');
  seo.og_image=ogImg?ogImg.getAttribute('content')||'':'';
  var h1s=document.querySelectorAll('h1');
  seo.h1=[];
  for(var i=0;i<h1s.length;i++) seo.h1.push((h1s[i].textContent||'').trim().substring(0,200));
  seo.h1_count=h1s.length;
  r.seo=seo;

  // Forms analysis
  var forms=document.querySelectorAll('form');
  var formsData=[];
  for(var f=0;f<forms.length&&f<10;f++){
    var form=forms[f];
    var fields=[];
    var inputs=form.querySelectorAll('input,textarea,select');
    for(var j=0;j<inputs.length&&j<20;j++){
      var inp=inputs[j];
      var tag=(inp.tagName||'').toLowerCase();
      var type=tag==='input'?(inp.type||'text'):tag;
      if(type==='hidden') continue;
      fields.push({
        type:type,
        name:inp.getAttribute('name')||'',
        placeholder:inp.getAttribute('placeholder')||'',
        required:inp.required||false,
        label:(inp.labels&&inp.labels[0]?(inp.labels[0].textContent||'').trim():'').substring(0,60)
      });
    }
    formsData.push({
      action:form.getAttribute('action')||'',
      method:(form.getAttribute('method')||'GET').toUpperCase(),
      id:form.id||'',
      field_count:inputs.length,
      fields:fields
    });
  }
  r.forms={count:forms.length,forms:formsData};

  // Links analysis
  var links=document.querySelectorAll('a[href]');
  var internal=0,external=0,navLinks=[],contentLinks=[];
  var host=location.hostname;
  for(var i=0;i<links.length;i++){
    var href=links[i].getAttribute('href')||'';
    var text=(links[i].textContent||'').trim().substring(0,60);
    var isExt=href.match(/^https?:\/\//)&&href.indexOf(host)<0;
    if(isExt) external++; else internal++;
    var inNav=false;
    for(var p=links[i].parentElement;p;p=p.parentElement){
      if((p.tagName||'').toLowerCase()==='nav'||p.getAttribute('role')==='navigation'){ inNav=true; break; }
    }
    if(inNav&&navLinks.length<20) navLinks.push({text:text,href:href.substring(0,200)});
    else if(contentLinks.length<20) contentLinks.push({text:text,href:href.substring(0,200)});
  }
  r.links={total:links.length,internal:internal,external:external,nav_links:navLinks,content_links:contentLinks};

  // Accessibility
  var a11y={issues:[]};
  var imgs=document.querySelectorAll('img');
  var noAlt=0;
  for(var i=0;i<imgs.length;i++){ if(!imgs[i].getAttribute('alt')&&!imgs[i].getAttribute('aria-label')) noAlt++; }
  if(noAlt>0) a11y.issues.push({type:'img_no_alt',count:noAlt,severity:'error'});

  var inputsAll=document.querySelectorAll('input:not([type="hidden"]),textarea,select');
  var noLabel=0;
  for(var i=0;i<inputsAll.length;i++){
    var inp=inputsAll[i];
    var hasLabel=inp.getAttribute('aria-label')||inp.getAttribute('aria-labelledby')||inp.getAttribute('placeholder')||(inp.labels&&inp.labels.length>0)||inp.getAttribute('title');
    if(!hasLabel) noLabel++;
  }
  if(noLabel>0) a11y.issues.push({type:'input_no_label',count:noLabel,severity:'error'});

  // Check heading hierarchy
  var prevLevel=0;
  var headingSkips=0;
  var allH=document.querySelectorAll('h1,h2,h3,h4,h5,h6');
  for(var i=0;i<allH.length;i++){
    var level=parseInt((allH[i].tagName||'H1').charAt(1));
    if(prevLevel>0&&level>prevLevel+1) headingSkips++;
    prevLevel=level;
  }
  if(headingSkips>0) a11y.issues.push({type:'heading_skip',count:headingSkips,severity:'warning'});

  a11y.score=Math.max(0,100-noAlt*10-noLabel*10-headingSkips*5);
  a11y.images_total=imgs.length;
  a11y.inputs_total=inputsAll.length;
  r.accessibility=a11y;

  // Tech detection
  var tech=[];
  if(window.__NEXT_DATA__) tech.push('Next.js');
  if(window.__NUXT__) tech.push('Nuxt.js');
  if(window.__GATSBY) tech.push('Gatsby');
  if(window.React||document.querySelector('[data-reactroot]')) tech.push('React');
  if(window.Vue||document.querySelector('[data-v-]')||document.querySelector('[class*="v-"]')) tech.push('Vue');
  if(window.angular||document.querySelector('[ng-app]')||document.querySelector('[ng-controller]')) tech.push('Angular');
  if(window.jQuery||window.$&&window.$.fn) tech.push('jQuery');
  if(document.querySelector('meta[name="generator"][content*="WordPress"]')||window.wp) tech.push('WordPress');
  if(window.Shopify) tech.push('Shopify');
  if(window.__remixContext) tech.push('Remix');
  if(window.__SVELTE__||document.querySelector('[class^="svelte-"]')) tech.push('Svelte');
  if(document.querySelector('[data-turbo]')||window.Turbo) tech.push('Turbo/Hotwire');
  if(window.htmx) tech.push('htmx');
  if(window.Alpine) tech.push('Alpine.js');
  r.tech=tech;

  // Performance hints
  var perf={};
  perf.scripts=document.querySelectorAll('script').length;
  perf.stylesheets=document.querySelectorAll('link[rel="stylesheet"],style').length;
  perf.images=imgs.length;
  perf.iframes=document.querySelectorAll('iframe').length;
  perf.dom_nodes=document.querySelectorAll('*').length;
  perf.dom_depth=0;
  var deepest=body;
  function maxDepth(el,d){ if(d>perf.dom_depth) perf.dom_depth=d; for(var c=el.firstElementChild;c&&d<50;c=c.nextElementSibling) maxDepth(c,d+1); }
  maxDepth(body,0);
  r.performance=perf;

  return JSON.stringify(r);
})()"#;
