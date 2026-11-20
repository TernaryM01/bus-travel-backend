//! A drop-in replacement for `sea_orm::DatabaseConnection` that answers repeated
//! reads from memory and stays correct without any per-query code.
//!
//! Handlers use it exactly like a `DatabaseConnection` (`.all(&state.db)`,
//! `.insert(&state.db)`, ...). There is nothing to remember when adding queries.
//!
//! Assumptions this relies on:
//! - This process is the only writer to the database, and only one instance runs at
//!   a time. (A redeploy that briefly runs old and new side by side, scaling out, or
//!   editing data by hand with psql would leave this cache serving old data until
//!   restart.)
//! - Entity fields for `timestamptz` columns use `DateTimeWithTimeZone` (what
//!   sea-orm-cli generates). See `to_cached_row`.
//!
//! How it stays correct:
//! - Every SQL statement SeaORM generates passes through `ConnectionTrait`, so this
//!   type sees all of them. It parses the SQL to learn which tables a statement reads
//!   or writes.
//! - Each table has a generation number. A cached result is stored under a key that
//!   includes the generation of every table the query read.
//! - Any write (INSERT / UPDATE / DELETE, or anything that is not a plain SELECT)
//!   bumps the generation of the tables it touches, plus every table that a foreign
//!   key cascade could change. Old entries can then never be hit again; the
//!   size-capped cache drops them in due course.
//! - The generations are read BEFORE the remote read starts, so a read that races
//!   with a write can only ever file its result under the old generation.
//! - Anything it doesn't understand (unparseable SQL, unknown tables, views,
//!   functions not on the allowlist, FOR UPDATE) is not cached; any write it doesn't
//!   understand bumps everything. Mistakes cost performance, never correctness.
//! - SeaORM keeps cached rows in a map sorted by column name, so reading a cached
//!   row by position (as `into_tuple()` does) gives the columns in alphabetical
//!   order. A result is only cached when that can't matter; see `safe_to_cache`.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::ops::ControlFlow;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicU64, Ordering::SeqCst};
use std::sync::{Arc, Mutex};

use moka::future::Cache;
use sea_orm::{
    ConnectionTrait, DatabaseConnection, DatabaseTransaction, DbBackend, DbErr, ExecResult,
    FromQueryResult, ProxyRow, QueryResult, Statement, TransactionTrait,
    from_query_result_to_proxy_row,
};
use sea_orm::sea_query::Value;
use sqlparser::ast::{Expr, ObjectName, ObjectNamePart, Statement as SqlStatement};
use sqlparser::dialect::PostgreSqlDialect;
use sqlparser::parser::Parser;

/// Functions whose result depends only on their arguments. A query calling any
/// other function (now(), random(), ...) is never cached.
const DETERMINISTIC_FUNCTIONS: &[&str] = &[
    "count", "sum", "avg", "min", "max", "coalesce", "nullif", "lower", "upper",
    "length", "abs", "round", "floor", "ceil", "greatest", "least", "concat",
];

#[derive(Clone, Debug)]
enum Kind {
    /// A plain SELECT over these base tables: cacheable.
    Read(Vec<String>),
    /// Must go to the database but changes nothing (e.g. SELECT ... FOR UPDATE).
    Bypass,
    /// Changes these tables (cascades not yet added). Empty = unknown, bump all.
    Write(Vec<String>),
}

#[derive(Clone, Hash, PartialEq, Eq)]
struct Key {
    one: bool,
    sql: Arc<str>,
    values: String,
    gens: Vec<u64>,
}

#[derive(Default)]
pub struct Stats {
    pub hits: AtomicU64,
    pub misses: AtomicU64,
    pub bypassed: AtomicU64,
    pub writes: AtomicU64,
}

struct Shared {
    rows: Cache<Key, Arc<Vec<ProxyRow>>>,
    parsed: Cache<Arc<str>, Kind>,
    /// Base tables known at startup, each with its generation.
    gens: HashMap<String, AtomicU64>,
    /// Bumped by writes the cache can't attribute to specific tables.
    global: AtomicU64,
    /// Table -> every table a write to it may change (itself, cascades, transitively).
    affects: HashMap<String, BTreeSet<String>>,
    /// Tables with user-defined triggers: their writes bump everything.
    triggered: HashSet<String>,
    /// Each table's columns, to recognise results made of whole rows.
    columns: HashMap<String, BTreeSet<String>>,
    pub stats: Stats,
}

