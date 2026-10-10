var n=0;var a=[2];a[n++]+=3;var object={get x(){return n++;},set x(value){n=value;}};object.x++;
function get(){return function(x){return x;};}get()((n++,1));
switch(2){case ++n:break;case ++n:break;default:break;}
