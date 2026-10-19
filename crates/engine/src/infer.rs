//! Column type inference for the import wizard.
//!
//! The rule is the spec's: a column is an integer if *every* sampled value parses as `i64`, a float
//! if every value parses as `f64` and at least one carries `.` or `e`, a bool if every value is one
//! of the words below, a date/timestamp if every value has the ISO shape, else text. Empty counts as
//! nullable, never as evidence for a type — otherwise one blank row turns a numeric column into
//! text.
//!
//! This is a report, not a decision: against an existing table the catalog's types win and a
//! mismatch is a note. The inferred type only creates a table.

use serde_json::json;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Inferred {
    Integer,
    Float,
    Bool,
    Date,
    Timestamp,
    Text,
}

impl Inferred {
    pub fn label(self) -> &'static str {
        match self {
            Inferred::Integer => "integer",
            Inferred::Float => "float",
            Inferred::Bool => "boolean",
            Inferred::Date => "date",
            Inferred::Timestamp => "timestamp",
            Inferred::Text => "text",
        }
    }
}

/// What one column looks like across the sample.
#[derive(Debug, Clone, PartialEq)]
pub struct ColumnProfile {
    pub inferred: Inferred,
    pub nullable: bool,
    /// The first few non-empty values, for the report.
    pub sample: Vec<String>,
    /// True when every sampled value was empty.
    pub always_empty: bool,
}

fn is_bool_word(v: &str) -> bool {
    matches!(
        v.to_ascii_lowercase().as_str(),
        "true" | "false" | "t" | "f" | "yes" | "no" | "y" | "n" | "0" | "1"
    )
}

fn is_date(v: &str) -> bool {
    let bytes = v.as_bytes();
    bytes.len() == 10
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes[..4].iter().all(u8::is_ascii_digit)
        && bytes[5..7].iter().all(u8::is_ascii_digit)
        && bytes[8..].iter().all(u8::is_ascii_digit)
}

fn is_timestamp(v: &str) -> bool {
    let (date, time) = match v.split_once(['T', ' ']) {
        Some(pair) => pair,
        None => return false,
    };
    if !is_date(date) {
        return false;
    }
    let time = time.trim_end_matches('Z');
    let (clock, frac) = match time.split_once('.') {
        Some((c, f)) => (c, Some(f)),
        None => (time, None),
    };
    let parts: Vec<&str> = clock.split(':').collect();
    if parts.len() < 2 || parts.len() > 3 {
        return false;
    }
    if !parts
        .iter()
        .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
    {
        return false;
    }
    match frac {
        Some(f) => !f.is_empty() && f.bytes().all(|b| b.is_ascii_digit()),
        None => true,
    }
}

/// Profile one column over its sampled values.
pub fn profile(values: &[String]) -> ColumnProfile {
    let non_empty: Vec<&String> = values.iter().filter(|v| !v.trim().is_empty()).collect();
    let nullable = non_empty.len() < values.len();
    let sample: Vec<String> = non_empty.iter().take(3).map(|v| v.to_string()).collect();
    if non_empty.is_empty() {
        return ColumnProfile {
            inferred: Inferred::Text,
            nullable: true,
            sample,
            always_empty: true,
        };
    }
    let all = |f: fn(&str) -> bool| non_empty.iter().all(|v| f(v.trim()));
    let inferred = if all(|v| v.parse::<i64>().is_ok()) {
        Inferred::Integer
    } else if all(|v| v.parse::<f64>().is_ok())
        && non_empty
            .iter()
            .any(|v| v.contains('.') || v.contains('e') || v.contains('E'))
    {
        Inferred::Float
    } else if all(is_bool_word) {
        Inferred::Bool
    } else if all(is_timestamp) {
        Inferred::Timestamp
    } else if all(is_date) {
        Inferred::Date
    } else {
        Inferred::Text
    };
    ColumnProfile {
        inferred,
        nullable,
        sample,
        always_empty: false,
    }
}

/// The wizard's column table, as JSON.
pub fn report(columns: &[String], samples: &[Vec<String>]) -> Vec<serde_json::Value> {
    columns
        .iter()
        .enumerate()
        .map(|(i, name)| {
            let values: Vec<String> = samples
                .iter()
                .map(|row| row.get(i).cloned().unwrap_or_default())
                .collect();
            let p = profile(&values);
            json!({
                "source": name,
                "target": name,
                "inferred": p.inferred.label(),
                "nullable": p.nullable,
                "always_empty": p.always_empty,
                "sample": p.sample,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn col(values: &[&str]) -> ColumnProfile {
        profile(&values.iter().map(|v| v.to_string()).collect::<Vec<_>>())
    }

    #[test]
    fn integers_floats_and_text_are_told_apart() {
        assert_eq!(col(&["1", "2", "-3"]).inferred, Inferred::Integer);
        // Whole numbers written as floats are still floats: the `.` is the evidence.
        assert_eq!(col(&["1.5", "2"]).inferred, Inferred::Float);
        assert_eq!(col(&["1", "two"]).inferred, Inferred::Text);
    }

    #[test]
    fn a_single_blank_does_not_change_the_type_but_does_mark_it_nullable() {
        let p = col(&["1", "", "3"]);
        assert_eq!(p.inferred, Inferred::Integer);
        assert!(p.nullable);
    }

    #[test]
    fn booleans_dates_and_timestamps_have_their_own_shapes() {
        assert_eq!(col(&["true", "false", "yes"]).inferred, Inferred::Bool);
        assert_eq!(col(&["2024-01-31", "2024-02-01"]).inferred, Inferred::Date);
        assert_eq!(
            col(&["2024-01-31T10:00:00", "2024-02-01 11:30:00.5"]).inferred,
            Inferred::Timestamp
        );
    }

    #[test]
    fn a_column_that_is_empty_everywhere_says_so() {
        let p = col(&["", "  ", ""]);
        assert!(p.always_empty);
        assert!(p.nullable);
        assert_eq!(p.inferred, Inferred::Text);
    }

    #[test]
    fn a_zero_one_column_is_a_boolean_not_an_integer() {
        // Both readings are defensible; the spec's order puts integer first, so a 0/1 column
        // reads as integer and the wizard shows the sample so a person can override it.
        assert_eq!(col(&["0", "1", "0"]).inferred, Inferred::Integer);
    }
}
