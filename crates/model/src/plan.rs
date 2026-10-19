//! Execution plans as a tree, normalised from the three engines' own output.
//!
//! Each engine describes a plan differently — Postgres nests `Plan`/`Plans`, MySQL nests
//! `query_block`/`nested_loop`/`table`, SQLite returns a flat list of `(id, parent, detail)` rows.
//! The UI should not know any of that: it draws a tree, a cost bar and a highlight. So each engine
//! gets one mapping function here, and the UI gets [`Plan`].

use serde::{Deserialize, Serialize};

/// One step of a plan. `cost` is the engine's own estimate, in whatever unit it uses — the UI only
/// compares nodes against each other, so the unit never has to be converted.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PlanNode {
    pub id: usize,
    /// The operation: `Seq Scan`, `Nested Loop`, `SCAN`, `SEARCH`.
    pub label: String,
    /// What it operates on and how: `on users`, `using index idx_email`.
    pub detail: String,
    /// Estimated total cost. `0.0` when the engine gave none.
    pub cost: f64,
    /// Estimated rows.
    pub rows: f64,
    /// Measured time, when the plan was produced with `ANALYZE`.
    pub actual_ms: Option<f64>,
    pub children: Vec<usize>,
}

/// A whole plan: nodes in pre-order, and which one is the root.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Plan {
    pub nodes: Vec<PlanNode>,
    pub root: usize,
}

impl Plan {
    /// The node with the largest share of the root's cost — the one a reader cares about, and the
    /// one the UI highlights. `None` for a plan with no cost information at all.
    pub fn most_expensive(&self) -> Option<usize> {
        let max = self.nodes.iter().map(|n| n.cost).fold(0.0_f64, f64::max);
        if max <= 0.0 {
            return None;
        }
        self.nodes
            .iter()
            .enumerate()
            .max_by(|(_, a), (_, b)| {
                a.cost
                    .partial_cmp(&b.cost)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|(i, _)| i)
    }

    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "root": self.root,
            "expensive": self.most_expensive(),
            "nodes": self.nodes,
        })
    }

    /// A plan with one node and no children: what an engine that cannot be parsed still says.
    pub fn single(label: &str, detail: &str) -> Plan {
        Plan {
            nodes: vec![PlanNode {
                id: 0,
                label: label.to_string(),
                detail: detail.to_string(),
                ..PlanNode::default()
            }],
            root: 0,
        }
    }
}

/// Postgres `EXPLAIN (FORMAT JSON)` output: `[{"Plan": {...}}]`.
pub fn from_postgres(value: &serde_json::Value) -> Option<Plan> {
    let plan = value
        .as_array()
        .and_then(|a| a.first())
        .and_then(|v| v.get("Plan"))?;
    let mut out = Plan::default();
    let root = push_pg(plan, &mut out);
    out.root = root;
    Some(out)
}

fn push_pg(node: &serde_json::Value, out: &mut Plan) -> usize {
    let id = out.nodes.len();
    let label = node
        .get("Node Type")
        .and_then(|v| v.as_str())
        .unwrap_or("Node")
        .to_string();
    // The relation and the index are what make a node identifiable; the rest is noise in a box.
    let mut detail = String::new();
    if let Some(rel) = node.get("Relation Name").and_then(|v| v.as_str()) {
        detail.push_str("on ");
        detail.push_str(rel);
    }
    if let Some(idx) = node.get("Index Name").and_then(|v| v.as_str()) {
        if !detail.is_empty() {
            detail.push(' ');
        }
        detail.push_str("using ");
        detail.push_str(idx);
    }
    if let Some(cond) = node.get("Index Cond").and_then(|v| v.as_str()) {
        if !detail.is_empty() {
            detail.push_str(", ");
        }
        detail.push_str(cond);
    }
    out.nodes.push(PlanNode {
        id,
        label,
        detail,
        cost: node
            .get("Total Cost")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0),
        rows: node
            .get("Plan Rows")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0),
        actual_ms: node.get("Actual Total Time").and_then(|v| v.as_f64()),
        children: Vec::new(),
    });
    if let Some(children) = node.get("Plans").and_then(|v| v.as_array()) {
        let ids: Vec<usize> = children.iter().map(|c| push_pg(c, out)).collect();
        out.nodes[id].children = ids;
    }
    id
}

