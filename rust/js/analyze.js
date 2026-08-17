var forms = Array.from(document.querySelectorAll('form')).map(function(f, fi) {
        var fields = Array.from(f.querySelectorAll('input,select,textarea')).map(function(el) {
            var label = '';
            if (el.id) { var l = document.querySelector('label[for="'+el.id+'"]'); if(l) label = l.textContent.trim(); }
            if (!label) label = el.placeholder || el.name || el.type || '';
            return {tag: el.tagName.toLowerCase(), type: el.type||'', name: el.name||'', id: el.id||'', label: label, value: el.value||''};
        });
        return {index: fi, action: f.action||'', method: f.method||'get', fields: fields};
    });
    var buttons = Array.from(document.querySelectorAll('button,[role=button],input[type=submit],input[type=button]')).slice(0,20).map(function(b) {
        return {tag: b.tagName.toLowerCase(), text: (b.textContent||b.value||'').trim().slice(0,60), type: b.type||''};
    });
    var overlays = Array.from(document.querySelectorAll('*')).filter(function(e) {
        var s = window.getComputedStyle(e);
        return (s.position==='fixed'||s.position==='sticky') && parseInt(s.zIndex)>100 && e.offsetHeight>50;
    }).slice(0,5).map(function(e){ return {tag: e.tagName.toLowerCase(), id: e.id||'', cls: e.className.toString().slice(0,60)}; });
    var active = document.activeElement ? {tag: document.activeElement.tagName.toLowerCase(), id: document.activeElement.id||''} : null;
    return JSON.stringify({forms: forms, buttons: buttons, overlays: overlays, active_element: active});
