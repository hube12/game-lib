//! Enumerates the concrete ways a player can pay a cost right now.
//!
//! A cost can contain choices (an alternative, "any 3 resources", which card to
//! destroy). Every combination the player could really pay with what they hold
//! becomes one [`Payment`]; the index of a payment in the returned list is what
//! actions carry. The list is deterministic for a given state.

use std::collections::{BTreeMap, BTreeSet};

use crate::{
    card::Discount,
    cost::{Cost, Count, Payment, Which},
    id::{EntityId, PlayerId, ResourceId},
    resource::{Pool, Supply},
    state::GameState,
    zone::Owner,
};

/// Upper bound on enumerated payments (a safety net for huge pools).
const MAX_PAYMENTS: usize = 4_000;

/// A payment under construction with what is still available.
#[derive(Clone)]
struct Partial {
    payment: Payment,
    pool:    BTreeMap<ResourceId, u32>,
    card:    BTreeMap<ResourceId, u32>,
}

fn counts(pool: &Pool) -> BTreeMap<ResourceId, u32> {
    pool.iter()
        .filter_map(|(res, supply)| match supply {
            Supply::Finite(0) => None,
            Supply::Finite(n) => Some((res, n)),
            Supply::Infinite => Some((res, u32::MAX / 2)),
        })
        .collect()
}

/// The cards in a player's active zones (their components in play).
pub fn own_cards(state: &GameState, player: PlayerId) -> Vec<EntityId> {
    state
        .zones()
        .filter(|z| z.abilities_active && z.owner.contains(player))
        .flat_map(|z| z.entities().iter().copied())
        .collect()
}

/// Whether the card's definition carries the tag.
pub fn has_tag(state: &GameState, entity: EntityId, tag: &str) -> bool {
    state
        .entity(entity)
        .ok()
        .and_then(|e| state.defs.get(&e.def))
        .is_some_and(|d| d.tags.iter().any(|t| t == tag))
}

/// All multisets of exactly `k` units over kinds with limited supply.
pub(crate) fn multisets(kinds: &[(ResourceId, u32)], k: u32) -> Vec<BTreeMap<ResourceId, u32>> {
    fn go(
        kinds: &[(ResourceId, u32)],
        i: usize,
        k: u32,
        cur: &mut BTreeMap<ResourceId, u32>,
        out: &mut Vec<BTreeMap<ResourceId, u32>>,
    ) {
        if k == 0 {
            out.push(cur.clone());
            return;
        }
        let Some(&(res, cap)) = kinds.get(i) else {
            return;
        };
        for take in 0..=cap.min(k) {
            if take > 0 {
                cur.insert(res, take);
            }
            go(kinds, i + 1, k - take, cur, out);
        }
        cur.remove(&res);
    }
    let mut out = Vec::new();
    go(kinds, 0, k, &mut BTreeMap::new(), &mut out);
    out
}

