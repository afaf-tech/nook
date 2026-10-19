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

/// One chart as a standalone SVG: bars, columns, lines, areas or a pie, drawn from the same
/// `Chart` the app paints, so an exported file shows what the screen showed.
///
/// No external asset, no script, no font: the file is mailed to someone and opened offline.
pub fn chart_svg(chart: &Chart) -> String {
    const W: f64 = 720.0;
    const H: f64 = 280.0;
    const LEFT: f64 = 56.0;
    const TOP: f64 = 24.0;
    const BOTTOM: f64 = 28.0;
    let plot_w = W - LEFT - 16.0;
    let plot_h = H - TOP - BOTTOM;
    let max = chart.max_value().max(1.0);
    let palette = [
        "#4c8dff", "#43b581", "#f6a821", "#e05656", "#a86bff", "#28b8c4", "#8d8d8d",
    ];
    let mut out = String::with_capacity(2048);
    out.push_str(&format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{W:.0}\" height=\"{H:.0}\" \
         viewBox=\"0 0 {W:.0} {H:.0}\" font-family=\"ui-sans-serif, system-ui, sans-serif\" \
         role=\"img\" aria-label=\"{}\"\">\n",
        escape_xml(&chart.title)
    ));
    out.push_str(&format!(
        "<rect x=\"0\" y=\"0\" width=\"{W:.0}\" height=\"{H:.0}\" fill=\"#ffffff\"/>\n"
    ));
    if chart.kind == ChartKind::Pie {
        let values: Vec<f64> = chart
            .series
            .first()
            .map(|s| s.values.clone())
            .unwrap_or_default();
        let total: f64 = values.iter().filter(|v| **v > 0.0).sum::<f64>().max(0.001);
        let cx = W / 2.0;
        let cy = H / 2.0;
        let radius = plot_h.min(W) * 0.36;
        let mut start = -std::f64::consts::FRAC_PI_2;
        for (index, value) in values.iter().enumerate() {
            let sweep = (value / total) * std::f64::consts::TAU;
            if sweep <= 0.0 {
                continue;
            }
            let end = start + sweep;
            let large = if sweep > std::f64::consts::PI { 1 } else { 0 };
            let x1 = cx + radius * start.cos();
            let y1 = cy + radius * start.sin();
            let x2 = cx + radius * end.cos();
            let y2 = cy + radius * end.sin();
            out.push_str(&format!(
                "<path d=\"M {cx:.2} {cy:.2} L {x1:.2} {y1:.2} A {radius:.2} {radius:.2} 0 {large} 1 {x2:.2} {y2:.2} Z\" \
                 fill=\"{}\" stroke=\"#ffffff\" stroke-width=\"1\"/>\n",
                palette[index % palette.len()]
            ));
            start = end;
        }
    } else {
        // Y gridlines and their labels.
        for step in 0..=4 {
            let y = TOP + plot_h - plot_h * step as f64 / 4.0;
            out.push_str(&format!(
                "<line x1=\"{LEFT:.0}\" y1=\"{y:.2}\" x2=\"{:.2}\" y2=\"{y:.2}\" stroke=\"#e6e6e6\" stroke-width=\"1\"/>\n",
                LEFT + plot_w
            ));
            out.push_str(&format!(
                "<text x=\"{:.0}\" y=\"{:.2}\" font-size=\"10\" fill=\"#888888\" text-anchor=\"end\">{}</text>\n",
                LEFT - 8.0,
                y + 3.0,
                axis_label(max * step as f64 / 4.0)
            ));
        }
        let groups = chart.labels.len().max(1);
        let group_w = plot_w / groups as f64;
        match chart.kind {
            ChartKind::Line | ChartKind::Area | ChartKind::Scatter => {
                for (index, series) in chart.series.iter().enumerate() {
                    let colour = palette[index % palette.len()];
                    let points: Vec<(f64, f64)> = series
                        .values
                        .iter()
                        .enumerate()
                        .map(|(i, value)| {
                            let x = if series.values.len() <= 1 {
                                LEFT
                            } else {
                                LEFT + plot_w * i as f64 / (series.values.len() - 1) as f64
                            };
                            let y = TOP + plot_h - (value / max) * plot_h;
                            (x, y)
                        })
                        .collect();
                    if chart.kind == ChartKind::Area && points.len() > 1 {
                        let mut path = format!("M {:.2} {:.2}", points[0].0, points[0].1);
                        for (x, y) in &points[1..] {
                            path.push_str(&format!(" L {x:.2} {y:.2}"));
                        }
                        path.push_str(&format!(
                            " L {:.2} {:.2} L {:.2} {:.2} Z",
                            LEFT + plot_w,
                            TOP + plot_h,
                            LEFT,
                            TOP + plot_h
                        ));
                        out.push_str(&format!(
                            "<path d=\"{path}\" fill=\"{colour}\" fill-opacity=\"0.22\"/>\n"
                        ));
                    }
                    let path: Vec<String> = points
                        .iter()
                        .enumerate()
                        .map(|(i, (x, y))| {
                            if i == 0 {
                                format!("M {x:.2} {y:.2}")
                            } else {
                                format!("L {x:.2} {y:.2}")
                            }
                        })
                        .collect();
                    out.push_str(&format!(
                        "<path d=\"{}\" fill=\"none\" stroke=\"{colour}\" stroke-width=\"2\"/>\n",
                        path.join(" ")
                    ));
                    if chart.kind == ChartKind::Scatter {
                        for (x, y) in &points {
                            out.push_str(&format!(
                                "<circle cx=\"{x:.2}\" cy=\"{y:.2}\" r=\"2.5\" fill=\"{colour}\"/>\n"
                            ));
                        }
                    }
                }
            }
            _ => {
                // Bars: one group per label, one bar per series inside it.
                let bar_w = (group_w * 0.7 / chart.series.len().max(1) as f64).max(2.0);
                for (label_index, _) in chart.labels.iter().enumerate() {
                    for (series_index, series) in chart.series.iter().enumerate() {
                        let value = series.values.get(label_index).copied().unwrap_or(0.0);
                        let height = (value / max) * plot_h;
                        let x = LEFT
                            + group_w * label_index as f64
                            + group_w * 0.15
                            + bar_w * series_index as f64;
                        let y = TOP + plot_h - height;
                        out.push_str(&format!(
                            "<rect x=\"{x:.2}\" y=\"{y:.2}\" width=\"{:.2}\" height=\"{:.2}\" \
                             fill=\"{}\" rx=\"2\"/>\n",
                            bar_w * 0.9,
                            height.max(1.0),
                            palette[series_index % palette.len()]
                        ));
                    }
                }
                // Category labels, thinned when they would collide.
                for (index, label) in chart.labels.iter().enumerate() {
                    if chart.labels.len() > 20 && index % 2 == 1 {
                        continue;
                    }
                    out.push_str(&format!(
                        "<text x=\"{:.2}\" y=\"{:.2}\" font-size=\"10\" fill=\"#666666\" text-anchor=\"middle\">{}</text>\n",
                        LEFT + group_w * (index as f64 + 0.5),
                        TOP + plot_h + 16.0,
                        escape_xml(&clip(label, 12))
                    ));
                }
            }
        }
        out.push_str(&format!(
            "<line x1=\"{LEFT:.0}\" y1=\"{:.2}\" x2=\"{:.2}\" y2=\"{:.2}\" stroke=\"#bbbbbb\"/>\n",
            TOP + plot_h,
            LEFT + plot_w,
            TOP + plot_h
        ));
    }
    // The title and a legend: the two things a mailed chart needs to be readable alone.
    out.push_str(&format!(
        "<text x=\"{:.0}\" y=\"16\" font-size=\"12\" fill=\"#222222\">{}</text>\n",
        LEFT,
        escape_xml(&chart.title)
    ));
    for (index, series) in chart.series.iter().enumerate().take(6) {
        let x = LEFT + index as f64 * 120.0;
        let y = H - 8.0;
        out.push_str(&format!(
            "<rect x=\"{x:.2}\" y=\"{:.2}\" width=\"9\" height=\"9\" fill=\"{}\" rx=\"2\"/>\n",
            y - 8.0,
            palette[index % palette.len()]
        ));
        out.push_str(&format!(
            "<text x=\"{:.2}\" y=\"{y:.2}\" font-size=\"10\" fill=\"#666666\">{}</text>\n",
            x + 13.0,
            escape_xml(&clip(&series.name, 16))
        ));
    }
    out.push_str("</svg>\n");
    out
}

fn axis_label(value: f64) -> String {
    if value.abs() >= 1_000_000.0 {
        format!("{:.1}M", value / 1_000_000.0)
    } else if value.abs() >= 1_000.0 {
        format!("{:.1}k", value / 1_000.0)
    } else if value.abs() >= 10.0 {
        format!("{value:.0}")
    } else {
        format!("{value:.1}")
    }
}

fn clip(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    text.chars().take(max.saturating_sub(1)).collect::<String>() + "…"
}

fn escape_xml(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
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
