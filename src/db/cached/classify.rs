//! Reading SQL: which tables a statement reads or writes, and whether a read can
//! be cached. Knows nothing about SeaORM.

use std::collections::{BTreeSet, HashMap};
use std::ops::ControlFlow;
use std::sync::atomic::AtomicU64;

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
pub(super) enum Kind {
    /// A plain SELECT over these base tables: cacheable.
    Read(Vec<String>),
    /// Must go to the database but changes nothing (e.g. SELECT ... FOR UPDATE).
    Bypass,
    /// Changes these tables (cascades not yet added). Empty = unknown, bump all.
    Write(Vec<String>),
}

fn table_name(name: &ObjectName) -> Option<String> {
    match name.0.last()? {
        ObjectNamePart::Identifier(id) if id.quote_style.is_some() => Some(id.value.clone()),
        ObjectNamePart::Identifier(id) => Some(id.value.to_lowercase()),
        _ => None,
    }
}

pub(super) fn classify_sql(sql: &str, known: &HashMap<String, AtomicU64>) -> Kind {
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
