//! The database library file (`.DbLib`): an INI file with the connection
//! (`[DatabaseLinks]`), one `[TableN]` per table and one `[FieldMapN]` per
//! column mapping (`Options=` holds `key=value` pairs joined by `|`).
//! Layout observed in a private corpus file; see `docs/FORMAT_NOTES.md`.

use std::collections::BTreeMap;

/// How the database is reached, from `[DatabaseLinks]`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Links {
    pub connection_string: String,
    /// `LibraryDatabaseType`, for example `Microsoft Access`.
    pub database_type: String,
    /// `LibraryDatabasePath`: the database file, absolute or relative.
    pub database_path: String,
    /// `DatabasePathRelative`: the path is relative to the `.DbLib`.
    pub path_relative: bool,
    /// `LibrarySearchPath`, split at `;`.
    pub search_path: Vec<String>,
    /// `SearchSubDirectories`.
    pub search_subdirs: bool,
    pub left_quote: String,
    pub right_quote: String,
    pub quote_table_names: bool,
}

/// One column mapping of a table.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FieldMap {
    /// `FieldName`: `<table>.<column>`.
    pub field_name: String,
    pub table_name_only: String,
    pub field_name_only: String,
    /// `ParameterName`: a part parameter, or a reserved name in brackets.
    pub parameter_name: String,
    /// `FieldType`: 0 marks the key field.
    pub field_type: i32,
    pub visible_on_add: bool,
    pub add_mode: String,
    pub update_mode: String,
    pub remove_mode: String,
}

impl FieldMap {
    pub fn is_key(&self) -> bool {
        self.field_type == 0
    }

