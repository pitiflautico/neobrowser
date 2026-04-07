// IndexedDB — functional stub that stores data in memory
// Real implementation would use SQLite via ops
if (!globalThis.indexedDB) {
    const databases = new Map();

    class IDBRequest extends EventTarget {
        constructor() { super(); this.result = null; this.error = null; this.readyState = 'pending'; this.onsuccess = null; this.onerror = null; }
        _resolve(result) {
            this.result = result; this.readyState = 'done';
            Promise.resolve().then(() => {
                this.dispatchEvent(new Event('success'));
                if (this.onsuccess) this.onsuccess({ target: this });
            });
        }
        _reject(error) {
            this.error = error; this.readyState = 'done';
            Promise.resolve().then(() => {
                this.dispatchEvent(new Event('error'));
                if (this.onerror) this.onerror({ target: this });
            });
        }
    }

    class IDBObjectStore {
        constructor(name) { this.name = name; this._data = new Map(); this.keyPath = null; this.indexNames = []; }
        put(value, key) { const req = new IDBRequest(); this._data.set(key || value[this.keyPath] || Date.now(), value); req._resolve(key); return req; }
        get(key) { const req = new IDBRequest(); req._resolve(this._data.get(key)); return req; }
        delete(key) { const req = new IDBRequest(); this._data.delete(key); req._resolve(undefined); return req; }
        clear() { const req = new IDBRequest(); this._data.clear(); req._resolve(undefined); return req; }
        getAll() { const req = new IDBRequest(); req._resolve([...this._data.values()]); return req; }
        count() { const req = new IDBRequest(); req._resolve(this._data.size); return req; }
        createIndex() { return {}; }
        index() { return { get(key) { return new IDBRequest(); }, getAll() { return new IDBRequest(); } }; }
    }

    class IDBTransaction extends EventTarget {
        constructor(db, storeNames) {
            super(); this.db = db; this._stores = storeNames;
            this.oncomplete = null; this.onerror = null; this.onabort = null;
        }
        objectStore(name) {
            if (!this.db._stores.has(name)) this.db._stores.set(name, new IDBObjectStore(name));
            return this.db._stores.get(name);
        }
    }

    class IDBDatabase extends EventTarget {
        constructor(name, version) {
            super(); this.name = name; this.version = version;
            this._stores = new Map();
            this.objectStoreNames = [];
            this.onversionchange = null;
        }
        createObjectStore(name, options) {
            const store = new IDBObjectStore(name);
            if (options?.keyPath) store.keyPath = options.keyPath;
            this._stores.set(name, store);
            this.objectStoreNames.push(name);
            return store;
        }
        transaction(storeNames, mode) { return new IDBTransaction(this, Array.isArray(storeNames) ? storeNames : [storeNames]); }
        close() {}
    }

    globalThis.indexedDB = {
        open(name, version) {
            const req = new IDBRequest();
            let db = databases.get(name);
            if (!db) { db = new IDBDatabase(name, version || 1); databases.set(name, db); }
            req.result = db;
            req._resolve(db);
            // Fire onupgradeneeded if new
            if (req.onupgradeneeded) {
                Promise.resolve().then(() => req.onupgradeneeded({ target: req, oldVersion: 0, newVersion: version }));
            }
            return req;
        },
        deleteDatabase(name) { databases.delete(name); const req = new IDBRequest(); req._resolve(undefined); return req; },
    };

    globalThis.IDBKeyRange = {
        only: (v) => ({ lower: v, upper: v }),
        lowerBound: (v) => ({ lower: v }),
        upperBound: (v) => ({ upper: v }),
        bound: (l, u) => ({ lower: l, upper: u }),
    };
}
