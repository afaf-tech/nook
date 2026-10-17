//! The Navicat feature catalog: what a database administration tool is expected to do, and the
//! command id that does it here. `parity` compares this list with the live command registry, so
//! coverage is measured rather than claimed.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CatalogEntry {
    /// Navicat's area, mirroring the main toolbar.
    pub area: &'static str,
    /// Feature id. When it matches a registered command, the feature is implemented.
    pub id: &'static str,
    pub label: &'static str,
    /// How it is done here, or why it is not.
    pub note: &'static str,
}

/// `(area, id, label, note)`.
const ENTRIES: &[(&str, &str, &str, &str)] = &[
    // ---------------------------------------------------------------- Connection
    (
        "Connection",
        "conn.new",
        "New connection",
        "profile dialog, any supported engine",
    ),
    (
        "Connection",
        "conn.edit",
        "Edit connection",
        "endpoint, database, SSL, SSH, read-only",
    ),
    (
        "Connection",
        "conn.delete",
        "Delete connection",
        "removes the profile and its keychain item",
    ),
    (
        "Connection",
        "conn.duplicate",
        "Duplicate connection",
        "copies a profile for a second database",
    ),
    (
        "Connection",
        "conn.open",
        "Open connection",
        "tunnel, connect, read server info and catalog",
    ),
    (
        "Connection",
        "conn.close",
        "Close connection",
        "drops the driver and the tunnel",
    ),
    ("Connection", "conn.closeAll", "Close all", ""),
    (
        "Connection",
        "conn.test",
        "Test connection",
        "connects and reports version and table count",
    ),
    ("Connection", "conn.reconnect", "Reconnect", ""),
    ("Connection", "conn.select", "Make active", ""),
    (
        "Connection",
        "conn.refresh",
        "Refresh server",
        "re-read catalog: schemas, tables, columns, keys",
    ),
    (
        "Connection",
        "conn.databases",
        "Databases",
        "list and switch database",
    ),
    (
        "Connection",
        "conn.useDatabase",
        "Switch database",
        "Postgres reconnects; MySQL issues USE",
    ),
    (
        "Connection",
        "conn.serverInfo",
        "Server information",
        "version, uptime, size, read-only flag",
    ),
    (
        "Connection",
        "conn.savePassword",
        "Save password",
        "macOS login keychain, never the profile file",
    ),
    (
        "Connection",
        "conn.forgetPassword",
        "Forget password",
        "deletes the keychain item",
    ),
    (
        "Connection",
        "conn.setReadOnly",
        "Read-only connection",
        "refused before the statement reaches the server",
    ),
    (
        "Connection",
        "conn.setSshTunnel",
        "SSH tunnel",
        "shells out to ssh, so agent keys and ~/.ssh/config work",
    ),
    (
        "Connection",
        "conn.setSsl",
        "SSL/TLS",
        "disable, prefer, require, verify-ca, verify-full",
    ),
    (
        "Connection",
        "conn.groups",
        "Connection groups",
        "folders in the connection list",
    ),
    (
        "Connection",
        "conn.parseUri",
        "Connection string",
        "parse postgres:// and mysql:// URIs",
    ),
    (
        "Connection",
        "conn.mongodb",
        "MongoDB connection",
        "catalogued, no driver yet",
    ),
    (
        "Connection",
        "conn.redis",
        "Redis connection",
        "catalogued, no driver yet",
    ),
    (
        "Connection",
        "conn.sqlserver",
        "SQL Server connection",
        "catalogued, no driver yet",
    ),
    (
        "Connection",
        "conn.oracle",
        "Oracle connection",
        "catalogued, no driver yet",
    ),
    (
        "Connection",
        "conn.httpTunnel",
        "HTTP tunnel",
        "not implemented; use SSH",
    ),
    (
        "Connection",
        "conn.sshPassword",
        "SSH by password",
        "macOS login keychain under a separate account; ssh reads it back through --ssh-askpass",
    ),
    (
        "Connection",
        "conn.cloud",
        "Cloud database connections",
        "not implemented; connect over a tunnel instead",
    ),
    // ---------------------------------------------------------------- Data grid
    ("Data", "grid.open", "Open table data", ""),
    (
        "Data",
        "grid.fetchPage",
        "Paging",
        "LIMIT/OFFSET pages with a configurable size",
    ),
    ("Data", "grid.nextPage", "Next page", ""),
    ("Data", "grid.prevPage", "Previous page", ""),
    ("Data", "grid.firstPage", "First page", ""),
    (
        "Data",
        "grid.lastPage",
        "Last page",
        "counts rows first, then jumps",
    ),
    ("Data", "grid.setPageSize", "Rows per page", ""),
    (
        "Data",
        "grid.sort",
        "Sort by column",
        "click twice to flip direction",
    ),
    ("Data", "grid.filter", "Filter rows", "raw WHERE clause"),
    ("Data", "grid.clearFilter", "Clear filter", ""),
    ("Data", "grid.countRows", "Row count", ""),
    (
        "Data",
        "grid.editCell",
        "Edit inline",
        "coerces to the column type, queues a statement",
    ),
    ("Data", "grid.setNull", "Set NULL", ""),
    ("Data", "grid.insertRow", "Insert row", ""),
    ("Data", "grid.deleteRow", "Delete row", ""),
    (
        "Data",
        "grid.duplicateRow",
        "Duplicate row",
        "drops generated keys so the copy is new",
    ),
    (
        "Data",
        "grid.pending",
        "Pending changes",
        "every queued statement, visible before apply",
    ),
    ("Data", "grid.revert", "Discard changes", ""),
    (
        "Data",
        "grid.apply",
        "Apply changes",
        "one transaction, rolled back on the first failure",
    ),
    (
        "Data",
        "grid.export",
        "Export the grid",
        "csv, json, sql, markdown",
    ),
    (
        "Data",
        "grid.exportTable",
        "Export a whole table",
        "streams in pages, no LIMIT surprises",
    ),
    ("Data", "grid.find", "Find in the grid", ""),
    (
        "Data",
        "grid.cellValue",
        "Read a cell",
        "typed value, display string and type",
    ),
    ("Data", "grid.copyAsSql", "Copy as INSERT", ""),
    ("Data", "grid.formView", "Form view", ""),
    (
        "Data",
        "grid.blobEditor",
        "Text and hex editors for blobs",
        "bytes are shown and exported, not edited",
    ),
    (
        "Data",
        "grid.rawMode",
        "Edit the SQL of a row",
        "grid edits become SQL in the pending list instead",
    ),
    // ---------------------------------------------------------------- Design
    (
        "Design",
        "schema.tables",
        "Object list",
        "tables, views, materialized views",
    ),
    (
        "Design",
        "schema.columns",
        "Columns",
        "type, nullability, default, comment, ordinal",
    ),
    (
        "Design",
        "schema.indexes",
        "Indexes",
        "name, columns, uniqueness, kind",
    ),
    (
        "Design",
        "schema.foreignKeys",
        "Foreign keys",
        "columns, target, on delete, on update",
    ),
    (
        "Design",
        "schema.ddl",
        "Object DDL",
        "SHOW CREATE where the server has it, else generated",
    ),
    ("Design", "schema.createTable", "New table", ""),
    (
        "Design",
        "schema.copyTable",
        "Copy table",
        "structure, optionally with data",
    ),
    ("Design", "schema.dropTable", "Drop table", "needs confirm"),
    (
        "Design",
        "schema.truncateTable",
        "Truncate table",
        "DELETE on SQLite, TRUNCATE elsewhere",
    ),
    ("Design", "schema.renameTable", "Rename table", ""),
    ("Design", "schema.addColumn", "Add column", ""),
    (
        "Design",
        "schema.dropColumn",
        "Drop column",
        "needs confirm",
    ),
    ("Design", "schema.renameColumn", "Rename column", ""),
    (
        "Design",
        "schema.changeColumnType",
        "Change data type",
        "refused with an explanation where the engine cannot",
    ),
    ("Design", "schema.setNullable", "Change NOT NULL", ""),
    ("Design", "schema.setDefault", "Change default", ""),
    (
        "Design",
        "schema.createIndex",
        "Create index",
        "unique supported",
    ),
    ("Design", "schema.dropIndex", "Drop index", "needs confirm"),
    (
        "Design",
        "schema.addForeignKey",
        "Add foreign key",
        "SQLite refused: recreate the table",
    ),
    ("Design", "schema.createView", "Create or replace view", ""),
    ("Design", "schema.dropView", "Drop view", ""),
    (
        "Design",
        "schema.search",
        "Search objects",
        "matches table and column names",
    ),
    (
        "Design",
        "schema.emptyDatabase",
        "Empty database",
        "drops every user table, needs confirm",
    ),
    (
        "Design",
        "schema.triggerEditor",
        "Trigger and procedure editors",
        "not implemented",
    ),
    (
        "Design",
        "schema.functionEditor",
        "Function editor with debugger",
        "not implemented",
    ),
    (
        "Design",
        "schema.sequenceEditor",
        "Sequence and type editors",
        "not implemented",
    ),
    (
        "Design",
        "schema.tableRebuild",
        "Rebuild a table in place",
        "SQLite DDL that needs it says so instead",
    ),
    // ---------------------------------------------------------------- Query
    ("Query", "sql.newQuery", "New query", "editor per tab"),
    (
        "Query",
        "sql.run",
        "Run script",
        "every statement, bounded by max_rows",
    ),
    ("Query", "sql.runStatement", "Run statement at cursor", ""),
    ("Query", "sql.runSelection", "Run the selection", ""),
    (
        "Query",
        "sql.statements",
        "Statement list",
        "the split a run would use",
    ),
    (
        "Query",
        "sql.format",
        "Format SQL",
        "keyword case from preferences",
    ),
    (
        "Query",
        "sql.complete",
        "Code completion",
        "tables, columns of the tables in the statement, keywords, functions",
    ),
    (
        "Query",
        "sql.explain",
        "Explain",
        "EXPLAIN, with EXPLAIN ANALYZE on demand",
    ),
    ("Query", "sql.history", "Query history", "kept per session"),
    ("Query", "sql.clearHistory", "Clear history", ""),
    (
        "Query",
        "sql.transaction",
        "Transaction control",
        "begin, commit, rollback on the live connection",
    ),
    (
        "Query",
        "sql.runInTransaction",
        "Run atomically",
        "rolls back at the first failure",
    ),
    (
        "Query",
        "sql.stop",
        "Stop a running statement",
        "not implemented; runs are row-bounded instead",
    ),
    (
        "Query",
        "sql.parameterised",
        "Parameterised queries",
        "not implemented; literals are escaped on the client",
    ),
    (
        "Query",
        "sql.debugger",
        "Stored procedure debugger",
        "not implemented",
    ),
    (
        "Query",
        "sql.profiler",
        "Query profiler",
        "not implemented; EXPLAIN covers the common case",
    ),
    (
        "Query",
        "sql.planVisualiser",
        "Visual explain plan",
        "explain output is tabular, not drawn",
    ),
    // ---------------------------------------------------------------- Import/Export
    (
        "Import/Export",
        "tools.importCsv",
        "Import CSV",
        "type inference, optional table creation",
    ),
    (
        "Import/Export",
        "tools.importJson",
        "Import JSON",
        "array of objects, or {\"rows\": [...]}",
    ),
    (
        "Import/Export",
        "tools.importSql",
        "Run a .sql file",
        "statement splitter understands quotes and dollar quoting",
    ),
    (
        "Import/Export",
        "tools.exportQuery",
        "Export a query result",
        "csv, json, sql, markdown",
    ),
    (
        "Import/Export",
        "tools.datagen",
        "Data generation",
        "type and name aware, deterministic by seed",
    ),
    ("Import/Export", "tools.generators", "Generator list", ""),
    (
        "Import/Export",
        "tools.exportExcel",
        "Export to Excel",
        "query result as a plain one-sheet workbook",
    ),
    (
        "Import/Export",
        "tools.importExcel",
        "Import from Excel",
        "first sheet of an .xlsx, every cell as text, like the CSV import",
    ),
    (
        "Import/Export",
        "tools.importXml",
        "Import XML",
        "not implemented",
    ),
    (
        "Import/Export",
        "tools.importDbf",
        "Import DBF",
        "not implemented",
    ),
    (
        "Import/Export",
        "tools.importWizard",
        "Mapping wizard with preview",
        "columns map by name via columnMapping (src=target), preview is the first rows",
    ),
    // ---------------------------------------------------------------- Transfer and sync
    (
        "Transfer",
        "transfer.data",
        "Data transfer",
        "any two open connections, across engines",
    ),
    (
        "Transfer",
        "transfer.preview",
        "Transfer preview",
        "row counts on both sides before copying",
    ),
    (
        "Transfer",
        "transfer.columnMapping",
        "Column mapping",
        "name mapping via columnMapping (src=target) for transfer.data and imports",
    ),
    (
        "Transfer",
        "transfer.speedTest",
        "Transfer speed settings",
        "pages of 2000 rows per transaction",
    ),
    (
        "Sync",
        "diff.structure",
        "Compare structures",
        "the DDL that would close the gap",
    ),
    (
        "Sync",
        "sync.structure",
        "Structure synchronization",
        "applies the diff, destructive steps need a flag",
    ),
    (
        "Sync",
        "sync.data",
        "Data synchronization",
        "row diff by primary key, dry run available",
    ),
    (
        "Sync",
        "sync.dataDelete",
        "Delete rows missing from the source",
        "part of the data sync diff",
    ),
    (
        "Sync",
        "sync.bidirectional",
        "Bi-directional sync",
        "not implemented; choose a source",
    ),
    (
        "Sync",
        "sync.conflictRules",
        "Conflict rules",
        "source wins, per column",
    ),
    // ---------------------------------------------------------------- Backup
    (
        "Backup",
        "backup.dump",
        "Backup",
        "SQL script for SQLite, pg_dump and mysqldump elsewhere",
    ),
    ("Backup", "backup.restore", "Restore", ""),
    (
        "Backup",
        "backup.list",
        "Backup list",
        "size and time per file",
    ),
    (
        "Backup",
        "backup.local",
        "Local backup copy",
        "not implemented; the file is written where you say",
    ),
    (
        "Backup",
        "backup.compression",
        "Compressed backups",
        "not implemented; the tools write plain files",
    ),
    (
        "Backup",
        "backup.encryption",
        "Encrypted backups",
        "not implemented",
    ),
    // ---------------------------------------------------------------- Automation
    ("Automation", "job.list", "Job list", ""),
    (
        "Automation",
        "job.create",
        "Create a job",
        "query, backup, restore, transfer, sync or datagen",
    ),
    ("Automation", "job.delete", "Delete a job", ""),
    ("Automation", "job.setEnabled", "Enable or disable", ""),
    ("Automation", "job.run", "Run now", ""),
    (
        "Automation",
        "job.runDue",
        "Scheduler tick",
        "every 30 s in the app, or nook-cli job serve",
    ),
    (
        "Automation",
        "job.logs",
        "Job log",
        "last runs with duration and result",
    ),
    (
        "Automation",
        "job.notifications",
        "Email or webhook on failure",
        "webhook POST on failure via the system curl; email is a webhook to a relay",
    ),
    (
        "Automation",
        "job.agent",
        "Deploy jobs to an automation agent",
        "not implemented; jobs run on this machine",
    ),
    (
        "Automation",
        "job.chain",
        "Chained jobs",
        "\"then\" runs jobs in order after a success; cycles rejected at create time",
    ),
    (
        "Automation",
        "job.profiles",
        "Job export and import",
        "the whole store is JSON under the app's config directory",
    ),
    // ---------------------------------------------------------------- Monitor
    (
        "Monitor",
        "monitor.processes",
        "Process list",
        "who is connected and what they are running",
    ),
    ("Monitor", "monitor.kill", "Kill a session", "needs confirm"),
    (
        "Monitor",
        "monitor.variables",
        "Server variables",
        "filterable",
    ),
    (
        "Monitor",
        "monitor.status",
        "Server status",
        "the numbers each engine exposes",
    ),
    (
        "Monitor",
        "monitor.dashboard",
        "Dashboard",
        "biggest tables by size and row estimate",
    ),
    (
        "Monitor",
        "monitor.charts",
        "Live charts",
        "the dashboard data feeds the report charts",
    ),
    (
        "Monitor",
        "monitor.locks",
        "Lock and deadlock reports",
        "not implemented",
    ),
    (
        "Monitor",
        "monitor.replication",
        "Replication status",
        "not implemented",
    ),
    (
        "Monitor",
        "monitor.alerts",
        "Threshold alerts",
        "not implemented",
    ),
    // ---------------------------------------------------------------- Privileges
    (
        "Privileges",
        "priv.users",
        "Users and roles",
        "Postgres and MySQL",
    ),
    ("Privileges", "priv.grants", "Show grants", ""),
    ("Privileges", "priv.createUser", "Create user", ""),
    ("Privileges", "priv.dropUser", "Drop user", "needs confirm"),
    (
        "Privileges",
        "priv.setPassword",
        "Set password",
        "never echoed back",
    ),
    (
        "Privileges",
        "priv.grant",
        "Grant",
        "privileges checked against a known list",
    ),
    ("Privileges", "priv.revoke", "Revoke", ""),
    (
        "Privileges",
        "priv.roles",
        "Role membership editor",
        "memberships are shown in the grant list",
    ),
    (
        "Privileges",
        "priv.audit",
        "Privilege audit report",
        "not implemented",
    ),
    // ---------------------------------------------------------------- Report and modeler
    (
        "Report",
        "report.aggregate",
        "Aggregate report",
        "count, sum, avg, min, max, count distinct",
    ),
    (
        "Report",
        "report.chart",
        "Charts",
        "column, bar, line, area, pie, scatter, table",
    ),
    (
        "Report",
        "report.pivot",
        "Pivot table",
        "row key, column key, measure",
    ),
    (
        "Report",
        "report.designer",
        "Report designer with layout",
        "not implemented; charts are live views",
    ),
    (
        "Report",
        "report.biDashboard",
        "BI dashboard",
        "not implemented",
    ),
    (
        "Report",
        "report.export",
        "Export a report",
        "export the underlying result as csv or markdown",
    ),
    (
        "Modeler",
        "diagram.erDiagram",
        "ER diagram",
        "boxes per table, arrows per foreign key",
    ),
    (
        "Modeler",
        "diagram.exportSvg",
        "Export the diagram",
        "SVG, opens in any browser",
    ),
    (
        "Modeler",
        "diagram.tableCard",
        "Table card",
        "columns, keys, what references it",
    ),
    (
        "Modeler",
        "diagram.reverseEngineer",
        "Reverse engineer a model",
        "the diagram is generated from the live catalog",
    ),
    (
        "Modeler",
        "diagram.forwardEngineer",
        "Forward engineer a model to DDL",
        "use schema.createTable or sync.structure",
    ),
    (
        "Modeler",
        "diagram.manualLayout",
        "Move boxes by hand",
        "not implemented; the layout is deterministic",
    ),
    (
        "Modeler",
        "diagram.modelFile",
        "Save a model file",
        "not implemented; the catalog is the source of truth",
    ),
    // ---------------------------------------------------------------- Tools and platform
    (
        "Tools",
        "tools.queryBuilder",
        "Visual query builder",
        "tables, joins, columns, filters into one SELECT",
    ),
    (
        "Tools",
        "app.settings",
        "Preferences",
        "editor, grid and connection defaults",
    ),
    ("Tools", "app.setSetting", "Change a preference", ""),
    ("Tools", "app.log", "Message log", ""),
    ("Tools", "app.clearLog", "Clear the log", ""),
    (
        "Tools",
        "app.help",
        "Command reference",
        "every command with its parameters",
    ),
    (
        "Tools",
        "app.about",
        "About",
        "version, licence, command count",
    ),
    ("Tools", "tab.new", "Query tabs", ""),
    ("Tools", "tab.newData", "Data tabs", ""),
    ("Tools", "tab.list", "Tab list", ""),
    ("Tools", "tab.select", "Switch tab", ""),
    (
        "Tools",
        "tab.close",
        "Close tab",
        "refused while edits are pending",
    ),
    ("Tools", "tab.rename", "Rename tab", ""),
    (
        "Tools",
        "tab.setEditorText",
        "Editor text",
        "used by the control channel and MCP",
    ),
    ("Tools", "snippet.list", "Snippet list", ""),
    ("Tools", "snippet.save", "Save a snippet", ""),
    ("Tools", "snippet.delete", "Delete a snippet", ""),
    (
        "Tools",
        "sql.snippets",
        "Code snippets",
        "built-in set plus your own",
    ),
    (
        "Tools",
        "tools.dataProfiling",
        "Data profiling",
        "not implemented; use an aggregate report",
    ),
    (
        "Tools",
        "tools.dataDictionary",
        "Data dictionary export",
        "schema.ddl plus the diagram SVG",
    ),
    (
        "Tools",
        "tools.serverSearch",
        "Server-wide search",
        "schema.search covers the cached catalog",
    ),
    (
        "Tools",
        "tools.textImporter",
        "Import a text file as a table",
        "not implemented",
    ),
    (
        "Tools",
        "tools.updater",
        "Auto-update",
        "not implemented; builds are local",
    ),
    (
        "Cloud",
        "cloud.team",
        "Team collaboration",
        "not implemented",
    ),
    (
        "Cloud",
        "cloud.syncSettings",
        "Cloud-synced settings",
        "settings are JSON on this machine",
    ),
    (
        "Cloud",
        "cloud.sharedQueries",
        "Shared query library",
        "snippets are local to this machine",
    ),
    (
        "Platform",
        "app.platform",
        "Native desktop app",
        "egui on the GPU: no Electron, no browser",
    ),
    (
        "Platform",
        "app.mcp",
        "Agent access over MCP",
        "every command is an MCP tool",
    ),
    (
        "Platform",
        "platform.control",
        "JSON control channel",
        "drive the running app over a local socket",
    ),
    (
        "Platform",
        "platform.cli",
        "Command line tool",
        "nook-cli exists and shares command ids; not a command itself",
    ),
    (
        "Platform",
        "platform.i18n",
        "Localisation",
        "English only for now",
    ),
    (
        "Platform",
        "platform.windows",
        "Windows build",
        "not built here; cargo builds it",
    ),
    (
        "Platform",
        "platform.linux",
        "Linux build",
        "not built here; cargo builds it",
    ),
    (
        "Platform",
        "platform.web",
        "Browser build",
        "not implemented; this is a native tool",
    ),
    // ---------------------------------------------------------------- AI
    (
        "AI",
        "ai.generate",
        "Generate SQL",
        "one sentence in, SQL out; an @table mention pulls in that table's structure",
    ),
    (
        "AI",
        "ai.providers",
        "AI providers",
        "OpenAI, Anthropic, DeepSeek, Kimi, GLM, Ollama and vLLM, each with its own base URL",
    ),
    (
        "AI",
        "ai.setKey",
        "AI API key",
        "stored in the login keychain, never in the profile file",
    ),
    (
        "AI",
        "ai.clearKey",
        "Forget an AI key",
        "removes the keychain entry",
    ),
    (
        "AI",
        "ai.tables",
        "Tables for the prompt",
        "the candidates an @ mention resolves against, from the live catalog",
    ),
];

pub fn catalog() -> &'static [CatalogEntry] {
    static PARSED: std::sync::OnceLock<Vec<CatalogEntry>> = std::sync::OnceLock::new();
    PARSED.get_or_init(|| {
        ENTRIES
            .iter()
            .map(|(area, id, label, note)| CatalogEntry {
                area,
                id,
                label,
                note,
            })
            .collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_ids_are_unique_and_the_list_is_real() {
        let mut ids: Vec<&str> = catalog().iter().map(|e| e.id).collect();
        let before = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(before, ids.len(), "duplicate catalog ids");
        assert!(before > 150, "the catalog should be a real feature list");
        assert!(catalog().iter().all(|e| !e.label.trim().is_empty()));
    }
}
