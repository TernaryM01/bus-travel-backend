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
//!   sea-orm-cli generates). See `seaorm_compat::to_cached_row`.
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
//!   order. A result is only cached when that can't matter; see
//!   `seaorm_compat::safe_to_cache`.
//!
//! Layout: this file is the cache itself (table generations, invalidation,
//! transactions). `classify` reads the SQL; `seaorm_compat` holds the SeaORM
//! workarounds, which can go once SeaORM is fixed.

mod classify;
mod seaorm_compat;

use std::collections::{BTreeSet, HashMap, HashSet};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicU64, Ordering::SeqCst};
use std::sync::{Arc, Mutex};

use moka::future::Cache;
use sea_orm::{
    ConnectionTrait, DatabaseConnection, DatabaseTransaction, DbBackend, DbErr, ExecResult,
    FromQueryResult, ProxyRow, QueryResult, Statement, TransactionTrait,
};
use sea_orm::sea_query::Value;

use classify::{Kind, classify_sql};
use seaorm_compat::{load_columns, safe_to_cache, to_cached_row};

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
        let columns = load_columns(&inner).await?;
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
