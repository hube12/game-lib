//! A Res Arcana player that tries every legal move on a copy of the game and
//! keeps the one that leaves it in the best position.
//!
//! The position is scored by [`Eval`]: victory points (worth more as the end
//! comes closer), essences, what the components in play will still produce,
//! and how close the player is to the cards worth getting (artifacts in hand,
//! monuments, places of power). A rival's position counts against the bot, so
//! a Dragon's power is valued by what it really costs the rivals in the copy,
//! and the bot attacks while they can still be hurt.
//!
//! Hidden information: the bot receives the whole state because it has to run
//! the engine on a copy, but it shuffles its own deck and the monument deck
//! in that copy first and never looks at a rival's hand or deck, so it plays
//! with what its player could know.

// The evaluation is a page of small weighted sums over counts. They are
// written as plain arithmetic so the numbers can be read and tuned; the
// counts are tiny, so the casts lose nothing.
#![allow(
    clippy::suboptimal_flops,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]

use std::fmt::Write as _;

use gamelib::{
    bot::Bot,
    card::CardDef,
    cost::{Cost, Count},
    effect::{Ability, Condition, Effect, Target, Trigger},
    engine::{Action, Engine},
    id::{DefId, EntityId, PlayerId, ResourceId},
    state::GameState,
    view::PlayerView,
    zone::Owner,
};

use crate::{
    CardKind,
    compile::{
        COLLECT_STEP,
        DECK,
        GOLD,
        HAND,
        LIFE,
        LIFE_LOSS,
        MONUMENT_DECK,
        TABLEAU,
        VICTORY_CHECK,
        resource,
        tag,
    },
    rules::{CANDIDATES, FIRST_TOKEN, MONUMENT_ROW, PLACES, ResArcanaRules, WIN_SCORE},
};

/// The numbers the evaluation is built on. Values are in "essence units": one
/// ordinary essence in the pool is about one unit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Params {
    /// Worth of an ordinary essence in the pool.
    pub essence:      f64,
    /// Worth of a Gold in the pool.
    pub gold:         f64,
    /// Essences lying on a component, relative to the pool.
    pub on_card:      f64,
    /// A card in hand that is good for nothing but discarding.
    pub hand_card:    f64,
    /// Victory points a leader is expected to make per round (sets how many
    /// rounds are thought to be left).
    pub pace:         f64,
    /// Most rounds the bot plans ahead.
    pub horizon:      f64,
    /// Worth of a victory point far from the end.
    pub vp_base:      f64,
    /// Added to a victory point's worth, divided by the rounds left.
    pub vp_late:      f64,
    /// Share of a wanted card's profit already counted when it is affordable.
    pub goal_ready:   f64,
    /// That share shrinks by this factor per round still needed to afford it.
    pub goal_decay:   f64,
    /// Share of a power's usual gain counted while its card is still ready.
    pub ready:        f64,
    /// Units a rival is expected to lose per Life an attack takes.
    pub attack:       f64,
    /// Extra for a move that hurts the rivals, so it is made before they pass.
    pub attack_first: f64,
    /// A move must improve the position by this much, otherwise the bot passes.
    pub min_gain:     f64,
    /// How much a rival's position counts against the bot's own.
    pub rival:        f64,
    /// Extra worth of a Dragon, in hand (so it is not thrown away for Gold)
    /// and as a card to get.
    pub dragon:       f64,
    /// Share of the points a player can still buy this round (affordable
    /// places and monuments) that counts towards how close the game is to its
    /// end. 0 gives the old pace estimate.
    pub reach:        f64,
    /// Units a move may lose and still be played instead of passing, when a
    /// rival who has not passed can finish the game this round.
    pub tempo:        f64,
    /// Victory points counted against a position in which a rival with a
    /// ready victory check can end the game ahead of the bot.
    pub check_threat: f64,
}

impl Default for Params {
    fn default() -> Self {
        Self {
            essence:      0.8,
            gold:         1.6,
            on_card:      0.9,
            hand_card:    1.2,
            pace:         2.2,
            horizon:      4.0,
            vp_base:      5.0,
            vp_late:      6.0,
            goal_ready:   0.85,
            goal_decay:   0.7,
            ready:        0.5,
            attack:       1.0,
            attack_first: 0.4,
            min_gain:     0.05,
            rival:        1.0,
            dragon:       1.0,
            reach:        0.7,
            tempo:        1.0,
            check_threat: 3.0,
        }
    }
}

impl Params {
    /// Sets one number by its field name (for tuning from a command line).
    /// Returns whether the name exists.
    pub fn set(&mut self, name: &str, value: f64) -> bool {
        let field = match name {
            "essence" => &mut self.essence,
            "gold" => &mut self.gold,
            "on_card" => &mut self.on_card,
            "hand_card" => &mut self.hand_card,
            "pace" => &mut self.pace,
            "horizon" => &mut self.horizon,
            "vp_base" => &mut self.vp_base,
            "vp_late" => &mut self.vp_late,
            "goal_ready" => &mut self.goal_ready,
            "goal_decay" => &mut self.goal_decay,
            "ready" => &mut self.ready,
            "attack" => &mut self.attack,
            "attack_first" => &mut self.attack_first,
            "min_gain" => &mut self.min_gain,
            "rival" => &mut self.rival,
            "dragon" => &mut self.dragon,
            "reach" => &mut self.reach,
            "tempo" => &mut self.tempo,
            "check_threat" => &mut self.check_threat,
            _ => return false,
        };
        *field = value;
        true
    }
}