#[derive(Clone)]
pub struct CachedDb {
    inner: DatabaseConnection,
    shared: Arc<Shared>,
}

impl CachedDb {
    /// Build after migrations have run: the schema is read once from pg_catalog.
    pub async fn new(inner: DatabaseConnection, max_bytes: u64) -> Result<Self, DbErr> {
        #[derive(FromQueryResult)]
        struct Name {
            name: String,
        }
        #[derive(FromQueryResult)]
        struct Fk {
            child: String,
            parent: String,
            on_delete: String,
            on_update: String,
        }
        let tables = Name::find_by_statement(Statement::from_string(
            DbBackend::Postgres,
            "SELECT relname::text AS name FROM pg_class \
             WHERE relkind IN ('r','p') AND relnamespace = 'public'::regnamespace",
        ))
        .all(&inner)
        .await?;
        let fks = Fk::find_by_statement(Statement::from_string(
            DbBackend::Postgres,
            "SELECT ch.relname::text AS child, pa.relname::text AS parent, \
                    con.confdeltype::text AS on_delete, con.confupdtype::text AS on_update \
             FROM pg_constraint con \
             JOIN pg_class ch ON ch.oid = con.conrelid \
             JOIN pg_class pa ON pa.oid = con.confrelid \
             WHERE con.contype = 'f'",
        ))
        .all(&inner)
        .await?;
        #[derive(FromQueryResult)]
        struct Col {
            table_name: String,
            column_name: String,
        }
        let cols = Col::find_by_statement(Statement::from_string(
            DbBackend::Postgres,
            "SELECT c.relname::text AS table_name, a.attname::text AS column_name \
             FROM pg_attribute a JOIN pg_class c ON c.oid = a.attrelid \
             WHERE c.relkind IN ('r','p') AND c.relnamespace = 'public'::regnamespace \
               AND a.attnum > 0 AND NOT a.attisdropped",
        ))
        .all(&inner)
        .await?;
        let mut columns: HashMap<String, BTreeSet<String>> = HashMap::new();
        for c in cols {
            columns.entry(c.table_name).or_default().insert(c.column_name);
        }
        let triggered = Name::find_by_statement(Statement::from_string(
            DbBackend::Postgres,
            "SELECT DISTINCT c.relname::text AS name FROM pg_trigger t \
             JOIN pg_class c ON c.oid = t.tgrelid WHERE NOT t.tgisinternal",
        ))
        .all(&inner)
        .await?;

        // 'a' = NO ACTION, 'r' = RESTRICT: the child is left alone. Anything else
        // (CASCADE, SET NULL, SET DEFAULT) means writing the parent changes the child.
        let mut direct: HashMap<String, Vec<String>> = HashMap::new();
        for fk in fks {
            let changes_child = |a: &str| a != "a" && a != "r";
            if changes_child(&fk.on_delete) || changes_child(&fk.on_update) {
                direct.entry(fk.parent).or_default().push(fk.child);
            }
        }
        let mut affects = HashMap::new();
        for t in &tables {
            let mut seen = BTreeSet::new();
            let mut stack = vec![t.name.clone()];
            while let Some(x) = stack.pop() {
                if seen.insert(x.clone()) {
                    stack.extend(direct.get(&x).cloned().unwrap_or_default());
                }
            }
            affects.insert(t.name.clone(), seen);
        }

        let rows = Cache::builder()
            .max_capacity(max_bytes)
            .weigher(|k: &Key, v: &Arc<Vec<ProxyRow>>| {
                let bytes = k.sql.len() + k.values.len() + v.iter().map(row_bytes).sum::<usize>();
                bytes.try_into().unwrap_or(u32::MAX)
            })
            .eviction_policy(moka::policy::EvictionPolicy::lru())
            .build();

        Ok(Self {
            inner,
            shared: Arc::new(Shared {
                rows,
                parsed: Cache::new(10_000),
                gens: tables.into_iter().map(|t| (t.name, AtomicU64::new(0))).collect(),
                global: AtomicU64::new(0),
                affects,
                triggered: triggered.into_iter().map(|t| t.name).collect(),
                columns,
                stats: Stats::default(),
            }),
        })
    }

    pub fn stats(&self) -> &Stats {
        &self.shared.stats
    }

