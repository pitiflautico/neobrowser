// NeoRender EventSource (SSE) polyfill — fetches text/event-stream and parses SSE events.
// Uses globalThis.fetch under the hood (which routes through Rust op_neorender_fetch).

if (!globalThis.EventSource) {
    globalThis.EventSource = class EventSource extends EventTarget {
        static CONNECTING = 0;
        static OPEN = 1;
        static CLOSED = 2;

        constructor(url, options) {
            super();
            this.url = url;
            this.readyState = EventSource.CONNECTING;
            this.withCredentials = options?.withCredentials || false;
            this.onopen = null;
            this.onmessage = null;
            this.onerror = null;
            // Fetch SSE stream
            this._connect();
        }

        async _connect() {
            try {
                const resp = await fetch(this.url, {
                    headers: { 'Accept': 'text/event-stream' }
                });
                this.readyState = EventSource.OPEN;
                this.dispatchEvent(new Event('open'));
                if (this.onopen) this.onopen(new Event('open'));

                const text = await resp.text();
                // Parse SSE format
                let eventType = 'message';
                let dataLines = [];
                for (const line of text.split('\n')) {
                    if (line.startsWith('event: ')) {
                        eventType = line.slice(7).trim();
                    } else if (line.startsWith('data: ')) {
                        dataLines.push(line.slice(6));
                    } else if (line === '' && dataLines.length > 0) {
                        // Empty line = dispatch event
                        const data = dataLines.join('\n');
                        const evt = new MessageEvent(eventType, { data });
                        this.dispatchEvent(evt);
                        if (eventType === 'message' && this.onmessage) this.onmessage(evt);
                        dataLines = [];
                        eventType = 'message';
                    }
                }
                // Flush remaining data
                if (dataLines.length > 0) {
                    const data = dataLines.join('\n');
                    const evt = new MessageEvent('message', { data });
                    this.dispatchEvent(evt);
                    if (this.onmessage) this.onmessage(evt);
                }
            } catch (e) {
                this.readyState = EventSource.CLOSED;
                this.dispatchEvent(new Event('error'));
                if (this.onerror) this.onerror(new Event('error'));
            }
        }

        close() { this.readyState = EventSource.CLOSED; }
    };
}
