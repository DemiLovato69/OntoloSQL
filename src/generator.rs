use std::fs;
use std::path::PathBuf;

use anyhow::Result;

use crate::mapper::map_schema_to_ontology;
use crate::parser::parse_postgres_schema;
use crate::renderer::render_module;

#[derive(Debug, Clone)]
pub struct GenerateOptions {
    pub input: PathBuf,
    pub output: PathBuf,
}

pub fn generate_file(options: GenerateOptions) -> Result<()> {
    let sql = fs::read_to_string(&options.input)?;
    let schema = parse_postgres_schema(&sql)?;
    let module = map_schema_to_ontology(&schema)?;
    let rendered = render_module(&module);

    for warning in &module.warnings {
        eprintln!("Warning: {warning}");
    }

    if let Some(parent) = options.output.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }

    fs::write(&options.output, rendered)?;
    Ok(())
}
