//! Node is an optional test oracle only. It is never linked into the product.
use js::{NullHost, Vm};
#[test]
#[ignore = "requires an installed Node oracle; run explicitly in Phase 5 verification"]
fn representative_language_results_match_node() {
    let sources = [
        "(()=>{var x=1;return x+2;})()",
        "(()=>{let xs=[1,2,3];return xs.map(x=>x*2).join(',');})()",
        "(()=>{function f(x){return y=>x+y;}return f(3)(4);})()",
        "(()=>{let {x:y}={x:4};let [a,...rest]=[1,2,3];return `${y+a}:${rest.length}`;})()",
        "(()=>{function C(x){this.x=x;}C.prototype.get=function(){return this.x;};return new C(8).get();})()",
        "(()=>{class C{constructor(x){this.x=x;}get(){return this.x;}}return new C(9).get();})()",
        "(()=>{let n=0;for(let i=0;i<5;i++){n+=i;}return n;})()",
        "(()=>{let x='a,b,c';return x.split(',').length;})()",
        "(()=>{return /^[a-z]+$/i.test('Abc');})()",
        "(()=>{return JSON.stringify({x:1,y:[true,null]});})()",
        "(()=>{var n=0;var a=[2];a[n++]+=3;return n*10+a[0];})()",
        "(()=>{var order=[];function get(){order.push('callee');return x=>order.join(',');}return get()((order.push('arg'),1));})()",
        "(()=>{var n=0;switch(2){case ++n:break;case ++n:break;}return n;})()",
        "(()=>{try{typeof x;let x;}catch(e){return e.name;}})()",
        "(()=>{function f(){var eval=x=>x+2;return eval(3);}return f();})()",
        "(()=>{function f(){var local=1;var indirect=eval;return indirect('typeof local');}return f();})()",
        "(()=>{class A{constructor(){this.x=4;}get value(){return this.x;}get(){return this.x;}}class B extends A{get(){return super.get()+super.value;}}return new B().get();})()",
        "(()=>{function f(){return typeof this;}return f.call(3);})()",
        "(()=>{function f(){'use strict';return typeof this;}return f.call(3);})()",
        "(()=>{return [parseFloat('1.25e2suffix'),parseFloat('1e+'),parseInt('0xff')].join(',');})()",
        "(()=>{var x=1;function f(){try{return x;}finally{x=2;}}return f()+x;})()",
        "(()=>{let fs=[];for(let i=0;i<3;i++)fs.push(()=>i);return fs.map(f=>f()).join(',');})()",
        "(()=>{var x={};Object.defineProperty(x,'v',{value:3});x.v=4;return x.v;})()",
        "(()=>{var x=Symbol('x');var o={};o[x]=7;return o[x]+Object.keys(o).length;})()",
        "(()=>{function f(){'use strict';eval('var hidden=2');return typeof hidden;}return f();})()",
        "(()=>{function f(){'use strict';var indirect=eval;return indirect('leaked=3');}return f();})()",
        "(()=>{return [isNaN(Date.UTC(1e30,1e30)),isNaN(Date.UTC(NaN,0)),Date.UTC(2020,0,2.9)===Date.UTC(2020,0,2)].join(',');})()",
    ];
    for source in sources {
        let mut vm = Vm::new(NullHost);
        let result = vm.eval_source(source).unwrap();
        let ours = vm.to_string(result).unwrap();
        let output = std::process::Command::new("node")
            .args(["-e", &format!("process.stdout.write(String({source}));")])
            .output()
            .expect("Node oracle available");
        assert!(output.status.success());
        assert_eq!(ours, String::from_utf8(output.stdout).unwrap(), "{source}");
    }
    println!(
        "Node differential: {0}/{0} representative expressions",
        sources.len()
    );
}
