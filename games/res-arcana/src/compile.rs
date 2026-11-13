//! Compiles the typed [`CardSpec`]s into engine [`CardDef`]s.
//!
//! The engine crate names no game concept. Everything Res Arcana specific
//! lives here as data: the resource, counter, phase, zone and reaction ids
//! (constants below), the card tags, and the way each printed power is
//! composed from the engine's generic effects and costs.
//!
//! Conventions the rest of the game (setup, rules, scoring) relies on:
//!
//! * Every card gets the tag of its [`CardKind`] (see [`tag`]) and, for
//!   artifacts, `dragon` and/or `creature`.
//! * `CardDef::abilities` holds, in printed order, the collect abilities,
//!   powers and reactions. `discount` lines become `CardDef::discounts` and
//!   `onclaim` becomes `CardDef::on_gain`, so they do not take an ability
//!   index.
//! * Collect abilities are reactions of the kind [`COLLECT_STEP`], each usable
//!   once per round: the rules open a window in the [`TAKE`] phase in which a
//!   player does them in the order they like. The two that depend on what was
//!   left on the card (Vault, Windup Man) trigger afterwards, on the start of
//!   the [`COLLECT`] phase.
//! * A "lose Life" power is an [`Effect::Affect`] aimed at the rivals whose
//!   defense offers their [`LIFE_LOSS`] reactions (and [`DRAGON_LIFE_LOSS`]
//!   when the card is a Dragon) plus the escape printed on the card.
//! * Effects that pick one of your components (straighten any, put an essence
//!   on any component) look in your [`TABLEAU`] zone, so mages and magic items
//!   must live there too.
//! * Victory points are not compiled here: scoring formulas are read from the
//!   specs by a later unit. [`victory_check`] and the [`TEMP_VP`] counter (for
//!   temporary points) are the only scoring related pieces.

use std::{error::Error, fmt};

use gamelib::{
    card::{CardDef, Discount},
    cost::{self, Cost},
    effect::{
        Ability,
        Condition,
        Defense,
        Effect,
        Escape,
        Filter,
        Limit,
        OwnerRef,
        Target,
        Trigger,
        Who,
    },
    event::EventKind,
    id::{CounterId, DefId, PhaseId, ReactKind, ResourceId, ZoneKind},
};

use crate::spec::{
    self,
    Amount,
    CardKind,
    CardSpec,
    CardType,
    CostPart,
    CostSource,
    Count,
    Deck,
    DiscountOn,
    EffectPart,
    Essence,
    Options,
    Price,
    PutOn,
    RivalReact,
    Selector,
    Straighten,
    Zone,
};

// ---------------------------------------------------------------------------------------------------
// Ids the game defines
// ---------------------------------------------------------------------------------------------------

/// Elan, the red essence.
pub const ELAN: ResourceId = ResourceId(0);
/// Life, the green essence.
pub const LIFE: ResourceId = ResourceId(1);
/// Calm, the blue essence.
pub const CALM: ResourceId = ResourceId(2);
/// Death, the black essence.
pub const DEATH: ResourceId = ResourceId(3);
/// Gold, the yellow essence.
pub const GOLD: ResourceId = ResourceId(4);

/// The engine resource of an essence.
pub const fn resource(essence: Essence) -> ResourceId {
    match essence {
        Essence::Elan => ELAN,
        Essence::Life => LIFE,
        Essence::Calm => CALM,
        Essence::Death => DEATH,
        Essence::Gold => GOLD,
    }
}

/// The essence of an engine resource, if it is one of the five.
pub const fn essence(res: ResourceId) -> Option<Essence> {
    match res.0 {
        0 => Some(Essence::Elan),
        1 => Some(Essence::Life),
        2 => Some(Essence::Calm),
        3 => Some(Essence::Death),
        4 => Some(Essence::Gold),
        _ => None,
    }
}

/// Reaction window: a rival's effect makes you lose Life.
pub const LIFE_LOSS: ReactKind = ReactKind(0);
/// Reaction window: a Dragon's power (Life loss from a Dragon).
pub const DRAGON_LIFE_LOSS: ReactKind = ReactKind(1);
/// Reaction window: a victory check.
pub const VICTORY_CHECK: ReactKind = ReactKind(2);