/// Simulated moves the bot allows itself per choice.
const BUDGET: usize = 4_000;
/// How many of its own decisions in a row the bot tries out in full after a
/// move; further ones are answered by what looks best at once.
const FULL_DEPTH: usize = 2;
/// Worth of winning; nothing else comes close.
const WIN: f64 = 10_000.0;
/// Most actions per round, a guard against a loop of powers.
const ACTIONS_PER_ROUND: u32 = 60;

fn index(res: ResourceId) -> Option<usize> {
    Some(usize::from(res.0)).filter(|i| *i < 5)
}

/// What a power, a collect ability or a claim gives, in rough numbers.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Gain {
    /// Essence units for the owner.
    units:  f64,
    /// Victory points (essences put on a card that counts them).
    vp:     f64,
    /// Life every rival loses.
    attack: f64,
    /// Essence units every rival receives.
    gift:   f64,
}

impl Gain {
    fn add(&mut self, other: Self) {
        self.units += other.units;
        self.vp += other.vp;
        self.attack += other.attack;
        self.gift += other.gift;
    }
}

/// A power of a card.
#[derive(Clone, Debug, PartialEq)]
struct Power {
    gain:     Gain,
    /// Essence units paid.
    cost:     f64,
    /// Turns its own card.
    turn:     bool,
    /// Needs another ready card of the owner with this tag.
    needs:    Option<String>,
    /// Destroys a card or discards from hand: never counted as income.
    one_shot: bool,
    /// Holds a victory check: using it can end the game at once.
    ends:     bool,
}

/// Whether an effect checks for a winner.
fn checks_win(effect: &Effect) -> bool {
    match effect {
        Effect::CheckWin => true,
        Effect::Sequence(parts) => parts.iter().any(checks_win),
        _ => false,
    }
}

/// A placement cost.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Price {
    specific:    [u32; 5],
    any:         u32,
    any_no_gold: bool,
}

impl Price {
    fn of(cost: &Cost) -> Self {
        let mut price = Self::default();
        price.read(cost);
        price
    }

    fn read(&mut self, cost: &Cost) {
        match cost {
            Cost::Resources(list) => {
                for (res, n) in list {
                    if let Some(i) = index(*res) {
                        self.specific[i] += n;
                    }
                }
            },
            Cost::Any {
                n: Count::Fixed(n),
                exclude,
                ..
            } => {
                self.any += n;
                self.any_no_gold = exclude.contains(&GOLD);
            },
            Cost::All(parts) => parts.iter().for_each(|p| self.read(p)),
            _ => {},
        }
    }

    /// Number of essences in the cost.
    fn count(&self) -> u32 {
        self.specific.iter().sum::<u32>() + self.any
    }
}

/// What the bot knows about a card definition.
#[derive(Clone, Debug, PartialEq)]
struct Model {
    name:        String,
    kind:        CardKind,
    tags:        Vec<String>,
    vp:          f64,
    /// `(points, essence)` counted per essence on the card.
    per_essence: Option<(f64, ResourceId)>,
    price:       Option<Price>,
    /// Essence units collected every round.
    collect:     f64,
    /// Collected only while this essence lies on the card: `(essence, units)`.
    collect_if:  Option<(ResourceId, f64)>,
    /// Grows by this many per kind of essence lying on it.
    grow:        f64,
    powers:      Vec<Power>,
    /// Has a reaction against Life loss.
    defense:     bool,
    /// Has a reaction that gives points at a victory check.
    check_react: bool,
    /// Units gained once when it is claimed.
    on_gain:     f64,
    /// Placement discounts it grants: `(tag, units)`.
    discounts:   Vec<(Option<String>, u32, bool)>,
}

impl Model {
    fn has(&self, wanted: &str) -> bool {
        self.tags.iter().any(|t| t == wanted)
    }

    fn attacks(&self) -> bool {
        self.powers.iter().any(|p| p.gain.attack > 0.0)
    }
}

fn unit(res: ResourceId) -> f64 {
    if res == GOLD { 2.0 } else { 1.0 }
}

/// Reads an effect into rough numbers. `spec_vp` is the card's per-essence
/// scoring, which turns essences put on the card into points.
fn gain_of(effect: &Effect, per_essence: Option<(f64, ResourceId)>) -> Gain {
    let mut g = Gain::default();
    match effect {
        Effect::Gain { to, res, n } => {
            let n = f64::from(*n);
            match per_essence {
                Some((each, counted)) if *to == Target::Source && counted == *res => {
                    g.vp += each * n;
                },
                _ => g.units += n * unit(*res),
            }
        },
        Effect::GainX { res, .. } => g.units += 3.0 * unit(*res),
        Effect::GainChoose { n, exclude, .. } => {
            let each = if exclude.contains(&GOLD) { 1.0 } else { 2.0 };
            g.units += each
                * match n {
                    Count::Fixed(n) => f64::from(*n),
                    Count::Var => 3.0,
                    Count::CostOf { bonus, .. } => 3.0 + f64::from(*bonus),
                };
        },
        // One essence moved onto a card, or one card put into play.
        Effect::GainPaid { .. } | Effect::PlaceCard { .. } => g.units += 1.0,
        Effect::GainAll { res, n, .. } => g.gift += f64::from(*n) * unit(*res),
        Effect::GainPerRival { .. } => g.units += 2.0,
        Effect::Pay { res, n } => g.units -= f64::from(*n) * unit(*res),
        Effect::Draw { n, .. } => g.units += 1.3 * f64::from(*n),
        Effect::DiscardChoose { n } => g.units -= 1.3 * f64::from(*n),
        Effect::DrawDiscard { .. } => {},
        Effect::Affect {
            effect, defense, ..
        } => {
            if let Effect::Lose { n, .. } = **effect {
                let dodge = if defense.escape.is_some() { 0.75 } else { 1.0 };
                g.attack += f64::from(n) * dodge;
            }
        },
        Effect::Pick { then, .. } => {
            if matches!(**then, Effect::Ready(_)) {
                g.units += 1.5;
            } else {
                g.add(gain_of(then, per_essence));
            }
        },
        Effect::Reorder { .. } => g.units += 0.1,
        Effect::Choose(options) => {
            let best = options
                .iter()
                .map(|o| gain_of(o, per_essence))
                .max_by(|a, b| (a.units + 5.0 * a.vp).total_cmp(&(b.units + 5.0 * b.vp)));
            if let Some(best) = best {
                g.add(best);
            }
        },
        Effect::Sequence(steps) => {
            for step in steps {
                g.add(gain_of(step, per_essence));
            }
        },
        _ => {},
    }
    g
}

