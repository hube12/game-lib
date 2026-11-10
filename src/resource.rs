//! Resource pools. A [`Bank`] is just a pool whose entries may be infinite;
//! players and cards hold pools too.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::id::ResourceId;

/// How much of one resource a pool holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Supply {
    /// A limited amount.
    Finite(u32),
    /// Never runs out and never overflows.
    Infinite,
}

/// Amounts per resource type. A missing entry means `Finite(0)`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pool {
    amounts: BTreeMap<ResourceId, Supply>,
}

/// The shared supply is a pool, typically with finite or infinite entries.
pub type Bank = Pool;

/// Why a resource operation was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResourceError {
    /// The source does not hold enough.
    Insufficient {
        /// The resource.
        res:       ResourceId,
        /// Amount requested.
        needed:    u32,
        /// Amount actually held.
        available: u32,
    },
    /// The destination would exceed `u32::MAX`.
    Overflow(ResourceId),
}

impl Pool {
    /// An empty pool.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets a resource to a finite amount.
    pub fn set(&mut self, res: ResourceId, n: u32) {
        self.amounts.insert(res, Supply::Finite(n));
    }

    /// Makes a resource infinite.
    pub fn set_infinite(&mut self, res: ResourceId) {
        self.amounts.insert(res, Supply::Infinite);
    }

    /// Current supply of a resource.
    pub fn get(&self, res: ResourceId) -> Supply {
        self.amounts.get(&res).copied().unwrap_or(Supply::Finite(0))
    }

    /// Finite amount held, or `None` for an infinite supply.
    pub fn finite(&self, res: ResourceId) -> Option<u32> {
        match self.get(res) {
            Supply::Finite(n) => Some(n),
            Supply::Infinite => None,
        }
    }

    /// Whether at least `n` can be taken.
    pub fn has(&self, res: ResourceId, n: u32) -> bool {
        self.finite(res).is_none_or(|have| have >= n)
    }

    /// Iterates over the entries that exist.
    pub fn iter(&self) -> impl Iterator<Item = (ResourceId, Supply)> + '_ {
        self.amounts.iter().map(|(r, s)| (*r, *s))
    }
}

/// Moves `n` of `res` from one pool to another, enforcing limits on both sides.
/// Nothing changes on error.
pub fn transfer(
    from: &mut Pool,
    to: &mut Pool,
    res: ResourceId,
    n: u32,
) -> Result<(), ResourceError> {
    if let Some(have) = from.finite(res)
        && have < n
    {
        return Err(ResourceError::Insufficient {
            res,
            needed: n,
            available: have,
        });
    }
    let new_to = match to.get(res) {
        Supply::Infinite => Supply::Infinite,
        Supply::Finite(b) => Supply::Finite(b.checked_add(n).ok_or(ResourceError::Overflow(res))?),
    };
    if let Supply::Finite(have) = from.get(res) {
        from.amounts.insert(res, Supply::Finite(have - n));
    }
    to.amounts.insert(res, new_to);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOLD: ResourceId = ResourceId(0);

    fn pool(n: u32) -> Pool {
        let mut p = Pool::new();
        p.set(GOLD, n);
        p
    }

    #[test]
    fn transfer_moves_finite_amounts() {
        let (mut a, mut b) = (pool(5), pool(1));
        transfer(&mut a, &mut b, GOLD, 3).unwrap();
        assert_eq!((a.finite(GOLD), b.finite(GOLD)), (Some(2), Some(4)));
    }

    #[test]
    fn transfer_refuses_overdraw_and_changes_nothing() {
        let (mut a, mut b) = (pool(2), pool(0));
        let err = transfer(&mut a, &mut b, GOLD, 3).unwrap_err();
        assert_eq!(
            err,
            ResourceError::Insufficient {
                res:       GOLD,
                needed:    3,
                available: 2,
            }
        );
        assert_eq!((a.finite(GOLD), b.finite(GOLD)), (Some(2), Some(0)));
    }

    #[test]
    fn infinite_bank_never_runs_out() {
        let mut bank = Bank::new();
        bank.set_infinite(GOLD);
        let mut p = pool(0);
        transfer(&mut bank, &mut p, GOLD, 1_000).unwrap();
        assert_eq!(bank.get(GOLD), Supply::Infinite);
        assert_eq!(p.finite(GOLD), Some(1_000));
        transfer(&mut p, &mut bank, GOLD, 400).unwrap();
        assert_eq!(bank.get(GOLD), Supply::Infinite);
        assert_eq!(p.finite(GOLD), Some(600));
    }

    #[test]
    fn overflow_is_refused_without_side_effects() {
        let (mut a, mut b) = (pool(5), pool(u32::MAX));
        assert_eq!(
            transfer(&mut a, &mut b, GOLD, 1),
            Err(ResourceError::Overflow(GOLD))
        );
        assert_eq!(a.finite(GOLD), Some(5));
    }

    #[test]
    fn missing_entry_is_zero() {
        let p = Pool::new();
        assert!(!p.has(GOLD, 1));
        assert!(p.has(GOLD, 0));
    }
}