/// Reaction window: the collect step. A player's collect abilities, and
/// taking the essences off a component, in the order the player chooses.
pub const COLLECT_STEP: ReactKind = ReactKind(3);

/// Round phase: each player does their collect abilities and takes essences
/// off their components, in the order they choose.
pub const TAKE: PhaseId = PhaseId(0);
/// Round phase: the abilities that depend on what was left on a component
/// resolve.
pub const COLLECT: PhaseId = PhaseId(1);
/// Round phase: players act in turn.
pub const ACTIONS: PhaseId = PhaseId(2);
/// Round phase: the victory check.
pub const VICTORY: PhaseId = PhaseId(3);
/// Round phase: everything straightens.
pub const CLEANUP: PhaseId = PhaseId(4);

/// Each player's victory points counter.
pub const VP: CounterId = CounterId(0);
/// Each player's temporary victory points, counted in one victory check only.
pub const TEMP_VP: CounterId = CounterId(1);

/// A player's artifact draw pile.
pub const DECK: ZoneKind = ZoneKind(0);
/// A player's hand.
pub const HAND: ZoneKind = ZoneKind(1);
/// A player's discard pile.
pub const DISCARD: ZoneKind = ZoneKind(2);
/// A player's components in play (mage, magic item, artifacts, monuments and
/// places): the zone whose abilities are active.
pub const TABLEAU: ZoneKind = ZoneKind(3);
/// The shared draw pile of unseen monuments.
pub const MONUMENT_DECK: ZoneKind = ZoneKind(10);

/// Other essences lost for each Life that cannot be lost.
pub const LIFE_SHORTFALL: u32 = 2;

/// A discount large enough to make any placement free.
pub const FREE: u32 = 1_000;

/// Card tags.
pub mod tag {
    /// Mages.
    pub const MAGE: &str = "mage";
    /// Artifacts.
    pub const ARTIFACT: &str = "artifact";
    /// Monuments.
    pub const MONUMENT: &str = "monument";
    /// Places of Power.
    pub const PLACE: &str = "place";
    /// Magic items.
    pub const ITEM: &str = "item";
    /// Dragon artifacts.
    pub const DRAGON: &str = "dragon";
    /// Creature artifacts.
    pub const CREATURE: &str = "creature";
}

/// The tag of a card family.
pub const fn kind_tag(kind: CardKind) -> &'static str {
    match kind {
        CardKind::Mage => tag::MAGE,
        CardKind::Artifact => tag::ARTIFACT,
        CardKind::Monument => tag::MONUMENT,
        CardKind::Place => tag::PLACE,
        CardKind::Item => tag::ITEM,
    }
}

/// The tag of an artifact type.
pub const fn type_tag(ty: CardType) -> &'static str {
    match ty {
        CardType::Dragon => tag::DRAGON,
        CardType::Creature => tag::CREATURE,
    }
}

/// A victory check: every player may react (Golden Statue) as often as they
/// like, then the winner is decided, then the temporary points are forgotten.
pub fn victory_check(temp: CounterId) -> Effect {
    Effect::Sequence(vec![
        Effect::ForEachPlayer(Box::new(Effect::ReactWindow {
            kind:    VICTORY_CHECK,
            default: Box::new(Effect::Ignore),
            repeat:  true,
        })),
        Effect::CheckWin,
        Effect::ResetCounters { counter: temp },
    ])
}

// ---------------------------------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------------------------------

/// A card the compiler cannot express with the engine's primitives.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompileError {
    /// The printed name of the card.
    pub card:   String,
    /// What is not supported.
    pub reason: Reason,
}

/// Why a card does not compile.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reason {
    /// The engine has no primitive for this construct yet.
    Unsupported(&'static str),
    /// The card data combines things the compiler does not know how to map.
    Invalid(String),
}

impl fmt::Display for CompileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.reason {
            Reason::Unsupported(what) => {
                write!(f, "{}: not supported by the engine yet: {what}", self.card)
            },
            Reason::Invalid(what) => write!(f, "{}: {what}", self.card),
        }
    }
}

impl Error for CompileError {}

type Res<T> = Result<T, Reason>;

fn invalid<T>(what: &str) -> Res<T> {
    Err(Reason::Invalid(what.into()))
}

// ---------------------------------------------------------------------------------------------------
// Amounts, costs
// ---------------------------------------------------------------------------------------------------