// One arm per cost kind; splitting it up is on the plan (plan.md, 8.1).
#[allow(clippy::too_many_lines)]
fn expand(
    state: &GameState,
    player: PlayerId,
    source: Option<EntityId>,
    cost: &Cost,
    partials: Vec<Partial>,
) -> Vec<Partial> {
    let mut out = Vec::new();
    match cost {
        Cost::Resources(list) => {
            for mut p in partials {
                if list.iter().all(|(r, n)| p.pool.get(r).copied().unwrap_or(0) >= *n) {
                    for (r, n) in list {
                        *p.pool.entry(*r).or_insert(0) -= n;
                        *p.payment.resources.entry(*r).or_insert(0) += n;
                    }
                    out.push(p);
                }
            }
        },
        Cost::Or(options) => {
            for option in options {
                out.extend(expand(state, player, source, option, partials.clone()));
            }
        },
        Cost::All(parts) => {
            out = partials;
            for part in parts {
                out = expand(state, player, source, part, out);
            }
        },
        Cost::Exhaust => {
            let ready = source.and_then(|e| state.entity(e).ok()).is_some_and(|e| !e.exhausted);
            for mut p in partials {
                if ready && !p.payment.exhaust {
                    p.payment.exhaust = true;
                    out.push(p);
                }
            }
        },
        Cost::Any {
            n,
            exclude,
            identical,
        } => {
            for p in partials {
                let kinds: Vec<_> = p
                    .pool
                    .iter()
                    .filter(|(r, c)| **c > 0 && !exclude.contains(r))
                    .map(|(r, c)| (*r, *c))
                    .collect();
                let chosen: Vec<(BTreeMap<ResourceId, u32>, Option<u32>)> = match (n, identical) {
                    (Count::Fixed(k), false) => {
                        multisets(&kinds, *k).into_iter().map(|m| (m, None)).collect()
                    },
                    (Count::Fixed(k), true) => kinds
                        .iter()
                        .filter(|(_, c)| c >= k)
                        .map(|(r, _)| (BTreeMap::from([(*r, *k)]), None))
                        .collect(),
                    // A gain size, not a price: nothing to enumerate.
                    (Count::CostOf { .. }, _) => Vec::new(),
                    (Count::Var, _) => kinds
                        .iter()
                        .flat_map(|(r, c)| {
                            (1..=*c).map(move |m| (BTreeMap::from([(*r, m)]), Some(m)))
                        })
                        .collect(),
                };
                for (mix, x) in chosen {
                    let mut q = p.clone();
                    for (r, m) in mix {
                        *q.pool.entry(r).or_insert(0) -= m;
                        *q.payment.resources.entry(r).or_insert(0) += m;
                    }
                    if x.is_some() {
                        q.payment.x = x;
                    }
                    out.push(q);
                }
            }
        },
        Cost::TakeFromCard { res, n } => {
            for mut p in partials {
                if p.card.get(res).copied().unwrap_or(0) >= *n {
                    *p.card.entry(*res).or_insert(0) -= n;
                    *p.payment.from_card.entry(*res).or_insert(0) += n;
                    out.push(p);
                }
            }
        },
        Cost::TurnOther { tag } => {
            let candidates: Vec<_> = own_cards(state, player)
                .into_iter()
                .filter(|e| Some(*e) != source && has_tag(state, *e, tag))
                .filter(|e| state.entity(*e).is_ok_and(|c| !c.exhausted))
                .collect();
            for p in partials {
                for c in &candidates {
                    if !p.payment.turned.contains(c) {
                        let mut q = p.clone();
                        q.payment.turned.push(*c);
                        out.push(q);
                    }
                }
            }
        },
        Cost::Destroy { which, tag } => {
            let candidates: Vec<_> = match which {
                Which::This => source.into_iter().collect(),
                Which::Other => own_cards(state, player)
                    .into_iter()
                    .filter(|e| Some(*e) != source && has_tag(state, *e, tag))
                    .collect(),
                Which::Own => own_cards(state, player)
                    .into_iter()
                    .filter(|e| has_tag(state, *e, tag))
                    .collect(),
            };
            for p in partials {
                for c in &candidates {
                    if p.payment.destroyed.is_none() {
                        let mut q = p.clone();
                        q.payment.destroyed = Some(*c);
                        out.push(q);
                    }
                }
            }
        },
        Cost::Discard { from, n } => {
            let hand: Vec<_> = state
                .zone_id(&Owner::Player(player), *from)
                .and_then(|z| state.zone(z))
                .map(|z| z.entities().to_vec())
                .unwrap_or_default();
            for p in partials {
                let free: Vec<_> =
                    hand.iter().copied().filter(|e| !p.payment.discarded.contains(e)).collect();
                for combo in combinations(&free, *n as usize) {
                    let mut q = p.clone();
                    q.payment.discarded.extend(combo);
                    q.payment.discarded.sort();
                    out.push(q);
                }
            }
        },
    }
    out.truncate(MAX_PAYMENTS * 4);
    out
}

/// All ways to pick `k` distinct items.
pub fn combinations<T: Clone>(items: &[T], k: usize) -> Vec<Vec<T>> {
    fn go<T: Clone>(items: &[T], k: usize, start: usize, cur: &mut Vec<T>, out: &mut Vec<Vec<T>>) {
        if cur.len() == k {
            out.push(cur.clone());
            return;
        }
        for i in start..items.len() {
            cur.push(items[i].clone());
            go(items, k, i + 1, cur, out);
            cur.pop();
        }
    }
    let mut out = Vec::new();
    go(items, k, 0, &mut Vec::new(), &mut out);
    out
}

