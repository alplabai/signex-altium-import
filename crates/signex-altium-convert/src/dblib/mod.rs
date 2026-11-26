//! Database libraries (`.DbLib`): parse, read rows, map columns, resolve
//! library paths. The conversion itself is `libset` with a `DatabaseSet`.

pub mod build;
pub mod map;
pub mod parse;
pub mod resolve;
pub mod source;

pub use build::{PrepareInput, Prepared, prepare};
pub use map::MapFile;
pub use parse::{DbLib, FieldMap, Links, Table, parse as parse_dblib};
pub use resolve::Resolver;
pub use source::{DbSource, SourceError, TableData};