/// The resources of excluded essences, sorted and without duplicates so equal
/// sets compare equal.
fn kinds(list: &[Essence]) -> Vec<ResourceId> {
    let mut out: Vec<ResourceId> = list.iter().map(|e| resource(*e)).collect();
    out.sort();
    out.dedup();
    out
}

const fn engine_count(count: Count) -> cost::Count {
    match count {
        Count::Fixed(n) => cost::Count::Fixed(n),
        Count::Var => cost::Count::Var,
    }
}

/// The fixed number of a specific essence (what a plain `3D` is).
const fn specific(amount: &Amount) -> Option<(ResourceId, u32)> {
    match (&amount.what, amount.count) {
        (Selector::Essence(e), Count::Fixed(n)) => Some((resource(*e), n)),
        _ => None,
    }
}

/// An amount that is paid as a stand-alone cost part, other than a plain
/// number of a specific essence.
fn pay_any(amount: &Amount) -> Res<Cost> {
    match (&amount.what, amount.count) {
        (Selector::Any { exclude }, count) => {
            if count == Count::Var && !amount.identical {
                return Err(Reason::Unsupported(
                    "a variable number of non-identical essences",
                ));
            }
            Ok(Cost::Any {
                n:         engine_count(count),
                exclude:   kinds(exclude),
                identical: amount.identical,
            })
        },
        (Selector::Essence(_), Count::Var) => Err(Reason::Unsupported(
            "a variable number of one specific essence in a cost",
        )),
        (Selector::Essence(_), Count::Fixed(_)) => invalid("not a stand-alone cost part"),
    }
}

/// Builds a cost out of parts: plain essences are merged into one
/// `Cost::Resources`, other parts keep their order. No part means free.
struct CostBuilder {
    parts:     Vec<Cost>,
    resources: Option<usize>,
}

impl CostBuilder {
    const fn new() -> Self {
        Self {
            parts:     Vec::new(),
            resources: None,
        }
    }

    fn pay(&mut self, amount: &Amount) -> Res<()> {
        if let Some((res, n)) = specific(amount) {
            let at = *self.resources.get_or_insert_with(|| {
                self.parts.push(Cost::Resources(Vec::new()));
                self.parts.len() - 1
            });
            if let Cost::Resources(list) = &mut self.parts[at] {
                match list.iter_mut().find(|(r, _)| *r == res) {
                    Some((_, have)) => *have += n,
                    None => list.push((res, n)),
                }
            }
            return Ok(());
        }
        self.push(pay_any(amount)?);
        Ok(())
    }

    fn push(&mut self, part: Cost) {
        self.parts.push(part);
    }

    /// The cost, `None` when there is nothing to pay.
    fn finish(mut self) -> Option<Cost> {
        match self.parts.len() {
            0 => None,
            1 => self.parts.pop(),
            _ => Some(Cost::All(self.parts)),
        }
    }
}

const fn destroy_which(which: spec::Which) -> cost::Which {
    match which {
        spec::Which::This => cost::Which::This,
        spec::Which::Other => cost::Which::Other,
        spec::Which::Own => cost::Which::Own,
    }
}

/// The cost of a power or reaction.
fn power_cost(parts: &[CostPart]) -> Res<Option<Cost>> {
    let mut b = CostBuilder::new();
    for part in parts {
        match part {
            CostPart::Turn => b.push(Cost::Exhaust),
            CostPart::Pay(amount) => b.pay(amount)?,
            CostPart::PayOneOf(options) => {
                let mut alternatives = Vec::new();
                for amount in options {
                    let mut one = CostBuilder::new();
                    one.pay(amount)?;
                    let Some(alternative) = one.finish() else {
                        return invalid("a pay-one-of alternative that costs nothing");
                    };
                    alternatives.push(alternative);
                }
                b.push(Cost::Or(alternatives));
            },
            CostPart::TurnOther(ty) => b.push(Cost::TurnOther {
                tag: type_tag(*ty).into(),
            }),
            CostPart::Destroy(which) => b.push(Cost::Destroy {
                which: destroy_which(*which),
                tag:   tag::ARTIFACT.into(),
            }),
            CostPart::Discard(n) => b.push(Cost::Discard {
                from: HAND,
                n:    *n,
            }),
            CostPart::TakeFromCard(amount) => {
                let Some((res, n)) = specific(amount) else {
                    return Err(Reason::Unsupported(
                        "taking a variable or any essence off a card",
                    ));
                };
                b.push(Cost::TakeFromCard { res, n });
            },
        }
    }
    Ok(b.finish())
}

