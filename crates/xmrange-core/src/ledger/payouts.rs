//! p2pool payouts (schema v4): what p2pool reported paying this wallet while XMRange ran.
use super::{sql, Ledger, LedgerError};
use serde::Serialize;

/// All-time payouts to one wallet: how many, and their sum in atomic units.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct PayoutTotals {
    pub count: u64,
    pub atomic: u64,
}

impl Ledger {
    /// Records one payout. Ok(false) when this block's payout to this wallet is already there.
    pub fn insert_payout(&mut self, t: f64, payout: crate::p2pool::Payout, wallet: &str) -> Result<bool, LedgerError> {
        let tag = self.wallet_tag(wallet);
        let n = self
            .conn
            .execute(
                "INSERT OR IGNORE INTO payouts (t, height, atomic, wallet_tag) VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![t, payout.height as i64, payout.atomic as i64, tag],
            )
            .map_err(sql)?;
        Ok(n == 1)
    }

    pub fn payout_totals(&self, wallet: &str) -> Result<PayoutTotals, LedgerError> {
        let tag = self.wallet_tag(wallet);
        self.conn
            .query_row("SELECT count(*), coalesce(sum(atomic), 0) FROM payouts WHERE wallet_tag = ?1", [tag], |r| {
                Ok(PayoutTotals { count: r.get::<_, i64>(0)? as u64, atomic: r.get::<_, i64>(1)? as u64 })
            })
            .map_err(sql)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::p2pool::Payout;

    #[test]
    fn payouts_add_up_per_wallet_and_a_repeat_counts_once() {
        let d = crate::paths::scratch_dir("ledger-payouts").unwrap();
        let mut l = Ledger::open(&d.join("ledger.db"), &super::super::LedgerKey::generate()).unwrap();
        let p = Payout { atomic: 412_345_678, height: 3_775_700 };
        assert!(l.insert_payout(1.0, p, "4wallet").unwrap());
        assert!(!l.insert_payout(2.0, p, "4wallet").unwrap(), "the same block again is ignored");
        assert!(l.insert_payout(3.0, Payout { atomic: 1_000, height: 3_775_701 }, "4wallet").unwrap());
        assert!(l.insert_payout(4.0, p, "4other").unwrap());
        assert_eq!(l.payout_totals("4wallet").unwrap(), PayoutTotals { count: 2, atomic: 412_346_678 });
        assert_eq!(l.payout_totals("4none").unwrap(), PayoutTotals::default());
        let raw: String = l.conn.query_row("SELECT wallet_tag FROM payouts LIMIT 1", [], |r| r.get(0)).unwrap();
        assert!(!raw.contains("wallet"), "the wallet is stored only as its salted tag");
    }
}
