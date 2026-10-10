#![no_main]
use libfuzzer_sys::fuzz_target;
fuzz_target!(|data:&[u8]|{
    let Ok(source)=std::str::from_utf8(data)else{return;};
    let opts=js::ParseOpts{max_source_bytes:8192,max_tokens:5000,max_nodes:2000,max_depth:32};
    if let Ok(program)=js::parse(source,opts){let mut vm=js::Vm::new(js::NullHost);vm.limits=js::Limits{max_steps:10000,max_duration:std::time::Duration::from_millis(100),max_call_depth:16,max_objects:10000,max_environments:10000,max_string_bytes:65536,max_memory_bytes:32*1024*1024,..Default::default()};let _=vm.eval(&program);vm.gc_collect();}
});
