//! Test-data generation, the feature Navicat sells as "Data Generation".
//! Deterministic for a given seed, so the same script can be reproduced.

use datacraft_core::{quote_ident, quote_qualified, sql_literal, Dialect, Error, Result, Value};
use datacraft_model::{Column, Table};

/// xorshift64*: small, seedable, no dependency.
#[derive(Debug, Clone)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng(if seed == 0 {
            0x9E37_79B9_7F4A_7C15
        } else {
            seed
        })
    }
    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    pub fn below(&mut self, n: u64) -> u64 {
        if n == 0 {
            0
        } else {
            self.next_u64() % n
        }
    }
    pub fn int_between(&mut self, min: i64, max: i64) -> i64 {
        if max <= min {
            return min;
        }
        min + self.below((max - min + 1) as u64) as i64
    }
    pub fn pick<'a, T>(&mut self, items: &'a [T]) -> Option<&'a T> {
        if items.is_empty() {
            None
        } else {
            Some(&items[self.below(items.len() as u64) as usize])
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Generator {
    /// Follow the column type.
    Auto,
    Sequence,
    Int {
        min: i64,
        max: i64,
    },
    Float {
        min: f64,
        max: f64,
    },
    Bool,
    FirstName,
    LastName,
    FullName,
    Email,
    Phone,
    City,
    Country,
    Company,
    Word,
    Sentence,
    /// `#` is replaced by a random digit.
    Pattern(String),
    Pick(Vec<String>),
    DateDaysAgo {
        max_days: i64,
    },
    TimestampDaysAgo {
        max_days: i64,
    },
    Uuid,
    Null,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GenColumn {
    pub name: String,
    pub generator: Generator,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GenSpec {
    pub columns: Vec<GenColumn>,
    pub rows: usize,
    pub seed: u64,
    /// Skip values for columns that fill themselves (identity columns).
    pub skip_identity: bool,
}

impl GenSpec {
    /// Pick a generator per column from the declared type and the column name.
    pub fn from_table(table: &Table, rows: usize, seed: u64) -> GenSpec {
        GenSpec {
            columns: table
                .columns
                .iter()
                .map(|c| GenColumn {
                    name: c.name.clone(),
                    generator: generator_for(c),
                })
                .collect(),
            rows: rows.clamp(0, 100_000),
            seed,
            skip_identity: true,
        }
    }

    /// `name=email,status=live\|trial` overrides, as the dialog and the CLI both send them.
    pub fn apply_overrides(
        &mut self,
        overrides: &std::collections::BTreeMap<String, String>,
    ) -> Result<()> {
        for (name, spec) in overrides {
            let Some(column) = self.columns.iter_mut().find(|c| &c.name == name) else {
                return Err(Error::not_found(format!("no column named '{name}'")));
            };
            column.generator = parse_generator(spec)?;
        }
        Ok(())
    }
}

/// `int:1-100`, `pick:a|b|c`, `pattern:ORD-####`, `email`, `null`, …
pub fn parse_generator(spec: &str) -> Result<Generator> {
    let text = spec.trim();
    let (head, arg) = match text.split_once(':') {
        Some((h, a)) => (h.trim().to_ascii_lowercase(), a.to_string()),
        None => (text.to_ascii_lowercase(), String::new()),
    };
    let range = |arg: &str| -> Result<(f64, f64)> {
        let (lo, hi) = arg
            .split_once('-')
            .ok_or_else(|| Error::config(format!("'{spec}' needs a min-max range")))?;
        let lo: f64 = lo
            .trim()
            .parse()
            .map_err(|_| Error::config(format!("bad range in '{spec}'")))?;
        let hi: f64 = hi
            .trim()
            .parse()
            .map_err(|_| Error::config(format!("bad range in '{spec}'")))?;
        Ok((lo, hi))
    };
    Ok(match head.as_str() {
        "auto" | "" => Generator::Auto,
        "sequence" | "seq" => Generator::Sequence,
        "int" | "integer" => {
            let (lo, hi) = range(&arg)?;
            Generator::Int {
                min: lo as i64,
                max: hi as i64,
            }
        }
        "float" | "decimal" | "number" => {
            let (lo, hi) = range(&arg)?;
            Generator::Float { min: lo, max: hi }
        }
        "bool" | "boolean" => Generator::Bool,
        "firstname" | "first_name" | "givenname" => Generator::FirstName,
        "lastname" | "last_name" | "surname" => Generator::LastName,
        "name" | "fullname" | "full_name" => Generator::FullName,
        "email" | "mail" => Generator::Email,
        "phone" | "mobile" => Generator::Phone,
        "city" => Generator::City,
        "country" => Generator::Country,
        "company" => Generator::Company,
        "word" => Generator::Word,
        "sentence" | "text" | "lorem" => Generator::Sentence,
        "pattern" => Generator::Pattern(if arg.is_empty() { "####".into() } else { arg }),
        "pick" | "enum" | "oneof" => Generator::Pick(
            arg.split('|')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect(),
        ),
        "date" => Generator::DateDaysAgo {
            max_days: arg.trim().parse().unwrap_or(365),
        },
        "timestamp" | "datetime" => Generator::TimestampDaysAgo {
            max_days: arg.trim().parse().unwrap_or(365),
        },
        "uuid" | "guid" => Generator::Uuid,
        "null" | "skip" => Generator::Null,
        other => {
            return Err(Error::config(format!(
                "unknown generator '{other}'; try int:1-100, pick:a|b, pattern:INV-####, email, sentence, uuid"
            )))
        }
    })
}

/// Column-name hints win over the declared type: `email varchar(255)` should produce addresses.
pub fn generator_for(column: &Column) -> Generator {
    let name = column.name.to_ascii_lowercase();
    let ty = column.type_name.to_ascii_uppercase();
    if column.auto_increment {
        return Generator::Sequence;
    }
    if !column.enum_values.is_empty() {
        return Generator::Pick(column.enum_values.clone());
    }
    match name.as_str() {
        "email" | "e_mail" | "mail" => return Generator::Email,
        "phone" | "phone_number" | "mobile" | "tel" => return Generator::Phone,
        "first_name" | "given_name" => return Generator::FirstName,
        "last_name" | "surname" | "family_name" => return Generator::LastName,
        "name" | "full_name" | "customer_name" | "username" => return Generator::FullName,
        "city" | "town" => return Generator::City,
        "country" | "country_code" => return Generator::Country,
        "company" | "organization" | "org" => return Generator::Company,
        "uuid" | "guid" => return Generator::Uuid,
        _ => {}
    }
    if name.ends_with("_at") || name.ends_with("_date") || ty.contains("TIMESTAMP") {
        return Generator::TimestampDaysAgo { max_days: 365 };
    }
    if name == "status" || name == "state" || name == "kind" || name.ends_with("_type") {
        return Generator::Pick(vec!["draft".into(), "live".into(), "archived".into()]);
    }
    if ty.contains("SERIAL") {
        return Generator::Sequence;
    }
    if ty.contains("INT") {
        return Generator::Int {
            min: 1,
            max: 10_000,
        };
    }
    if ty.contains("REAL")
        || ty.contains("FLOAT")
        || ty.contains("DOUBLE")
        || ty.contains("NUMERIC")
        || ty.contains("DECIMAL")
    {
        return Generator::Float {
            min: 0.0,
            max: 1_000.0,
        };
    }
    if ty.contains("BOOL") {
        return Generator::Bool;
    }
    if ty.contains("DATE") && !ty.contains("TIME") {
        return Generator::DateDaysAgo { max_days: 365 };
    }
    if ty.contains("TIME") {
        return Generator::TimestampDaysAgo { max_days: 365 };
    }
    if ty.contains("JSON") {
        return Generator::Pattern("{\"n\": #}".into());
    }
    Generator::Sentence
}

const FIRST_NAMES: &[&str] = &[
    "Aisha", "Ben", "Chloe", "Diego", "Elena", "Farid", "Grace", "Hiro", "Ines", "Jonas", "Kira",
    "Liam", "Maya", "Noor", "Omar", "Priya", "Raffi", "Sofia", "Tomas", "Uma", "Victor", "Wen",
    "Ximena", "Yusuf", "Zara",
];
const LAST_NAMES: &[&str] = &[
    "Abadi", "Bianchi", "Chen", "Dubois", "Eriksen", "Fontana", "Garcia", "Haddad", "Ibrahim",
    "Jensen", "Kowalski", "Larsen", "Mbeki", "Novak", "Okafor", "Petrov", "Quintero", "Rossi",
    "Silva", "Tanaka", "Ueda", "Vargas", "Wibowo", "Xu", "Yilmaz", "Zhao",
];
const CITIES: &[&str] = &[
    "Amsterdam",
    "Balikpapan",
    "Berlin",
    "Cairo",
    "Delhi",
    "Dublin",
    "Helsinki",
    "Istanbul",
    "Jakarta",
    "Kyoto",
    "Lisbon",
    "Manila",
    "Nairobi",
    "Oslo",
    "Prague",
    "Quito",
    "Riyadh",
    "Seoul",
    "Taipei",
    "Vienna",
    "Warsaw",
    "Zurich",
];
const COUNTRIES: &[&str] = &[
    "Australia",
    "Brazil",
    "Canada",
    "Denmark",
    "Egypt",
    "France",
    "Germany",
    "India",
    "Indonesia",
    "Italy",
    "Japan",
    "Kenya",
    "Mexico",
    "Netherlands",
    "Norway",
    "Poland",
    "Singapore",
    "Spain",
    "Sweden",
    "Turkey",
    "United Kingdom",
    "United States",
];
const WORDS: &[&str] = &[
    "amber", "bridge", "candle", "dawn", "ember", "forest", "glade", "harbour", "isle", "jetty",
    "kernel", "lantern", "meadow", "nimbus", "orchid", "pebble", "quartz", "river", "stone",
    "thicket", "umbra", "valley", "willow", "xenon", "yarrow", "zephyr",
];
const COMPANIES: &[&str] = &[
    "Northwind Trading",
    "Atlas Freight",
    "Blue Harbour Foods",
    "Cedarline Studio",
    "Delta Instruments",
    "Everest Analytics",
    "Fairwind Logistics",
    "Granite Systems",
    "Hollow Moon Media",
    "Ironwood Supply",
];

fn value_for(gen: &Generator, rng: &mut Rng, sequence: i64) -> Value {
    match gen {
        Generator::Auto | Generator::Sentence => {
            let mut parts = Vec::new();
            for _ in 0..rng.below(6) + 3 {
                if let Some(w) = rng.pick(WORDS) {
                    parts.push(*w);
                }
            }
            Value::text(parts.join(" "))
        }
        Generator::Sequence => Value::Int(sequence),
        Generator::Int { min, max } => Value::Int(rng.int_between(*min, *max)),
        Generator::Float { min, max } => {
            let span = (max - min).max(0.0);
            let raw = min + (rng.below(1_000_000) as f64 / 1_000_000.0) * span;
            Value::Float((raw * 100.0).round() / 100.0)
        }
        Generator::Bool => Value::Bool(rng.below(2) == 1),
        Generator::FirstName => Value::text(rng.pick(FIRST_NAMES).copied().unwrap_or("Sam")),
        Generator::LastName => Value::text(rng.pick(LAST_NAMES).copied().unwrap_or("Nova")),
        Generator::FullName => {
            let first = rng.pick(FIRST_NAMES).copied().unwrap_or("Sam");
            let last = rng.pick(LAST_NAMES).copied().unwrap_or("Nova");
            Value::text(format!("{first} {last}"))
        }
        Generator::Email => {
            let first = rng
                .pick(FIRST_NAMES)
                .copied()
                .unwrap_or("sam")
                .to_ascii_lowercase();
            let last = rng
                .pick(LAST_NAMES)
                .copied()
                .unwrap_or("nova")
                .to_ascii_lowercase();
            Value::text(format!("{first}.{last}{}@example.com", rng.below(1000)))
        }
        Generator::Phone => Value::text(format!(
            "+{}{:03}{:04}",
            rng.below(90) + 10,
            rng.below(1000),
            rng.below(10000)
        )),
        Generator::City => Value::text(rng.pick(CITIES).copied().unwrap_or("Nowhere")),
        Generator::Country => Value::text(rng.pick(COUNTRIES).copied().unwrap_or("Nowhere")),
        Generator::Company => Value::text(rng.pick(COMPANIES).copied().unwrap_or("Acme")),
        Generator::Word => Value::text(rng.pick(WORDS).copied().unwrap_or("word")),
        Generator::Pattern(pattern) => {
            let mut out = String::with_capacity(pattern.len());
            for ch in pattern.chars() {
                if ch == '#' {
                    out.push(char::from_digit(rng.below(10) as u32, 10).unwrap_or('0'));
                } else {
                    out.push(ch);
                }
            }
            Value::text(out)
        }
        Generator::Pick(options) => match rng.pick(options) {
            Some(v) => Value::text(v.clone()),
            None => Value::Null,
        },
        Generator::DateDaysAgo { max_days } => {
            let days = rng.int_between(0, *max_days);
            let (y, m, d) = ymd(days_ago(days));
            Value::Date(format!("{y:04}-{m:02}-{d:02}"))
        }
        Generator::TimestampDaysAgo { max_days } => {
            let days = rng.int_between(0, *max_days);
            let secs = rng.below(86_400);
            let (y, m, d) = ymd(days_ago(days));
            Value::Timestamp(format!(
                "{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02}",
                secs / 3600,
                (secs / 60) % 60,
                secs % 60
            ))
        }
        Generator::Uuid => {
            let a = rng.next_u64();
            let b = rng.next_u64();
            Value::text(format!(
                "{:08x}-{:04x}-4{:03x}-{:04x}-{:012x}",
                (a >> 32) as u32,
                (a >> 16) as u16,
                (a & 0x0fff) as u16,
                ((b >> 48) as u16 & 0x3fff) | 0x8000,
                b & 0xffff_ffff_ffff
            ))
        }
        Generator::Null => Value::Null,
    }
}

fn days_ago(days: i64) -> i64 {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    now / 86_400 - days
}

/// Civil date from days since the epoch (Howard Hinnant's algorithm).
fn ymd(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// `rows` of values, one vector per row, in the table's column order.
pub fn generate(spec: &GenSpec, table: &Table) -> Result<Vec<Vec<Value>>> {
    let mut rng = Rng::new(spec.seed);
    let mut out = Vec::with_capacity(spec.rows);
    for row in 0..spec.rows {
        let mut values = Vec::with_capacity(spec.columns.len());
        for column in &spec.columns {
            let generator = if spec.skip_identity
                && table
                    .column(&column.name)
                    .map(|c| c.auto_increment)
                    .unwrap_or(false)
            {
                &Generator::Null
            } else {
                &column.generator
            };
            values.push(value_for(generator, &mut rng, row as i64 + 1));
        }
        out.push(values);
    }
    Ok(out)
}

/// `INSERT` script for generated rows, and the same text the import wizard previews.
pub fn insert_script(
    spec: &GenSpec,
    table: &Table,
    rows: &[Vec<Value>],
    dialect: Dialect,
) -> Result<String> {
    let keep: Vec<&GenColumn> = spec
        .columns
        .iter()
        .filter(|c| {
            !(spec.skip_identity
                && table
                    .column(&c.name)
                    .map(|col| col.auto_increment)
                    .unwrap_or(false))
        })
        .collect();
    if keep.is_empty() {
        return Err(Error::config("every column is generated by the database"));
    }
    let index_of = |name: &str| spec.columns.iter().position(|c| c.name == name);
    let target = quote_qualified(table.schema.as_deref(), &table.name, dialect);
    let cols = keep
        .iter()
        .map(|c| quote_ident(&c.name, dialect))
        .collect::<Vec<_>>()
        .join(", ");
    let mut out = String::new();
    for row in rows {
        let vals = keep
            .iter()
            .map(|c| {
                index_of(&c.name)
                    .and_then(|i| row.get(i))
                    .map(|v| sql_literal(v, dialect))
                    .unwrap_or_else(|| "NULL".into())
            })
            .collect::<Vec<_>>()
            .join(", ");
        out.push_str(&format!("INSERT INTO {target} ({cols}) VALUES ({vals});\n"));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table() -> Table {
        let mut t = Table::new(None, "customers");
        let mut id = Column::new("id", "INTEGER");
        id.primary_key = true;
        id.auto_increment = true;
        t.columns.push(id);
        t.columns.push(Column::new("email", "varchar(120)"));
        t.columns.push(Column::new("first_name", "varchar(60)"));
        t.columns.push(Column::new("status", "varchar(20)"));
        t.columns.push(Column::new("created_at", "timestamp"));
        t.columns.push(Column::new("balance", "numeric(10,2)"));
        t.columns.push(Column::new("active", "boolean"));
        t
    }

    #[test]
    fn generators_follow_names_and_types() {
        let t = table();
        let spec = GenSpec::from_table(&t, 3, 42);
        let by = |n: &str| {
            spec.columns
                .iter()
                .find(|c| c.name == n)
                .map(|c| c.generator.clone())
        };
        assert_eq!(by("id"), Some(Generator::Sequence));
        assert_eq!(by("email"), Some(Generator::Email));
        assert_eq!(by("first_name"), Some(Generator::FirstName));
        assert!(matches!(by("status"), Some(Generator::Pick(_))));
        assert!(matches!(
            by("created_at"),
            Some(Generator::TimestampDaysAgo { .. })
        ));
        assert!(matches!(by("balance"), Some(Generator::Float { .. })));
        assert_eq!(by("active"), Some(Generator::Bool));
    }

    #[test]
    fn generation_is_deterministic_and_complete() {
        let t = table();
        let spec = GenSpec::from_table(&t, 5, 7);
        let a = generate(&spec, &t).expect("generate");
        let b = generate(&spec, &t).expect("generate");
        assert_eq!(a, b);
        assert_eq!(a.len(), 5);
        assert!(a.iter().all(|r| r.len() == 7));
        let differing = generate(
            &GenSpec {
                seed: 8,
                ..spec.clone()
            },
            &t,
        )
        .expect("generate");
        assert_ne!(a, differing);
    }

    #[test]
    fn identity_columns_are_skipped_but_sequences_can_be_forced() {
        let t = table();
        let spec = GenSpec::from_table(&t, 2, 1);
        let rows = generate(&spec, &t).expect("rows");
        let script = insert_script(&spec, &t, &rows, Dialect::Postgres).expect("script");
        assert!(!script.contains("\"id\""));
        assert!(script.contains("\"email\""));
        assert_eq!(script.lines().count(), 2);
        let mut forced = spec.clone();
        forced.skip_identity = false;
        forced.columns[0].generator = Generator::Sequence;
        let rows = generate(&forced, &t).expect("rows");
        let script = insert_script(&forced, &t, &rows, Dialect::Sqlite).expect("script");
        assert!(script.contains("\"id\""));
        assert!(script.contains("VALUES (1,"));
    }

    #[test]
    fn overrides_and_errors() {
        let t = table();
        let mut spec = GenSpec::from_table(&t, 1, 1);
        let mut overrides = std::collections::BTreeMap::new();
        overrides.insert("status".to_string(), "pick:new|closed".to_string());
        spec.apply_overrides(&overrides).expect("override");
        assert_eq!(
            spec.columns
                .iter()
                .find(|c| c.name == "status")
                .map(|c| c.generator.clone()),
            Some(Generator::Pick(vec!["new".into(), "closed".into()]))
        );
        let mut bad = std::collections::BTreeMap::new();
        bad.insert("nope".to_string(), "int:1-2".to_string());
        assert!(spec.apply_overrides(&bad).is_err());
        assert!(parse_generator("nonsense").is_err());
        assert!(parse_generator("int:5").is_err());
    }

    #[test]
    fn patterns_and_values_stay_in_range() {
        let mut rng = Rng::new(3);
        match value_for(&Generator::Pattern("INV-####".into()), &mut rng, 0) {
            Value::Text(s) => {
                assert!(s.starts_with("INV-"));
                assert_eq!(s.len(), 8);
            }
            other => panic!("unexpected {other:?}"),
        }
        for _ in 0..100 {
            match value_for(&Generator::Int { min: 5, max: 9 }, &mut rng, 0) {
                Value::Int(v) => assert!((5..=9).contains(&v)),
                other => panic!("unexpected {other:?}"),
            }
        }
        match value_for(&Generator::Uuid, &mut rng, 0) {
            Value::Text(s) => assert_eq!(s.len(), 36),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn zero_rows_is_fine_and_huge_requests_are_clamped() {
        let t = table();
        let spec = GenSpec::from_table(&t, 0, 1);
        assert!(generate(&spec, &t).expect("rows").is_empty());
        let huge = GenSpec::from_table(&t, 10_000_000, 1);
        assert_eq!(huge.rows, 100_000);
    }
}