/// Reads a power's cost: `(essence units, turns itself, needs a card with this
/// tag, one shot)`.
fn cost_of(cost: Option<&Cost>, power: &mut Power) {
    let Some(cost) = cost else {
        return;
    };
    match cost {
        Cost::Resources(list) => {
            power.cost += list.iter().map(|(res, n)| f64::from(*n) * unit(*res)).sum::<f64>();
        },
        Cost::Or(options) => {
            let mut cheapest: Option<Power> = None;
            for option in options {
                let mut tried = power.clone();
                cost_of(Some(option), &mut tried);
                if cheapest.as_ref().is_none_or(|c| tried.cost < c.cost) {
                    cheapest = Some(tried);
                }
            }
            if let Some(cheapest) = cheapest {
                *power = cheapest;
            }
        },
        Cost::All(parts) => parts.iter().for_each(|p| cost_of(Some(p), power)),
        Cost::Exhaust => power.turn = true,
        Cost::Any { n, .. } => {
            power.cost += match n {
                Count::Fixed(n) => f64::from(*n),
                _ => 3.0,
            };
        },
        Cost::TakeFromCard { res, n } => power.cost += f64::from(*n) * unit(*res),
        Cost::TurnOther { tag } => power.needs = Some(tag.clone()),
        Cost::Destroy { .. } | Cost::Discard { .. } => power.one_shot = true,
    }
}

fn model_of(def: &CardDef, spec: &crate::CardSpec) -> Model {
    let per_essence = spec.vp.per_essence.map(|(each, e)| (f64::from(each), resource(e)));
    let mut model = Model {
        name: spec.english.clone().unwrap_or_else(|| spec.name.clone()),
        kind: spec.kind,
        tags: def.tags.clone(),
        vp: f64::from(spec.vp.base),
        per_essence,
        price: def.cost.as_ref().map(Price::of),
        collect: 0.0,
        collect_if: None,
        grow: 0.0,
        powers: Vec::new(),
        defense: false,
        check_react: false,
        on_gain: def.on_gain.as_ref().map_or(0.0, |e| gain_of(e, None).units),
        discounts: def
            .discounts
            .iter()
            .map(|d| (d.tag.clone(), d.n, d.exclude.contains(&GOLD)))
            .collect(),
    };
    for ability in &def.abilities {
        read_ability(ability, &mut model);
    }
    model
}

/// The essence a collect ability needs to find on its own card, if it has
/// such a condition.
fn needs_on_card(condition: &Condition) -> Option<ResourceId> {
    match condition {
        Condition::SourceHas(res) => Some(*res),
        Condition::All(all) => all.iter().find_map(needs_on_card),
        _ => None,
    }
}

fn read_ability(ability: &Ability, model: &mut Model) {
    match ability.trigger {
        Trigger::OnEvent(_) => {
            if let Effect::Grow { per_kind } = ability.effect {
                model.grow += f64::from(per_kind);
                return;
            }
            let units = gain_of(&ability.effect, model.per_essence).units.max(0.0);
            match needs_on_card(&ability.condition) {
                Some(res) => model.collect_if = Some((res, units)),
                None => model.collect += units,
            }
        },
        Trigger::Activated => {
            let mut power = Power {
                gain:     gain_of(&ability.effect, model.per_essence),
                cost:     0.0,
                turn:     false,
                needs:    None,
                one_shot: false,
                ends:     checks_win(&ability.effect),
            };
            cost_of(ability.cost.as_ref(), &mut power);
            model.powers.push(power);
        },
        Trigger::React(kind) if kind == COLLECT_STEP => {
            // Taking what lies on the card is no income.
            if !matches!(ability.effect, Effect::TakeAll { .. }) {
                model.collect += gain_of(&ability.effect, model.per_essence).units.max(0.0);
            }
        },
        Trigger::React(kind) if kind == LIFE_LOSS => model.defense = true,
        Trigger::React(kind) if kind == VICTORY_CHECK => model.check_react = true,
        Trigger::React(_) => {},
    }
}

/// What stays the same for every position compared during one choice.
#[derive(Clone, Copy, Debug)]
struct Frame {
    /// The round the choice is made in.
    round:    u32,
    /// Rounds thought to be left.
    left:     f64,
    /// Worth of a victory point now.
    vp_worth: f64,
}

/// One card the player could get, and how good that would be.
struct Goal {
    profit:      f64,
    price:       Price,
    /// Units the player's discounts take off.
    off:         u32,
    /// Whether the discounts cannot reduce Gold.
    off_no_gold: bool,
}

