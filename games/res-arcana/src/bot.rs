//! A simple Res Arcana player: place valuable artifacts, claim places and
//! monuments, use powers, and pass when there is nothing better to do.
//!
//! It only sees what a player may see ([`PlayerView`]) plus the printed card
//! data, so it plays by the same information as a human.

use gamelib::{
    bot::Bot,
    effect::Effect,
    engine::Action,
    id::{DefId, EntityId},
    rng::Rng,
    view::PlayerView,
    zone::Owner,
};

use crate::{
    CardKind,
    compile::{GOLD, HAND},
    rules::ResArcanaRules,
    spec::Count,
};

/// Most actions the bot takes in one round before it passes. A power without
/// the turning cost could otherwise be used for ever.
const ACTIONS_PER_ROUND: u32 = 7;

/// The heuristic Res Arcana bot.
#[derive(Debug)]
pub struct HeuristicBot {
    rules:   ResArcanaRules,
    rng:     Rng,
    round:   u32,
    actions: u32,
}

impl HeuristicBot {
    /// A bot that reads card values from the base game's data.
    pub fn new() -> Self {
        Self::with_seed(0)
    }

    /// The same bot with its own deterministic stream for breaking ties.
    pub fn with_seed(seed: u64) -> Self {
        Self {
            rules:   ResArcanaRules::new(),
            rng:     Rng::from_seed(seed),
            round:   0,
            actions: 0,
        }
    }

    /// The definition of a card the viewer can see.
    fn def_of(view: &PlayerView, entity: EntityId) -> Option<DefId> {
        view.zones
            .iter()
            .flat_map(|z| z.entities.iter())
            .find(|e| e.id == entity)
            .and_then(|e| e.def)
    }

    /// How much the bot wants to put this card into play: victory points count
    /// most, then how many powers and collect abilities it brings.
    fn card_value(&self, def: Option<DefId>) -> i64 {
        let Some(spec) = def.and_then(|d| self.rules.spec(d)) else {
            return 0;
        };
        let cost = spec.cost.as_ref().map_or(0, |c| {
            c.iter()
                .map(|a| match a.count {
                    Count::Fixed(n) => i64::from(n),
                    Count::Var => 1,
                })
                .sum::<i64>()
        });
        let vp = i64::from(spec.vp.base) + i64::from(spec.vp.per_essence.is_some());
        let kind_bonus = match spec.kind {
            CardKind::Place => 6,
            CardKind::Monument => 4,
            _ => 0,
        };
        vp * 10 + kind_bonus + i64::try_from(spec.abilities.len()).unwrap_or(0) * 2 - cost / 2
    }

    /// The option of a pending decision to take.
    fn decide(options: Option<&[Effect]>, legal: &[Action]) -> usize {
        // The collect step ("do nothing" first): take what is free, i.e. the
        // essences on the components, before ending it.
        if let Some(all) = options
            && all.first() == Some(&Effect::Ignore)
        {
            let free = |action: &Action| {
                let Action::Decide(i) = action else {
                    return false;
                };
                let first = match all.get(*i) {
                    Some(Effect::Sequence(steps)) => steps.first(),
                    other => other,
                };
                matches!(first, Some(Effect::Use { payment, .. })
                    if payment.total() == 0 && !payment.exhaust)
            };
            if let Some(at) = legal.iter().position(free) {
                return at;
            }
        }
        // When the default is losing something, use a reaction or an escape if
        // there is one.
        if let Some([first, ..]) = options
            && matches!(first, Effect::Lose { .. })
            && legal.len() > 1
        {
            return 1;
        }
        0
    }
}

impl Default for HeuristicBot {
    fn default() -> Self {
        Self::new()
    }
}

impl Bot for HeuristicBot {
    fn choose(&mut self, view: &PlayerView, legal: &[Action]) -> usize {
        if view.round != self.round {
            self.round = view.round;
            self.actions = 0;
        }
        if matches!(legal.first(), Some(Action::Decide(_))) {
            let options = view.pending.as_ref().and_then(|p| p.options.as_deref());
            return Self::decide(options, legal);
        }
        let pass = legal.iter().position(|a| *a == Action::Pass).unwrap_or(0);
        if self.actions >= ACTIONS_PER_ROUND {
            return pass;
        }

        let gold = view.pools.get(&view.viewer).and_then(|p| p.finite(GOLD)).unwrap_or(0);
        let hand = view
            .zones
            .iter()
            .find(|z| z.owner == Owner::Player(view.viewer) && z.kind == HAND)
            .map_or(0, |z| z.count);

        // Monuments and places are the main source of points and cost 4 Gold:
        // claim whenever possible, collect Gold by discarding while short of
        // it, place valuable artifacts, and use powers last.
        let mut best: Vec<(i64, usize)> = Vec::new();
        for (i, action) in legal.iter().enumerate() {
            let score = match action {
                Action::Buy { entity, .. } => 120 + self.card_value(Self::def_of(view, *entity)),
                Action::Place { entity, pay } => {
                    let value = self.card_value(Self::def_of(view, *entity));
                    if value > 0 {
                        90 + value - i64::try_from(*pay).unwrap_or(0)
                    } else {
                        10
                    }
                },
                Action::Discard { .. } if gold < 4 && hand >= 2 => 70,
                Action::Activate { .. } => 20,
                Action::Discard { .. } if hand > 4 => 5,
                _ => continue,
            };
            match best.first() {
                Some((top, _)) if score < *top => {},
                Some((top, _)) if score == *top => best.push((score, i)),
                _ => best = vec![(score, i)],
            }
        }
        if best.is_empty() {
            return pass;
        }
        // Equally good actions (most powers) are tried in turn, not always the
        // first.
        self.actions += 1;
        best[self.rng.below(best.len())].1
    }
}