    /// The column name: `FieldNameOnly`, else what follows the table in
    /// `FieldName`.
    pub fn column(&self) -> &str {
        if !self.field_name_only.is_empty() {
            &self.field_name_only
        } else {
            self.field_name
                .strip_prefix(&format!("{}.", self.table_name_only))
                .or_else(|| self.field_name.split_once('.').map(|x| x.1))
                .unwrap_or(&self.field_name)
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Table {
    pub name: String,
    pub schema: String,
    pub enabled: bool,
    /// `UserWhere`: the user's filter is in effect.
    pub user_where: bool,
    pub user_where_text: String,
    pub fields: Vec<FieldMap>,
}

impl Table {
    pub fn key(&self) -> Option<&FieldMap> {
        self.fields.iter().find(|f| f.is_key())
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct DbLib {
    pub links: Links,
    pub tables: Vec<Table>,
}

#[derive(Debug, thiserror::Error)]
pub enum DbLibError {
    #[error("not a database library: no [DatabaseLinks] section")]
    NoLinks,
}

fn flag(v: &str) -> bool {
    matches!(v.trim(), "1" | "true" | "True" | "TRUE" | "-1")
}

/// `[Name12]` as (`Name`, 12), or (`Name`, 0) without digits.
fn split_section(name: &str) -> (&str, usize) {
    let stem = name.trim_end_matches(|c: char| c.is_ascii_digit());
    (stem, name[stem.len()..].parse().unwrap_or(0))
}

fn options(line: &str) -> BTreeMap<String, String> {
    line.split('|')
        .filter_map(|kv| kv.split_once('='))
        .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
        .collect()
}

/// Parse a `.DbLib`. CRLF, a byte-order mark and unknown keys are tolerated.
pub fn parse(text: &str) -> Result<DbLib, DbLibError> {
    let text = text.trim_start_matches('\u{feff}');
    let mut sections: Vec<(String, Vec<(String, String)>)> = Vec::new();
    for raw in text.lines() {
        let line = raw.trim_end_matches('\r').trim();
        if line.is_empty() || line.starts_with(';') {
            continue;
        }
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            sections.push((name.to_string(), Vec::new()));
        } else if let Some((k, v)) = line.split_once('=')
            && let Some((_, kv)) = sections.last_mut()
        {
            kv.push((k.trim().to_string(), v.to_string()));
        }
    }
    let get = |kv: &[(String, String)], k: &str| -> String {
        kv.iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(k))
            .map(|(_, v)| v.trim().to_string())
            .unwrap_or_default()
    };
    let links = sections
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case("DatabaseLinks"))
        .map(|(_, kv)| Links {
            connection_string: get(kv, "ConnectionString"),
            database_type: get(kv, "LibraryDatabaseType"),
            database_path: get(kv, "LibraryDatabasePath"),
            path_relative: flag(&get(kv, "DatabasePathRelative")),
            search_path: get(kv, "LibrarySearchPath")
                .split(';')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect(),
            search_subdirs: flag(&get(kv, "SearchSubDirectories")),
            left_quote: get(kv, "LeftQuote"),
            right_quote: get(kv, "RightQuote"),
            quote_table_names: flag(&get(kv, "QuoteTableNames")),
        })
        .ok_or(DbLibError::NoLinks)?;

    let mut tables: Vec<(usize, Table)> = Vec::new();
    let mut maps: Vec<(usize, FieldMap)> = Vec::new();
    for (name, kv) in &sections {
        let (stem, n) = split_section(name);
        if stem.eq_ignore_ascii_case("Table") {
            tables.push((
                n,
                Table {
                    name: get(kv, "TableName"),
                    schema: get(kv, "SchemaName"),
                    enabled: flag(&get(kv, "Enabled")),
                    user_where: flag(&get(kv, "UserWhere")),
                    user_where_text: get(kv, "UserWhereText"),
                    fields: Vec::new(),
                },
            ));
        } else if stem.eq_ignore_ascii_case("FieldMap") {
            let o = options(&get(kv, "Options"));
            let g = |k: &str| o.get(k).cloned().unwrap_or_default();
            maps.push((
                n,
                FieldMap {
                    field_name: g("FieldName"),
                    table_name_only: g("TableNameOnly"),
                    field_name_only: g("FieldNameOnly"),
                    parameter_name: g("ParameterName"),
                    field_type: g("FieldType").parse().unwrap_or(-1),
                    visible_on_add: flag(&g("VisibleOnAdd")),
                    add_mode: g("AddMode"),
                    update_mode: g("UpdateMode"),
                    remove_mode: g("RemoveMode"),
                },
            ));
        }
    }
    tables.sort_by_key(|(n, _)| *n);
    maps.sort_by_key(|(n, _)| *n);
    let mut tables: Vec<Table> = tables.into_iter().map(|(_, t)| t).collect();
    for (_, m) in maps {
        let owner = tables.iter_mut().find(|t| {
            t.name.eq_ignore_ascii_case(&m.table_name_only)
                || m.field_name
                    .to_lowercase()
                    .starts_with(&format!("{}.", t.name.to_lowercase()))
        });
        if let Some(t) = owner {
            t.fields.push(m);
        }
    }
    Ok(DbLib { links, tables })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\u{feff}[OutputDatabaseLinkFile]\r\nVersion=1\r\n[DatabaseLinks]\r\nConnectionString=Provider=X;Data Source=C:\\data\\parts.mdb\r\nLibraryDatabaseType=Microsoft Access\r\nLibraryDatabasePath=parts.mdb\r\nDatabasePathRelative=1\r\nLibrarySearchPath=Footprints;Symbols\r\nSearchSubDirectories=1\r\nFuture=1\r\n[Table1]\r\nSchemaName=\r\nTableName=Resistors\r\nEnabled=1\r\nUserWhere=0\r\nUserWhereText=\r\n[Table2]\r\nTableName=Hidden\r\nEnabled=0\r\n[FieldMap1]\r\nOptions=FieldName=Resistors.Part Number|TableNameOnly=Resistors|FieldNameOnly=Part Number|FieldType=0|ParameterName=Part Number|VisibleOnAdd=True|AddMode=1|RemoveMode=0|UpdateMode=2\r\n[FieldMap2]\r\nOptions=FieldName=Resistors.Library Ref|TableNameOnly=Resistors|FieldNameOnly=Library Ref|FieldType=1|ParameterName=[Library Ref]\r\n[FieldMap3]\r\nOptions=FieldName=Hidden.Id|TableNameOnly=Hidden|FieldNameOnly=Id|FieldType=0|ParameterName=Id\r\n";

    #[test]
    fn parses_links_tables_and_the_key_field() {
        let d = parse(SAMPLE).unwrap();
        assert_eq!(d.links.database_type, "Microsoft Access");
        assert!(d.links.path_relative && d.links.search_subdirs);
        assert_eq!(d.links.search_path, ["Footprints", "Symbols"]);
        assert_eq!(d.tables.len(), 2);
        let r = &d.tables[0];
        assert!(r.enabled && !d.tables[1].enabled);
        assert_eq!(r.fields.len(), 2);
        assert_eq!(r.key().unwrap().column(), "Part Number");
        assert_eq!(r.fields[1].parameter_name, "[Library Ref]");
        assert_eq!(d.tables[1].key().unwrap().column(), "Id");
    }

    #[test]
    fn a_file_without_links_is_refused() {
        assert!(parse("[Table1]\nTableName=x\n").is_err());
    }

    #[test]
    fn the_column_falls_back_to_the_field_name() {
        let m = FieldMap {
            field_name: "T.My Col".into(),
            table_name_only: "T".into(),
            ..Default::default()
        };
        assert_eq!(m.column(), "My Col");
    }
}