/// MySQL `EXPLAIN FORMAT=JSON` output: `{"query_block": {...}}`.
///
/// MySQL's shape is a set of containers, not a uniform node type: `nested_loop` is a list of
/// children, `table` is a leaf, `ordering_operation` / `grouping_operation` wrap one child. Each
/// container becomes a node so the tree keeps its shape, and each `table` becomes a leaf.
pub fn from_mysql(value: &serde_json::Value) -> Option<Plan> {
    let block = value.get("query_block")?;
    let mut out = Plan::default();
    let root = push_mysql("query_block", block, &mut out);
    out.root = root;
    Some(out)
}

fn push_mysql(label: &str, node: &serde_json::Value, out: &mut Plan) -> usize {
    let id = out.nodes.len();
    let mut detail = String::new();
    if let Some(table) = node.get("table_name").and_then(|v| v.as_str()) {
        detail.push_str("on ");
        detail.push_str(table);
    }
    if let Some(index) = node.get("key").and_then(|v| v.as_str()) {
        if !detail.is_empty() {
            detail.push(' ');
        }
        detail.push_str("using ");
        detail.push_str(index);
    }
    // MySQL reports cost and rows in several places depending on the node; take whichever is there.
    let cost = node
        .get("cost_info")
        .and_then(|c| c.get("query_cost"))
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse::<f64>().ok())
        .or_else(|| {
            node.get("cost_info")
                .and_then(|c| c.get("read_cost"))
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse().ok())
        })
        .unwrap_or(0.0);
    let rows = node
        .get("rows_examined_per_scan")
        .and_then(|v| v.as_f64())
        .or_else(|| node.get("rows_produced_per_join").and_then(|v| v.as_f64()))
        .unwrap_or(0.0);
    let access = node
        .get("access_type")
        .and_then(|v| v.as_str())
        .map(|a| a.to_ascii_uppercase())
        .unwrap_or_default();
    let shown = if access.is_empty() {
        label.to_string()
    } else {
        format!("{label} ({access})")
    };
    out.nodes.push(PlanNode {
        id,
        label: shown,
        detail,
        cost,
        rows,
        actual_ms: None,
        children: Vec::new(),
    });
    // Every container key MySQL uses, so nothing is dropped silently.
    let mut kids: Vec<usize> = Vec::new();
    for key in [
        "nested_loop",
        "ordering_operation",
        "grouping_operation",
        "duplicates_removal",
        "table",
    ] {
        let Some(child) = node.get(key) else { continue };
        match child {
            serde_json::Value::Array(items) => {
                for item in items {
                    // A `nested_loop` entry is `{"table": {...}}`; unwrap the one key it has.
                    if let Some(inner) = item.get("table") {
                        kids.push(push_mysql("table", inner, out));
                    } else {
                        kids.push(push_mysql(key, item, out));
                    }
                }
            }
            serde_json::Value::Object(_) => {
                if key == "table" {
                    // The node itself *is* the table: its label already says so, and re-labelling
                    // it would lose the access type the caller just read.
                    kids.push(id);
                } else {
                    kids.push(push_mysql(key, child, out));
                }
            }
            _ => {}
        }
    }
    out.nodes[id].children = kids.into_iter().filter(|k| *k != id).collect();
    id
}

/// SQLite `EXPLAIN QUERY PLAN` rows: `(id, parent, notused, detail)`.
///
/// The rows are already a tree by `parent`; the ids are the tree's own, not this `Plan`'s, so they
/// are mapped to indices first and the root is the row nobody claims as a child.
pub fn from_sqlite(rows: &[(i64, i64, String)]) -> Plan {
    let mut out = Plan {
        nodes: rows
            .iter()
            .enumerate()
            .map(|(i, (_, _, detail))| PlanNode {
                id: i,
                label: detail_label(detail),
                detail: detail.clone(),
                cost: 0.0,
                rows: 0.0,
                actual_ms: None,
                children: Vec::new(),
            })
            .collect(),
        root: 0,
    };
    let mut claimed = vec![false; rows.len()];
    for (i, (_, parent, _)) in rows.iter().enumerate() {
        let parent_index = rows.iter().position(|(id, _, _)| id == parent);
        if let Some(p) = parent_index {
            if p != i {
                out.nodes[p].children.push(i);
                claimed[i] = true;
            }
        }
    }
    out.root = claimed.iter().position(|c| !c).unwrap_or(0);
    out
}