/// Every way `player` can pay `cost` with what they hold. `None` is free (one
/// empty payment). `source` is the card carrying the ability, if any.
pub fn enumerate(
    state: &GameState,
    player: PlayerId,
    source: Option<EntityId>,
    cost: Option<&Cost>,
) -> Vec<Payment> {
    let Some(cost) = cost else {
        return vec![Payment::default()];
    };
    let Ok(pool) = state.player_pool(player) else {
        return Vec::new();
    };
    let start = Partial {
        payment: Payment::default(),
        pool:    counts(pool),
        card:    source
            .and_then(|e| state.entity(e).ok())
            .map(|e| counts(&e.pool))
            .unwrap_or_default(),
    };
    let unique: BTreeSet<Payment> =
        expand(state, player, source, cost, vec![start]).into_iter().map(|p| p.payment).collect();
    unique.into_iter().take(MAX_PAYMENTS).collect()
}

// ------------------------------------------------------------------------------------------------
// Placement prices with discounts
// ------------------------------------------------------------------------------------------------

/// A cost reduced to specific resources plus a number of "any" units.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Price {
    specific: BTreeMap<ResourceId, u32>,
    any:      u32,
    any_excl: Vec<ResourceId>,
}

#[allow(clippy::items_after_statements)]
fn price_of(cost: &Cost) -> Option<Price> {
    let mut price = Price {
        specific: BTreeMap::new(),
        any:      0,
        any_excl: Vec::new(),
    };
    fn add(cost: &Cost, price: &mut Price) -> bool {
        match cost {
            Cost::Resources(list) => {
                for (r, n) in list {
                    *price.specific.entry(*r).or_insert(0) += n;
                }
                true
            },
            Cost::Any {
                n: Count::Fixed(k),
                exclude,
                identical: false,
            } => {
                price.any += k;
                price.any_excl.clone_from(exclude);
                true
            },
            Cost::All(parts) => parts.iter().all(|p| add(p, price)),
            _ => false,
        }
    }
    add(cost, &mut price).then_some(price)
}

/// Applies one discount bucket: reduces by `min(n, reducible)` units, in every
/// possible distribution.
#[allow(clippy::items_after_statements)]
fn reduce(price: &Price, n: u32, exclude: &[ResourceId]) -> Vec<Price> {
    let allowed: Vec<(ResourceId, u32)> = price
        .specific
        .iter()
        .filter(|(r, _)| !exclude.contains(r))
        .map(|(r, c)| (*r, *c))
        .collect();
    let reducible: u32 = allowed.iter().map(|(_, c)| c).sum::<u32>() + price.any;
    let total = n.min(reducible);
    // Bins: each allowed specific kind, then the any-units.
    let mut caps: Vec<u32> = allowed.iter().map(|(_, c)| *c).collect();
    caps.push(price.any);
    let mut out = BTreeSet::new();
    fn go(
        i: usize,
        left: u32,
        caps: &[u32],
        cur: &mut Vec<u32>,
        base: &Price,
        allowed: &[(ResourceId, u32)],
        out: &mut BTreeSet<Price>,
    ) {
        if i == caps.len() {
            if left == 0 {
                let mut p = base.clone();
                for (j, (r, _)) in allowed.iter().enumerate() {
                    let left_over = p.specific[r] - cur[j];
                    if left_over == 0 {
                        p.specific.remove(r);
                    } else {
                        p.specific.insert(*r, left_over);
                    }
                }
                p.any -= cur[allowed.len()];
                out.insert(p);
            }
            return;
        }
        for take in 0..=caps[i].min(left) {
            cur.push(take);
            go(i + 1, left - take, caps, cur, base, allowed, out);
            cur.pop();
        }
    }
    go(0, total, &caps, &mut Vec::new(), price, &allowed, &mut out);
    out.into_iter().collect()
}