    /// Start a transaction. Reads inside it skip the cache; its writes invalidate
    /// when it commits. (CachedDb deliberately does not implement TransactionTrait,
    /// so there is no way to get a raw, cache-blind transaction by accident.)
    pub async fn begin(&self) -> Result<CachedTxn, DbErr> {
        Ok(CachedTxn {
            txn: self.inner.begin().await?,
            db: self.clone(),
            touched: Mutex::new(Vec::new()),
        })
    }

    async fn classify(&self, sql: &str) -> Kind {
        let sql: Arc<str> = sql.into();
        if let Some(k) = self.shared.parsed.get(&sql).await {
            return k;
        }
        let kind = classify_sql(&sql, &self.shared.gens);
        self.shared.parsed.insert(sql, kind.clone()).await;
        kind
    }

    fn key(&self, one: bool, stmt: &Statement, tables: &[String]) -> Key {
        let mut gens = Vec::with_capacity(tables.len() + 1);
        gens.push(self.shared.global.load(SeqCst));
        gens.extend(tables.iter().map(|t| self.shared.gens[t].load(SeqCst)));
        Key {
            one,
            sql: stmt.sql.as_str().into(),
            values: format!("{:?}", stmt.values),
            gens,
        }
    }

    fn bump(&self, tables: &[String]) {
        self.shared.stats.writes.fetch_add(1, SeqCst);
        if tables.is_empty() || tables.iter().any(|t| self.shared.triggered.contains(t)) {
            self.shared.global.fetch_add(1, SeqCst);
            return;
        }
        for t in tables {
            match self.shared.affects.get(t) {
                Some(all) => all.iter().for_each(|a| {
                    self.shared.gens[a].fetch_add(1, SeqCst);
                }),
                None => {
                    self.shared.global.fetch_add(1, SeqCst);
                }
            }
        }
    }

    /// Run a read through the cache. `fetch` performs the remote read.
    async fn cached(
        &self,
        one: bool,
        stmt: &Statement,
        tables: &[String],
        fetch: impl AsyncFnOnce() -> Result<Vec<QueryResult>, DbErr>,
    ) -> Result<Vec<QueryResult>, DbErr> {
        let key = self.key(one, stmt, tables); // generations captured before the read
        if let Some(rows) = self.shared.rows.get(&key).await {
            self.shared.stats.hits.fetch_add(1, SeqCst);
            return Ok(rows.iter().cloned().map(QueryResult::from).collect());
        }
        self.shared.stats.misses.fetch_add(1, SeqCst);
        let results = fetch().await?;
        let names = results.first().map(|r| r.column_names()).unwrap_or_default();
        if !safe_to_cache(&names, tables, &self.shared.columns) {
            return Ok(results);
        }
        // SeaORM's converter panics on column types it doesn't know; just don't cache then.
        if let Ok(rows) = catch_unwind(AssertUnwindSafe(|| {
            results.iter().map(to_cached_row).collect::<Vec<_>>()
        })) {
            self.shared.rows.insert(key, Arc::new(rows)).await;
        }
        Ok(results)
    }
}

#[async_trait::async_trait]
impl ConnectionTrait for CachedDb {
    fn get_database_backend(&self) -> DbBackend {
        self.inner.get_database_backend()
    }

    async fn execute(&self, stmt: Statement) -> Result<ExecResult, DbErr> {
        let tables = match self.classify(&stmt.sql).await {
            Kind::Write(t) => t,
            _ => Vec::new(), // a "read" sent through execute: be conservative
        };
        let out = self.inner.execute(stmt).await;
        self.bump(&tables); // even on error: the outcome may be unknown
        out
    }

    async fn execute_unprepared(&self, sql: &str) -> Result<ExecResult, DbErr> {
        let out = self.inner.execute_unprepared(sql).await;
        self.bump(&[]);
        out
    }

    async fn query_one(&self, stmt: Statement) -> Result<Option<QueryResult>, DbErr> {
        match self.classify(&stmt.sql).await {
            Kind::Read(tables) => {
                let inner = &self.inner;
                let s = stmt.clone();
                let rows = self
                    .cached(true, &stmt, &tables, async || {
                        Ok(inner.query_one(s).await?.into_iter().collect())
                    })
                    .await?;
                Ok(rows.into_iter().next())
            }
            Kind::Bypass => {
                self.shared.stats.bypassed.fetch_add(1, SeqCst);
                self.inner.query_one(stmt).await
            }
            // INSERT/UPDATE ... RETURNING arrive here too.
            Kind::Write(tables) => {
                let out = self.inner.query_one(stmt).await;
                self.bump(&tables);
                out
            }
        }
    }