/// Scores positions.
#[derive(Clone, Debug)]
pub struct Eval {
    models: Vec<Option<Model>>,
    params: Params,
}

impl Eval {
    /// Reads every card of the rules.
    pub fn new(rules: &ResArcanaRules, params: Params) -> Self {
        let compiled = rules.cards();
        let mut models = vec![None; compiled.names.len()];
        for def in &compiled.defs {
            if let (Ok(i), Some(spec)) = (usize::try_from(def.id.0), rules.spec(def.id)) {
                models[i] = Some(model_of(def, spec));
            }
        }
        Self { models, params }
    }

    fn model(&self, def: DefId) -> Option<&Model> {
        self.models.get(usize::try_from(def.0).ok()?)?.as_ref()
    }

    fn model_of_entity(&self, state: &GameState, entity: EntityId) -> Option<&Model> {
        self.model(state.entity(entity).ok()?.def)
    }

    fn zone<'a>(
        state: &'a GameState,
        owner: &Owner,
        kind: gamelib::id::ZoneKind,
    ) -> &'a [EntityId] {
        state
            .zone_id(owner, kind)
            .and_then(|z| state.zone(z))
            .map_or(&[], gamelib::zone::Zone::entities)
    }

    fn worth(&self, res: ResourceId) -> f64 {
        if res == GOLD {
            self.params.gold
        } else {
            self.params.essence
        }
    }

    /// The gain of a power for its owner, in units.
    fn power_worth(&self, power: &Power, frame: &Frame, rivals: f64, can_pay_tag: bool) -> f64 {
        if power.one_shot || power.needs.is_some() && !can_pay_tag {
            return 0.0;
        }
        let p = &self.params;
        (power.gain.units - power.cost) * p.essence
            + power.gain.vp * frame.vp_worth
            + power.gain.attack * rivals * p.attack
            - power.gain.gift * rivals * p.essence * 0.7
    }

    /// What a card in play makes per round: `(all of it in units, the part
    /// that is essences, the best power that turns the card)`.
    fn income(
        &self,
        model: &Model,
        on_card: Option<&gamelib::resource::Pool>,
        frame: &Frame,
        rivals: f64,
        threatened: bool,
        own_tags: &[&str],
    ) -> (f64, f64, f64) {
        let p = &self.params;
        let mut essences = model.collect;
        if let Some((res, units)) = model.collect_if {
            let held = on_card.is_none_or(|pool| pool.finite(res).unwrap_or(0) > 0);
            essences += if held { units } else { units * 0.3 };
        }
        if model.grow > 0.0 {
            let kinds = on_card.map_or(1.0, |pool| {
                pool.iter().filter(|(_, s)| *s != gamelib::resource::Supply::Finite(0)).count()
                    as f64
            });
            essences += model.grow * kinds.max(0.5);
        }
        let can_pay =
            |power: &Power| power.needs.as_ref().is_none_or(|t| own_tags.iter().any(|o| o == t));
        let mut best_turn = 0.0_f64;
        let mut best_turn_essences = 0.0_f64;
        let mut repeat = 0.0_f64;
        for power in &model.powers {
            let worth = self.power_worth(power, frame, rivals, can_pay(power));
            if power.turn {
                if worth > best_turn {
                    best_turn = worth;
                    best_turn_essences = (power.gain.units - power.cost).max(0.0);
                }
            } else {
                repeat += worth.max(0.0) * 0.5;
            }
        }
        let mut all = essences * p.essence + best_turn + repeat.min(4.0);
        if model.defense {
            all += if threatened { 1.2 } else { 0.3 };
        }
        for (wanted, n, _) in &model.discounts {
            if wanted.as_deref() == Some(tag::ARTIFACT) {
                all += f64::from(*n) * p.essence;
            }
        }
        (all, essences + best_turn_essences, best_turn)
    }

    /// How many essences are missing to pay a price from a pool, after a
    /// discount.
    fn missing(price: &Price, pool: &[u32; 5], off: u32, off_no_gold: bool) -> u32 {
        let gold = index(GOLD).unwrap_or(4);
        let mut short_gold = 0;
        let mut short_other = 0;
        let mut spare = 0;
        for (i, (&need, &have)) in price.specific.iter().zip(pool).enumerate() {
            if need > have {
                if i == gold {
                    short_gold += need - have;
                } else {
                    short_other += need - have;
                }
            } else if !(price.any_no_gold && i == gold) {
                spare += have - need;
            }
        }
        short_other += price.any.saturating_sub(spare);
        if off_no_gold {
            short_gold + short_other.saturating_sub(off)
        } else {
            (short_gold + short_other).saturating_sub(off)
        }
    }

    /// Takes what a price needs out of a pool (as far as it is there).
    fn reserve(price: &Price, pool: &mut [u32; 5]) {
        let gold = index(GOLD).unwrap_or(4);
        for (have, need) in pool.iter_mut().zip(price.specific) {
            *have = have.saturating_sub(need);
        }
        for _ in 0..price.any {
            let richest = (0..5)
                .filter(|i| !(price.any_no_gold && *i == gold))
                .max_by_key(|i| pool[*i])
                .unwrap_or(0);
            pool[richest] = pool[richest].saturating_sub(1);
        }
    }

    /// The position of one player, in units. `detail` reads the player's own
    /// hand; for a rival only the number of cards is used.
    #[allow(clippy::too_many_lines)]
    fn player(&self, state: &GameState, player: PlayerId, frame: &Frame, detail: bool) -> f64 {
        let p = &self.params;
        let owner = Owner::Player(player);
        let order = state.flow.turn_order.order();
        let rivals = order.len().saturating_sub(1) as f64;
        let tableau = Self::zone(state, &owner, TABLEAU);
        let new_round = state.flow.round > frame.round;

        // Rivals with a ready way to make this player lose Life.
        let threatened = order.iter().filter(|r| **r != player).any(|r| {
            Self::zone(state, &Owner::Player(*r), TABLEAU)
                .iter()
                .any(|e| self.model_of_entity(state, *e).is_some_and(Model::attacks))
        });

        let mut pool = [0_u32; 5];
        if let Ok(held) = state.player_pool(player) {
            for (i, slot) in pool.iter_mut().enumerate() {
                *slot = held.finite(ResourceId(i as u16)).unwrap_or(0);
            }
        }

        let mut total = 0.0;

        // Victory points; the first player token only counts at the last check.
        let token = state.counter(FIRST_TOKEN, &owner).map_or(0, |c| c.value);
        let score = self.score(state, player) - token;
        total += score as f64 * frame.vp_worth;
        total += token as f64 * (0.4 + frame.vp_worth * (1.0 - frame.left * 0.6).clamp(0.0, 1.0));

        // Essences in the pool; a little Life is a shield against attacks.
        for (i, n) in pool.iter().enumerate() {
            total += f64::from(*n) * self.worth(ResourceId(i as u16));
        }
        if threatened && !state.flow.turn_order.has_passed(player) {
            total += f64::from(pool[index(LIFE).unwrap_or(1)].min(2)) * 0.6;
        }

        // Components in play.
        let own_tags: Vec<&str> = tableau
            .iter()
            .filter_map(|e| self.model_of_entity(state, *e))
            .flat_map(|m| m.tags.iter().map(String::as_str))
            .collect();
        let mut essence_income = 0.0;
        let mut dragon_off = 0;
        let mut artifact_off = 0;
        let mut off_no_gold = false;
        for &id in tableau {
            let Ok(entity) = state.entity(id) else {
                continue;
            };
            let Some(model) = self.model(entity.def) else {
                continue;
            };
            let (all, essences, turn) = self.income(
                model,
                Some(&entity.pool),
                frame,
                rivals,
                threatened,
                &own_tags,
            );
            essence_income += essences;
            total += all * frame.left;
            // A copy that ran into the next round stops before its collect
            // step; the powers readied there are not this round's.
            if !new_round && !entity.exhausted {
                total += turn * p.ready;
            }
            if model.check_react {
                let gold = pool[index(GOLD).unwrap_or(4)];
                total += frame.vp_worth * if gold >= 3 { 2.0 } else { 1.0 };
            }
            for (i, slot) in [0_u16, 1, 2, 3, 4].iter().enumerate() {
                let res = ResourceId(*slot);
                let n = f64::from(entity.pool.finite(res).unwrap_or(0));
                let counted = model.per_essence.is_some_and(|(_, r)| r == res);
                if !counted {
                    let _ = i;
                    total += n * self.worth(res) * p.on_card;
                }
            }
            for (wanted, n, no_gold) in &model.discounts {
                match wanted.as_deref() {
                    Some(tag::DRAGON) => dragon_off += n,
                    _ => artifact_off += n,
                }
                off_no_gold |= no_gold;
            }
        }

        // Cards in hand, and the cards worth getting.
        let hand = Self::zone(state, &owner, HAND);
        total += hand.len() as f64 * p.hand_card;
        if detail {
            let dragons = hand
                .iter()
                .filter(|e| self.model_of_entity(state, **e).is_some_and(|m| m.has(tag::DRAGON)))
                .count();
            total += dragons as f64 * p.dragon;
        }
        let flexible = (0.7 * essence_income + 0.5 * hand.len() as f64).max(2.0);

        let mut goals: Vec<Goal> = Vec::new();
        let mut consider = |model: &Model, off: u32, from_hand: bool| {
            let Some(price) = &model.price else {
                return;
            };
            let missing = Self::missing(price, &pool, off, off_no_gold);
            let rounds = f64::from(missing) / flexible;
            if rounds >= frame.left + 0.5 {
                return;
            }
            let (all, ..) = self.income(model, None, frame, rivals, threatened, &own_tags);
            let paid = price
                .specific
                .iter()
                .enumerate()
                .map(|(i, n)| f64::from(*n) * self.worth(ResourceId(i as u16)))
                .sum::<f64>()
                + f64::from(price.any) * p.essence
                - f64::from(off.min(price.count())) * p.essence;
            let mut profit = model.vp * frame.vp_worth
                + all * (frame.left - rounds).max(0.0)
                + model.on_gain * p.essence
                - paid.max(0.0);
            if model.check_react {
                profit += frame.vp_worth;
            }
            if from_hand {
                profit -= p.hand_card;
            }
            if model.has(tag::DRAGON) {
                profit += p.dragon;
            }
            if profit > 0.0 {
                goals.push(Goal {
                    profit: profit * p.goal_ready * p.goal_decay.powf(rounds),
                    price: price.clone(),
                    off,
                    off_no_gold,
                });
            }
        };
        if detail {
            for &id in hand {
                if let Some(model) = self.model_of_entity(state, id) {
                    let off = artifact_off
                        + if model.has(tag::DRAGON) {
                            dragon_off
                        } else {
                            0
                        };
                    consider(model, off, true);
                }
            }
        }
        for kind in [MONUMENT_ROW, PLACES] {
            for &id in Self::zone(state, &Owner::Shared, kind) {
                if let Some(model) = self.model_of_entity(state, id) {
                    consider(model, 0, false);
                }
            }
        }
        // The best goals first; each one keeps the essences it needs away
        // from the next.
        goals.sort_by(|a, b| b.profit.total_cmp(&a.profit));
        let mut left_over = pool;
        for (rank, goal) in goals.iter().take(5).enumerate() {
            let missing = Self::missing(&goal.price, &left_over, goal.off, goal.off_no_gold);
            let first = Self::missing(&goal.price, &pool, goal.off, goal.off_no_gold);
            // A goal that lost its essences to a better one counts for less.
            let shared = p.goal_decay.powf(f64::from(missing.saturating_sub(first)) / flexible);
            total += goal.profit * shared * if rank == 0 { 1.0 } else { 0.8 };
            Self::reserve(&goal.price, &mut left_over);
        }
        total
    }

    /// A player's victory points right now.
    fn score(&self, state: &GameState, player: PlayerId) -> i64 {
        let owner = Owner::Player(player);
        let mut total = 0.0;
        for &id in Self::zone(state, &owner, TABLEAU) {
            let Ok(entity) = state.entity(id) else {
                continue;
            };
            let Some(model) = self.model(entity.def) else {
                continue;
            };
            total += model.vp;
            if let Some((each, res)) = model.per_essence {
                total += each * f64::from(entity.pool.finite(res).unwrap_or(0));
            }
        }
        let counters: i64 = [crate::compile::VP, crate::compile::TEMP_VP, FIRST_TOKEN]
            .iter()
            .map(|c| state.counter(*c, &owner).map_or(0, |c| c.value))
            .sum();
        total as i64 + counters
    }

    /// Victory points a player could still claim right now from the places
    /// of power and monuments on the table, with what is in the pool.
    fn imminent(&self, state: &GameState, player: PlayerId) -> f64 {
        let mut pool = [0_u32; 5];
        if let Ok(held) = state.player_pool(player) {
            for (i, slot) in pool.iter_mut().enumerate() {
                *slot = held.finite(ResourceId(i as u16)).unwrap_or(0);
            }
        }
        let mut options: Vec<(f64, &Price)> = Vec::new();
        for kind in [MONUMENT_ROW, PLACES] {
            for &id in Self::zone(state, &Owner::Shared, kind) {
                if let Some(model) = self.model_of_entity(state, id)
                    && let Some(price) = &model.price
                {
                    options.push((model.vp, price));
                }
            }
        }
        options.sort_by(|a, b| b.0.total_cmp(&a.0));
        let mut total = 0.0;
        for (vp, price) in options {
            if Self::missing(price, &pool, 0, false) == 0 {
                total += vp;
                Self::reserve(price, &mut pool);
            }
        }
        total
    }

    /// Whether the player has a victory check ready to use.
    fn can_end(&self, state: &GameState, player: PlayerId) -> bool {
        Self::zone(state, &Owner::Player(player), TABLEAU).iter().any(|&id| {
            state.entity(id).is_ok_and(|e| {
                !e.exhausted
                    && self.model(e.def).is_some_and(|m| m.powers.iter().any(|p| p.ends))
            })
        })
    }

    /// The frame for choices made in this state.
    fn frame(&self, state: &GameState) -> Frame {
        let p = &self.params;
        // How far the leader is, counting what a player can buy right now.
        let lead = state
            .flow
            .turn_order
            .order()
            .iter()
            .map(|q| self.score(state, *q) as f64 + p.reach * self.imminent(state, *q))
            .fold(0.0, f64::max);
        let left = ((WIN_SCORE as f64 - lead) / p.pace).clamp(0.4, p.horizon);
        Frame {
            round: state.flow.round,
            left,
            vp_worth: p.vp_base + p.vp_late / left,
        }
    }

    /// How good the state is for `me`: the own position minus the rivals'.
    fn position(&self, state: &GameState, me: PlayerId, frame: &Frame, detail: bool) -> f64 {
        if let Some(winners) = &state.winners {
            return if winners.contains(&me) {
                WIN / winners.len() as f64 + self.score(state, me) as f64
            } else {
                -WIN
            };
        }
        let mine = self.player(state, me, frame, detail);
        let rivals: Vec<f64> = state
            .flow
            .turn_order
            .order()
            .iter()
            .filter(|q| **q != me)
            .map(|q| self.player(state, *q, frame, false))
            .collect();
        if rivals.is_empty() {
            return mine;
        }
        let mut value = mine - self.params.rival * rivals.iter().sum::<f64>() / rivals.len() as f64;
        // A rival with a ready victory check who can end the game ahead.
        let my_reach = self.score(state, me);
        let turn = &state.flow.turn_order;
        let threat = turn.order().iter().filter(|q| **q != me && !turn.has_passed(**q)).any(|q| {
            let reach = self.score(state, *q) + self.imminent(state, *q).floor() as i64;
            self.can_end(state, *q) && reach >= WIN_SCORE && reach >= my_reach
        });
        if threat {
            value -= self.params.check_threat * frame.vp_worth;
        }
        value
    }
}

