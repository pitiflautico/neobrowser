// NeoRender WebSocket polyfill — functional stub so sites that check for WebSocket don't break.
// Auto-transitions to OPEN state. send() is a no-op (future: forward to Rust for real connections).

if (!globalThis.WebSocket || globalThis.WebSocket.toString().includes('stub')) {
    globalThis.WebSocket = class WebSocket extends EventTarget {
        static CONNECTING = 0;
        static OPEN = 1;
        static CLOSING = 2;
        static CLOSED = 3;

        constructor(url, protocols) {
            super();
            this.url = url;
            this.protocol = '';
            this.readyState = WebSocket.CONNECTING;
            this.bufferedAmount = 0;
            this.extensions = '';
            this.binaryType = 'blob';
            this.onopen = null;
            this.onmessage = null;
            this.onerror = null;
            this.onclose = null;

            // Auto-connect simulation (transitions to OPEN)
            Promise.resolve().then(() => {
                this.readyState = WebSocket.OPEN;
                const evt = new Event('open');
                this.dispatchEvent(evt);
                if (this.onopen) this.onopen(evt);
            });
        }

        send(data) {
            if (this.readyState !== WebSocket.OPEN) throw new Error('WebSocket not open');
            // In future: forward to Rust via op for real WebSocket
        }

        close(code, reason) {
            this.readyState = WebSocket.CLOSED;
            const evt = new CloseEvent('close', { code: code || 1000, reason: reason || '', wasClean: true });
            this.dispatchEvent(evt);
            if (this.onclose) this.onclose(evt);
        }
    };

    globalThis.CloseEvent = globalThis.CloseEvent || class CloseEvent extends Event {
        constructor(type, init = {}) {
            super(type, init);
            this.code = init.code || 1000;
            this.reason = init.reason || '';
            this.wasClean = init.wasClean !== false;
        }
    };
}
