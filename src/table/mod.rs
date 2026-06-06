mod definition;
pub use definition::*;

use anyhow::{Context, Result};
use std::fs;

/// Load a numbered table (1–8) from `assets/tables/tableN.json`.
pub fn load_table(n: u8) -> Result<TableDef> {
    let path = format!("assets/tables/table{}.json", n);
    let text = fs::read_to_string(&path)
        .with_context(|| format!("could not read {}", path))?;
    let def: TableDef = serde_json::from_str(&text)
        .with_context(|| format!("could not parse {}", path))?;
    Ok(def)
}