/// The placement cost of a card. A free card has `Cost::Resources(vec![])`.
fn placement_cost(amounts: &[Amount]) -> Res<Cost> {
    let mut specifics = CostBuilder::new();
    let mut any: Option<(u32, Vec<ResourceId>)> = None;
    for amount in amounts {
        match (&amount.what, amount.count) {
            (Selector::Essence(_), Count::Fixed(_)) => specifics.pay(amount)?,
            (Selector::Any { exclude }, Count::Fixed(n)) if !amount.identical => {
                let exclude = kinds(exclude);
                match &mut any {
                    None => any = Some((n, exclude)),
                    Some((have, ex)) if *ex == exclude => *have += n,
                    Some(_) => {
                        return Err(Reason::Unsupported(
                            "a placement cost with different kinds of any-essence",
                        ));
                    },
                }
            },
            _ => {
                return Err(Reason::Unsupported(
                    "a placement cost with identical or variable amounts",
                ));
            },
        }
    }
    let mut parts: Vec<Cost> = specifics.finish().into_iter().collect();
    if let Some((n, exclude)) = any {
        parts.push(Cost::Any {
            n: cost::Count::Fixed(n),
            exclude,
            identical: false,
        });
    }
    Ok(match parts.len() {
        0 => Cost::Resources(Vec::new()),
        1 => parts.remove(0),
        _ => Cost::All(parts),
    })
}

// ---------------------------------------------------------------------------------------------------
// Effects
// ---------------------------------------------------------------------------------------------------

/// Taking one printed amount from the supply.
fn gain(to: Target, amount: &Amount) -> Effect {
    match (&amount.what, amount.count) {
        (Selector::Essence(e), Count::Fixed(n)) => Effect::Gain {
            to,
            res: resource(*e),
            n,
        },
        (Selector::Essence(e), Count::Var) => Effect::GainX {
            to,
            res: resource(*e),
        },
        (Selector::Any { exclude }, count) => Effect::GainChoose {
            to,
            n: engine_count(count),
            exclude: kinds(exclude),
            identical: amount.identical,
        },
    }
}

/// One effect for several (the effects happen in order).
fn sequence(mut effects: Vec<Effect>) -> Effect {
    match effects.len() {
        0 => Effect::Ignore,
        1 => effects.remove(0),
        _ => Effect::Sequence(effects),
    }
}

/// The options of a gain: with several groups the player chooses one group.
fn gain_options(to: Target, options: &Options) -> Res<Effect> {
    if options.0.iter().any(Vec::is_empty) {
        return invalid("a gain option without essences");
    }
    let mut groups: Vec<Effect> = options
        .0
        .iter()
        .map(|group| sequence(group.iter().map(|a| gain(to, a)).collect()))
        .collect();
    match groups.len() {
        0 => invalid("gain without options"),
        1 => Ok(groups.remove(0)),
        _ => Ok(Effect::Choose(groups)),
    }
}

/// A fixed number of a specific essence (what a simple pay or lose needs).
fn fixed_essence(amount: &Amount, what: &'static str) -> Res<(ResourceId, u32)> {
    specific(amount).ok_or(Reason::Unsupported(what))
}

fn escape(unless: Option<&RivalReact>) -> Res<Option<Escape>> {
    Ok(match unless {
        None => None,
        Some(RivalReact::Pay(amount)) => {
            let (res, n) = fixed_essence(amount, "a rival escape that is not a fixed essence")?;
            Some(Escape::Pay(res, n))
        },
        Some(RivalReact::DiscardCard) => Some(Escape::DiscardCard),
        Some(RivalReact::DestroyArtifact) => Some(Escape::Destroy(tag::ARTIFACT.into())),
    })
}

