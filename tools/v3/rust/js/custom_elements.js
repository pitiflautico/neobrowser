// NeoRender Custom Elements (Web Components) — registry polyfill.
// Required by GitHub, Twitch, and many modern sites using class extends HTMLElement.

if (!globalThis.customElements) {
    const registry = new Map();
    globalThis.customElements = {
        define(name, constructor, options) {
            registry.set(name, { constructor, options });
        },
        get(name) {
            const entry = registry.get(name);
            return entry ? entry.constructor : undefined;
        },
        whenDefined(name) {
            return registry.has(name) ? Promise.resolve(registry.get(name).constructor) : new Promise(() => {});
        },
        upgrade(node) { /* no-op */ },
    };
}
