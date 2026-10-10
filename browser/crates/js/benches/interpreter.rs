use criterion::{black_box, criterion_group, criterion_main, BatchSize, Criterion};
use js::{parse, NullHost, Vm};
fn benches(c: &mut Criterion) {
    let notebook = include_str!("../../shell/tests/fixtures/js-demo/app.js");
    c.bench_function("js/parse-notebook", |b| {
        b.iter(|| parse(black_box(notebook), Default::default()).unwrap())
    });
    let large = "var x=1+2;\n".repeat(4096);
    c.bench_function("js/parse-44KiB", |b| {
        b.iter(|| parse(black_box(&large), Default::default()).unwrap())
    });
    let program = parse("function make(a){return b=>a+b;}var f=make(2);var sum=0;for(var i=0;i<1000;i++)sum+=f(i);sum;", Default::default()).unwrap();
    c.bench_function("js/1000-closure-calls", |b| {
        b.iter_batched(
            || Vm::new(NullHost),
            |mut vm| black_box(vm.eval(&program).unwrap()),
            BatchSize::SmallInput,
        )
    });
    let cycles = parse(
        "for(var i=0;i<1000;i++){var a={};var b={};a.b=b;b.a=a;}a=null;b=null;undefined;",
        Default::default(),
    )
    .unwrap();
    c.bench_function("js/gc-1000-cycles", |b| {
        b.iter_batched(
            || {
                let mut vm = Vm::new(NullHost);
                vm.eval(&cycles).unwrap();
                vm
            },
            |mut vm| {
                vm.gc_collect();
                black_box(vm.heap_objects());
            },
            BatchSize::SmallInput,
        )
    });
}
criterion_group!(group, benches);
criterion_main!(group);