fn peek(count: u32, decks: &[Deck]) -> Res<Effect> {
    let n = count as usize;
    let mut options: Vec<Effect> = decks
        .iter()
        .map(|deck| match deck {
            Deck::Artifact => Effect::Reorder {
                owner: OwnerRef::Controller,
                zone: DECK,
                n,
            },
            Deck::Monument => Effect::Reorder {
                owner: OwnerRef::Shared,
                zone: MONUMENT_DECK,
                n,
            },
        })
        .collect();
    match options.len() {
        0 => invalid("peek without a deck"),
        1 => Ok(options.remove(0)),
        _ => Ok(Effect::Choose(options)),
    }
}

/// Picks one of your turned components (optionally with a tag) and straightens
/// it.
fn straighten(only_tag: Option<&str>) -> Effect {
    Effect::Pick {
        from:   TABLEAU,
        owner:  OwnerRef::Controller,
        filter: Filter {
            tag:           only_tag.map(Into::into),
            exhausted:     Some(true),
            has_resources: None,
        },
        then:   Box::new(Effect::Ready(Target::Card)),
    }
}

fn place(only: Option<CardType>, from: Zone, price: &Price) -> Res<Effect> {
    let discount = match price {
        Price::Free => Discount {
            tag:     None,
            n:       FREE,
            exclude: Vec::new(),
        },
        Price::Discount(amount) => match (&amount.what, amount.count) {
            (Selector::Any { exclude }, Count::Fixed(n)) if !amount.identical => Discount {
                tag: None,
                n,
                exclude: kinds(exclude),
            },
            _ => {
                return Err(Reason::Unsupported(
                    "a placement discount that is not any-essence",
                ));
            },
        },
    };
    Ok(Effect::PlaceCard {
        from:     (from == Zone::Discard).then_some(DISCARD),
        tag:      Some(only.map_or(tag::ARTIFACT, type_tag).into()),
        discount: Some(discount),
    })
}

fn effect_part(part: &EffectPart, dragon: bool) -> Res<Effect> {
    Ok(match part {
        EffectPart::Gain { options, on_card } => gain_options(
            if *on_card {
                Target::Source
            } else {
                Target::Controller
            },
            options,
        )?,
        EffectPart::Draw(n) => Effect::Draw {
            who:  OwnerRef::Controller,
            from: DECK,
            to:   HAND,
            n:    *n,
        },
        EffectPart::Discard(n) => Effect::DiscardChoose { n: *n },
        EffectPart::Peek { count, decks } => peek(*count, decks)?,
        EffectPart::Straighten(Straighten::Any) => straighten(None),
        EffectPart::Straighten(Straighten::Creature) => straighten(Some(tag::CREATURE)),
        EffectPart::Straighten(Straighten::This) => Effect::Ready(Target::Source),
        EffectPart::IgnoreLifeLoss => Effect::Ignore,
        EffectPart::RivalsLoseLife { amount, unless } => {
            let mut react = vec![LIFE_LOSS];
            if dragon {
                react.push(DRAGON_LIFE_LOSS);
            }
            Effect::Affect {
                who:     Who::Rivals,
                defense: Defense {
                    react,
                    escape: escape(unless.as_ref())?,
                    skip_passed: true,
                },
                effect:  Box::new(Effect::Lose {
                    res:       LIFE,
                    n:         *amount,
                    shortfall: LIFE_SHORTFALL,
                }),
            }
        },
        EffectPart::RivalsGain(amount) => {
            let (res, n) = fixed_essence(amount, "rivals gaining a variable or any essence")?;
            Effect::GainAll {
                who: Who::Rivals,
                res,
                n,
            }
        },
        EffectPart::GainPerRival { gain, counted } => Effect::GainPerRival {
            res:     resource(*gain),
            counted: resource(*counted),
        },
        EffectPart::GainCost { of, bonus, exclude } => Effect::GainChoose {
            to:        Target::Controller,
            n:         cost::Count::CostOf {
                discarded: *of == CostSource::Discarded,
                bonus:     *bonus,
            },
            exclude:   kinds(exclude),
            identical: false,
        },
        EffectPart::Place { only, from, price } => place(*only, *from, price)?,
        EffectPart::PutPaid(PutOn::This) => Effect::GainPaid { to: Target::Source },
        EffectPart::PutPaid(PutOn::AnyComponent) => Effect::Pick {
            from:   TABLEAU,
            owner:  OwnerRef::Controller,
            filter: Filter::default(),
            then:   Box::new(Effect::GainPaid { to: Target::Card }),
        },
        EffectPart::VictoryCheck => victory_check(TEMP_VP),
        EffectPart::Vp { points, temporary } => Effect::ModifyCounter {
            counter: if *temporary { TEMP_VP } else { VP },
            owner:   OwnerRef::Controller,
            delta:   i64::from(*points),
        },
    })
}

