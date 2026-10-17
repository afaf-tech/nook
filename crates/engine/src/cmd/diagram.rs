//! ER diagram commands: layout for the canvas, and SVG export for sharing.

use nook_core::{Error, Result};
use serde_json::json;

use crate::CommandSpec;

pub fn commands() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("diagram.erDiagram", "ER diagram", "Modeler", er_diagram)
            .params(r#"{"conn":"local","schema":"public"}"#)
            .help("Boxes per table, arrows per foreign key, laid out deterministically."),
        CommandSpec::new(
            "diagram.exportSvg",
            "Export the diagram as SVG",
            "Modeler",
            export_svg,
        )
        .params(r#"{"path":"~/diagrams/schema.svg","schema":"public"}"#),
        CommandSpec::new("diagram.tableCard", "Table card", "Modeler", table_card)
            .params(r#"{"table":"public.users"}"#)
            .help("One table's columns, keys and relationships, for the diagram's side panel."),
    ]
}

fn er_diagram(session: &mut crate::Session, p: &crate::Params) -> Result<serde_json::Value> {
    let index = session.resolve_conn(p)?;
    let schema = p.str_opt("schema");
    let catalog = session.catalog_of(index)?;
    let diagram = nook_model::layout_diagram(catalog, schema.as_deref());
    Ok(json!({
        "nodes": diagram.nodes.iter().map(|n| json!({
            "key": n.key, "schema": n.schema, "table": n.table,
            "x": n.x, "y": n.y, "w": n.w, "h": n.h,
            "columns": n.columns, "primary_key": n.primary_key, "rows": n.row_estimate,
        })).collect::<Vec<_>>(),
        "edges": diagram.edges.iter().map(|e| json!({
            "from": e.from, "to": e.to, "from_column": e.from_column,
            "to_column": e.to_column, "label": e.label,
        })).collect::<Vec<_>>(),
        "width": diagram.width, "height": diagram.height,
    }))
}

fn export_svg(session: &mut crate::Session, p: &crate::Params) -> Result<serde_json::Value> {
    let index = session.resolve_conn(p)?;
    let schema = p.str_opt("schema");
    let (diagram, title) = {
        let catalog = session.catalog_of(index)?;
        let title = format!(
            "{}{}",
            catalog
                .database
                .clone()
                .unwrap_or_else(|| "database".into()),
            schema
                .as_deref()
                .map(|s| format!(" · {s}"))
                .unwrap_or_default()
        );
        (
            nook_model::layout_diagram(catalog, schema.as_deref()),
            title,
        )
    };
    if diagram.nodes.is_empty() {
        return Err(Error::not_found("this connection has no tables to draw"));
    }
    let svg = to_svg(&diagram, &title);
    let path = crate::cmd::grid::expand(
        &p.str_opt("path")
            .unwrap_or_else(|| "diagrams/schema.svg".into()),
        &session.home,
    );
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| Error::io(format!("create {}: {e}", parent.display())))?;
    }
    std::fs::write(&path, &svg).map_err(|e| Error::io(format!("write {}: {e}", path.display())))?;
    session.log(
        crate::Level::Success,
        format!("diagram written: {}", path.display()),
    );
    Ok(json!({
        "path": path.to_string_lossy(),
        "bytes": svg.len(),
        "tables": diagram.nodes.len(),
        "relationships": diagram.edges.len(),
    }))
}

fn table_card(session: &mut crate::Session, p: &crate::Params) -> Result<serde_json::Value> {
    let index = session.resolve_conn(p)?;
    let reference = crate::TableRef::parse(&p.str_arg("table")?);
    let table = session.table_of(index, reference.schema.as_deref(), &reference.table)?;
    let catalog = session.catalog_of(index)?;
    // Which tables point at this one, and which it points at.
    let mut referenced_by: Vec<serde_json::Value> = Vec::new();
    for schema in &catalog.schemas {
        for other in &schema.tables {
            for fk in &other.foreign_keys {
                if fk.ref_table == table.name {
                    referenced_by.push(json!({
                        "table": other.key(), "columns": fk.columns, "on_delete": fk.on_delete,
                    }));
                }
            }
        }
    }
    Ok(json!({
        "key": table.key(),
        "kind": table.kind.label(),
        "comment": table.comment,
        "row_estimate": table.row_estimate,
        "size_bytes": table.size_bytes,
        "primary_key": table.primary_key(),
        "columns": table.columns.iter().map(|c| json!({
            "name": c.name, "type": c.type_name, "nullable": c.nullable,
            "primary_key": c.primary_key, "default": c.default,
        })).collect::<Vec<_>>(),
        "indexes": table.indexes.iter().map(|i| json!({
            "name": i.name, "columns": i.columns, "unique": i.unique,
        })).collect::<Vec<_>>(),
        "references": table.foreign_keys.iter().map(|f| json!({
            "name": f.name, "columns": f.columns, "ref_table": f.ref_table, "ref_columns": f.ref_columns,
        })).collect::<Vec<_>>(),
        "referenced_by": referenced_by,
    }))
}

