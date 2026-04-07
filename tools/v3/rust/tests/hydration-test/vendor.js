// Vendor bundle — TLA + Promise.allSettled + getAll pattern

async function initPolyfills() {
  const results = await Promise.allSettled([
    Promise.resolve("p1"),
    Promise.resolve("p2"),
    new Promise(r => setTimeout(r, 10, "p3")),
  ]);
  return results;
}

// TLA
const polyfillResults = await initPolyfills();

// Object with availableHints but no getAll (React Router Early Hints)
const response = { availableHints: ["Link"], status: 200 };
const hints = ["Link", "X-Link"].flatMap(h => response.getAll(h)).flatMap(v => v.split(","));

export const React = { createElement: (t, p, ...c) => ({type: t, props: p, children: c}), StrictMode: "StrictMode", startTransition: (fn) => fn() };
export const ReactDOM = { hydrateRoot: (doc, app) => { window.__neo_hydration_complete = true; return {}; } };
export const jsx = (type, props) => ({type, props});
export const version = "vendor-2.0";
window.__reactRouterVersion = version;