/// The bot that looks one move ahead with [`Eval`].
#[derive(Debug)]
pub struct PlannerBot {
    rules:   ResArcanaRules,
    eval:    Eval,
    note:    Option<String>,
    round:   u32,
    actions: u32,
}

impl PlannerBot {
    /// A bot with the default numbers.
    pub fn new() -> Self {
        Self::with_params(Params::default())
    }

    /// A bot with its own numbers (for tuning).
    pub fn with_params(params: Params) -> Self {
        let rules = ResArcanaRules::new();
        let eval = Eval::new(&rules, params);
        Self {
            rules,
            eval,
            note: None,
            round: 0,
            actions: 0,
        }
    }

    /// The name of a card for the notes.
    fn name(&self, state: &GameState, entity: EntityId) -> String {
        self.eval
            .model_of_entity(state, entity)
            .map_or_else(|| format!("card {}", entity.0), |m| m.name.clone())
    }

    fn describe(&self, state: &GameState, action: &Action) -> String {
        match action {
            Action::Pass => "pass".to_owned(),
            Action::EndTurn => "end turn".to_owned(),
            Action::Buy { entity, .. } => format!("claim {}", self.name(state, *entity)),
            Action::Place { entity, pay } => {
                format!("place {} (payment {pay})", self.name(state, *entity))
            },
            Action::Discard { entity } => format!("discard {}", self.name(state, *entity)),
            Action::Activate {
                entity,
                ability,
                pay,
            } => format!(
                "use {} power {ability} (payment {pay})",
                self.name(state, *entity)
            ),
            Action::Decide(i) => format!("option {i}"),
        }
    }