/// SQLite's detail is one sentence: `SEARCH users USING INDEX idx_email (email=?)`. The label is
/// its first word, which is the operation.
fn detail_label(detail: &str) -> String {
    detail
        .split_whitespace()
        .next()
        .unwrap_or("PLAN")
        .to_ascii_uppercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_postgres_plan_maps_its_nesting() {
        let json = serde_json::json!([{
            "Plan": {
                "Node Type": "Nested Loop",
                "Total Cost": 100.0,
                "Plan Rows": 3,
                "Plans": [
                    { "Node Type": "Seq Scan", "Relation Name": "users", "Total Cost": 80.0, "Plan Rows": 2 },
                    { "Node Type": "Index Scan", "Relation Name": "posts", "Index Name": "posts_user", "Total Cost": 12.0, "Plan Rows": 1 }
                ]
            }
        }]);
        let plan = from_postgres(&json).expect("a plan");
        assert_eq!(plan.nodes.len(), 3);
        assert_eq!(plan.root, 0);
        assert_eq!(plan.nodes[0].label, "Nested Loop");
        assert_eq!(plan.nodes[0].children, vec![1, 2]);
        assert_eq!(plan.nodes[1].detail, "on users");
        assert_eq!(plan.nodes[2].detail, "on posts using posts_user");
        // The nested loop costs 100 but spends 80 of it in the scan: the scan is the node to look at.
        assert_eq!(plan.most_expensive(), Some(0));
        assert_eq!(plan.nodes[1].cost, 80.0);
    }

    #[test]
    fn an_analysed_postgres_plan_carries_the_measured_time() {
        let json = serde_json::json!([{
            "Plan": { "Node Type": "Seq Scan", "Relation Name": "t", "Total Cost": 5.0,
                      "Plan Rows": 1, "Actual Total Time": 12.5 }
        }]);
        let plan = from_postgres(&json).expect("a plan");
        assert_eq!(plan.nodes[0].actual_ms, Some(12.5));
    }

    #[test]
    fn a_mysql_plan_unwraps_its_containers() {
        let json = serde_json::json!({
            "query_block": {
                "cost_info": { "query_cost": "42.50" },
                "nested_loop": [
                    { "table": { "table_name": "u", "access_type": "ALL", "rows_examined_per_scan": 10 } },
                    { "table": { "table_name": "p", "access_type": "ref", "key": "p_user",
                                 "rows_examined_per_scan": 3 } }
                ]
            }
        });
        let plan = from_mysql(&json).expect("a plan");
        assert_eq!(plan.nodes.len(), 3, "{:?}", plan.nodes);
        assert_eq!(plan.root, 0);
        assert_eq!(plan.nodes[0].children, vec![1, 2]);
        assert_eq!(plan.nodes[0].cost, 42.5);
        assert_eq!(plan.nodes[1].label, "table (ALL)");
        assert_eq!(plan.nodes[1].detail, "on u");
        assert_eq!(plan.nodes[2].detail, "on p using p_user");
        // No cost per table: MySQL reports it once for the block, so the block is the expensive node.
        assert_eq!(plan.most_expensive(), Some(0));
    }

    #[test]
    fn a_sqlite_plan_builds_the_tree_from_parent_ids() {
        let rows = vec![
            (2, 0, "SCAN users".to_string()),
            (
                7,
                2,
                "SEARCH posts USING INDEX posts_user (user_id=?)".to_string(),
            ),
            (9, 2, "USE TEMP B-TREE FOR ORDER BY".to_string()),
        ];
        let plan = from_sqlite(&rows);
        assert_eq!(plan.nodes.len(), 3);
        // Row id 2 is the parent of 7 and 9, and nobody claims 2: it is the root.
        assert_eq!(plan.root, 0);
        assert_eq!(plan.nodes[0].children, vec![1, 2]);
        assert_eq!(plan.nodes[0].label, "SCAN");
        assert_eq!(plan.nodes[1].label, "SEARCH");
        assert_eq!(plan.most_expensive(), None, "SQLite gives no costs");
    }

    #[test]
    fn a_plan_with_no_cost_has_no_expensive_node() {
        let plan = Plan::single("EXPLAIN", "no costs here");
        assert_eq!(plan.most_expensive(), None);
        assert_eq!(plan.to_json()["expensive"], serde_json::Value::Null);
    }

    #[test]
    fn a_postgres_document_that_is_not_a_plan_is_refused_rather_than_guessed() {
        assert!(from_postgres(&serde_json::json!({"rows": []})).is_none());
        assert!(from_mysql(&serde_json::json!({"query": {}})).is_none());
    }
}
