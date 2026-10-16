 fetch('/x',{get method(){root.textContent='getter';return 'POST';},body:new URLSearchParams({x:1})}).then(r=>r.json()).catch(e=>e.name);
