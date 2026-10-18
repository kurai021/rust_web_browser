let count = 0;
const name = location.pathname.endsWith('/article') ? 'Reading sample' : 'Browser workshop';
document.title = name;
document.getElementById('page-title').textContent = name;
sessionStorage.setItem('workshop', 'tab-local');
document.getElementById('session').textContent = 'Session: ' + sessionStorage.getItem('workshop');
document.getElementById('increment').onclick = () => {
    count++;
    document.getElementById('counter').textContent = 'Counter: ' + count;
};
document.getElementById('report').onclick = async () => {
    try {
        const response = await fetch('/api/data');
        const blob = await response.blob();
        const uri = URL.createObjectURL(blob);
        const link = document.createElement('a');
        link.href = uri; link.download = 'browser-report.json';
        link.click(); URL.revokeObjectURL(uri);
        document.getElementById('report-status').textContent = 'Report handed to the native save UI.';
    } catch (error) { document.getElementById('report-status').textContent = error.message; }
};
if (location.pathname.endsWith('/article')) {
    const fragment = document.createDocumentFragment();
    for (let i = 1; i <= 28; i++) {
        const p = document.createElement('p');
        p.textContent = 'Section ' + i + ': The browser keeps each page realm independent. Navigation, history, rendering and streaming downloads exchange messages so the user interface remains responsive.';
        fragment.appendChild(p);
    }
    document.getElementById('reading').appendChild(fragment);
}
