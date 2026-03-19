// NeoRender — Stealth / anti-detection layer
// Injected BEFORE page scripts run. Consistent fingerprint (not random).

Object.defineProperty(navigator, 'webdriver', { get: () => false });
Object.defineProperty(navigator, 'plugins', { get: () => [
    {name: 'Chrome PDF Plugin', filename: 'internal-pdf-viewer'},
    {name: 'Chrome PDF Viewer', filename: 'mhjfbmdgcfjbbpaeojofohoefgiehjai'},
    {name: 'Native Client', filename: 'internal-nacl-plugin'},
] });
Object.defineProperty(navigator, 'languages', { get: () => ['es-ES', 'es', 'en'] });
Object.defineProperty(screen, 'width', { get: () => 1920 });
Object.defineProperty(screen, 'height', { get: () => 1080 });
Object.defineProperty(screen, 'availWidth', { get: () => 1920 });
Object.defineProperty(screen, 'availHeight', { get: () => 1040 });
Object.defineProperty(screen, 'colorDepth', { get: () => 24 });

// Chrome-specific globals that bot detectors check for
globalThis.chrome = { runtime: {}, loadTimes: () => ({}), csi: () => ({}) };

// Permissions API stub (Notification.permission etc.)
if (!navigator.permissions) {
    navigator.permissions = {
        query: () => Promise.resolve({ state: 'prompt', onchange: null })
    };
}

// Connection API stub
if (!navigator.connection) {
    Object.defineProperty(navigator, 'connection', { get: () => ({
        effectiveType: '4g', rtt: 50, downlink: 10, saveData: false
    }) });
}
