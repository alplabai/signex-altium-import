//! Building part rows: shared by parts that come from schematic library
//! components and parts that come from database rows.

use std::collections::{BTreeMap, HashMap};

use altium_format::sch::Component;
use chrono::{DateTime, Utc};
use serde_json::json;
use signex_library::component::{ComponentRow, DESIGNATOR_PREFIX_PARAM, PlmReserved};
use signex_library::param::ParamValue;
use signex_library::templates::TemplateRegistry;
use signex_library::{ComponentClass, InternalPn, LibraryRow, LifecycleState, PrimitiveRef};
use uuid::Uuid;

use super::bind::{FootprintIndex, bind};
use super::snxlib::row_cells;
use super::{LibraryOptions, VERSION, bump_patch, classes, fields};
use crate::report::{Item, ItemKind, Report, Severity};

/// What building a part reads but never changes.
pub struct PartCx<'a> {
    pub lib_id: Uuid,
    pub opts: &'a LibraryOptions,
    pub stamp: DateTime<Utc>,
    pub templates: &'a TemplateRegistry,
    pub index: &'a FootprintIndex,
    pub footprint_versions: &'a HashMap<Uuid, String>,
}

/// One part to build.
pub struct PartInput<'a> {
    pub comp: &'a Component,
    /// `<library>:<name>` in the report.
    pub source_name: String,
    pub row_id: Uuid,
    pub symbol_id: Uuid,
    pub symbol_version: String,
    pub released: bool,
    /// The table the row goes in; the class's table when `None`.
    pub table: Option<&'a str>,
    /// The part number before collision suffixes.
    pub pn: &'a str,
}

/// The rows and counters of the parts built so far.
#[derive(Default)]
pub struct Parts {
    taken_pns: HashMap<String, usize>,
    pub rows: BTreeMap<String, Vec<LibraryRow>>,
    pub classes: BTreeMap<String, &'static str>,
    pub written: usize,
    pub row_ids: Vec<Uuid>,
}

impl Parts {
    /// Build one part; `false` when it was not written (the report says why).
    pub fn add(&mut self, cx: &PartCx, report: &mut Report, input: PartInput) -> bool {
        let comp = input.comp;
        report.set_scope_with_id(input.source_name.clone(), input.row_id);
        for _ in 0..comp.ansi_pipe_assumed {
            report.warn(
                "part.ansi_pipe_assumed",
                "a value holds byte 0x8E with no UTF-8 form; read as an escaped | (a real \u{17D} would read the same)",
            );
        }
        let Some(binding) = bind(comp, cx.index, report) else {
            // The error item says why; the part is not written.
            report.set_scope("");
            return false;
        };

        let prefix =
            classes::designator_prefix(comp.designator.as_ref().map_or("", |d| d.text.as_str()));
        let class = classes::class_for_prefix(&prefix);
        let template = cx
            .templates
            .resolve(Uuid::nil(), class)
            .or_else(|| cx.templates.resolve(Uuid::nil(), "generic"));
        let f = fields::map_fields(comp, template, report);
        let mut parameters = f.parameters;
        if !prefix.is_empty() {
            parameters.insert(DESIGNATOR_PREFIX_PARAM.into(), ParamValue::Text(prefix));
        }
        if let Some(t) = template {
            let missing: Vec<&str> = t
                .required_params
                .iter()
                .filter(|p| !parameters.contains_key(&p.name))
                .map(|p| p.name.as_str())
                .collect();
            if !missing.is_empty() {
                report.warn_detail(
                    "part.template_incomplete",
                    format!("the {class} template wants {}", missing.join(", ")),
                    json!({ "class": class, "missing": missing }),
                );
            }
        }

        // Part number: the name, `~2`, `~3` for repeats.
        let (base, cleaned) = crate::text::cell_safe(input.pn);
        if cleaned {
            report.warn(
                "part.text_sanitized",
                "control characters or a triple apostrophe in the part name replaced",
            );
        }
        // The number, or the first `~n` nobody holds yet (a generated number
        // is taken too, so it cannot collide with a later source name).
        let mut n = *self.taken_pns.entry(base.clone()).or_default() + 1;
        let mut pn = if n == 1 {
            base.clone()
        } else {
            format!("{base}~{n}")
        };
        while n > 1 && self.taken_pns.contains_key(&pn) {
            n += 1;
            pn = format!("{base}~{n}");
        }
        self.taken_pns.insert(base.clone(), n);
        self.taken_pns.entry(pn.clone()).or_insert(1);
        if n > 1 {
            report.warn_detail(
                "part.pn_collision",
                format!("part number {base} is taken; this part is {pn}"),
                json!({ "part_number": pn, "name": base }),
            );
        }

        let mut row = ComponentRow {
            row_id: input.row_id,
            internal_pn: InternalPn::new(pn),
            class: ComponentClass::new(class),
            datasheet: Default::default(),
            datasheet_label: String::new(),
            description: f.description,
            more_datasheets: Vec::new(),
            state: if input.released {
                LifecycleState::Released
            } else {
                LifecycleState::Draft
            },
            symbol_ref: PrimitiveRef::new(cx.lib_id, input.symbol_id),
            alternate_symbols: Vec::new(),
            footprint_ref: binding.default.map(|id| PrimitiveRef::new(cx.lib_id, id)),
            alternate_footprints: binding.alternate_bindings(cx.lib_id, cx.footprint_versions),
            sim_ref: None,
            pin_map_overrides: binding.default_overrides,
            primary_mpn: f.primary,
            alternates: f.alternates,
            supply: f.supply,
            parameters,
            plm: PlmReserved::default(),
            version: VERSION.into(),
            released: input.released,
            symbol_version: input.symbol_version,
            footprint_version: binding
                .default
                .and_then(|id| cx.footprint_versions.get(&id).cloned())
                .unwrap_or_default(),
            sim_version: String::new(),
            created: cx.stamp,
            updated: cx.stamp,
            content_hash: [0; 32],
            server_revision: None,
        };
        row.set_documents(f.docs);
        let previous = cx.opts.previous.get(&input.row_id);
        if let Some(p) = previous {
            row.state = p.state;
            row.released = p.released;
            row.created = p.created;
            row.version = p.version.clone();
            row.updated = p.updated;
        }
        let made = signex_library::validate::validate_row_for_write(&row)
            .map_err(|e| super::BuildError::Row(e.to_string()))
            .and_then(|()| {
                row.content_hash = signex_library::hash_row_content(&row)
                    .map_err(|e| super::BuildError::Row(e.to_string()))?;
                if previous.is_some_and(|p| {
                    p.content_hash != row.content_hash
                        || p.symbol_version != row.symbol_version
                        || p.footprint_version != row.footprint_version
                }) {
                    row.version = bump_patch(&row.version);
                    row.updated = cx.opts.now;
                }
                row_cells(&row)
            });
        report.set_scope("");
        match made {
            Ok(cells) => {
                self.classes
                    .insert(class.to_string(), classes::class_label(class));
                let table = input
                    .table
                    .map_or_else(|| class.to_string(), str::to_string);
                self.rows.entry(table).or_default().push(cells);
                self.written += 1;
                self.row_ids.push(input.row_id);
                true
            }
            Err(e) => {
                report.push(Item {
                    kind: ItemKind::Part,
                    source: input.source_name,
                    id: None,
                    code: "part.row_invalid",
                    severity: Severity::Error,
                    message: format!("part not written: {e}"),
                    detail: serde_json::Value::Null,
                });
                false
            }
        }
    }
}
