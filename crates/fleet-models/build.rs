use fluorite_codegen::code_gen::rust::RustOptions;

fn main() -> anyhow::Result<()> {
    println!("cargo:rerun-if-changed=fluorite");
    let out_dir = std::env::var("OUT_DIR")?;
    let options = RustOptions::new(out_dir).with_any_type("serde_json::Value");
    fluorite_codegen::compile_with_options(options, &["fluorite/"])?;
    Ok(())
}