/// The payments for placing a card whose cost is `cost`, after the given
/// discount buckets `(units, kinds that cannot be reduced)`.
///
/// Costs with parts that are not plain resources are enumerated without
/// discounts.
pub fn enumerate_discounted(
    state: &GameState,
    player: PlayerId,
    cost: Option<&Cost>,
    discounts: &[(u32, Vec<ResourceId>)],
) -> Vec<Payment> {
    let Some(cost) = cost else {
        return vec![Payment::default()];
    };
    let Some(price) = price_of(cost) else {
        return enumerate(state, player, None, Some(cost));
    };
    let Ok(pool) = state.player_pool(player) else {
        return Vec::new();
    };
    let mut prices: BTreeSet<Price> = BTreeSet::from([price]);
    for (n, exclude) in discounts {
        prices = prices.iter().flat_map(|p| reduce(p, *n, exclude)).collect();
    }
    let available = counts(pool);
    let mut out = BTreeSet::new();
    for price in prices {
        if !price.specific.iter().all(|(r, n)| available.get(r).copied().unwrap_or(0) >= *n) {
            continue;
        }
        let mut rest = available.clone();
        for (r, n) in &price.specific {
            *rest.entry(*r).or_insert(0) -= n;
        }
        let kinds: Vec<_> = rest
            .iter()
            .filter(|(r, c)| **c > 0 && !price.any_excl.contains(r))
            .map(|(r, c)| (*r, *c))
            .collect();
        for mix in multisets(&kinds, price.any) {
            let mut payment = Payment {
                resources: price.specific.clone(),
                ..Payment::default()
            };
            for (r, m) in mix {
                *payment.resources.entry(r).or_insert(0) += m;
            }
            out.insert(payment);
        }
    }
    out.into_iter().take(MAX_PAYMENTS).collect()
}

/// The discount buckets that apply when `player` places `card`: the passive
/// discounts of their cards in play whose tag the card carries, plus an
/// optional extra one (from the power being used to place it).
pub fn discount_buckets(
    state: &GameState,
    player: PlayerId,
    card: EntityId,
    extra: Option<&Discount>,
) -> Vec<(u32, Vec<ResourceId>)> {
    let mut out = Vec::new();
    for owned in own_cards(state, player) {
        let Some(def) = state.entity(owned).ok().and_then(|e| state.defs.get(&e.def)) else {
            continue;
        };
        for d in &def.discounts {
            if d.tag.as_ref().is_none_or(|t| has_tag(state, card, t)) {
                out.push((d.n, d.exclude.clone()));
            }
        }
    }
    if let Some(d) = extra
        && d.tag.as_ref().is_none_or(|t| has_tag(state, card, t))
    {
        out.push((d.n, d.exclude.clone()));
    }
    out
}

