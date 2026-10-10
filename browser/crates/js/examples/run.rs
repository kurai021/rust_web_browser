//! Explicit parser/runtime diagnostic harness, not a product JS shell.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let source = std::env::args()
        .nth(1)
        .ok_or("expected JavaScript source argument")?;
    let mut vm = js::Vm::new(js::NullHost);
    let value = vm.eval_source(&source)?;
    println!("{}", vm.to_string(value)?);
    Ok(())
}
