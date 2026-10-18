use js::{CloneData, CloneValue, JsValue, NullHost, Vm};
#[test]
fn data_graph_preserves_cycles_dates_nan_and_own_proto_name() {
    let mut source = Vm::new(NullHost);
    let value=source.eval_source("var a={n:NaN,date:new Date(12)};a.self=a;Object.defineProperty(a,'__proto__',{value:{safe:true},enumerable:true});a;").unwrap();
    let data = source.serialize_clone(value).unwrap();
    let mut target = Vm::new(NullHost);
    let result = target.deserialize_clone(&data).unwrap();
    target.set_global("copy", result).unwrap();
    assert_eq!(target.eval_source("copy===copy.self && Number.isNaN(copy.n) && copy.date.getTime()===12 && Object.getPrototypeOf(copy)===Object.prototype && Object.prototype.hasOwnProperty.call(copy,'__proto__');").unwrap(),JsValue::Bool(true));
    let function = source.eval_source("()=>1;").unwrap();
    assert!(source.serialize_clone(function).is_err());
}
#[test]
fn malformed_and_oversized_message_graphs_fail_before_use() {
    let mut vm = Vm::new(NullHost);
    assert!(vm
        .deserialize_clone(&CloneData {
            root: CloneValue::Ref(0),
            nodes: Vec::new()
        })
        .is_err());
    vm.limits.max_string_bytes = 32;
    assert!(vm
        .deserialize_clone(&CloneData {
            root: CloneValue::String("x".repeat(33)),
            nodes: Vec::new()
        })
        .is_err());
}
#[test]
fn modern_builtins_clone_across_realms_without_sharing_identity() {
    let mut source = Vm::new(NullHost);
    let value=source.eval_source("var key={id:1};var map=new Map();var set=new Set([key]);map.set(key,{big:2n**100n,set:set});map.set(map,map);var result={key:key,map:map,regex:/a/gi,boxed:Object(9n),error:new TypeError('failure')};result.regex.lastIndex=7;result.error.cause=key;result;").unwrap();
    let data = source.serialize_clone(value).unwrap();
    let encoded = serde_json::to_string(&data).unwrap();
    let data: CloneData = serde_json::from_str(&encoded).unwrap();
    let mut target = Vm::new(NullHost);
    let copy = target.deserialize_clone(&data).unwrap();
    target.set_global("copy", copy).unwrap();
    target.gc_collect();
    assert_eq!(target.eval_source("(copy.map instanceof Map)+':'+(copy.map.get(copy.map)===copy.map)+':'+copy.map.get(copy.key).set.has(copy.key)+':'+copy.map.get(copy.key).big+':'+copy.regex.lastIndex+':'+copy.boxed.valueOf()+':'+copy.error.name+':'+copy.error.message+':'+(copy.error.cause===copy.key);").unwrap(),JsValue::string("true:true:true:1267650600228229401496703205376:0:9:TypeError:failure:true"));
    assert!(data.retained_bytes() > encoded.len());
}
