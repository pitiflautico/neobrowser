return (function(force){
    const ACCEPT = ['accept all','accept','agree','i agree','got it','ok','allow all','allow',
        'aceptar','acepto','permitir','entendido','continuar','cerrar'];
    const CLOSE = ['close','dismiss','no thanks','skip','×','✕','✗','x'];
    const click = (el) => { try { el.scrollIntoView(); el.click(); return true; } catch(e){ return false; } };
    const findBtn = (texts, root) => {
        const btns = Array.from((root||document).querySelectorAll(
            'button,a,[role=button],[class*=accept],[class*=agree],[class*=consent],[class*=cookie]'));
        for (const t of texts) {
            const b = btns.find(x => { const s=(x.innerText||'').trim().toLowerCase(); return s===t || s.startsWith(t); });
            if (b) return b;
        }
        return null;
    };
    const overlays = Array.from(document.querySelectorAll('*')).filter(e => {
        const s = getComputedStyle(e);
        return (s.position==='fixed'||s.position==='sticky') && parseInt(s.zIndex||0) > 50
            && e.offsetHeight > 40 && e.offsetWidth > 100;
    });
    if (!overlays.length) return JSON.stringify({dismissed:false, reason:'no overlay detected'});
    for (const o of overlays) { const b = findBtn(ACCEPT, o); if (b && click(b))
        return JSON.stringify({dismissed:true, method:'accept', text:(b.innerText||'').trim().slice(0,30)}); }
    for (const o of overlays) { const b = findBtn(CLOSE, o); if (b && click(b))
        return JSON.stringify({dismissed:true, method:'close', text:(b.innerText||'').trim().slice(0,30)}); }
    if (force) {
        document.dispatchEvent(new KeyboardEvent('keydown', {key:'Escape', bubbles:true}));
        const bd = document.querySelector('[class*=backdrop],[class*=overlay],[class*=mask]');
        if (bd) click(bd);
        return JSON.stringify({dismissed:true, method:'escape_backdrop'});
    }
    return JSON.stringify({dismissed:false, reason:'no dismiss button found, try force=true'});
})(FORCE);
