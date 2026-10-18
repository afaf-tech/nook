//! The grid's structured filter: a list of conditions compiled into the one `WHERE` clause
//! `grid.filter` already takes.
//!
//! The compiler is deliberately the only place a filter becomes SQL. A column name goes through
//! `quote_ident`, a value through `sql_literal`, and the operator is a closed list — so a value
//! typed into a box is data, never syntax, and no operator string ever reaches the server
//! unvalidated. `grid.filter` stays as the escape hatch for a clause someone wants to write by hand.

use nook_core::{quote_ident, sql_literal, Dialect, Error, Result};
use serde_json::Value as Json;

/// One condition. `value` is `None` for the operators that take none (`is null`, `is not null`).
#[derive(Debug, Clone, PartialEq)]
pub struct Filter {
    pub column: String,
    pub op: String,
    pub value: Option<Json>,
}

/// The operators a filter may use, spelled the way the SQL is: one list, so the refusal and the
/// compile can never disagree about what is accepted.
pub const OPERATORS: &[&str] = &[
    "=",
    "!=",
    "<",
    "<=",
    ">",
    ">=",
    "like",
    "ilike",
    "in",
    "is null",
    "is not null",
    "between",
];

impl Filter {
    /// Read one condition out of a command's params.
    pub fn from_json(v: &Json) -> Result<Filter> {
        let Json::Object(map) = v else {
            return Err(Error::config(format!(
                "each filter must be an object, got {}",
                crate::json_kind(v)
            )));
        };
        let column = map
            .get("column")
            .and_then(Json::as_str)
            .map(str::trim)
            .filter(|c| !c.is_empty())
            .ok_or_else(|| Error::config("each filter needs a \"column\""))?;
        let op = map
            .get("op")
            .and_then(Json::as_str)
            .map(|o| o.trim().to_ascii_lowercase())
            .ok_or_else(|| Error::config("each filter needs an \"op\""))?;
        let op = canonical_op(&op).ok_or_else(|| {
            Error::config(format!(
                "'{op}' is not a filter operator; use one of: {}",
                OPERATORS.join(", ")
            ))
        })?;
        let value = match map.get("value") {
            None | Some(Json::Null) => None,
            Some(v) => Some(v.clone()),
        };
        // An operator that needs a value refuses a missing one here rather than compiling to
        // `column = NULL`, which matches nothing and reads like a filter that found no rows.
        if value.is_none() && needs_value(op) {
            return Err(Error::config(format!(
                "'{op}' needs a value; for a NULL test use 'is null'"
            )));
        }
        Ok(Filter {
            column: column.to_string(),
            op: op.to_string(),
            value,
        })
    }
}

fn needs_value(op: &str) -> bool {
    !matches!(op, "is null" | "is not null")
}

/// The operator as the compiler spells it: lower-case, single spaces, `!=` as written. `<>` and
/// `==` are accepted because a person types them.
fn canonical_op(op: &str) -> Option<&'static str> {
    let normalised = op.split_whitespace().collect::<Vec<_>>().join(" ");
    let wanted = match normalised.as_str() {
        "<>" | "!=" | "not ==" => "!=",
        "==" => "=",
        other => other,
    };
    OPERATORS.iter().copied().find(|o| *o == wanted)
}

/// Compile conditions into one `WHERE` body. Empty input is an empty clause, which the callers
/// treat as "no filter" — the same thing `grid.clearFilter` leaves behind.
pub fn compile(filters: &[Filter], dialect: Dialect) -> Result<String> {
    let mut parts: Vec<String> = Vec::with_capacity(filters.len());
    for filter in filters {
        parts.push(condition(filter, dialect)?);
    }
    Ok(parts.join(" AND "))
}

