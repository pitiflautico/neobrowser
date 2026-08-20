var els = document.querySelectorAll('a,button,input,select,textarea,[role=button],[role=link]');
    var forms = document.querySelectorAll('form');
    var overlays = Array.from(document.querySelectorAll('*')).filter(function(e) {
        var s = window.getComputedStyle(e);
        return (s.position === 'fixed' || s.position === 'sticky') &&
               parseInt(s.zIndex) > 100 && e.offsetHeight > 50;
    });
    return JSON.stringify({
        url: location.href, title: document.title,
        interactive: els.length, forms: forms.length,
        has_overlay: overlays.length > 0, overlay_count: overlays.length
    });
