let xs=[1,2,3];let {x:y}={x:2};function f(a,...rest){return rest.reduce((s,x)=>s+x,a);}f(y,...xs);`${xs.length}`;