    /// Plays `action` for `who` on a copy and returns the copy.
    fn after(&self, state: &GameState, who: PlayerId, action: &Action) -> Option<GameState> {
        let mut engine = Engine::from_state(&self.rules, state.clone());
        engine.apply(who, action.clone()).ok()?;
        Some(engine.state().clone())
    }

    /// Answers the decisions that follow a move, then scores the position: the
    /// bot's own decisions by trying each option, a rival's by what is best
    /// for that rival at first sight.
    fn settle(
        &self,
        mut state: GameState,
        me: PlayerId,
        frame: &Frame,
        budget: &mut usize,
        depth: usize,
    ) -> f64 {
        loop {
            if state.winners.is_some() {
                break;
            }
            let Some(pending) = &state.pending else {
                break;
            };
            // The next round's collect step is not this move's business.
            if state.flow.round > frame.round {
                break;
            }
            // A decision several players take at once: stop here.
            if pending.players.len() > 1 {
                break;
            }
            let Some(&who) = pending.players.iter().find(|q| pending.waiting_for(**q)) else {
                break;
            };
            let options = pending.options.len();
            if *budget < options || options == 0 {
                // Out of budget: the first option is the default everywhere.
                match self.after(&state, who, &Action::Decide(0)) {
                    Some(next) => state = next,
                    None => break,
                }
                continue;
            }
            *budget -= options;
            if who == me && depth < FULL_DEPTH {
                // Try every option to the end and keep the best.
                let mut best = f64::NEG_INFINITY;
                for i in 0..options {
                    if let Some(next) = self.after(&state, me, &Action::Decide(i)) {
                        best = best.max(self.settle(next, me, frame, budget, depth + 1));
                    }
                }
                return best;
            }
            // A rival's decision, or one of the bot's own deep in a chain
            // (the order of a long collect step): what looks best at once.
            let mut best: Option<(f64, GameState)> = None;
            for i in 0..options {
                if let Some(next) = self.after(&state, who, &Action::Decide(i)) {
                    let value = self.eval.position(&next, who, frame, who == me);
                    if best.as_ref().is_none_or(|(b, _)| value > *b) {
                        best = Some((value, next));
                    }
                }
            }
            match best {
                Some((_, next)) => state = next,
                None => break,
            }
        }
        self.eval.position(&state, me, frame, true)
    }

