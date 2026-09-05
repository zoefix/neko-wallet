//! Bringing an existing vault up to the current schema.
//!
//! There was no runner before this: `0001` created a database and that was
//! that. Adding a chain needs the schema to change, and the databases that
//! need changing are the ones with funds in them - so this has to be careful
//! in ways a fresh install never exercises.
//!
//! Three properties, each deliberate:
//!
//! * **One transaction per migration.** A power cut leaves the database at the
//!   version before or the version after, never between.
//! * **Refuses to open a database from the future.** A vault touched by a
//!   newer build is not something an older one should guess about; the file is
//!   the user's only copy.
//! * **Foreign keys are suspended only while a table is rebuilt**, because
//!   SQLite cannot alter a CHECK constraint in place and `balances` points at
//!   the table being replaced.

use rusqlite::Connection;

use crate::error::StoreError;
use crate::vault_row::{schema_version, CURRENT_SCHEMA};

struct Migration {
    to: i32,
    sql: &'static str,
}

const MIGRATIONS: &[Migration] = &[
    Migration {
        to: 2,
        sql: include_str!("../migrations/0002_bsc.sql"),
    },
    Migration {
        to: 3,
        sql: include_str!("../migrations/0003_solana.sql"),
    },
    Migration {
        to: 4,
        sql: include_str!("../migrations/0004_bitcoin.sql"),
    },
    Migration {
        to: 5,
        sql: include_str!("../migrations/0005_ethereum.sql"),
    },
    Migration {
        to: 6,
        sql: include_str!("../migrations/0006_ton.sql"),
    },
    Migration {
        to: 7,
        sql: include_str!("../migrations/0007_polygon.sql"),
    },
    Migration {
        to: 8,
        sql: include_str!("../migrations/0008_base.sql"),
    },
];

/// Apply whatever this database is missing.
pub fn run(conn: &Connection) -> Result<i32, StoreError> {
    let mut version = schema_version(conn)?;

    // Version 0 means no schema at all - the file has been keyed but not yet
    // populated. There is nothing to migrate, and the caller is about to
    // create the schema at the current version.
    if version == 0 {
        return Ok(0);
    }

    if version > CURRENT_SCHEMA {
        return Err(StoreError::SchemaTooNew {
            found: version,
            supported: CURRENT_SCHEMA,
        });
    }

    for m in MIGRATIONS {
        if version >= m.to {
            continue;
        }
        // Suspended around the transaction, not inside it: SQLite ignores the
        // pragma while one is open.
        conn.execute_batch("PRAGMA foreign_keys = OFF;")?;
        let applied = conn
            .execute_batch(&format!("BEGIN;\n{}\nCOMMIT;", m.sql))
            .map_err(StoreError::from);
        conn.execute_batch("PRAGMA foreign_keys = ON;")?;
        applied?;

        // Trust the file, not the migration's own claim about what it did.
        version = schema_version(conn)?;
        if version < m.to {
            return Err(StoreError::MigrationFailed(m.to));
        }
    }
    Ok(version)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A database at the current version needs nothing done to it, and doing
    /// nothing must be idempotent - `run` is called on every open.
    #[test]
    fn running_twice_changes_nothing() {
        let conn = Connection::open_in_memory().unwrap();
        crate::vault_row::init_schema(&conn).unwrap();
        let first = run(&conn).unwrap();
        let second = run(&conn).unwrap();
        assert_eq!(first, CURRENT_SCHEMA);
        assert_eq!(second, CURRENT_SCHEMA);
    }

    /// A file written by a newer build must not be opened and guessed at. It
    /// is the user's only copy of their keys.
    #[test]
    fn a_database_from_the_future_is_refused() {
        let conn = Connection::open_in_memory().unwrap();
        crate::vault_row::init_schema(&conn).unwrap();
        conn.execute_batch("PRAGMA user_version = 99;").unwrap();
        assert!(matches!(
            run(&conn),
            Err(StoreError::SchemaTooNew { found: 99, .. })
        ));
    }
}
