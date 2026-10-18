import { store, changed } from './store.js';
import * as api from './api.js';
import { text, render, action } from './view.js';

let filter = location.pathname.endsWith('/completed') ? 'completed' : location.pathname.endsWith('/about') ? 'about' : 'all';
const worker = new Worker('/modern/worker.js');
worker.onmessage = event => text('worker-state', 'Worker: ' + event.data.pending + ' pending · Map: ' + event.data.collection + ' · DOM: ' + event.data.dom);
function update(reason) { if (reason === changed) { render(store, filter); worker.postMessage(store.snapshot()); } }
store.subscribe(update);

async function refresh() {
    text('status', 'Fetching tasks…');
    const rows = [];
    for await (const item of api.hydrated(await api.load())) rows.push(item);
    store.replace(rows); text('status', 'Ready · async fetch and generator hydration complete');
}
async function route() {
    filter = location.pathname.endsWith('/completed') ? 'completed' : location.pathname.endsWith('/about') ? 'about' : 'all';
    render(store, filter);
    if (filter === 'about') {
        const details = await import('./details.js');
        text('about', details.describe()); text('module-state', 'Dynamic module evaluated ' + detailsLoads + ' time · ' + details.source);
    } else text('about', '');
}
for (const link of document.querySelectorAll('nav a')) link.addEventListener('click', async event => {
    event.preventDefault(); history.pushState({}, '', link.getAttribute('href')); await route();
});
window.addEventListener('popstate', route);
document.getElementById('task-form').addEventListener('submit', async event => {
    event.preventDefault(); const input = document.getElementById('task-title'); const title = input.value.trim();
    if (!title) return;
    try { store.put(await api.create(title)); input.value = ''; text('status', 'Task saved through POST'); }
    catch (error) { text('status', error.message); }
});
document.getElementById('tasks').addEventListener('click', async event => {
    const selected = action(event.target); if (!selected) return;
    try { store.put(await api.toggle(store.rows.get(selected.id))); text('status', 'Task updated through PATCH'); }
    catch (error) { text('status', error.message); }
});
document.getElementById('refresh').addEventListener('click', refresh);

function checkOrder() {
    const log = ['sync']; const target = document.getElementById('order-target');
    const observer = new MutationObserver(() => log.push('observer')); observer.observe(target, { childList: true });
    Promise.resolve().then(() => log.push('promise'));
    target.textContent = target.textContent === 'ready' ? 'changed' : 'ready';
    queueMicrotask(() => log.push('microtask'));
    setTimeout(() => { observer.disconnect(); log.push('timer'); text('order-state', log.join(' → ')); }, 0);
    log.push('end');
}
document.getElementById('check-order').addEventListener('click', checkOrder);
window.addEventListener('unhandledrejection', event => text('status', 'Unhandled: ' + event.reason));
requestAnimationFrame(() => queueMicrotask(() => text('frame-state', 'Animation frame → microtask checkpoint')));
checkOrder();
await refresh();
if (filter === 'about') await route();
