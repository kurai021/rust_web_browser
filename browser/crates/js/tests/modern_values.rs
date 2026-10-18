use js::{JsValue, NullHost, Vm};
fn run(source: &str) -> JsValue {
    Vm::new(NullHost).eval_source(source).unwrap()
}
#[test]
fn well_known_symbols_control_coercion_instanceof_and_string_operations() {
    assert_eq!(run("var hints=[];var value={[Symbol.toPrimitive](hint){hints.push(hint);return hint==='string'?'key':4;}};var object={[value]:9};var instance={};var matcher={[Symbol.hasInstance](v){return v===instance;}};var pattern={[Symbol.match](text){return text+'!';},[Symbol.search](){return 7;},[Symbol.replace](text,replacement){return replacement+text;},[Symbol.split](){return ['split'];}};object.key+':'+(value+1)+':'+(instance instanceof matcher)+':'+('a'.match(pattern))+':'+('a'.search(pattern))+':'+('a'.replace(pattern,'b'))+':'+('a'.split(pattern)[0])+':'+hints.join(',');"),JsValue::string("9:5:true:a!:7:ba:split:string,default"));
    assert_eq!(run("var regex=/a/g;RegExp.prototype[Symbol.match].call(regex,'aba').join(',')+':'+typeof Symbol.asyncIterator+':'+Object.prototype.toString.call(Object(Symbol('x')));"),JsValue::string("a,a:symbol:[object Symbol]"));
}
#[test]
fn symbol_registry_and_descriptions_survive_gc_and_property_round_trips() {
    let mut vm = Vm::new(NullHost);
    vm.eval_source("var symbol=Symbol('key');var object={[symbol]:1};var registered=Symbol.for('shared');var empty=Symbol();").unwrap();
    vm.gc_collect();
    assert_eq!(vm.eval_source("var key=Object.getOwnPropertySymbols(object)[0];(key===symbol)+':'+key.description+':'+key.toString()+':'+(Symbol.for('shared')===registered)+':'+Symbol.keyFor(registered)+':'+empty.description;").unwrap(),JsValue::string("true:key:Symbol(key):true:shared:undefined"));
}
#[test]
fn array_from_uses_live_iterators_and_closes_on_a_mapping_error() {
    assert_eq!(run("var closed=false;function* values(){try{yield 1;yield 2;}finally{closed=true;}}try{Array.from(values(),()=>{throw 'stop';});}catch(e){}Array.from(new Set([1,2,2]),value=>value*2).join(',')+':'+closed+':'+[].concat({0:'spread',length:1,[Symbol.isConcatSpreadable]:true})[0];"),JsValue::string("2,4:true:spread"));
}
#[test]
fn bigint_explicit_conversion_and_boxing_obey_numeric_hints() {
    assert_eq!(run("var value=Object(12345678901234567890n);(Object.getPrototypeOf(value)===BigInt.prototype)+':'+value.toString()+':'+Number(Object(12n))+':'+BigInt({[Symbol.toPrimitive](){return '42';}})+':'+Object.prototype.toString.call(value);"),JsValue::string("true:12345678901234567890:12:42:[object BigInt]"));
}