    async fn query_all(&self, stmt: Statement) -> Result<Vec<QueryResult>, DbErr> {
        match self.classify(&stmt.sql).await {
            Kind::Read(tables) => {
                let inner = &self.inner;
                let s = stmt.clone();
                self.cached(false, &stmt, &tables, async || inner.query_all(s).await)
                    .await
            }
            Kind::Bypass => {
                self.shared.stats.bypassed.fetch_add(1, SeqCst);
                self.inner.query_all(stmt).await
            }
            Kind::Write(tables) => {
                let out = self.inner.query_all(stmt).await;
                self.bump(&tables);
                out
            }
        }
    }

    fn support_returning(&self) -> bool {
        self.inner.support_returning()
    }
}

/// A transaction: reads go straight to it, writes are remembered and invalidated
/// once it commits.
pub struct CachedTxn {
    txn: DatabaseTransaction,
    db: CachedDb,
    touched: Mutex<Vec<String>>,
}

impl CachedTxn {
    pub async fn commit(self) -> Result<(), DbErr> {
        let touched = std::mem::take(&mut *self.touched.lock().unwrap());
        let out = self.txn.commit().await;
        // Always bump, also when commit errors: the commit may have happened anyway.
        // An empty list here just means nothing was written.
        if !touched.is_empty() {
            self.db.bump(&touched);
        }
        out
    }

    pub async fn rollback(self) -> Result<(), DbErr> {
        self.txn.rollback().await
    }

    async fn note(&self, sql: &str) {
        match self.db.classify(sql).await {
            Kind::Read(_) | Kind::Bypass => {}
            Kind::Write(t) if !t.is_empty() => self.touched.lock().unwrap().extend(t),
            // Unknown write: remember it as "everything".
            Kind::Write(_) => {
                self.touched.lock().unwrap().extend(self.db.shared.gens.keys().cloned())
            }
        }
    }
}

#[async_trait::async_trait]
impl ConnectionTrait for CachedTxn {
    fn get_database_backend(&self) -> DbBackend {
        self.txn.get_database_backend()
    }
    async fn execute(&self, stmt: Statement) -> Result<ExecResult, DbErr> {
        self.note(&stmt.sql).await;
        self.txn.execute(stmt).await
    }
    async fn execute_unprepared(&self, sql: &str) -> Result<ExecResult, DbErr> {
        self.touched.lock().unwrap().extend(self.db.shared.gens.keys().cloned());
        self.txn.execute_unprepared(sql).await
    }
    async fn query_one(&self, stmt: Statement) -> Result<Option<QueryResult>, DbErr> {
        self.note(&stmt.sql).await;
        self.txn.query_one(stmt).await
    }
    async fn query_all(&self, stmt: Statement) -> Result<Vec<QueryResult>, DbErr> {
        self.note(&stmt.sql).await;
        self.txn.query_all(stmt).await
    }
    fn support_returning(&self) -> bool {
        self.txn.support_returning()
    }
}

/// Whether a result with these columns (in SELECT order) reads back correctly from
/// the cache whichever way SeaORM reads it. SeaORM keeps cached rows in a map
/// sorted by column name, so reading by position returns columns alphabetically.
/// Safe when:
/// - there are no rows (nothing to misread), or
/// - the names are unique and already in alphabetical order (by position and by
///   name agree), or
/// - the result is made of whole rows of the tables read, each table's columns
///   optionally prefixed `A_`, `B_`, ... as in SeaORM joins: SeaORM reads entity
///   models by name.
/// Anything else (e.g. a partial `select_only()` read with `into_tuple()`) is not
/// cached. That costs a database round-trip, never a wrong answer.
///
/// Workaround for a SeaORM 1.1.20 limitation: `ProxyRow` stores values in a
/// `BTreeMap` (sorted by name), and reading by index takes the n-th entry of that
/// map, not the n-th column of the SELECT. If a later SeaORM keeps column order
/// in `ProxyRow` (e.g. an `IndexMap`), this function and the `columns` catalog
/// query in `CachedDb::new` can be removed, and every result cached; check 7 in
/// the cache-tests kit tells you whether positional reads come back right.
fn safe_to_cache(
    names: &[String],
    tables: &[String],
    columns: &HashMap<String, BTreeSet<String>>,
) -> bool {
    if names.is_empty() {
        return true;
    }
    let mut unique = HashSet::new();
    if !names.iter().all(|n| unique.insert(n)) {
        return false;
    }
    if names.windows(2).all(|w| w[0] < w[1]) {
        return true;
    }
    // Group the names by SeaORM's join prefix ("A_id", "B_id", ...; none for a plain
    // select), then every group must be exactly the columns of one table read.
    let mut groups: HashMap<&str, BTreeSet<String>> = HashMap::new();
    for n in names {
        let b = n.as_bytes();
        let (prefix, col) = if b.len() > 2 && b[0].is_ascii_uppercase() && b[1] == b'_' {
            (&n[..2], &n[2..])
        } else {
            ("", n.as_str())
        };
        groups.entry(prefix).or_default().insert(col.to_string());
    }
    if groups.contains_key("") && groups.len() > 1 {
        return false;
    }
    groups.values().all(|cols| {
        tables
            .iter()
            .any(|t| columns.get(t).is_some_and(|tc| tc == cols))
    })
}

