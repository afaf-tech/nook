//! One cell type for every driver: the grid, transfer, sync and dumps all speak this.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "t", content = "v")]
pub enum Value {
    Null,
    Int(i64),
    UInt(u64),
    Float(f64),
    Text(String),
    Bool(bool),
    Bytes(Vec<u8>),
    /// Raw JSON text (Postgres json/jsonb, MySQL json, SQLite text-shaped json).
    Json(String),
    Date(String),
    Time(String),
    Timestamp(String),
}

impl Value {
    pub fn text(s: impl Into<String>) -> Self {
        Value::Text(s.into())
    }
    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }

    /// Name shown in the grid's type gutter and in `--format json` output.
    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Null => "null",
            Value::Int(_) => "int",
            Value::UInt(_) => "uint",
            Value::Float(_) => "float",
            Value::Text(_) => "text",
            Value::Bool(_) => "bool",
            Value::Bytes(_) => "bytes",
            Value::Json(_) => "json",
            Value::Date(_) => "date",
            Value::Time(_) => "time",
            Value::Timestamp(_) => "timestamp",
        }
    }

    /// What the grid paints. NULL is spelled out so an empty string is never confused with it.
    pub fn display(&self) -> String {
        match self {
            Value::Null => "NULL".into(),
            Value::Int(v) => v.to_string(),
            Value::UInt(v) => v.to_string(),
            Value::Float(v) => {
                if v.fract() == 0.0 && v.abs() < 1e15 {
                    format!("{v:.1}")
                } else {
                    v.to_string()
                }
            }
            Value::Text(s)
            | Value::Json(s)
            | Value::Date(s)
            | Value::Time(s)
            | Value::Timestamp(s) => s.clone(),
            Value::Bool(b) => b.to_string(),
            Value::Bytes(b) => format!("<{} bytes>", b.len()),
        }
    }

    /// Text as stored, without the NULL placeholder: what import/export round-trips.
    pub fn as_text(&self) -> String {
        match self {
            Value::Null => String::new(),
            Value::Bytes(b) => String::from_utf8_lossy(b).into_owned(),
            other => other.display(),
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Value::Int(v) => Some(*v),
            Value::UInt(v) => i64::try_from(*v).ok(),
            Value::Float(v) => Some(*v as i64),
            Value::Bool(b) => Some(u8::from(*b) as i64),
            Value::Text(s) => s.trim().parse().ok(),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Float(v) => Some(*v),
            Value::Int(v) => Some(*v as f64),
            Value::UInt(v) => Some(*v as f64),
            Value::Text(s) => s.trim().parse().ok(),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            Value::Int(v) => Some(*v != 0),
            Value::Text(s) => match s.trim().to_ascii_lowercase().as_str() {
                "true" | "t" | "1" | "yes" | "y" => Some(true),
                "false" | "f" | "0" | "no" | "n" => Some(false),
                _ => None,
            },
            _ => None,
        }
    }

    /// Parse a user-typed grid cell or an imported field into a value of a target type.
    pub fn coerce(text: &str, target: &str) -> Result<Value, String> {
        let t = target.to_ascii_uppercase();
        let raw = text.trim();
        if t.contains("NULL") && raw.eq_ignore_ascii_case("null") {
            return Ok(Value::Null);
        }
        if raw.eq_ignore_ascii_case("null") && !t.contains("CHAR") && !t.contains("TEXT") {
            return Ok(Value::Null);
        }
        if t.contains("INT") && !t.contains("POINT") {
            return raw
                .parse::<i64>()
                .map(Value::Int)
                .map_err(|_| format!("'{raw}' is not an integer"));
        }
        if t.contains("REAL")
            || t.contains("FLOAT")
            || t.contains("DOUBLE")
            || t.contains("NUMERIC")
            || t.contains("DECIMAL")
        {
            return raw
                .parse::<f64>()
                .map(Value::Float)
                .map_err(|_| format!("'{raw}' is not a number"));
        }
        if t.contains("BOOL") {
            return Value::text(raw)
                .as_bool()
                .map(Value::Bool)
                .ok_or_else(|| format!("'{raw}' is not a boolean"));
        }
        if t.contains("JSON") {
            return serde_json::from_str::<serde_json::Value>(raw)
                .map(|_| Value::Json(raw.to_string()))
                .map_err(|e| format!("invalid json: {e}"));
        }
        if t.contains("DATE") && !t.contains("TIME") {
            return Ok(Value::Date(raw.to_string()));
        }
        if t.contains("TIME") && !t.contains("STAMP") {
            return Ok(Value::Time(raw.to_string()));
        }
        if t.contains("TIMESTAMP") || t.contains("DATETIME") {
            return Ok(Value::Timestamp(raw.to_string()));
        }
        Ok(Value::text(raw))
    }

    /// JSON shape used by the CLI, the MCP server and the control channel.
    pub fn to_json(&self) -> serde_json::Value {
        match self {
            Value::Null => serde_json::Value::Null,
            Value::Int(v) => (*v).into(),
            Value::UInt(v) => (*v).into(),
            Value::Float(v) => serde_json::Number::from_f64(*v)
                .map(serde_json::Value::Number)
                .unwrap_or(serde_json::Value::Null),
            Value::Bool(b) => (*b).into(),
            Value::Bytes(b) => serde_json::Value::String(base64(b)),
            Value::Text(s)
            | Value::Json(s)
            | Value::Date(s)
            | Value::Time(s)
            | Value::Timestamp(s) => serde_json::Value::String(s.clone()),
        }
    }

    /// Inverse of [`Value::to_json`], for command params.
    pub fn from_json(v: &serde_json::Value) -> Value {
        match v {
            serde_json::Value::Null => Value::Null,
            serde_json::Value::Bool(b) => Value::Bool(*b),
            serde_json::Value::Number(n) => {
                if let Some(i) = n.as_i64() {
                    Value::Int(i)
                } else if let Some(u) = n.as_u64() {
                    Value::UInt(u)
                } else {
                    Value::Float(n.as_f64().unwrap_or(0.0))
                }
            }
            serde_json::Value::String(s) => Value::text(s.clone()),
            other => Value::Json(other.to_string()),
        }
    }

    /// Cheap guard so a single hostile cell cannot blow memory in the grid.
    pub fn approx_bytes(&self) -> usize {
        match self {
            Value::Text(s)
            | Value::Json(s)
            | Value::Date(s)
            | Value::Time(s)
            | Value::Timestamp(s) => s.len(),
            Value::Bytes(b) => b.len(),
            _ => 8,
        }
    }
}

fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            T[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            T[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_and_null_are_distinct() {
        assert_eq!(Value::Null.display(), "NULL");
        assert_eq!(Value::text("").display(), "");
        assert_eq!(Value::Float(2.0).display(), "2.0");
        assert_eq!(Value::Float(2.5).display(), "2.5");
    }

    #[test]
    fn coerce_follows_declared_type() {
        assert_eq!(Value::coerce(" 42 ", "INTEGER").ok(), Some(Value::Int(42)));
        assert!(Value::coerce("x", "INTEGER").is_err());
        assert_eq!(
            Value::coerce("true", "BOOLEAN").ok(),
            Some(Value::Bool(true))
        );
        assert_eq!(Value::coerce("null", "INTEGER").ok(), Some(Value::Null));
        assert_eq!(
            Value::coerce("null", "TEXT").ok(),
            Some(Value::text("null"))
        );
        assert!(Value::coerce("{\"a\":1}", "JSON").is_ok());
        assert!(Value::coerce("{oops}", "JSON").is_err());
    }

    #[test]
    fn json_round_trip() {
        for v in [
            Value::Null,
            Value::Int(-5),
            Value::Float(1.25),
            Value::Bool(false),
            Value::text("hi"),
            Value::Bytes(vec![1, 2, 3]),
        ] {
            let j = v.to_json();
            let back = Value::from_json(&j);
            if let Value::Bytes(_) = v {
                assert_eq!(back, Value::text("AQID"));
            } else {
                assert_eq!(back, v);
            }
        }
    }

    #[test]
    fn hostile_cell_is_bounded() {
        assert_eq!(Value::text("abc").approx_bytes(), 3);
    }
}