    /// Which of two dealt mages to keep: the one that makes more per round.
    fn keep_mage(&self, state: &GameState, me: PlayerId, legal: &[Action]) -> usize {
        let frame = self.eval.frame(state);
        let dealt = Eval::zone(state, &Owner::Player(me), CANDIDATES);
        let worth = |i: usize| {
            dealt.get(i).and_then(|e| self.eval.model_of_entity(state, *e)).map_or(0.0, |m| {
                self.eval.income(m, None, &frame, 1.0, false, &[]).0
            })
        };
        let Some(pending) = &state.pending else {
            return 0;
        };
        let mut best = (f64::NEG_INFINITY, 0);
        for (at, action) in legal.iter().enumerate() {
            let Action::Decide(option) = action else {
                continue;
            };
            let kept = match pending.options.get(*option) {
                Some(Effect::Sequence(steps)) => steps.iter().find_map(|s| match s {
                    Effect::MoveNth { index, .. } => Some(*index),
                    _ => None,
                }),
                _ => None,
            };
            let value = kept.map_or(0.0, worth);
            if value > best.0 {
                best = (value, at);
            }
        }
        best.1
    }

    /// In the collect step: a collect ability that only gives something (no
    /// price to pay, nothing to take off a card). These are always done and
    /// their order among each other does not matter, so they go first without
    /// trying anything out; what needs thought (paying a curse, taking
    /// essences that score on their card) is decided afterwards, with
    /// everything collected.
    fn plain_collect(state: &GameState, legal: &[Action]) -> Option<usize> {
        fn pays(effect: &Effect) -> bool {
            match effect {
                Effect::Pay { .. } | Effect::TakeAll { .. } => true,
                Effect::Choose(list) | Effect::Sequence(list) => list.iter().any(pays),
                _ => false,
            }
        }
        let pending = state.pending.as_ref()?;
        legal.iter().position(|action| {
            let Action::Decide(i) = action else {
                return false;
            };
            let first = match pending.options.get(*i) {
                Some(Effect::Sequence(steps)) => steps.first(),
                other => other,
            };
            let Some(Effect::Use {
                source, ability, ..
            }) = first
            else {
                return false;
            };
            state
                .entity(*source)
                .ok()
                .and_then(|e| state.defs.get(&e.def))
                .and_then(|d| d.abilities.get(*ability))
                .is_some_and(|a| a.trigger == Trigger::React(COLLECT_STEP) && !pays(&a.effect))
        })
    }

