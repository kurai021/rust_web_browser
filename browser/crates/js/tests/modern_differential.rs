//! Optional Node oracle, only in tests. Promise results and observable ordering
//! are compared, never an engine-specific stack string or wall-clock timestamp.
use js::{NullHost, Vm};
fn compare(source: &str) {
    let mut vm = Vm::new(NullHost);
    let value = vm
        .eval_source(source)
        .unwrap_or_else(|error| panic!("{error}\n{source}"));
    let pin = vm.pin(value.clone());
    vm.gc_collect();
    vm.drain_microtasks().unwrap();
    let value = if vm.is_promise(&value) {
        vm.promise_result(&value)
            .expect("oracle case settled")
            .unwrap_or_else(|error| panic!("rejection: {:?}\n{source}", vm.to_string(error)))
    } else {
        value
    };
    let ours = vm.to_string(value).unwrap();
    vm.unpin(pin);
    let output=std::process::Command::new("node").args(["-e",&format!("Promise.resolve({source}).then(value=>process.stdout.write(String(value)),error=>{{console.error(error);process.exitCode=1;}});")]).output().expect("installed Node oracle");
    assert!(
        output.status.success(),
        "{}\n{source}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(ours, String::from_utf8(output.stdout).unwrap(), "{source}");
}
#[test]
#[ignore = "requires the installed Node test oracle"]
fn modern_language_and_job_results_match_node() {
    let cases=[
        "(async()=>{var log=[];var value={get then(){log.push('get');return resolve=>{log.push('call');resolve(3);};}};var p=Promise.resolve(value);log.push('sync');p.then(v=>log.push(v));queueMicrotask(()=>log.push('micro'));await p;await 0;return log.join(',');})()",
        "(async()=>{var log=[];async function f(){try{return await Promise.resolve(1);}finally{log.push('finally');await 0;log.push('resumed');}}f().then(v=>log.push(v));queueMicrotask(()=>log.push('queued'));await 0;await 0;await 0;await 0;return log.join(',');})()",
        "(async()=>{var values=await Promise.allSettled([Promise.resolve(3),Promise.reject('failure'),4]);return JSON.stringify(values);})()",
        "(async()=>{try{await Promise.any([Promise.reject('first'),Promise.reject('second')]);}catch(error){return error.name+':'+error.errors.join(',');}})()",
        "(async()=>{var result=await Promise.resolve(3).finally(()=>Promise.resolve(4));var failed;try{await Promise.reject(5).finally(()=>Promise.reject(6));}catch(error){failed=error;}return result+':'+failed;})()",
        "(async()=>{var calls=0;function C(executor){calls++;return new Promise(executor);}var p=Promise.resolve(7);p.constructor={[Symbol.species]:C};var value=await p.then(x=>x+1);return value+':'+calls;})()",
        "(async()=>{var y=null;async function f(x=y()){}var promise=f();try{await promise;}catch(error){return (promise instanceof Promise)+':'+error.name;}})()",
        "(async()=>{var object={async method(x){return await x+1;},*values(){yield 3;},async *stream(){yield await 4;}};return (await object.method(1))+':'+object.values().next().value+':'+(await object.stream().next()).value;})()",
        "(async()=>{var log=[];async function* inner(){try{yield Promise.resolve(1);yield 2;}finally{log.push('closed');}}async function* outer(){yield* inner();}var g=outer();log.push(JSON.stringify(await g.next()));log.push(JSON.stringify(await g.return(7)));log.push(JSON.stringify(await g.next()));return log.join('|');})()",
        "(async()=>{var log=[];async function* f(){try{yield 1;}finally{await 0;yield 2;}}var g=f();var first=g.next(),end=g.return(Promise.resolve(8)),last=g.next();return JSON.stringify(await Promise.all([first,end,last]));})()",
        "(async()=>{var log=[];var iterator={[Symbol.asyncIterator](){return this;},next(){return {value:1,done:false};},return(){log.push('close');return Promise.resolve({done:true});}};for await(var value of iterator){log.push(value);break;}log.push('done');return log.join(',');})()",
        "(()=>{var map=new Map([[NaN,1],[-0,2],[1n,3],[1,4]]);var values=[];map.forEach((value,key)=>{values.push(String(key)+':'+value);if(key!==key)map.set('new',5);});return map.size+'|'+values.join(',');})()",
        "(()=>{var set=new Set([1,2]);var iterator=set.values();var first=iterator.next().value;set.delete(2);set.add(3);return first+':'+iterator.next().value+':'+iterator.next().done;})()",
        "(()=>{var weak=new WeakMap();var key=Symbol('weak');weak.set(key,42);var failed;try{weak.set(Symbol.for('registered'),1);}catch(error){failed=error.name;}return weak.get(key)+':'+failed;})()",
        "(()=>{var symbol=Symbol('key');var object={[symbol]:1};var key=Object.getOwnPropertySymbols(object)[0];return (key===symbol)+':'+key.description+':'+key.toString()+':'+Symbol.keyFor(Symbol.for('registered'));})()",
        "(()=>{var hints=[];var value={[Symbol.toPrimitive](hint){hints.push(hint);return hint==='string'?'key':2n;}};var object={[value]:3};var result=String(value+1n)+':'+String(value*3n)+':'+object.key;return result+':'+hints.join(',');})()",
        "(()=>{var x=Object(2n);var old=x++;return typeof old+':'+old+':'+x+':'+(-Object(4n))+':'+(~Object(0n))+':'+(1n==true)+':'+(0n<true)+':'+(Object(Symbol.for('x'))==Symbol.for('x'));})()",
        "(()=>{var n=1234567890123456789012345678901234567890n;return [n/3n,n%3n,-n/7n,-n%7n,n<<15n,-n>>15n,BigInt.asIntN(32,n),BigInt.asUintN(32,-n)].map(String).join(',');})()",
        "(()=>{var target={x:1};var fields=[];var proxy=new Proxy(target,{get(t,k,r){return Reflect.get(t,k,r);},defineProperty(t,k,d){fields.push(Object.keys(d).join(','));return Reflect.defineProperty(t,k,d);}});proxy.x=2;Reflect.defineProperty(proxy,'x',{value:3});return proxy.x+':'+fields.join('|')+':'+Object.getOwnPropertyDescriptor(target,'x').writable;})()",
        "(()=>{var key=Symbol('key');var expected=false;var proxy=new Proxy({},{get(target,property){expected=property===key;return property.description;}});return proxy[key]+':'+expected;})()",
        "(()=>{var original=new Map();original.set(original,new Set([original]));var copy=structuredClone(original);return (copy!==original)+':'+copy.get(copy).has(copy);})()",
        "(()=>{function* f(){try{yield 1;}catch(error){yield error;}finally{yield 3;}}var g=f();return JSON.stringify([g.next(),g.throw(2),g.return(4),g.next()]);})()",
    ];
    for source in cases {
        compare(source);
    }
    // Deterministic integer corpus: signed arithmetic, two's-complement bitwise
    // operations and rounding/comparisons around values too large for Number.
    let mut expressions = Vec::new();
    let mut seed = 0x6b8b4567u64;
    for _ in 0..32 {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        let a = (seed as i64) as i128;
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        let b = (seed as i64) as i128 | 1;
        for op in ["+", "-", "*", "/", "%", "&", "|", "^"] {
            expressions.push(format!("String(({a}n){op}({b}n))"));
        }
    }
    compare(&format!("[{}].join(',')", expressions.join(",")));
    println!(
        "Node modern differential: {} scenarios plus 256 signed BigInt operations",
        cases.len()
    );
}
#[test]
fn cached_bytecode_and_ast_reference_agree_on_the_existing_synchronous_language() {
    let sources=[
        "var sum=0;for(let i=0;i<20;i++){try{if(i%3===0)continue;sum+=i;}finally{sum++;}}sum;",
        "var x=0;outer:for(var i=0;i<5;i++){for(var j=0;j<5;j++){if(j===2)continue outer;x++;}}x;",
        "function f(x){try{if(x)throw 3;return 1;}catch(e){return e+1;}finally{x=2;}}f(true)+f(false);",
        "var x=0;switch(2){case 1:x=1;break;case 2:x+=2;case 3:x+=3;default:x+=4;}x;",
        "var object={n:2,get value(){return this.n;},set value(v){this.n=v;}};object.value+=3;object.n;",
        "function make(x){return y=>x+y;}var add=make(2);[1,2,3].map(add).join(',');",
        "class A{constructor(x){this.x=x;}read(){return this.x;}}class B extends A{read(){return super.read()+1;}}new B(5).read();",
        "var x=7;function f(){'use strict';eval('var x=3;');return x;}f();",
        "var fs=[];for(let i=0;i<5;i++)fs.push(()=>i);fs.map(f=>f()).join(',');",
        "var count=0;var object={x:2};function key(){count++;return 'x';}object[key()]*=3;object.x+count;",
        "var x=2n**90n;x/7n+':'+x%7n;",
        "var map=new Map([[1,2],[3,4]]);map.get(1)+map.get(3);",
    ];
    for source in sources {
        let program = js::parse(source, Default::default()).unwrap();
        let code = js::bytecode::compile(&program).unwrap();
        let mut ast = Vm::new(NullHost);
        let mut bytecode = Vm::new(NullHost);
        let a = ast.eval_ast_reference(&program).unwrap();
        let b = bytecode.eval_compiled(code).unwrap();
        assert_eq!(
            bytecode.to_string(b).unwrap(),
            ast.to_string(a).unwrap(),
            "{source}"
        );
    }
}
