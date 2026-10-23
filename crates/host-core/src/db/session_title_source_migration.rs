use super::{create_migration_backup, params, Connection, Context, Path, Result};
use crate::sessions::{is_default_title, TITLE_SOURCE_DEFAULT, TITLE_SOURCE_MANUAL};
use rusqlite::Transaction;

fn migrate_v22_to_v23_tx(tx: &Transaction<'_>) -> Result<()> {
    let has_title_source: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM pragma_table_info('sessions') WHERE name = 'title_source')",
        [],
        |row| row.get(0),
    )?;
    if !has_title_source {
        tx.execute_batch(
            "ALTER TABLE sessions ADD COLUMN title_source TEXT NOT NULL DEFAULT 'legacy'
             CHECK (title_source IN ('legacy', 'default', 'manual', 'generated'));",
        )?;
        // Older releases stored default and manually-entered titles in the
        // same column. Preserve the distinction where it is observable: only
        // known placeholders remain eligible for automatic replacement; every
        // other pre-existing title is treated as manual. Classification goes
        // through `sessions::is_default_title` rather than a SQL literal list,
        // because the renderer persists the lowercased `chat.untitledTask` /
        // `nav.newChat` label of the active locale: a case-sensitive
        // `title IN (...)` comparison recorded every Latin-script locale as
        // manual.
        classify_session_titles(tx, "SELECT id, title FROM sessions")?;
    }
    tx.pragma_update(None, "user_version", 23i64)?;
    Ok(())
}

/// Schema v23 classified pre-existing titles with a case-sensitive placeholder
/// list, so sessions whose stored title was the lowercased localized
/// placeholder — every non-CJK locale — were recorded as `manual` and silently
/// lost their eligibility for an automatic title. Re-classify exactly those
/// rows with the shared matcher; stored titles are never touched.
fn migrate_v24_to_v25_tx(tx: &Transaction<'_>) -> Result<()> {
    classify_session_titles(
        tx,
        "SELECT id, title FROM sessions WHERE title_source = 'manual'",
    )?;
    tx.pragma_update(None, "user_version", 25i64)?;
    Ok(())
}

/// Rewrite `title_source` for the rows the query returns through the same
/// matcher session creation and `derive_session_title` use, so no migration can
/// drift from `sessions::PLACEHOLDER_TITLES`.
fn classify_session_titles(tx: &Transaction<'_>, query: &str) -> Result<()> {
    let rows: Vec<(String, String)> = {
        let mut stmt = tx.prepare(query)?;
        let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };
    let mut update = tx.prepare("UPDATE sessions SET title_source = ?1 WHERE id = ?2")?;
    for (id, title) in rows {
        let source = if is_default_title(&title) {
            TITLE_SOURCE_DEFAULT
        } else {
            TITLE_SOURCE_MANUAL
        };
        update.execute(params![source, id])?;
    }
    Ok(())
}

pub(crate) fn migrate_v22_to_v23(conn: &Connection, path: &Path) -> Result<()> {
    let backup = create_migration_backup(conn, path, 22)?;
    let tx = conn.unchecked_transaction()?;
    migrate_v22_to_v23_tx(&tx)?;
    tx.commit().with_context(|| {
        format!(
            "commit schema v22 to v23 migration; backup {} remains",
            backup.display()
        )
    })?;
    Ok(())
}

pub(crate) fn migrate_v24_to_v25(conn: &Connection, path: &Path) -> Result<()> {
    let backup = create_migration_backup(conn, path, 24)?;
    let tx = conn.unchecked_transaction()?;
    migrate_v24_to_v25_tx(&tx)?;
    tx.commit().with_context(|| {
        format!(
            "commit schema v24 to v25 migration; backup {} remains",
            backup.display()
        )
    })?;
    Ok(())
}
