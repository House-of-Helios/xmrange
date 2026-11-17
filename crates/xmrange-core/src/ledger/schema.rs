//! Ledger schema, versioned with `PRAGMA user_version`. MIGRATIONS[i] takes the Ledger from
//! version i to i+1, each in its own transaction. Append only: never edit a shipped entry.
use super::{sql, LedgerError};
use rusqlite::Connection;

pub const SCHEMA_VERSION: u32 = 4;

pub(crate) const MIGRATIONS: &[&str] = &[
    // v1 (spec §4, minus its settings table: P3-R4): samples and trials. °C throughout; secrets
    // never; the wallet only as a salted tag; no IPs.
    r#"
    CREATE TABLE samples (
        t        REAL    NOT NULL,  -- unix seconds; a 1-minute row is stamped at its minute's start
        span_s   INTEGER NOT NULL,  -- 1 (live), 60 (after 7 days) or 3600 (after 90 days, kept)
        n        INTEGER NOT NULL,  -- 1 s samples this row stands for
        state    TEXT    NOT NULL,  -- starting | mining | paused
        threads  INTEGER NOT NULL,
        hs10     REAL,
        sys_w    REAL,
        cpu_w    REAL,
        ram_w    REAL,
        temp_c   REAL,
        p_mhz    INTEGER,
        e_mhz    INTEGER,
        p_active REAL,
        e_active REAL
    );
    CREATE INDEX samples_by_span_t ON samples (span_s, t);
    CREATE TABLE trials (
        id            INTEGER PRIMARY KEY,
        t_start       REAL    NOT NULL,  -- first measured tick (settle_s precedes it)
        t_end         REAL    NOT NULL,
        label         TEXT    NOT NULL,
        source        TEXT    NOT NULL,  -- cli | seeker
        chip          TEXT,
        config        TEXT    NOT NULL,  -- JSON ledger::TrialConfig
        settled       INTEGER NOT NULL,
        settle_s      INTEGER NOT NULL,
        partial       INTEGER NOT NULL,
        -- trial::Score, column for column; all NULL when the trial stopped before scoring
        n             INTEGER,
        hs            REAL,
        cpu_w         REAL,
        ram_w         REAL,
        sys_w         REAL,
        temp_c        REAL,
        hw_sys        REAL,
        hw_cpu        REAL,
        hw_sys_ci95   REAL,
        other_cpu_pct REAL,
        noisy         INTEGER,
        context_start TEXT    NOT NULL,  -- JSON context::Context
        context_end   TEXT    NOT NULL,
        wallet_tag    TEXT               -- salted hash, never the address (doctrine §3)
    );
    CREATE INDEX trials_by_t_end ON trials (t_end);
    "#,
    // v2 (Plan 4): the Seeker ranks by H/s and CPU H/W as well as whole-Mac H/W, so their CIs
    // are kept; and the worst thermal pressure a trial saw (roadmap carried item).
    r#"
    ALTER TABLE trials ADD COLUMN hs_ci95     REAL;
    ALTER TABLE trials ADD COLUMN hw_cpu_ci95 REAL;
    ALTER TABLE trials ADD COLUMN thermal_max INTEGER;
    "#,
    // v3 (ruling P5-O1, owner, 2026-09-28): the 1 Hz samples go. Nothing charts power history, and
    // the Ledger keeps what tuning needs long term: trials (the curve and profiles come from them).
    // Its index goes with it; `Ledger::open` then hands the freed pages back to the file system.
    r#"
    DROP TABLE IF EXISTS samples;
    "#,
    // v4 (owner, 2026-10-02): p2pool payouts, as p2pool reports them while XMRange runs. Amount
    // and Monero block height only (both public on the share chain); the wallet as its salted tag.
    // One row per block and wallet, so a repeated line is never counted twice.
    r#"
    CREATE TABLE payouts (
        id         INTEGER PRIMARY KEY,
        t          REAL    NOT NULL,  -- unix seconds, when XMRange saw it
        height     INTEGER NOT NULL,  -- Monero block height
        atomic     INTEGER NOT NULL,  -- piconero (1e-12 XMR)
        wallet_tag TEXT    NOT NULL,
        UNIQUE (height, wallet_tag)
    );
    "#,
];

pub(crate) fn user_version(conn: &Connection) -> Result<u32, LedgerError> {
    conn.pragma_query_value(None, "user_version", |r| r.get(0)).map_err(sql)
}

/// Runs every migration the Ledger hasn't had, up to version `target`. Returns true when any ran.
/// Callers check TooNew first (see `Ledger::open`).
pub(crate) fn migrate_to(conn: &mut Connection, target: u32) -> Result<bool, LedgerError> {
    let found = user_version(conn)?;
    let steps = MIGRATIONS.iter().enumerate().take(target as usize).skip(found as usize);
    let mut ran = false;
    for (i, step) in steps {
        let tx = conn.transaction().map_err(sql)?;
        tx.execute_batch(step).map_err(sql)?;
        tx.pragma_update(None, "user_version", (i + 1) as u32).map_err(sql)?;
        tx.commit().map_err(sql)?;
        ran = true;
    }
    Ok(ran)
}
