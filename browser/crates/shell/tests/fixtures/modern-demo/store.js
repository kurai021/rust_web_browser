export const changed = Symbol('changed');
class TaskStore {
    constructor() { this.rows = new Map(); this.listeners = new Set(); this.version = 0n; }
    replace(items) { this.rows.clear(); for (const item of items) this.rows.set(item.id, item); this.notify(); }
    put(item) { this.rows.set(item.id, item); this.notify(); }
    snapshot() { return structuredClone(this.rows); }
    subscribe(callback) { this.listeners.add(callback); return () => this.listeners.delete(callback); }
    notify() { this.version++; for (const callback of this.listeners) callback(changed); }
    *[Symbol.iterator]() { yield* this.rows.values(); }
}
export const store = new Proxy(new TaskStore(), {
    get(target, key, receiver) { return Reflect.get(target, key, receiver); },
    set(target, key, value, receiver) { return Reflect.set(target, key, value, receiver); }
});
