//! Prompt building: the schema block a `@table` mention pulls in, and the cleanup of what comes
//! back.

use nook_core::Dialect;
use nook_model::Table;

/// The table names a prompt mentions as `@name` or `@schema.name`.
///
/// Deliberately dumb: it collects the word after every `@` and lets the caller match it against the
/// live catalog. Something that merely looks like a mention — `me@example.com` in a sentence — is
/// dropped when no such table exists, and a name that only matches part of a word (`@users_backup`
/// next to a `users` table) is not silently truncated to fit.
pub fn mentioned(prompt: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut chars = prompt.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '@' {
            continue;
        }
        let mut name = String::new();
        while let Some(&next) = chars.peek() {
            if next.is_ascii_alphanumeric() || next == '_' || next == '.' || next == '$' {
                name.push(next);
                chars.next();
            } else {
                break;
            }
        }
        // A trailing dot is sentence punctuation, not part of a name.
        let name = name.trim_end_matches('.').to_string();
        if !name.is_empty() && !out.iter().any(|n| n.eq_ignore_ascii_case(&name)) {
            out.push(name);
        }
    }
    out
}

/// One table described for the model: name, columns with the facts that change how a query is
/// written, and the keys.
pub fn table_context(table: &Table) -> String {
    let mut lines: Vec<String> = Vec::new();
    let mut head = format!("TABLE {}", table.key());
    if !table.comment.trim().is_empty() {
        head.push_str(&format!("  -- {}", table.comment.trim()));
    }
    lines.push(head);

    for column in &table.columns {
        let ty = if column.type_name.trim().is_empty() {
            "unknown"
        } else {
            column.type_name.trim()
        };
        let mut line = format!("  {} {}", column.name, ty);
        if column.primary_key {
            line.push_str(" PRIMARY KEY");
        } else if !column.nullable {
            // SQLite reports the primary key as not-null; saying it twice adds nothing.
            line.push_str(" NOT NULL");
        }
        if column.auto_increment {
            line.push_str(" AUTO_INCREMENT");
        }
        if let Some(default) = &column.default {
            if !default.trim().is_empty() {
                line.push_str(&format!(" DEFAULT {}", default.trim()));
            }
        }
        if !column.comment.trim().is_empty() {
            line.push_str(&format!("  -- {}", column.comment.trim()));
        }
        lines.push(line);
    }

    for index in &table.indexes {
        // The primary key is already marked on its columns.
        if index.primary {
            continue;
        }
        let unique = if index.unique { " UNIQUE" } else { "" };
        lines.push(format!(
            "  INDEX {} ({}){unique}",
            index.name,
            index.columns.join(", ")
        ));
    }

    for fk in &table.foreign_keys {
        let target = match &fk.ref_schema {
            Some(schema) if !schema.is_empty() => format!("{schema}.{}", fk.ref_table),
            _ => fk.ref_table.clone(),
        };
        let mut line = format!(
            "  FOREIGN KEY ({}) REFERENCES {}({})",
            fk.columns.join(", "),
            target,
            fk.ref_columns.join(", ")
        );
        if !fk.on_delete.trim().is_empty() {
            line.push_str(&format!(" ON DELETE {}", fk.on_delete.trim()));
        }
        if !fk.on_update.trim().is_empty() {
            line.push_str(&format!(" ON UPDATE {}", fk.on_update.trim()));
        }
        lines.push(line);
    }

    lines.join("\n")
}

/// The system prompt: which dialect to write, which tables exist, and the one thing models keep
/// getting wrong — answering with prose or a markdown fence.
pub fn system_prompt(dialect: Dialect, contexts: &[String]) -> String {
    let dialect = match dialect {
        Dialect::Sqlite => "SQLite",
        Dialect::Postgres => "PostgreSQL",
        Dialect::MySql => "MySQL",
    };
    let mut prompt = String::new();
    prompt.push_str(&format!(
        "You write {dialect} queries for a database tool.\n"
    ));
    prompt.push_str(
        "Answer with SQL and nothing else: no explanation before or after, no markdown code fence.\n",
    );
    prompt.push_str("Use only the tables and columns given below; never invent a name.\n");
    if contexts.is_empty() {
        prompt.push_str(
            "No table structure was supplied, so write the query from the request alone.\n",
        );
    } else {
        prompt.push_str("\nThe available tables:\n\n");
        prompt.push_str(&contexts.join("\n\n"));
        prompt.push('\n');
    }
    prompt
}