fn table_name(name: &ObjectName) -> Option<String> {
    match name.0.last()? {
        ObjectNamePart::Identifier(id) if id.quote_style.is_some() => Some(id.value.clone()),
        ObjectNamePart::Identifier(id) => Some(id.value.to_lowercase()),
        _ => None,
    }
}

fn classify_sql(sql: &str, known: &HashMap<String, AtomicU64>) -> Kind {
    let Ok(stmts) = Parser::parse_sql(&PostgreSqlDialect {}, sql) else {
        return Kind::Write(Vec::new());
    };
    let [stmt] = stmts.as_slice() else {
        return Kind::Write(Vec::new());
    };

    let mut tables = BTreeSet::new();
    let mut unknown_relation = false;
    let _ = sqlparser::ast::visit_relations(stmt, |r| {
        match table_name(r) {
            Some(t) => {
                if !known.contains_key(&t) {
                    unknown_relation = true; // a view, CTE name, other schema, ...
                }
                tables.insert(t);
            }
            None => unknown_relation = true,
        }
        ControlFlow::<()>::Continue(())
    });

    // Any statement nested anywhere that is not a query (e.g. a data-modifying CTE)
    // makes the whole thing a write.
    let mut writes = false;
    let _ = sqlparser::ast::visit_statements(stmt, |s| {
        if !matches!(s, SqlStatement::Query(_)) {
            writes = true;
        }
        ControlFlow::<()>::Continue(())
    });
    if writes {
        let known_tables: Vec<String> =
            tables.into_iter().filter(|t| known.contains_key(t)).collect();
        return if unknown_relation { Kind::Write(Vec::new()) } else { Kind::Write(known_tables) };
    }

    let SqlStatement::Query(q) = stmt else { return Kind::Write(Vec::new()) };
    if !q.locks.is_empty() || unknown_relation || tables.is_empty() {
        return Kind::Bypass;
    }
    let mut nondeterministic = false;
    let _ = sqlparser::ast::visit_expressions(stmt, |e| {
        if let Expr::Function(f) = e {
            let name = f.name.0.last().and_then(|p| p.as_ident()).map(|i| i.value.to_lowercase());
            if !name.is_some_and(|n| DETERMINISTIC_FUNCTIONS.contains(&n.as_str())) {
                nondeterministic = true;
            }
        }
        ControlFlow::<()>::Continue(())
    });
    if nondeterministic {
        return Kind::Bypass;
    }
    Kind::Read(tables.into_iter().collect())
}

/// SeaORM's converter reads `timestamptz` as `DateTime<Utc>`, but a cached row is
/// decoded strictly by the entity's field type, and entities use
/// `DateTimeWithTimeZone` (the type sea-orm-cli generates). Convention: every
/// timestamptz field is `DateTimeWithTimeZone`. Breaking it fails loudly on the
/// first cache hit (a decode error), never silently.
///
/// Workaround for a SeaORM 1.1.20 limitation: decoding a `ProxyRow` value
/// (`ValueType::try_from`) accepts only the exact `Value` variant, with no
/// conversion between `ChronoDateTimeUtc` and `ChronoDateTimeWithTimeZone`.
/// After a SeaORM upgrade, check whether a `ProxyRow` holding
/// `ChronoDateTimeUtc` decodes into a `DateTimeWithTimeZone` field (check 1 in
/// the cache-tests kit covers it). If it does, this function can be replaced by
/// plain `from_query_result_to_proxy_row`, and the convention dropped.
fn to_cached_row(result: &QueryResult) -> ProxyRow {
    let mut row = from_query_result_to_proxy_row(result);
    for v in row.values.values_mut() {
        if let Value::ChronoDateTimeUtc(t) = v {
            *v = Value::ChronoDateTimeWithTimeZone(t.take().map(|t| Box::new(t.fixed_offset())));
        }
    }
    row
}

