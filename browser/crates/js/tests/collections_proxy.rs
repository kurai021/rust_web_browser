use js::{JsValue, NullHost, Vm};
fn eval(source: &str) -> JsValue {
    Vm::new(NullHost).eval_source(source).unwrap()
}
#[test]
fn maps_sets_have_same_value_zero_and_live_insertion_order() {
    assert_eq!(eval("var map=new Map([[NaN,1],[-0,2],['a',3]]);map.set(NaN,4);var iterator=map.entries();var first=iterator.next().value;map.delete('a');map.set('b',5);var rest=[];for(var item of iterator)rest.push(String(item[0])+':'+item[1]);String(first[0])+':'+first[1]+'|'+rest.join(',')+'|'+new Set([1,1,NaN,NaN]).size;"),JsValue::string("NaN:4|0:2,b:5|2"));
}
#[test]
fn collection_for_each_observes_mutation_and_can_reenter() {
    assert_eq!(eval("var map=new Map([['a',1],['b',2]]);var values=[];map.forEach((value,key)=>{values.push(key);if(key==='a'){map.delete('b');map.set('c',3);}});values.join(',');"),JsValue::string("a,c"));
}
#[test]
fn weak_maps_do_not_root_keys_and_trace_ephemeron_values_when_key_is_live() {
    let mut vm = Vm::new(NullHost);
    vm.eval_source("var weak=new WeakMap();var keep={};weak.set(keep,{value:'kept'});for(var i=0;i<1000;i++){var key={};weak.set(key,{key:key});}key=null;").unwrap();
    let before = vm.heap_objects();
    vm.gc_collect();
    assert!(vm.heap_objects() + 500 < before);
    assert_eq!(
        vm.eval_source("weak.get(keep).value;").unwrap(),
        JsValue::string("kept")
    );
}
#[test]
fn weak_symbol_keys_and_ephemeron_chains_are_traced_without_retaining_dead_keys() {
    let mut vm = Vm::new(NullHost);
    vm.eval_source("var first=new WeakMap();var second=new WeakMap();var root={};var keep=Symbol('keep');first.set(root,keep);second.set(keep,{value:42});for(var i=0;i<1000;i++){var dead=Symbol('dead');second.set(dead,{payload:i});}dead=null;keep=null;").unwrap();
    let before = vm.heap_objects();
    vm.gc_collect();
    assert!(vm.heap_objects() + 900 < before);
    assert_eq!(
        vm.eval_source("second.get(first.get(root)).value;")
            .unwrap(),
        JsValue::Number(42.0)
    );
    assert!(vm
        .eval_source("first.set(Symbol.for('registered'),1);")
        .is_err());
}
#[test]
fn long_weak_map_chains_reach_a_fixed_point_and_dead_cycles_are_reclaimed() {
    let mut vm = Vm::new(NullHost);
    vm.eval_source("var weak=new WeakMap();var root={};var key=root;for(var i=0;i<5000;i++){var next={value:i};weak.set(key,next);key=next;}key=null;next=null;").unwrap();
    vm.gc_collect();
    assert_eq!(
        vm.eval_source("var key=root;for(var i=0;i<5000;i++)key=weak.get(key);key.value;")
            .unwrap(),
        JsValue::Number(4999.0)
    );
    let before = vm.heap_objects();
    vm.eval_source("root=null;key=null;undefined;").unwrap();
    vm.gc_collect();
    assert!(vm.heap_objects() + 5000 <= before);
}
#[test]
fn proxy_get_set_has_keys_and_revocation_use_real_traps() {
    assert_eq!(eval("var log=[];var target={x:1};var proxy=new Proxy(target,{get(t,k,r){log.push(k);return Reflect.get(t,k,r);},set(t,k,v,r){return Reflect.set(t,k,v*2,r);},has(t,k){return k==='virtual'||Reflect.has(t,k);}});proxy.x=3;var x=proxy.x;var pair=Proxy.revocable(target,{});pair.revoke();var revoked=false;try{pair.proxy.x;}catch(e){revoked=e.name==='TypeError';}x+':'+('virtual' in proxy)+':'+Reflect.ownKeys(proxy).join(',')+':'+revoked;"),JsValue::string("6:true:x:true"));
}
#[test]
fn proxy_invariants_reject_lying_about_frozen_data_and_own_keys() {
    assert_eq!(eval("var target=Object.freeze({x:1});var failures=0;try{new Proxy(target,{get(){return 2;}}).x;}catch(e){failures++;}try{Object.keys(new Proxy(target,{ownKeys(){return [];}}));}catch(e){failures++;}try{Reflect.deleteProperty(new Proxy(target,{deleteProperty(){return true;}}),'x');}catch(e){failures++;}failures;"),JsValue::Number(3.0));
}
#[test]
fn callable_proxies_preserve_receiver_and_reflect_apply() {
    assert_eq!(eval("function f(x){return this.n+x;}var p=new Proxy(f,{apply(t,receiver,args){return Reflect.apply(t,receiver,args)*2;}});Reflect.apply(p,{n:3},[4]);"),JsValue::Number(14.0));
}
#[test]
fn partial_descriptors_preserve_attributes_and_proxy_traps_receive_only_present_fields() {
    assert_eq!(eval("var target={x:1};Object.defineProperty(target,'x',{value:2});var fields=[];var proxy=new Proxy(target,{defineProperty(t,k,d){fields.push(Object.keys(d).join(','));return Reflect.defineProperty(t,k,d);}});Reflect.defineProperty(proxy,'x',{value:3});proxy.x=4;Object.getOwnPropertyDescriptor(target,'x').writable+':'+fields.join('|')+':'+target.x;"),JsValue::string("true:value|value:4"));
    assert_eq!(eval("var failures=0;try{Object.defineProperty({},'x',{value:1,get:undefined});}catch(e){failures++;}var target=Object.freeze({x:1});try{Reflect.defineProperty(new Proxy(target,{defineProperty(){return true;}}),'x',{value:2});}catch(e){failures++;}failures;"),JsValue::Number(2.0));
}