fn condition(filter: &Filter, dialect: Dialect) -> Result<String> {
    let column = quote_ident(filter.column.trim(), dialect);
    let value = filter.value.as_ref();
    let sql = match filter.op.as_str() {
        "is null" => format!("{column} IS NULL"),
        "is not null" => format!("{column} IS NOT NULL"),
        "in" => {
            let items = match value {
                Some(Json::Array(items)) => items,
                Some(other) => {
                    return Err(Error::config(format!(
                        "'in' takes a list, got {}",
                        crate::json_kind(other)
                    )))
                }
                None => return Err(Error::config("'in' needs a list of values")),
            };
            if items.is_empty() {
                // An empty `IN ()` is a syntax error on every engine; a filter that excludes
                // everything is what the user asked for, so say it as one.
                return Err(Error::config(
                    "'in' needs at least one value; an empty list matches nothing",
                ));
            }
            let list: Vec<String> = items
                .iter()
                .map(|v| sql_literal(&nook_core::Value::from_json(v), dialect))
                .collect();
            format!("{column} IN ({})", list.join(", "))
        }
        "between" => {
            let pair = match value {
                Some(Json::Array(items)) if items.len() == 2 => items,
                _ => {
                    return Err(Error::config(
                        "'between' takes exactly two values, low then high",
                    ))
                }
            };
            format!(
                "{column} BETWEEN {} AND {}",
                sql_literal(&nook_core::Value::from_json(&pair[0]), dialect),
                sql_literal(&nook_core::Value::from_json(&pair[1]), dialect)
            )
        }
        op @ ("=" | "!=" | "<" | "<=" | ">" | ">=" | "like" | "ilike") => {
            let literal = match value {
                Some(v) => sql_literal(&nook_core::Value::from_json(v), dialect),
                None => return Err(Error::config(format!("'{op}' needs a value"))),
            };
            format!("{column} {} {literal}", op.to_ascii_uppercase())
        }
        other => {
            return Err(Error::config(format!(
                "'{other}' is not a filter operator; use one of: {}",
                OPERATORS.join(", ")
            )))
        }
    };
    Ok(sql)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn one(column: &str, op: &str, value: Json) -> Filter {
        Filter::from_json(&json!({"column": column, "op": op, "value": value}))
            .expect("the filter parses")
    }

    fn compile_one(filter: &Filter) -> String {
        compile(std::slice::from_ref(filter), Dialect::Sqlite).expect("it compiles")
    }

    #[test]
    fn every_comparison_operator_compiles_to_its_own_sql() {
        for (op, expected) in [
            ("=", "\"status\" = 'live'"),
            ("!=", "\"status\" != 'live'"),
            ("<", "\"status\" < 'live'"),
            ("<=", "\"status\" <= 'live'"),
            (">", "\"status\" > 'live'"),
            (">=", "\"status\" >= 'live'"),
            ("like", "\"status\" LIKE 'live'"),
            ("ilike", "\"status\" ILIKE 'live'"),
        ] {
            assert_eq!(
                compile_one(&one("status", op, json!("live"))),
                expected,
                "{op}"
            );
        }
    }

    #[test]
    fn conditions_are_joined_with_and() {
        let filters = [
            one("status", "=", json!("live")),
            one("total", ">", json!(100)),
        ];
        assert_eq!(
            compile(&filters, Dialect::Sqlite).expect("compiles"),
            "\"status\" = 'live' AND \"total\" > 100"
        );
        assert_eq!(compile(&[], Dialect::Sqlite).expect("compiles"), "");
    }

    #[test]
    fn a_quote_in_a_value_is_escaped_not_closed() {
        let sql = compile_one(&one("name", "=", json!("O'Brien")));
        assert_eq!(sql, "\"name\" = 'O''Brien'");
    }

    #[test]
    fn a_hostile_column_name_is_one_quoted_identifier() {
        let sql = compile_one(&one("a\" OR 1=1 --", "=", json!(1)));
        assert_eq!(sql, "\"a\"\" OR 1=1 --\" = 1");
    }

    #[test]
    fn in_takes_a_list_and_between_a_pair() {
        assert_eq!(
            compile_one(&one("status", "in", json!(["live", "draft"]))),
            "\"status\" IN ('live', 'draft')"
        );
        assert_eq!(
            compile_one(&one("total", "between", json!([10, 20]))),
            "\"total\" BETWEEN 10 AND 20"
        );
    }

    #[test]
    fn null_operators_take_no_value_and_do_not_compile_to_equals_null() {
        let is_null = Filter::from_json(&json!({"column": "email", "op": "is null"}))
            .expect("is null needs no value");
        assert_eq!(compile_one(&is_null), "\"email\" IS NULL");
        let not_null = Filter::from_json(&json!({"column": "email", "op": "is not null"}))
            .expect("is not null needs no value");
        assert_eq!(compile_one(&not_null), "\"email\" IS NOT NULL");
    }

    #[test]
    fn an_unknown_operator_is_refused_by_name() {
        let err = Filter::from_json(&json!({"column": "a", "op": "drop table", "value": 1}))
            .expect_err("an operator outside the list is refused");
        assert!(err.message().contains("not a filter operator"), "{err}");
        assert!(
            err.message().contains("between"),
            "the refusal lists what is: {err}"
        );
    }

    #[test]
    fn an_operator_that_needs_a_value_refuses_a_missing_one() {
        let err = Filter::from_json(&json!({"column": "a", "op": "="}))
            .expect_err("`=` with no value is refused");
        assert!(
            err.message().contains("is null"),
            "the refusal points at the NULL form: {err}"
        );
        let err = Filter::from_json(&json!({"column": "a", "op": "in", "value": []}))
            .and_then(|f| compile(std::slice::from_ref(&f), Dialect::Sqlite))
            .expect_err("an empty IN matches nothing and is refused");
        assert!(err.message().contains("at least one"), "{err}");
    }

    #[test]
    fn between_without_two_values_is_refused() {
        let err = Filter::from_json(&json!({"column": "a", "op": "between", "value": [1]}))
            .and_then(|f| compile(std::slice::from_ref(&f), Dialect::Sqlite))
            .expect_err("between takes two");
        assert!(err.message().contains("exactly two"), "{err}");
    }

    #[test]
    fn the_operator_is_case_and_space_insensitive() {
        let filter = Filter::from_json(&json!({"column": "a", "op": " IS   NOT NULL "}))
            .expect("it normalises");
        assert_eq!(filter.op, "is not null");
        assert_eq!(
            Filter::from_json(&json!({"column": "a", "op": "<>", "value": 1}))
                .expect("it normalises")
                .op,
            "!="
        );
    }

    #[test]
    fn a_missing_column_is_refused_rather_than_compiled() {
        let err = Filter::from_json(&json!({"op": "=", "value": 1}))
            .expect_err("no column is not a filter");
        assert!(err.message().contains("column"), "{err}");
    }
}