fn row_bytes(row: &ProxyRow) -> usize {
    row.values
        .iter()
        .map(|(k, v)| {
            k.len()
                + 16
                + match v {
                    Value::String(Some(s)) => s.len(),
                    Value::Bytes(Some(b)) => b.len(),
                    _ => 16,
                }
        })
        .sum::<usize>()
        + 32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn known() -> HashMap<String, AtomicU64> {
        ["user", "booking", "journey", "city"]
            .into_iter()
            .map(|t| (t.to_string(), AtomicU64::new(0)))
            .collect()
    }

    #[test]
    fn caches_only_what_reads_back_correctly() {
        let cols: HashMap<String, BTreeSet<String>> = [
            ("journey", vec!["id", "total_seats", "status"]),
            ("booking", vec!["id", "journey_id", "seats"]),
        ]
        .into_iter()
        .map(|(t, c)| (t.to_string(), c.into_iter().map(String::from).collect()))
        .collect();
        let names = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let j = vec!["journey".to_string()];
        let jb = vec!["journey".to_string(), "booking".to_string()];
        let safe = |n: &[&str], t: &[String]| safe_to_cache(&names(n), t, &cols);

        assert!(safe(&[], &j)); // no rows
        assert!(safe(&["num_items"], &j)); // a count
        assert!(safe(&["journey_id", "seats"], &jb)); // partial, but alphabetical
        assert!(safe(&["id", "total_seats", "status"], &j)); // whole journey rows
        assert!(safe(&["A_id", "A_total_seats", "A_status", "B_id", "B_journey_id", "B_seats"], &jb)); // join
        assert!(!safe(&["seats", "journey_id"], &jb)); // partial, out of order: by position would swap
        assert!(!safe(&["total_seats", "id"], &j)); // part of a row, out of order
        assert!(!safe(&["id", "id"], &jb)); // duplicate names collapse in the map
        assert!(!safe(&["id", "total_seats", "status", "B_id", "B_journey_id", "B_seats"], &jb));
    }

    #[test]
    fn classifies() {
        let k = known();
        let read = |s| match classify_sql(s, &k) {
            Kind::Read(t) => t.join(","),
            other => format!("{other:?}"),
        };
        assert_eq!(read(r#"SELECT "booking"."id" FROM "booking" WHERE "booking"."journey_id" = $1"#), "booking");
        assert_eq!(read(r#"SELECT COUNT(*) AS num_items FROM (SELECT "booking"."id" FROM "booking") AS "sub""#), "booking");
        assert_eq!(read(r#"SELECT a.id FROM "journey" a JOIN "city" c ON c.id = a.origin_city_id"#), "city,journey");
        assert!(matches!(classify_sql(r#"SELECT * FROM "booking" WHERE created_at > now()"#, &k), Kind::Bypass));
        assert!(matches!(classify_sql(r#"SELECT * FROM "booking" FOR UPDATE"#, &k), Kind::Bypass));
        assert!(matches!(classify_sql(r#"SELECT * FROM some_view"#, &k), Kind::Bypass));
        assert!(matches!(classify_sql(r#"DELETE FROM "user" WHERE "user"."id" = $1"#, &k), Kind::Write(t) if t == ["user"]));
        assert!(matches!(classify_sql(r#"INSERT INTO "booking" ("id") VALUES ($1) RETURNING "id""#, &k), Kind::Write(t) if t == ["booking"]));
        assert!(matches!(classify_sql(r#"UPDATE "journey" SET "driver_id" = $1 WHERE "journey"."id" = $2 RETURNING "id""#, &k), Kind::Write(t) if t == ["journey"]));
        assert!(matches!(classify_sql("CREATE TABLE x (id int)", &k), Kind::Write(t) if t.is_empty()));
        assert!(matches!(classify_sql("this is not sql", &k), Kind::Write(t) if t.is_empty()));
    }
}