    /// A copy of the state in which the zones the player cannot see are
    /// shuffled, so that trying a move reveals nothing.
    fn blind_copy(state: &GameState, me: PlayerId) -> GameState {
        let mut copy = state.clone();
        for (owner, kind) in [(Owner::Player(me), DECK), (Owner::Shared, MONUMENT_DECK)] {
            if let Ok(zone) = copy.zone_id(&owner, kind) {
                let _ = copy.shuffle(zone);
            }
        }
        copy.clear_events();
        copy
    }
}

impl Default for PlannerBot {
    fn default() -> Self {
        Self::new()
    }
}

impl Bot for PlannerBot {
    fn choose(&mut self, _view: &PlayerView, legal: &[Action]) -> usize {
        // Without the state there is nothing to try out: pass, or take the
        // default option.
        self.note = Some("no state given".to_owned());
        legal.iter().position(|a| *a == Action::Pass).unwrap_or(0)
    }

    fn choose_with_state(
        &mut self,
        state: &GameState,
        view: &PlayerView,
        legal: &[Action],
    ) -> usize {
        let me = view.viewer;
        if state.flow.round != self.round {
            self.round = state.flow.round;
            self.actions = 0;
        }
        if legal.len() == 1 {
            self.note = Some("only move".to_owned());
            return 0;
        }
        let deciding = matches!(legal.first(), Some(Action::Decide(_)));
        if deciding && state.pending.as_ref().is_some_and(|p| p.players.len() > 1) {
            let choice = self.keep_mage(state, me, legal);
            self.note = Some("kept the mage that makes more".to_owned());
            return choice;
        }

        if deciding && let Some(at) = Self::plain_collect(state, legal) {
            self.note = Some("collect step: a plain collect ability first".to_owned());
            return at;
        }

        let base = Self::blind_copy(state, me);
        let frame = self.eval.frame(&base);
        let now = self.eval.position(&base, me, &frame, true);
        let rivals_now: f64 = base
            .flow
            .turn_order
            .order()
            .iter()
            .filter(|q| **q != me)
            .map(|q| self.eval.player(&base, *q, &frame, false))
            .sum();
        let mut budget = BUDGET;
        let mut scored: Vec<(f64, usize)> = Vec::with_capacity(legal.len());
        for (i, action) in legal.iter().enumerate() {
            if budget == 0 {
                break;
            }
            budget -= 1;
            let Some(next) = self.after(&base, me, action) else {
                continue;
            };
            let mut gain = self.settle(next.clone(), me, &frame, &mut budget, 0) - now;
            if !deciding && *action != Action::Pass && next.winners.is_none() {
                // Hurting the rivals only works until they pass: do it first.
                let rivals_after: f64 = next
                    .flow
                    .turn_order
                    .order()
                    .iter()
                    .filter(|q| **q != me)
                    .map(|q| self.eval.player(&next, *q, &frame, false))
                    .sum();
                if rivals_after < rivals_now - 0.5 && next.pending.is_none() {
                    gain += self.eval.params.attack_first;
                }
            }
            scored.push((gain, i));
        }
        scored.sort_by(|a, b| b.0.total_cmp(&a.0));

        let pass = legal.iter().position(|a| *a == Action::Pass);
        let pass_gain = pass.and_then(|at| scored.iter().find(|(_, i)| *i == at)).map(|s| s.0);
        let best_move = scored.iter().find(|(_, i)| Some(*i) != pass).copied();
        let wins = |gain: f64| gain > WIN / 4.0;
        // A rival who has not passed can still reach the winning score this
        // round: passing hands them the last moves, so a move that costs
        // little is better than passing.
        let rushed = {
            let turn = &state.flow.turn_order;
            turn.order().iter().filter(|q| **q != me && !turn.has_passed(**q)).any(|q| {
                self.eval.score(state, *q) + self.eval.imminent(state, *q).floor() as i64
                    >= WIN_SCORE
            })
        };
        let floor = if rushed {
            self.eval.params.min_gain.min(-self.eval.params.tempo)
        } else {
            self.eval.params.min_gain
        };
        let choice = if deciding {
            scored.first().map_or(0, |s| s.1)
        } else {
            match (best_move, pass) {
                // A pass that wins beats everything that does not.
                (Some((gain, _)), Some(at)) if pass_gain.is_some_and(wins) && !wins(gain) => at,
                (Some((gain, i)), _)
                    if gain > floor && self.actions < ACTIONS_PER_ROUND =>
                {
                    i
                },
                (_, Some(at)) => at,
                (Some((_, i)), None) => i,
                (None, None) => 0,
            }
        };
        if !deciding {
            self.actions += 1;
        }

        let mut note = format!(
            "round {} left {:.1} vp {:.1} position {now:.1}; chose {}",
            frame.round,
            frame.left,
            frame.vp_worth,
            self.describe(state, &legal[choice]),
        );
        for (gain, i) in scored.iter().take(6) {
            let _ = write!(note, "; {gain:+.2} {}", self.describe(state, &legal[*i]));
        }
        if let Some(gain) = pass_gain {
            let _ = write!(note, "; pass {gain:+.2}");
        }
        let _ = write!(note, "; {} of {} moves tried", scored.len(), legal.len());
        self.note = Some(note);
        choice
    }

    fn explain(&self) -> Option<String> {
        self.note.clone()
    }
}