fn effect_of(parts: &[EffectPart], dragon: bool) -> Res<Effect> {
    let mut effects = Vec::new();
    let mut i = 0;
    while i < parts.len() {
        // "Draw n, then discard n" (Divination): the discard is as many cards
        // as were really drawn when the deck runs short (rulebook p.8).
        if let (EffectPart::Draw(a), Some(EffectPart::Discard(b))) = (&parts[i], parts.get(i + 1))
            && a == b
        {
            effects.push(Effect::DrawDiscard {
                from: DECK,
                to:   HAND,
                n:    *a,
            });
            i += 2;
            continue;
        }
        effects.push(effect_part(&parts[i], dragon)?);
        i += 1;
    }
    Ok(sequence(effects))
}

// ---------------------------------------------------------------------------------------------------
// Abilities and cards
// ---------------------------------------------------------------------------------------------------

const fn react_kind(trigger: spec::Trigger) -> ReactKind {
    match trigger {
        spec::Trigger::LifeLoss => LIFE_LOSS,
        spec::Trigger::DragonPower => DRAGON_LIFE_LOSS,
        spec::Trigger::VictoryCheck => VICTORY_CHECK,
    }
}

/// A collect ability the player does during the collect step, once a round,
/// when they choose to.
const fn collect_step(effect: Effect) -> Ability {
    Ability {
        trigger: Trigger::React(COLLECT_STEP),
        condition: Condition::Always,
        cost: None,
        effect,
        limit: Limit::OncePerRound,
    }
}

/// Taking everything that lies on a component, as a step of the collect
/// window. The rules give it to every component.
pub const fn take_from_card() -> Ability {
    Ability {
        trigger:   Trigger::React(COLLECT_STEP),
        condition: Condition::SourceHasResources,
        cost:      None,
        effect:    Effect::TakeAll {
            from: Target::Source,
        },
        limit:     Limit::OncePerRound,
    }
}

/// An ability that resolves by itself when the collect phase starts.
const fn collect(condition: Condition, effect: Effect) -> Ability {
    Ability {
        trigger: Trigger::OnEvent(EventKind::PhaseStarted),
        condition,
        cost: None,
        effect,
        limit: Limit::None,
    }
}

fn with_trigger(trigger: Trigger, cost: &[CostPart], effect: Effect) -> Res<Ability> {
    Ok(Ability {
        trigger,
        condition: Condition::Always,
        cost: power_cost(cost)?,
        effect,
        limit: Limit::None,
    })
}

/// What a card's abilities compile to.
#[derive(Default)]
struct Parts {
    abilities: Vec<Ability>,
    discounts: Vec<Discount>,
    on_gain:   Option<Effect>,
}

fn ability(a: &spec::Ability, dragon: bool, parts: &mut Parts) -> Res<()> {
    let in_collect = Condition::InPhase(COLLECT);
    match a {
        spec::Ability::Collect(options) => {
            parts.abilities.push(collect_step(gain_options(Target::Controller, options)?));
        },
        spec::Ability::CollectIfLeft { essence, gain } => parts.abilities.push(collect(
            Condition::All(vec![in_collect, Condition::SourceHas(resource(*essence))]),
            gain_options(Target::Controller, gain)?,
        )),
        spec::Ability::CollectGrow { per_kind } => parts.abilities.push(collect(
            Condition::All(vec![in_collect, Condition::SourceHasResources]),
            Effect::Grow {
                per_kind: *per_kind,
            },
        )),
        spec::Ability::CollectCurse { pay } => {
            let (res, n) = fixed_essence(pay, "a curse that is not a fixed essence")?;
            // Paying is a choice against turning the card; the engine only
            // offers `Pay` when it is affordable. The player may do their
            // other collect abilities first to have what the curse asks for.
            parts.abilities.push(collect_step(Effect::Choose(vec![
                Effect::Pay { res, n },
                Effect::Exhaust(Target::Source),
            ])));
        },
        spec::Ability::OnClaim(options) => {
            if parts.on_gain.is_some() {
                return invalid("two onclaim effects");
            }
            parts.on_gain = Some(gain_options(Target::Controller, options)?);
        },
        spec::Ability::Discount { on, amount } => {
            let Selector::Any { exclude } = &amount.what else {
                return Err(Reason::Unsupported("a discount that is not any-essence"));
            };
            let Count::Fixed(n) = amount.count else {
                return Err(Reason::Unsupported("a variable discount"));
            };
            parts.discounts.push(Discount {
                tag: Some(
                    match on {
                        DiscountOn::Artifacts => tag::ARTIFACT,
                        DiscountOn::Dragons => tag::DRAGON,
                    }
                    .into(),
                ),
                n,
                exclude: kinds(exclude),
            });
        },
        spec::Ability::Power { cost, effect } => {
            let effect = effect_of(effect, dragon)?;
            parts.abilities.push(with_trigger(Trigger::Activated, cost, effect)?);
        },
        spec::Ability::React {
            trigger,
            cost,
            effect,
        } => {
            let effect = effect_of(effect, dragon)?;
            parts.abilities.push(with_trigger(
                Trigger::React(react_kind(*trigger)),
                cost,
                effect,
            )?);
        },
    }
    Ok(())
}

