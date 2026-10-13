//! Charts and aggregations for the report tab: pure data shaping, so the UI only draws.

use nook_core::{quote_ident, quote_qualified, Dialect, Error, Result, Value};
use nook_model::{ResultSet, Table};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ChartKind {
    Bar,
    #[default]
    Column,
    Line,
    Area,
    Pie,
    Scatter,
    Table,
}

impl ChartKind {
    pub fn parse(s: &str) -> Option<ChartKind> {
        match s.to_ascii_lowercase().as_str() {
            "bar" => Some(ChartKind::Bar),
            "column" => Some(ChartKind::Column),
            "line" | "spline" => Some(ChartKind::Line),
            "area" => Some(ChartKind::Area),
            "pie" | "donut" | "doughnut" => Some(ChartKind::Pie),
            "scatter" | "point" => Some(ChartKind::Scatter),
            "table" | "grid" => Some(ChartKind::Table),
            _ => None,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            ChartKind::Bar => "bar",
            ChartKind::Column => "column",
            ChartKind::Line => "line",
            ChartKind::Area => "area",
            ChartKind::Pie => "pie",
            ChartKind::Scatter => "scatter",
            ChartKind::Table => "table",
        }
    }
    pub fn all() -> [ChartKind; 7] {
        [
            ChartKind::Bar,
            ChartKind::Column,
            ChartKind::Line,
            ChartKind::Area,
            ChartKind::Pie,
            ChartKind::Scatter,
            ChartKind::Table,
        ]
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Series {
    pub name: String,
    pub values: Vec<f64>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Chart {
    pub kind: ChartKind,
    pub title: String,
    pub labels: Vec<String>,
    pub series: Vec<Series>,
    pub skipped: usize,
}

impl Chart {
    pub fn max_value(&self) -> f64 {
        self.series
            .iter()
            .flat_map(|s| s.values.iter())
            .cloned()
            .fold(f64::MIN, f64::max)
    }
    pub fn min_value(&self) -> f64 {
        self.series
            .iter()
            .flat_map(|s| s.values.iter())
            .cloned()
            .fold(f64::MAX, f64::min)
    }
    pub fn summary(&self) -> String {
        format!(
            "{} {} · {} labels · {} series",
            self.kind.label(),
            if self.title.is_empty() {
                "(untitled)"
            } else {
                &self.title
            },
            self.labels.len(),
            self.series.len()
        )
    }
}

/// Turn a result set into a chart: `label_column` names the categories, every numeric column after
/// it becomes a series. Rows whose label or values are not usable are counted, not swallowed.
pub fn chart_from_result(
    result: &ResultSet,
    kind: ChartKind,
    title: impl Into<String>,
    label_column: Option<&str>,
) -> Result<Chart> {
    if !result.is_tabular() {
        return Err(Error::config("the query returned no columns to chart"));
    }
    let label_index = match label_column {
        Some(name) => result
            .index_of(name)
            .ok_or_else(|| Error::not_found(format!("no column named '{name}'")))?,
        None => 0,
    };
    let mut labels = Vec::new();
    let mut series: Vec<Series> = Vec::new();
    for (i, name) in result.columns.iter().enumerate() {
        if i == label_index {
            continue;
        }
        series.push(Series {
            name: name.clone(),
            values: Vec::new(),
        });
    }
    let mut skipped = 0usize;
    for row in &result.rows {
        let label = row.get(label_index).map(Value::as_text).unwrap_or_default();
        let mut numeric = Vec::new();
        let mut any = false;
        for (i, _) in result.columns.iter().enumerate() {
            if i == label_index {
                continue;
            }
            let v = row.get(i).cloned().unwrap_or(Value::Null);
            match v.as_f64() {
                Some(n) if !v.is_null() => {
                    numeric.push(n);
                    any = true;
                }
                _ => numeric.push(0.0),
            }
        }
        if !any && numeric.is_empty() {
            skipped += 1;
            continue;
        }
        labels.push(label);
        for (n, value) in numeric.into_iter().enumerate() {
            if let Some(s) = series.get_mut(n) {
                s.values.push(value);
            }
        }
    }
    Ok(Chart {
        kind,
        title: title.into(),
        labels,
        series,
        skipped,
    })
}

/// Which aggregate functions the report builder offers.
pub fn aggregates() -> &'static [&'static str] {
    &["count", "sum", "avg", "min", "max", "count distinct"]
}

/// `SELECT group, agg(value) … GROUP BY group`, the report builder's output.
pub fn aggregate_sql(
    table: &Table,
    group_column: &str,
    aggregate: &str,
    value_column: Option<&str>,
    dialect: Dialect,
    limit: usize,
) -> Result<String> {
    if table.column(group_column).is_none() {
        return Err(Error::not_found(format!(
            "{} has no column '{group_column}'",
            table.name
        )));
    }
    let agg = aggregate.trim().to_ascii_lowercase();
    let expr = if agg == "count" && value_column.is_none() {
        "count(*)".to_string()
    } else {
        let column = value_column.ok_or_else(|| {
            Error::config(format!(
                "'{aggregate}' needs a column, for example --value amount"
            ))
        })?;
        if table.column(column).is_none() {
            return Err(Error::not_found(format!(
                "{} has no column '{column}'",
                table.name
            )));
        }
        let quoted = quote_ident(column, dialect);
        match agg.as_str() {
            "count distinct" | "distinct" => format!("count(distinct {quoted})"),
            "count" | "sum" | "avg" | "min" | "max" => format!("{agg}({quoted})"),
            other => {
                return Err(Error::config(format!(
                    "unknown aggregate '{other}'; try {}",
                    aggregates().join(", ")
                )))
            }
        }
    };
    let group = quote_ident(group_column, dialect);
    let limit = limit.clamp(1, 100_000);
    Ok(format!(
        "SELECT {group} AS label, {expr} AS value FROM {} GROUP BY {group} ORDER BY value DESC LIMIT {limit};",
        quote_qualified(table.schema.as_deref(), &table.name, dialect)
    ))
}

/// A pivoted result: row keys, column keys, and one measure per (row, column).
pub type Pivot = (Vec<String>, Vec<String>, Vec<Vec<Option<f64>>>);

/// Pivot a result set: first column becomes the row key, second the column key, third the measure.
pub fn pivot(result: &ResultSet) -> Result<Pivot> {
    if result.column_count() < 3 {
        return Err(Error::config(
            "a pivot needs three columns: row key, column key, value",
        ));
    }
    let mut rows: Vec<String> = Vec::new();
    let mut cols: Vec<String> = Vec::new();
    let mut cells: Vec<Vec<Option<f64>>> = Vec::new();
    for row in &result.rows {
        let r = row.first().map(Value::as_text).unwrap_or_default();
        let c = row.get(1).map(Value::as_text).unwrap_or_default();
        let v = row.get(2).and_then(Value::as_f64);
        if !rows.contains(&r) {
            rows.push(r.clone());
            cells.push(vec![None; cols.len()]);
        }
        if !cols.contains(&c) {
            cols.push(c.clone());
            for line in cells.iter_mut() {
                line.push(None);
            }
        }
        let ri = rows.iter().position(|x| x == &r).unwrap_or(0);
        let ci = cols.iter().position(|x| x == &c).unwrap_or(0);
        if let Some(line) = cells.get_mut(ri) {
            if let Some(cell) = line.get_mut(ci) {
                *cell = v;
            }
        }
    }
    Ok((rows, cols, cells))
}

#[cfg(test)]
mod tests {
    use super::*;
    use nook_model::{Column, ResultSet, Table};

    fn result() -> ResultSet {
        ResultSet {
            columns: vec!["month".into(), "sales".into(), "refunds".into()],
            column_types: vec!["text".into(), "numeric".into(), "numeric".into()],
            rows: vec![
                vec![Value::text("Jan"), Value::Int(10), Value::Int(1)],
                vec![Value::text("Feb"), Value::Int(20), Value::Null],
                vec![Value::text("Mar"), Value::Int(30), Value::Int(3)],
            ],
            ..ResultSet::default()
        }
    }

    #[test]
    fn chart_keeps_labels_and_every_numeric_column() {
        let c =
            chart_from_result(&result(), ChartKind::Column, "Sales", Some("month")).expect("chart");
        assert_eq!(c.labels, vec!["Jan", "Feb", "Mar"]);
        assert_eq!(c.series.len(), 2);
        assert_eq!(c.series[0].name, "sales");
        assert_eq!(c.series[0].values, vec![10.0, 20.0, 30.0]);
        assert_eq!(c.series[1].values, vec![1.0, 0.0, 3.0]);
        assert_eq!(c.max_value(), 30.0);
        assert!(c.summary().contains("3 labels"));
    }

    #[test]
    fn unknown_label_column_is_an_error() {
        assert!(chart_from_result(&result(), ChartKind::Line, "x", Some("nope")).is_err());
        let empty = ResultSet::default();
        assert!(chart_from_result(&empty, ChartKind::Pie, "x", None).is_err());
    }

    fn orders() -> Table {
        let mut t = Table::new(Some("public".into()), "orders");
        t.columns.push(Column::new("status", "text"));
        t.columns.push(Column::new("amount", "numeric"));
        t
    }

    #[test]
    fn aggregate_sql_is_qualified_and_checked() {
        let t = orders();
        let sql =
            aggregate_sql(&t, "status", "sum", Some("amount"), Dialect::Postgres, 10).expect("sql");
        assert!(sql.contains("SELECT \"status\" AS label, sum(\"amount\") AS value"));
        assert!(sql.contains("FROM \"public\".\"orders\""));
        assert!(sql.ends_with("LIMIT 10;"));
        let sql = aggregate_sql(&t, "status", "count", None, Dialect::MySql, 5).expect("sql");
        assert!(sql.contains("count(*)"));
        assert!(aggregate_sql(&t, "status", "median", Some("amount"), Dialect::Sqlite, 5).is_err());
        assert!(aggregate_sql(&t, "nope", "count", None, Dialect::Sqlite, 5).is_err());
        assert!(aggregate_sql(&t, "status", "sum", Some("nope"), Dialect::Sqlite, 5).is_err());
    }

    #[test]
    fn pivot_shapes_rows_and_columns() {
        let rs = ResultSet {
            columns: vec!["region".into(), "quarter".into(), "total".into()],
            column_types: vec![],
            rows: vec![
                vec![Value::text("East"), Value::text("Q1"), Value::Float(5.0)],
                vec![Value::text("East"), Value::text("Q2"), Value::Float(7.0)],
                vec![Value::text("West"), Value::text("Q1"), Value::Float(2.0)],
            ],
            ..ResultSet::default()
        };
        let (rows, cols, cells) = pivot(&rs).expect("pivot");
        assert_eq!(rows, vec!["East", "West"]);
        assert_eq!(cols, vec!["Q1", "Q2"]);
        assert_eq!(cells[0][1], Some(7.0));
        assert_eq!(cells[1][1], None);
        assert!(pivot(&ResultSet::default()).is_err());
    }

    #[test]
    fn chart_kinds_parse() {
        assert_eq!(ChartKind::parse("Doughnut"), Some(ChartKind::Pie));
        assert!(ChartKind::parse("radar").is_none());
        assert_eq!(ChartKind::all().len(), 7);
    }
}
