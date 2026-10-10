'use strict';
let count = 0;
const counter = document.getElementById('count');
document.getElementById('increment').addEventListener('click', () => counter.textContent = ++count);
document.getElementById('reset-count').addEventListener('click', () => { count = 0; counter.textContent = count; });
const menu = document.getElementById('menu');
document.getElementById('toggle-menu').addEventListener('click', () => menu.hidden = !menu.hidden);
document.getElementById('validation').addEventListener('submit', event => {
  event.preventDefault();
  const name = document.getElementById('name').value.trim();
  const email = document.getElementById('email').value.trim();
  const valid = name.length > 0 && /^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(email);
  const result = document.getElementById('validation-result');
  result.textContent = valid ? `Valid form for ${name}` : 'Please enter a name and a valid email';
  result.style.color = valid ? 'green' : 'red';
});
let todos = [];
let nextId = 1;
let filter = 'all';
const input = document.getElementById('new-todo');
const list = document.getElementById('todo-list');
function render() {
  list.innerHTML = '';
  todos.filter(todo => filter === 'all' || (filter === 'completed' ? todo.done : !todo.done)).forEach(todo => {
    const row = document.createElement('li');
    row.setAttribute('data-id', todo.id);
    if (todo.done) row.classList.add('completed');
    const toggle = document.createElement('button');
    toggle.className = 'toggle';
    toggle.textContent = todo.done ? 'Undo' : 'Done';
    toggle.type = 'button';
    const label = document.createElement('span');
    label.textContent = todo.title;
    const remove = document.createElement('button');
    remove.className = 'destroy';
    remove.type = 'button';
    remove.textContent = 'Remove';
    row.appendChild(toggle); row.appendChild(label); row.appendChild(remove);
    list.appendChild(row);
  });
  document.getElementById('todo-count').textContent = `${todos.filter(todo => !todo.done).length} active`;
}
function addTodo() {
  const title = input.value.trim();
  if (!title) return;
  todos.push({ id: nextId++, title, done: false });
  input.value = '';
  render();
}
document.getElementById('add-todo').addEventListener('click', addTodo);
input.addEventListener('keydown', event => { if (event.key === 'Enter') { event.preventDefault(); addTodo(); } });
list.addEventListener('click', event => {
  const row = event.target.closest('li');
  if (!row) return;
  const id = Number(row.getAttribute('data-id'));
  if (event.target.classList.contains('destroy')) todos = todos.filter(todo => todo.id !== id);
  if (event.target.classList.contains('toggle')) todos.forEach(todo => { if (todo.id === id) todo.done = !todo.done; });
  render();
});
document.getElementById('filter-all').addEventListener('click', () => { filter = 'all'; render(); });
document.getElementById('filter-active').addEventListener('click', () => { filter = 'active'; render(); });
document.getElementById('filter-completed').addEventListener('click', () => { filter = 'completed'; render(); });
document.getElementById('clear-completed').addEventListener('click', () => { todos = todos.filter(todo => !todo.done); render(); });
setTimeout(() => document.getElementById('timer').textContent = 'Timer fired after first paint', 100);
document.getElementById('spin').addEventListener('click', () => { while (true) {} });
render();
