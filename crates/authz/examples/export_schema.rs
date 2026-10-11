//! Regenerates `policies/schema.cedarschema.json` (the Cedar JSON schema the
//! CDK stack uploads to Verified Permissions) from `policies/schema.cedarschema`.
//!
//! `cargo run -p kabados-authz --example export_schema`

fn main() -> std::io::Result<()> {
    let value: serde_json::Value = serde_json::from_str(&kabados_authz::schema_json()).expect("schema JSON");
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../policies/schema.cedarschema.json");
    let mut text = serde_json::to_string_pretty(&value).expect("serialize");
    text.push('\n');
    std::fs::write(&path, text)?;
    println!("wrote {}", path.display());
    Ok(())
}