/// The SQL inside whatever the model answered: a fenced block when there is one, the whole answer
/// otherwise.
pub fn extract_sql(answer: &str) -> String {
    let text = answer.trim();
    let fenced = || -> Option<String> {
        let start = text.find("```")?;
        let rest = &text[start + 3..];
        // Drop a language tag: a first line that is nothing but a short word, e.g. ```` ```sql ````.
        let rest = match rest.split_once('\n') {
            Some((first, tail))
                if !first.trim().is_empty()
                    && first.trim().len() <= 12
                    && first.trim().chars().all(|c| c.is_ascii_alphanumeric()) =>
            {
                tail
            }
            _ => rest,
        };
        let end = rest.find("```")?;
        Some(rest[..end].trim().to_string())
    };
    match fenced().filter(|sql| !sql.is_empty()) {
        Some(sql) => sql,
        None => text.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nook_model::{Column, ForeignKey, Index};

    fn users() -> Table {
        let mut table = Table::new(Some("public".into()), "users");
        let mut id = Column::new("id", "integer");
        id.primary_key = true;
        id.nullable = false;
        id.auto_increment = true;
        table.columns.push(id);
        let mut email = Column::new("email", "text");
        email.nullable = false;
        table.columns.push(email);
        table.columns.push(Column::new("nickname", "text"));
        table.indexes.push(Index {
            name: "ix_users_email".into(),
            columns: vec!["email".into()],
            unique: true,
            ..Index::default()
        });
        table
    }

    #[test]
    fn mentions_are_read_in_order_and_deduplicated() {
        assert_eq!(
            mentioned("show @users with their @orders and @users again"),
            vec!["users", "orders"]
        );
        assert_eq!(
            mentioned("@public.users joined to @orders"),
            vec!["public.users", "orders"]
        );
        assert_eq!(mentioned("no mentions here"), Vec::<String>::new());
        // Sentence punctuation is not part of a name, and an email is just a word.
        assert_eq!(mentioned("see @users."), vec!["users"]);
        assert_eq!(mentioned("mail me at bob@example.com"), vec!["example.com"]);
        assert_eq!(mentioned("@"), Vec::<String>::new());
    }

    #[test]
    fn a_table_context_carries_what_changes_how_a_query_is_written() {
        let context = table_context(&users());
        assert!(context.starts_with("TABLE public.users"), "{context}");
        assert!(
            context.contains("id integer PRIMARY KEY AUTO_INCREMENT"),
            "{context}"
        );
        assert!(context.contains("email text NOT NULL"), "{context}");
        assert!(
            context.contains("INDEX ix_users_email (email) UNIQUE"),
            "{context}"
        );
        // A nullable column says nothing extra.
        assert!(
            context.contains("nickname text\n") || context.ends_with("nickname text"),
            "{context}"
        );
        // The primary key is not repeated as an index line.
        assert_eq!(context.matches("INDEX").count(), 1);
    }

    #[test]
    fn a_foreign_key_names_its_target_in_full() {
        let mut posts = Table::new(Some("public".into()), "posts");
        posts.columns.push(Column::new("author_id", "integer"));
        posts.foreign_keys.push(ForeignKey {
            name: "fk_posts_author".into(),
            columns: vec!["author_id".into()],
            ref_schema: Some("public".into()),
            ref_table: "users".into(),
            ref_columns: vec!["id".into()],
            on_delete: "cascade".into(),
            ..ForeignKey::default()
        });
        let context = table_context(&posts);
        assert!(
            context
                .contains("FOREIGN KEY (author_id) REFERENCES public.users(id) ON DELETE cascade"),
            "{context}"
        );
    }

    #[test]
    fn the_system_prompt_names_the_dialect_and_forbids_prose() {
        let with_tables = system_prompt(Dialect::Postgres, &[table_context(&users())]);
        assert!(with_tables.contains("PostgreSQL"), "{with_tables}");
        assert!(
            with_tables.contains("no markdown code fence"),
            "{with_tables}"
        );
        assert!(with_tables.contains("TABLE public.users"), "{with_tables}");

        let bare = system_prompt(Dialect::Sqlite, &[]);
        assert!(bare.contains("SQLite"), "{bare}");
        assert!(bare.contains("No table structure was supplied"), "{bare}");
        assert!(!bare.contains("TABLE"), "{bare}");
    }

    #[test]
    fn only_the_sql_survives_whatever_wrapping_the_model_adds() {
        assert_eq!(extract_sql("SELECT 1;"), "SELECT 1;");
        assert_eq!(extract_sql("  SELECT 1;\n"), "SELECT 1;");
        assert_eq!(extract_sql("```sql\nSELECT 1;\n```"), "SELECT 1;");
        assert_eq!(extract_sql("```\nSELECT 1;\n```"), "SELECT 1;");
        assert_eq!(
            extract_sql("Here you go:\n```sql\nSELECT 1;\n```\nHope that helps!"),
            "SELECT 1;"
        );
        // A fence body that starts on the same line as the tag keeps every word.
        assert_eq!(extract_sql("```sql\nselect 1\n```"), "select 1");
        // An unterminated fence is not a fence: the text is returned as it came.
        assert_eq!(extract_sql("SELECT 1;"), "SELECT 1;");
    }
}