fn build(spec: &CardSpec, def_id: DefId) -> Res<CardDef> {
    let mut tags = vec![kind_tag(spec.kind).to_string()];
    tags.extend(spec.types.iter().map(|t| type_tag(*t).to_string()));
    let cost = spec.cost.as_deref().map(placement_cost).transpose()?;
    let dragon = spec.is(CardType::Dragon);
    let mut parts = Parts::default();
    for a in &spec.abilities {
        ability(a, dragon, &mut parts)?;
    }
    Ok(CardDef {
        id: def_id,
        name: spec.name.clone(),
        tags,
        cost,
        on_gain: parts.on_gain,
        abilities: parts.abilities,
        discounts: parts.discounts,
    })
}

/// Compiles one card. `def_id` is the id the definition gets.
pub fn compile(spec: &CardSpec, def_id: DefId) -> Result<CardDef, CompileError> {
    build(spec, def_id).map_err(|reason| CompileError {
        card: spec.name.clone(),
        reason,
    })
}

/// The result of compiling a list of cards.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Compiled {
    /// `(printed name, English name)` of every card; the index is the card's
    /// [`DefId`], including cards that failed.
    pub names:    Vec<(String, Option<String>)>,
    /// The definitions that compiled, in file order.
    pub defs:     Vec<CardDef>,
    /// The cards that did not compile, in file order.
    pub failures: Vec<CompileError>,
}

impl Compiled {
    /// The id of a card by its printed or English name. Stable: the position of
    /// the card in the file.
    pub fn id_of(&self, name: &str) -> Option<DefId> {
        self.names
            .iter()
            .position(|(german, english)| german == name || english.as_deref() == Some(name))
            .and_then(|i| u32::try_from(i).ok())
            .map(DefId)
    }

    /// A compiled definition by printed or English name.
    pub fn by_name(&self, name: &str) -> Option<&CardDef> {
        let id = self.id_of(name)?;
        self.defs.iter().find(|d| d.id == id)
    }

    /// The compile error of a card by printed or English name, if it failed.
    pub fn failure(&self, name: &str) -> Option<&CompileError> {
        let (german, _) = &self.names[usize::try_from(self.id_of(name)?.0).ok()?];
        self.failures.iter().find(|e| &e.card == german)
    }
}

/// Compiles every card. Ids follow the order of `specs`; cards that cannot be
/// compiled are reported in [`Compiled::failures`] and the rest still compile.
pub fn compile_all(specs: &[CardSpec]) -> Compiled {
    let mut out = Compiled {
        names:    Vec::with_capacity(specs.len()),
        defs:     Vec::with_capacity(specs.len()),
        failures: Vec::new(),
    };
    for (index, spec) in specs.iter().enumerate() {
        let id = DefId(u32::try_from(index).expect("fewer than 2^32 cards"));
        out.names.push((spec.name.clone(), spec.english.clone()));
        match compile(spec, id) {
            Ok(def) => out.defs.push(def),
            Err(e) => out.failures.push(e),
        }
    }
    out
}