/// Deterministic SVG: no fonts beyond the generic families, no scripts, no external references.
pub fn to_svg(diagram: &nook_model::Diagram, title: &str) -> String {
    const HEADER: f32 = 24.0;
    const ROW: f32 = 18.0;
    let mut out = String::with_capacity(4096);
    let width = diagram.width.max(320.0) + 20.0;
    let height = diagram.height.max(120.0) + 20.0;
    out.push_str(&format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{width:.0}\" height=\"{height:.0}\" viewBox=\"0 0 {width:.0} {height:.0}\" font-family=\"ui-sans-serif, system-ui, sans-serif\">\n"
    ));
    out.push_str("<rect width=\"100%\" height=\"100%\" fill=\"#11151c\"/>\n");
    out.push_str(&format!(
        "<text x=\"14\" y=\"22\" fill=\"#c8d3e0\" font-size=\"14\" font-weight=\"600\">{}</text>\n",
        escape(title)
    ));
    for edge in &diagram.edges {
        let (Some(from), Some(to)) = (diagram.nodes.get(edge.from), diagram.nodes.get(edge.to))
        else {
            continue;
        };
        let x1 = from.x + from.w;
        let y1 = from.y + HEADER + ROW * 0.6;
        let x2 = to.x;
        let y2 = to.y + HEADER + ROW * 0.6;
        let mid = (x1 + x2) / 2.0;
        out.push_str(&format!(
            "<path d=\"M {x1:.0} {y1:.0} C {mid:.0} {y1:.0}, {mid:.0} {y2:.0}, {x2:.0} {y2:.0}\" fill=\"none\" stroke=\"#4f7cff\" stroke-width=\"1.5\" marker-end=\"url(#arrow)\"/>\n"
        ));
    }
    out.push_str("<defs><marker id=\"arrow\" markerWidth=\"8\" markerHeight=\"8\" refX=\"7\" refY=\"3\" orient=\"auto\"><path d=\"M0,0 L7,3 L0,6 z\" fill=\"#4f7cff\"/></marker></defs>\n");
    for node in &diagram.nodes {
        out.push_str(&format!(
            "<g><rect x=\"{x:.0}\" y=\"{y:.0}\" width=\"{w:.0}\" height=\"{h:.0}\" rx=\"4\" fill=\"#1a2230\" stroke=\"#33415a\"/>\n",
            x = node.x,
            y = node.y,
            w = node.w,
            h = node.h
        ));
        out.push_str(&format!(
            "<rect x=\"{x:.0}\" y=\"{y:.0}\" width=\"{w:.0}\" height=\"{HEADER:.0}\" rx=\"4\" fill=\"#24304a\"/>\n",
            x = node.x,
            y = node.y,
            w = node.w
        ));
        out.push_str(&format!(
            "<text x=\"{:.0}\" y=\"{:.0}\" fill=\"#e6eefc\" font-size=\"12\" font-weight=\"600\">{}</text>\n",
            node.x + 8.0,
            node.y + 16.0,
            escape(&node.table)
        ));
        for (i, column) in node.columns.iter().enumerate() {
            let key = if node.primary_key.iter().any(|k| k == column) {
                " (PK)"
            } else {
                ""
            };
            out.push_str(&format!(
                "<text x=\"{:.0}\" y=\"{:.0}\" fill=\"#9fb0c8\" font-size=\"11\">{}</text>\n",
                node.x + 8.0,
                node.y + HEADER + ROW * (i as f32 + 0.8),
                escape(&format!("{column}{key}"))
            ));
        }
        out.push_str("</g>\n");
    }
    out.push_str("</svg>\n");
    out
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use nook_model::{Column, ForeignKey, Schema, Table};

    fn catalog() -> nook_model::Catalog {
        let mut users = Table::new(None, "users");
        let mut id = Column::new("id", "int");
        id.primary_key = true;
        users.columns.push(id);
        users.columns.push(Column::new("email", "text"));
        let mut posts = Table::new(None, "posts");
        posts.columns.push(Column::new("id", "int"));
        posts.columns.push(Column::new("author_id", "int"));
        posts.foreign_keys.push(ForeignKey {
            name: "fk".into(),
            columns: vec!["author_id".into()],
            ref_table: "users".into(),
            ref_columns: vec!["id".into()],
            ..ForeignKey::default()
        });
        nook_model::Catalog {
            database: Some("shop".into()),
            schemas: vec![Schema {
                name: "main".into(),
                tables: vec![users, posts],
                ..Default::default()
            }],
            ..nook_model::Catalog::default()
        }
    }

    #[test]
    fn svg_contains_boxes_edges_and_no_script() {
        let diagram = nook_model::layout_diagram(&catalog(), None);
        let svg = to_svg(&diagram, "shop · main");
        assert!(svg.starts_with("<svg"));
        assert!(svg.ends_with("</svg>\n"));
        assert!(svg.contains("users"));
        assert!(svg.contains("author_id"));
        assert!(svg.contains("marker-end"));
        assert!(!svg.contains("<script"));
        assert!(!svg.contains("http://") || svg.contains("xmlns=\"http://www.w3.org/2000/svg\""));
        assert_eq!(svg.matches("<g>").count(), 2);
    }

    #[test]
    fn svg_escapes_hostile_names() {
        let mut catalog = catalog();
        if let Some(table) = catalog.schemas[0].tables.first_mut() {
            table.name = "<script>alert(1)</script>".into();
        }
        let svg = to_svg(&nook_model::layout_diagram(&catalog, None), "x & y");
        assert!(!svg.contains("<script>alert"));
        assert!(svg.contains("&lt;script&gt;"));
        assert!(svg.contains("x &amp; y"));
    }

    #[test]
    fn diagram_commands_need_a_catalog() {
        let mut session = crate::Session::new();
        assert!(er_diagram(&mut session, &crate::Params::none()).is_err());
        assert!(export_svg(&mut session, &crate::Params::none()).is_err());
        assert!(table_card(&mut session, &crate::Params::none()).is_err());
    }
}