/// The ways `player` can pay to place `card` right now: its cost minus every
/// applicable discount. The same state always gives the same list.
pub fn place_payments(
    state: &GameState,
    player: PlayerId,
    card: EntityId,
    extra: Option<&Discount>,
) -> Vec<Payment> {
    let cost =
        state.entity(card).ok().and_then(|e| state.defs.get(&e.def)).and_then(|d| d.cost.as_ref());
    enumerate_discounted(
        state,
        player,
        cost,
        &discount_buckets(state, player, card, extra),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        card::CardDef,
        id::{DefId, ZoneKind},
        rng::Rng,
        zone::{Order, Visibility},
    };

    const E: ResourceId = ResourceId(0);
    const L: ResourceId = ResourceId(1);
    const G: ResourceId = ResourceId(2);
    const P0: PlayerId = PlayerId(0);
    const TABLEAU: ZoneKind = ZoneKind(3);
    const HAND: ZoneKind = ZoneKind(1);

    fn def(id: u32, tags: &[&str]) -> CardDef {
        CardDef {
            id:        DefId(id),
            name:      format!("c{id}"),
            tags:      tags.iter().map(ToString::to_string).collect(),
            cost:      None,
            on_gain:   None,
            abilities: vec![],
            discounts: vec![],
        }
    }

    /// Player 0 holding `pool`, a tableau and a hand.
    fn world(pool: &[(ResourceId, u32)]) -> GameState {
        let mut s = GameState::new(Rng::from_seed(1));
        s.bank.set_infinite(E);
        s.bank.set_infinite(L);
        s.bank.set_infinite(G);
        s.add_player(P0);
        for (r, n) in pool {
            s.gain_player(P0, *r, *n).unwrap();
        }
        let o = Owner::Player(P0);
        let t = s.add_zone(TABLEAU, o.clone(), Order::Unordered, Visibility::Public);
        s.zone_mut(t).unwrap().abilities_active = true;
        s.add_zone(HAND, o, Order::Unordered, Visibility::Owner);
        s.add_def(def(0, &["item"]));
        s.add_def(def(1, &["item", "beast"]));
        s
    }

    fn put(s: &mut GameState, def: u32, kind: ZoneKind) -> EntityId {
        let z = s.zone_id(&Owner::Player(P0), kind).unwrap();
        s.create_entity(DefId(def), z, true).unwrap()
    }

    fn any(n: Count, identical: bool) -> Cost {
        Cost::Any {
            n,
            exclude: vec![G],
            identical,
        }
    }

    #[test]
    fn plain_costs_need_the_resources() {
        let s = world(&[(E, 2)]);
        let cost = Cost::Resources(vec![(E, 2)]);
        assert_eq!(enumerate(&s, P0, None, Some(&cost)).len(), 1);
        let dear = Cost::Resources(vec![(E, 3)]);
        assert_eq!(enumerate(&s, P0, None, Some(&dear)).len(), 0);
        assert_eq!(enumerate(&s, P0, None, None), vec![Payment::default()]);
    }

    #[test]
    fn any_mixes_are_limited_by_the_pool_and_the_exclusions() {
        let s = world(&[(E, 2), (L, 1), (G, 5)]);
        // 2 of {E, L} excluding G: EE, EL.
        let pays = enumerate(&s, P0, None, Some(&any(Count::Fixed(2), false)));
        assert_eq!(pays.len(), 2);
        assert!(pays.iter().all(|p| !p.resources.contains_key(&G) && p.total() == 2));
        // 4 of {E, L} is impossible: only 3 exist outside G.
        assert_eq!(
            enumerate(&s, P0, None, Some(&any(Count::Fixed(4), false))).len(),
            0
        );
    }

    #[test]
    fn identical_payments_pick_one_kind_and_a_variable_count_binds_x() {
        let s = world(&[(E, 3), (L, 1)]);
        let fixed = enumerate(&s, P0, None, Some(&any(Count::Fixed(2), true)));
        assert_eq!(fixed.len(), 1, "only E has two of a kind");
        let var = enumerate(&s, P0, None, Some(&any(Count::Var, true)));
        let xs: Vec<_> = var.iter().map(|p| (p.resources.clone(), p.x)).collect();
        assert_eq!(var.len(), 4, "E x1..3 and L x1");
        assert!(xs.iter().all(|(r, x)| r.values().sum::<u32>() == x.unwrap()));
    }

    #[test]
    fn parts_share_one_pool() {
        // 2 E + any 1 non-G out of {E: 2, L: 0}: only one E is left
        // for the "any" part... none.
        let s = world(&[(E, 2)]);
        let cost = Cost::All(vec![
            Cost::Resources(vec![(E, 2)]),
            any(Count::Fixed(1), false),
        ]);
        assert_eq!(enumerate(&s, P0, None, Some(&cost)).len(), 0);
        let s = world(&[(E, 2), (L, 1)]);
        let pays = enumerate(&s, P0, None, Some(&cost));
        assert_eq!(pays.len(), 1);
        assert_eq!(pays[0].resources[&E], 2);
        assert_eq!(pays[0].resources[&L], 1);
    }

    #[test]
    fn or_alternatives_and_exhaust() {
        let mut s = world(&[(E, 1)]);
        let card = put(&mut s, 0, TABLEAU);
        let cost = Cost::All(vec![
            Cost::Exhaust,
            Cost::Or(vec![
                Cost::Resources(vec![(E, 1)]),
                Cost::Resources(vec![(L, 1)]),
            ]),
        ]);
        let pays = enumerate(&s, P0, Some(card), Some(&cost));
        assert_eq!(pays.len(), 1);
        assert!(pays[0].exhaust);
        s.entity_mut(card).unwrap().exhausted = true;
        assert_eq!(enumerate(&s, P0, Some(card), Some(&cost)).len(), 0);
        assert_eq!(
            enumerate(&s, P0, None, Some(&cost)).len(),
            0,
            "no source, nothing to turn"
        );
    }

    #[test]
    fn taking_resources_off_the_card_needs_them_there() {
        let mut s = world(&[]);
        let card = put(&mut s, 0, TABLEAU);
        let cost = Cost::TakeFromCard { res: E, n: 2 };
        assert_eq!(enumerate(&s, P0, Some(card), Some(&cost)).len(), 0);
        s.gain_entity(card, E, 3).unwrap();
        let pays = enumerate(&s, P0, Some(card), Some(&cost));
        assert_eq!(pays[0].from_card[&E], 2);
    }

    #[test]
    fn turning_another_card_picks_a_ready_card_with_the_tag() {
        let mut s = world(&[]);
        let me = put(&mut s, 0, TABLEAU);
        let beast = put(&mut s, 1, TABLEAU);
        let tired = put(&mut s, 1, TABLEAU);
        s.entity_mut(tired).unwrap().exhausted = true;
        let cost = Cost::TurnOther {
            tag: "beast".into(),
        };
        let pays = enumerate(&s, P0, Some(me), Some(&cost));
        assert_eq!(pays.len(), 1);
        assert_eq!(pays[0].turned, vec![beast]);
    }

    #[test]
    fn destroy_targets_follow_which() {
        let mut s = world(&[]);
        let me = put(&mut s, 0, TABLEAU);
        let other = put(&mut s, 0, TABLEAU);
        let destroy = |which| Cost::Destroy {
            which,
            tag: "item".into(),
        };
        let destroyed = |which| -> Vec<_> {
            enumerate(&s, P0, Some(me), Some(&destroy(which)))
                .into_iter()
                .filter_map(|p| p.destroyed)
                .collect()
        };
        assert_eq!(destroyed(Which::This), vec![me]);
        assert_eq!(destroyed(Which::Other), vec![other]);
        assert_eq!(destroyed(Which::Own), vec![me, other]);
    }

    #[test]
    fn discarding_enumerates_hand_cards() {
        let mut s = world(&[]);
        let a = put(&mut s, 0, HAND);
        let b = put(&mut s, 0, HAND);
        let cost = Cost::Discard {
            from: HAND,
            n:    1,
        };
        let pays = enumerate(&s, P0, None, Some(&cost));
        assert_eq!(
            pays.iter().map(|p| p.discarded.clone()).collect::<Vec<_>>(),
            vec![vec![a], vec![b]]
        );
        assert_eq!(
            enumerate(
                &s,
                P0,
                None,
                Some(&Cost::Discard {
                    from: HAND,
                    n:    3,
                })
            )
            .len(),
            0
        );
    }

    #[test]
    fn discounts_reduce_the_cost_in_every_distribution() {
        // Cost 1 E + 1 L; discount 1 of anything: pay either one.
        let s = world(&[(E, 1), (L, 1)]);
        let cost = Cost::Resources(vec![(E, 1), (L, 1)]);
        let pays = enumerate_discounted(&s, P0, Some(&cost), &[(1, vec![])]);
        assert_eq!(pays.len(), 2);
        assert!(pays.iter().all(|p| p.total() == 1));
        // A discount that cannot touch G leaves a G cost alone.
        let gold = Cost::Resources(vec![(G, 2)]);
        let s = world(&[(G, 2)]);
        let pays = enumerate_discounted(&s, P0, Some(&gold), &[(2, vec![G])]);
        assert_eq!(pays.len(), 1);
        assert_eq!(pays[0].resources[&G], 2);
    }

    #[test]
    fn discounts_never_go_below_zero_and_stack() {
        let s = world(&[]);
        let cost = Cost::Resources(vec![(E, 1)]);
        let free = enumerate_discounted(&s, P0, Some(&cost), &[(1000, vec![])]);
        assert_eq!(free, vec![Payment::default()]);
        let s = world(&[(E, 1)]);
        let cost = Cost::Resources(vec![(E, 3)]);
        let pays = enumerate_discounted(&s, P0, Some(&cost), &[(1, vec![]), (1, vec![])]);
        assert_eq!(pays[0].resources[&E], 1, "3 - 1 - 1");
    }

    #[test]
    fn discounts_apply_to_any_units_and_the_remainder_is_chosen_from_the_pool() {
        // Cost: 1 G + any 2 (non-G); discount 1: pay G + any 1.
        let s = world(&[(G, 1), (E, 1), (L, 1)]);
        let cost = Cost::All(vec![
            Cost::Resources(vec![(G, 1)]),
            any(Count::Fixed(2), false),
        ]);
        let pays = enumerate_discounted(&s, P0, Some(&cost), &[(1, vec![G])]);
        // Discount can only lower the any-part (G is excluded): G + one
        // of E / L.
        assert_eq!(pays.len(), 2);
        assert!(pays.iter().all(|p| p.resources[&G] == 1 && p.total() == 2));
    }

    #[test]
    fn combinations_choose_distinct_items() {
        assert_eq!(
            combinations(&[1, 2, 3], 2),
            vec![vec![1, 2], vec![1, 3], vec![2, 3]]
        );
        assert_eq!(combinations(&[1, 2], 0), vec![Vec::<i32>::new()]);
        assert_eq!(combinations(&[1], 2).len(), 0);
    }
}
