const metadata = new WeakMap();
export function text(id, value) { document.getElementById(id).textContent = String(value); }
export function action(target) { return metadata.get(target); }
export function render(store, filter) {
    const fragment = document.createDocumentFragment();
    let complete = 0;
    for (const item of store) {
        if (item.completed) complete++;
        if (filter === 'completed' && !item.completed) continue;
        const row = document.createElement('li'); row.className = item.completed ? 'task done' : 'task';
        const title = document.createElement('span'); title.className = 'task-title'; title.textContent = item.title;
        const button = document.createElement('button'); button.type = 'button'; button.textContent = item.completed ? 'Reopen' : 'Complete';
        metadata.set(button, { id: item.id }); row.append(title, button); fragment.append(row);
    }
    document.getElementById('tasks').replaceChildren(fragment);
    text('summary', store.rows.size + ' tasks · ' + complete + ' completed · Map snapshot');
    text('revision', 'BigInt revision ' + store.version);
    text('route-title', filter === 'completed' ? 'Completed tasks' : filter === 'about' ? 'About this board' : 'All tasks');
}
