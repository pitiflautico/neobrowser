return JSON.stringify((function(count){
    const out = [], seen = new Set();
    document.querySelectorAll('a.iusc').forEach(function(a){
        if (out.length >= count) return;
        let m = {}; try { m = JSON.parse(a.getAttribute('m') || '{}'); } catch(e) {}
        const img = m.murl || ''; if (!img || seen.has(img)) return; seen.add(img);
        let host = ''; try { host = new URL(img).hostname.replace(/^www\./,''); } catch(e) {}
        out.push({ image_url: img, source_url: m.purl || '', source_host: host, title: m.t || '', description: m.desc || '' });
    });
    return out;
})(COUNT))
