(function() {
    const btns = Array.from(document.querySelectorAll('button, [role="button"]'));
    const accept = btns.find(b => /accept all|aceptar todo|tout accepter|alle akzeptieren/i.test(b.innerText));
    if (accept) { accept.click(); return true; }
    return false;
})();
