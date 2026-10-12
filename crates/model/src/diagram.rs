//! ER diagram layout: deterministic boxes and FK edges, no physics, no randomness.

use crate::Catalog;

const CHAR_W: f32 = 7.0;
const ROW_H: f32 = 18.0;
const HEADER_H: f32 = 24.0;
const PAD: f32 = 12.0;
const GAP_X: f32 = 60.0;
const GAP_Y: f32 = 40.0;
const MIN_W: f32 = 140.0;
const MAX_COLS: usize = 32;

#[derive(Debug, Clone, PartialEq)]
pub struct DiagramNode {
    pub key: String,
    pub schema: String,
    pub table: String,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    /// Column labels drawn in the box, truncated to a sane number.
    pub columns: Vec<String>,
    pub primary_key: Vec<String>,
    pub row_estimate: Option<i64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DiagramEdge {
    pub from: usize,
    pub to: usize,
    pub from_column: String,
    pub to_column: String,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Diagram {
    pub nodes: Vec<DiagramNode>,
    pub edges: Vec<DiagramEdge>,
    pub width: f32,
    pub height: f32,
}

/// Layout one schema (or every schema when `schema_filter` is `None`) as a grid, FK edges included.
pub fn layout(catalog: &Catalog, schema_filter: Option<&str>) -> Diagram {
    let mut keys: Vec<(String, String, &crate::Table)> = Vec::new();
    for sc in &catalog.schemas {
        if let Some(want) = schema_filter {
            if sc.name != want {
                continue;
            }
        }
        for t in &sc.tables {
            if t.kind == crate::TableKind::Table {
                keys.push((t.key(), sc.name.clone(), t));
            }
        }
    }
    // Reference first so FK arrows point left to right most of the time.
    keys.sort_by(|a, b| {
        let ai = a.2.foreign_keys.len();
        let bi = b.2.foreign_keys.len();
        ai.cmp(&bi).then_with(|| a.0.cmp(&b.0))
    });

    let per_row = (keys.len() as f32).sqrt().ceil().max(1.0) as usize;
    let mut nodes = Vec::with_capacity(keys.len());
    let mut row_h = 0.0f32;
    for (i, (key, schema, t)) in keys.iter().enumerate() {
        let shown: Vec<String> = t
            .columns
            .iter()
            .take(MAX_COLS)
            .map(|c| c.name.clone())
            .collect();
        let widest = shown
            .iter()
            .map(|c| c.chars().count())
            .chain(std::iter::once(t.name.chars().count()))
            .max()
            .unwrap_or(8);
        let w = (CHAR_W * (widest as f32 + 3.0)).max(MIN_W);
        let h = HEADER_H + (shown.len() as f32 * ROW_H) + PAD;
        let col = i % per_row;
        let row = i / per_row;
        let x = PAD + col as f32 * (w + GAP_X);
        let y = PAD + row as f32 * (h + GAP_Y);
        row_h = row_h.max(h);
        nodes.push(DiagramNode {
            key: key.clone(),
            schema: schema.clone(),
            table: t.name.clone(),
            x,
            y,
            w,
            h,
            columns: shown,
            primary_key: t.primary_key(),
            row_estimate: t.row_estimate,
        });
    }

    let mut edges = Vec::new();
    for (i, (_, schema, t)) in keys.iter().enumerate() {
        for fk in &t.foreign_keys {
            let target_key = match &fk.ref_schema {
                Some(s) if !s.is_empty() => format!("{s}.{}", fk.ref_table),
                _ => format!("{schema}.{}", fk.ref_table),
            };
            let target = nodes
                .iter()
                .position(|n| n.key == target_key)
                .or_else(|| nodes.iter().position(|n| n.table == fk.ref_table));
            if let Some(to) = target {
                edges.push(DiagramEdge {
                    from: i,
                    to,
                    from_column: fk.columns.first().cloned().unwrap_or_default(),
                    to_column: fk.ref_columns.first().cloned().unwrap_or_default(),
                    label: fk.name.clone(),
                });
            }
        }
    }

    let width = nodes.iter().map(|n| n.x + n.w + PAD).fold(0.0f32, f32::max);
    let height = nodes.iter().map(|n| n.y + n.h + PAD).fold(0.0f32, f32::max);
    let _ = row_h;
    Diagram {
        nodes,
        edges,
        width,
        height,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Column, ForeignKey, Schema, Table, TableKind};

    fn two_tables() -> Catalog {
        let mut users = Table::new(Some("public".into()), "users");
        let mut id = Column::new("id", "int");
        id.primary_key = true;
        users.columns.push(id);
        users.columns.push(Column::new("email", "text"));
        let mut posts = Table::new(Some("public".into()), "posts");
        posts.columns.push(Column::new("id", "int"));
        posts.columns.push(Column::new("author_id", "int"));
        posts.foreign_keys.push(ForeignKey {
            name: "fk_posts_author".into(),
            columns: vec!["author_id".into()],
            ref_schema: Some("public".into()),
            ref_table: "users".into(),
            ref_columns: vec!["id".into()],
            ..ForeignKey::default()
        });
        let mut view = Table::new(Some("public".into()), "v_users");
        view.kind = TableKind::View;
        Catalog {
            schemas: vec![Schema {
                name: "public".into(),
                tables: vec![users, posts, view],
            }],
            ..Catalog::default()
        }
    }

    #[test]
    fn layout_is_deterministic_and_skips_views() {
        let c = two_tables();
        let a = layout(&c, Some("public"));
        let b = layout(&c, Some("public"));
        assert_eq!(a, b);
        assert_eq!(a.nodes.len(), 2);
        assert_eq!(a.edges.len(), 1);
        assert!(a.width > 0.0 && a.height > 0.0);
        assert!(a.nodes.iter().all(|n| n.x >= 0.0 && n.y >= 0.0));
        assert!(layout(&c, Some("nope")).nodes.is_empty());
    }

    #[test]
    fn edges_point_at_the_referenced_box() {
        let c = two_tables();
        let d = layout(&c, None);
        let e = &d.edges[0];
        assert_eq!(d.nodes[e.to].table, "users");
        assert_eq!(e.from_column, "author_id");
        assert_eq!(e.to_column, "id");
    }
}
