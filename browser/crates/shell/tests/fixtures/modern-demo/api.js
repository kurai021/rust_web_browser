async function request(url, options) {
    const response = await fetch(url, options);
    if (!response.ok) throw new Error('HTTP ' + response.status);
    return await response.json();
}
export async function load() { return (await request('/api/tasks')).items; }
export async function create(title) {
    return (await request('/api/tasks', { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ title }) })).item;
}
export async function toggle(item) {
    return (await request('/api/tasks/' + item.id, { method: 'PATCH', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ completed: !item.completed }) })).item;
}
export async function* hydrated(items) {
    for (const item of items) { await Promise.resolve(); yield item; }
}
