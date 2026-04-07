// Shadow DOM support
if (typeof Element !== 'undefined' && !Element.prototype.attachShadow) {
    Element.prototype.attachShadow = function(init) {
        const shadow = document.createDocumentFragment();
        shadow.host = this;
        shadow.mode = init?.mode || 'open';
        this._shadowRoot = shadow;
        // innerHTML support
        Object.defineProperty(shadow, 'innerHTML', {
            get() { return ''; },
            set(html) {
                const temp = document.createElement('div');
                temp.innerHTML = html;
                while (shadow.firstChild) shadow.removeChild(shadow.firstChild);
                while (temp.firstChild) shadow.appendChild(temp.firstChild);
            }
        });
        if (shadow.mode === 'open') {
            Object.defineProperty(this, 'shadowRoot', { value: shadow, configurable: true });
        }
        return shadow;
    };
}
