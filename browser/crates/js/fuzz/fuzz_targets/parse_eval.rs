#![no_main]
use libfuzzer_sys::fuzz_target;
fn realm()->js::Vm{
    let mut vm=js::Vm::new(js::NullHost);
    // ASan may relocate address-taken locals to its fake stack. A depth cap of
    // 16 bounds native recursion here; normal-thread stack probing has unit tests.
    vm.limits=js::Limits{max_native_stack_bytes:usize::MAX,max_steps:10000,max_duration:std::time::Duration::from_millis(100),max_call_depth:16,max_objects:10000,max_environments:10000,max_string_bytes:65536,max_memory_bytes:32*1024*1024,..Default::default()};vm
}
fuzz_target!(|data:&[u8]|{
    let Ok(source)=std::str::from_utf8(data)else{return;};
    let opts=js::ParseOpts{max_source_bytes:8192,max_tokens:5000,max_nodes:2000,max_depth:32};
    if let Ok(program)=js::parse(source,opts){
        let mut vm=realm();
        if let Ok(value)=vm.eval(&program){
            let pin=vm.pin(value.clone());
            if let Ok(data)=vm.serialize_clone(value){let mut target=realm();let _=target.deserialize_clone(&data);target.gc_collect();}
            vm.gc_collect();let _=vm.drain_microtasks();vm.unpin(pin);
        }
        vm.gc_collect();
    }
    if js::parse_module(source,opts).is_ok(){
        let mut vm=realm();
        if vm.define_module("https://fuzz.invalid/main.js",source).is_ok(){let _=vm.eval_module("https://fuzz.invalid/main.js");vm.gc_collect();let _=vm.drain_microtasks();vm.gc_collect();}
    }
});
