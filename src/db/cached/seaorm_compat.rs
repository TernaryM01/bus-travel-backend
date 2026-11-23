//! Workarounds for SeaORM 1.1.20 limitations in how cached rows (`ProxyRow`) are
//! read back. Each item says what would let it be deleted after a SeaORM upgrade.

use std::collections::{BTreeSet, HashMap, HashSet};

use sea_orm::{
    DatabaseConnection, DbBackend, DbErr, FromQueryResult, ProxyRow, QueryResult, Statement,
    from_query_result_to_proxy_row,
};
use sea_orm::sea_query::Value;

/// Each table's columns, read once from pg_catalog, to recognise results made of
/// whole rows (see `safe_to_cache`).
pub(super) async fn load_columns(
    inner: &DatabaseConnection,
) -> Result<HashMap<String, BTreeSet<String>>, DbErr> {
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
    .all(inner)
    .await?;
    let mut columns: HashMap<String, BTreeSet<String>> = HashMap::new();
    for c in cols {
        columns.entry(c.table_name).or_default().insert(c.column_name);
    }
    Ok(columns)
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
/// in `ProxyRow` (e.g. an `IndexMap`), this function and `load_columns` above
/// can be removed, and every result cached; check 7 in the cache-tests kit tells
/// you whether positional reads come back right.
pub(super) fn safe_to_cache(
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
pub(super) fn to_cached_row(result: &QueryResult) -> ProxyRow {
    let mut row = from_query_result_to_proxy_row(result);
    for v in row.values.values_mut() {
        if let Value::ChronoDateTimeUtc(t) = v {
            *v = Value::ChronoDateTimeWithTimeZone(t.take().map(|t| Box::new(t.fixed_offset())));
        }
    }
    row
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
