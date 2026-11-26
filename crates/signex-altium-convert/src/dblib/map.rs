//! Column to field mapping, and the mapping file (`--map`). The mapping a
//! run used is written beside the report in the same format, so passing it
//! back reproduces the run.
//!
//! ```toml
//! [tables."Resistors"]
//! "Part Number" = "[Key]"
//! "Library Ref" = "[Library Ref]"
//! "Maker"       = "Manufacturer"      # recognised as a manufacturer
//! "Internal"    = "[Skip]"            # not imported
//! ```

use std::collections::BTreeMap;

use super::parse::Table;
use super::source::TableData;

pub const KEY: &str = "[Key]";
pub const SKIP: &str = "[Skip]";

/// table name -> column name -> target.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MapFile {
    pub tables: BTreeMap<String, BTreeMap<String, String>>,
}

impl MapFile {
    pub fn parse(text: &str) -> Result<Self, String> {
        let v: toml::Value = text.parse().map_err(|e| format!("mapping file: {e}"))?;
        let mut out = MapFile::default();
        let Some(tables) = v.get("tables") else {
            return Ok(out);
        };
        let tables = tables
            .as_table()
            .ok_or("mapping file: `tables` must be a table")?;
        for (t, cols) in tables {
            let cols = cols
                .as_table()
                .ok_or_else(|| format!("mapping file: tables.{t} must be a table"))?;
            let mut m = BTreeMap::new();
            for (c, target) in cols {
                let target = target
                    .as_str()
                    .ok_or_else(|| format!("mapping file: {t}.{c} must be a string"))?;
                m.insert(c.clone(), target.to_string());
            }
            out.tables.insert(t.clone(), m);
        }
        Ok(out)
    }

    pub fn to_toml(&self) -> String {
        let mut s = String::from(
            "# The column mapping this run used; pass it back with --map to repeat the run.\n# A target is [Key], [Skip], a reserved name such as [Library Ref], or the name\n# of the part parameter the column fills.\n",
        );
        for (t, cols) in &self.tables {
            s.push_str(&format!("\n[tables.{}]\n", quote(t)));
            for (c, target) in cols {
                s.push_str(&format!("{} = {}\n", quote(c), quote(target)));
            }
        }
        s
    }

    fn lookup(&self, table: &str, column: &str) -> Option<&String> {
        self.tables
            .iter()
            .find(|(t, _)| t.eq_ignore_ascii_case(table))
            .and_then(|(_, cols)| {
                cols.iter()
                    .find(|(c, _)| c.eq_ignore_ascii_case(column))
                    .map(|(_, v)| v)
            })
    }
}

fn quote(s: &str) -> String {
    let mut o = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\t' => o.push_str("\\t"),
            c if c.is_control() => o.push_str(&format!("\\u{:04X}", c as u32)),
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

/// What each column of `data` becomes, in column order: the database's own
/// field map (key, reserved names, parameter names), a column nobody maps
/// being a parameter of its own name, then the user's overrides.
pub fn map_table(table: &Table, data: &TableData, user: &MapFile) -> Vec<String> {
    data.columns
        .iter()
        .map(|col| {
            if let Some(t) = user.lookup(&table.name, col) {
                return t.clone();
            }
            match table
                .fields
                .iter()
                .find(|f| f.column().eq_ignore_ascii_case(col))
            {
                Some(f) if f.is_key() => KEY.to_string(),
                Some(f) if !f.parameter_name.trim().is_empty() => {
                    f.parameter_name.trim().to_string()
                }
                _ => col.clone(),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dblib::parse::FieldMap;

    fn table() -> Table {
        let f = |col: &str, p: &str, ty: i32| FieldMap {
            field_name_only: col.into(),
            parameter_name: p.into(),
            field_type: ty,
            ..Default::default()
        };
        Table {
            name: "R".into(),
            fields: vec![
                f("Part Number", "Part Number", 0),
                f("Lib", "[Library Ref]", 1),
            ],
            ..Default::default()
        }
    }

    #[test]
    fn key_reserved_and_unmapped_columns() {
        let data = TableData {
            columns: vec!["Part Number".into(), "Lib".into(), "Value".into()],
            rows: vec![],
        };
        let m = map_table(&table(), &data, &MapFile::default());
        assert_eq!(m, [KEY, "[Library Ref]", "Value"]);
    }

    #[test]
    fn the_user_map_overrides_and_the_written_map_reproduces_the_run() {
        let data = TableData {
            columns: vec!["Part Number".into(), "Lib".into(), "Maker".into()],
            rows: vec![],
        };
        let user =
            MapFile::parse("[tables.\"r\"]\nmaker = \"Manufacturer\"\nLib = \"[Skip]\"\n").unwrap();
        let m = map_table(&table(), &data, &user);
        assert_eq!(m, [KEY, SKIP, "Manufacturer"]);
        let mut used = MapFile::default();
        used.tables.insert(
            "R".into(),
            data.columns
                .iter()
                .cloned()
                .zip(m.iter().cloned())
                .collect(),
        );
        let again = MapFile::parse(&used.to_toml()).unwrap();
        assert_eq!(again, used);
        assert_eq!(map_table(&table(), &data, &again), m);
    }

    #[test]
    fn a_bad_mapping_file_is_an_error() {
        assert!(MapFile::parse("tables = 3").is_err());
        assert!(MapFile::parse("[tables.x]\na = 1\n").is_err());
        assert!(MapFile::parse("not toml [").is_err());
    }
}
