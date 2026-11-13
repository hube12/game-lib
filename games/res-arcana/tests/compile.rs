//! Checks the compiled card definitions and plays some of them through the
//! engine.

use std::sync::LazyLock;

use gamelib::{
    card::CardDef,
    cost::{self, Cost},
    counter::Counter,
    effect::{Ability, Condition, Effect, Escape, Target, Trigger, Who},
    engine::{Action, Engine, GameRules},
    event::{Event, EventKind},
    flow::{EndCheck, Flow, GameEndPolicy, Phase, RoundEndPolicy, TurnOrder, TurnOrderPolicy},
    id::{DefId, EntityId, PlayerId, ResourceId, ZoneKind},
    rng::Rng,
    state::{CardRules, GameState},
    zone::{Order, Owner, Visibility},
};
use gamelib_res_arcana::{
    CardSpec,
    Compiled,
    Essence,
    compile::{
        self,
        ACTIONS,
        CALM,
        COLLECT,
        COLLECT_STEP,
        DEATH,
        DECK,
        DISCARD,
        DRAGON_LIFE_LOSS,
        ELAN,
        GOLD,
        HAND,
        LIFE,
        LIFE_LOSS,
        MONUMENT_DECK,
        TABLEAU,
        TEMP_VP,
        VICTORY_CHECK,
        VP,
        resource,
        tag,
    },
    compile_all,
    load,
    spec::{self, CardType, Count, Options, RivalReact, Selector},
};

const P0: PlayerId = PlayerId(0);
const P1: PlayerId = PlayerId(1);

fn specs() -> Vec<CardSpec> {
    static SPECS: LazyLock<Vec<CardSpec>> = LazyLock::new(|| load().expect("the card file parses"));
    SPECS.clone()
}

fn compiled() -> Compiled {
    static COMPILED: LazyLock<Compiled> = LazyLock::new(|| compile_all(&specs()));
    COMPILED.clone()
}

fn def(name: &str) -> CardDef {
    compiled().by_name(name).unwrap_or_else(|| panic!("{name} did not compile")).clone()
}

fn spec_of(name: &str) -> CardSpec {
    specs().into_iter().find(|s| s.name == name).unwrap_or_else(|| panic!("no card {name}"))
}

/// The cards the compiler cannot express yet (none: every base card compiles).
const UNSUPPORTED: [&str; 0] = [];

// ---------------------------------------------------------------------------------------------------
// Static checks
// ---------------------------------------------------------------------------------------------------

#[test]
fn every_card_compiles_except_the_known_unsupported_ones() {
    let specs = specs();
    let all = compile_all(&specs);
    let failed: Vec<&str> = all.failures.iter().map(|e| e.card.as_str()).collect();
    assert_eq!(failed, UNSUPPORTED);
    assert_eq!(all.defs.len() + all.failures.len(), specs.len());
    assert_eq!(all.defs.len(), specs.len() - UNSUPPORTED.len());
    for e in &all.failures {
        assert!(!e.to_string().is_empty());
    }
}

#[test]
fn def_ids_follow_the_file_order_and_names_are_the_printed_names() {
    let specs = specs();
    let all = compile_all(&specs);
    for (i, spec) in specs.iter().enumerate() {
        let id = DefId(u32::try_from(i).unwrap());
        assert_eq!(all.id_of(&spec.name), Some(id));
        if let Some(english) = &spec.english {
            assert_eq!(all.id_of(english), Some(id));
        }
        if let Some(def) = all.by_name(&spec.name) {
            assert_eq!(def.id, id);
            assert_eq!(def.name, spec.name);
        }
    }
    assert_eq!(all.id_of("no such card"), None);
    assert!(
        all.failure("Krypta").is_none(),
        "Crypt places from the discard pile"
    );
    assert!(all.failure("Heiler").is_none());
}

#[test]
fn tags_carry_the_kind_and_the_type() {
    for spec in specs() {
        let Some(def) = compiled().by_name(&spec.name).cloned() else {
            continue;
        };
        assert_eq!(def.tags[0], compile::kind_tag(spec.kind));
        for ty in [CardType::Dragon, CardType::Creature] {
            assert_eq!(
                def.tags.iter().any(|t| t == compile::type_tag(ty)),
                spec.is(ty),
                "{}",
                spec.name
            );
        }
    }
    let serpent = def("Seeschlange");
    assert_eq!(
        serpent.tags,
        [tag::ARTIFACT, tag::DRAGON, tag::CREATURE].map(String::from)
    );
    assert_eq!(def("Heiler").tags, [tag::MAGE.to_string()]);
    assert_eq!(def("Orakel").tags, [tag::MONUMENT.to_string()]);
    assert_eq!(def("Verfluchte Schmiede").tags, [tag::PLACE.to_string()]);
    assert_eq!(def("Forschung").tags, [tag::ITEM.to_string()]);
}

#[test]
fn resources_map_every_essence_to_its_own_id() {
    let ids: Vec<ResourceId> = Essence::ALL.iter().map(|e| resource(*e)).collect();
    assert_eq!(ids, [ELAN, LIFE, CALM, DEATH, GOLD]);
    for e in Essence::ALL {
        assert_eq!(compile::essence(resource(e)), Some(e));
    }
    assert_eq!(compile::essence(ResourceId(9)), None);
}

#[test]
fn placement_costs_follow_the_specs() {
    // Mages and magic items have no placement cost.
    assert_eq!(def("Heiler").cost, None);
    assert_eq!(def("Forschung").cost, None);
    // Monuments cost the same fixed amount of Gold.
    for spec in specs().iter().filter(|s| s.kind == spec::CardKind::Monument) {
        let def = def(&spec.name);
        assert_eq!(
            def.cost,
            Some(Cost::Resources(vec![(GOLD, 4)])),
            "{}",
            spec.name
        );
    }
    // A free artifact pays nothing.
    assert_eq!(def("Prisma").cost, Some(Cost::Resources(vec![])));
    // Specific essences are merged into one part, in printed order.
    assert_eq!(
        def("Knochendrache").cost,
        Some(Cost::Resources(vec![(DEATH, 4), (LIFE, 1)]))
    );
    // Any-essence parts stay separate.
    assert_eq!(
        def("Baum des Lebens").cost,
        Some(Cost::All(vec![
            Cost::Resources(vec![(LIFE, 1)]),
            Cost::Any {
                n:         cost::Count::Fixed(2),
                exclude:   vec![],
                identical: false,
            },
        ]))
    );
}

/// The total number of essences in the printed placement cost.
fn cost_size(spec: &CardSpec) -> u32 {
    spec.cost
        .iter()
        .flatten()
        .map(|a| match a.count {
            Count::Fixed(n) => n,
            Count::Var => 0,
        })
        .sum()
}

#[test]
fn compiled_placement_costs_have_the_printed_size() {
    for spec in specs() {
        let Some(def) = compiled().by_name(&spec.name).cloned() else {
            continue;
        };
        let size = def.cost.as_ref().map(units);
        assert_eq!(
            size,
            spec.cost.as_ref().map(|_| cost_size(&spec)),
            "{}",
            spec.name
        );
    }
}

fn units(cost: &Cost) -> u32 {
    match cost {
        Cost::Resources(list) => list.iter().map(|(_, n)| n).sum(),
        Cost::Any {
            n: cost::Count::Fixed(n),
            ..
        } => *n,
        Cost::All(parts) => parts.iter().map(units).sum(),
        other => panic!("unexpected placement cost {other:?}"),
    }
}

/// A collect ability: one the player does in the collect step, or one that
/// happens by itself afterwards (it depends on what was left on the card).
fn is_collect(a: &Ability) -> bool {
    a.trigger == Trigger::React(COLLECT_STEP)
        || a.trigger == Trigger::OnEvent(EventKind::PhaseStarted)
}

fn in_collect(c: &Condition) -> bool {
    match c {
        Condition::InPhase(phase) => *phase == COLLECT,
        Condition::All(all) => all.iter().any(in_collect),
        _ => false,
    }
}

#[test]
fn collect_abilities_hook_the_collect_phase() {
    let mut seen = 0;
    for spec in specs() {
        let Some(def) = compiled().by_name(&spec.name).cloned() else {
            continue;
        };
        let wanted = spec
            .abilities
            .iter()
            .filter(|a| {
                matches!(
                    a,
                    spec::Ability::Collect(_)
                        | spec::Ability::CollectIfLeft { .. }
                        | spec::Ability::CollectGrow { .. }
                        | spec::Ability::CollectCurse { .. }
                )
            })
            .count();
        let collects: Vec<_> = def.abilities.iter().filter(|a| is_collect(a)).collect();
        assert_eq!(collects.len(), wanted, "{}", spec.name);
        for a in collects {
            if a.trigger == Trigger::React(COLLECT_STEP) {
                // Done by the player, once a round, when they choose.
                assert_eq!(a.condition, Condition::Always, "{}", spec.name);
                assert_eq!(a.limit, gamelib::effect::Limit::OncePerRound);
            } else {
                assert!(in_collect(&a.condition), "{}", spec.name);
            }
            assert_eq!(a.cost, None);
        }
        seen += wanted;
    }
    assert!(seen > 20, "only {seen} collect abilities found");
    // Powers and reactions are not collect abilities.
    for a in &def("Zwergen-Spitzhacke").abilities {
        assert_eq!(a.trigger, Trigger::Activated);
        assert_eq!(a.condition, Condition::Always);
    }
}

#[test]
fn collect_options_become_a_choice() {
    let heiler = def("Heiler");
    let collect = heiler.abilities.iter().find(|a| is_collect(a)).unwrap();
    let Effect::Choose(options) = &collect.effect else {
        panic!("C/L is a choice, got {:?}", collect.effect);
    };
    assert_eq!(options.len(), 2);
    assert!(options.iter().all(|o| matches!(
        o,
        Effect::Gain {
            to: Target::Controller,
            ..
        }
    )));
    // Several essences together are a sequence, not a choice.
    let source = def("Elementarquelle");
    let collect = source.abilities.iter().find(|a| is_collect(a)).unwrap();
    assert!(matches!(&collect.effect, Effect::Sequence(s) if s.len() == 3));
}

#[test]
fn special_collect_abilities_have_their_conditions() {
    let vault = def("Tresor");
    let collect = vault.abilities.iter().find(|a| is_collect(a)).unwrap();
    assert!(matches!(
        &collect.condition,
        Condition::All(c) if c.contains(&Condition::SourceHas(GOLD))
    ));
    let windup = def("Aufziehmann");
    let collect = windup.abilities.iter().find(|a| is_collect(a)).unwrap();
    assert!(matches!(
        &collect.condition,
        Condition::All(c) if c.contains(&Condition::SourceHasResources)
    ));
    assert!(matches!(collect.effect, Effect::Grow { .. }));
}

fn react_kinds(def: &CardDef) -> Vec<gamelib::id::ReactKind> {
    def.abilities
        .iter()
        .filter_map(|a| match a.trigger {
            Trigger::React(kind) if kind != COLLECT_STEP => Some(kind),
            _ => None,
        })
        .collect()
}

#[test]
fn reactions_map_to_their_kind() {
    assert_eq!(react_kinds(&def("Heiler")), [LIFE_LOSS]);
    assert_eq!(react_kinds(&def("Drachenzaum")), [DRAGON_LIFE_LOSS]);
    assert_eq!(react_kinds(&def("Goldene Statue")), [VICTORY_CHECK]);
    // Every reaction in the data maps by its trigger.
    let mut count = 0;
    for spec in specs() {
        let Some(def) = compiled().by_name(&spec.name).cloned() else {
            continue;
        };
        let expected: Vec<_> = spec
            .abilities
            .iter()
            .filter_map(|a| match a {
                spec::Ability::React { trigger, .. } => Some(match trigger {
                    spec::Trigger::LifeLoss => LIFE_LOSS,
                    spec::Trigger::DragonPower => DRAGON_LIFE_LOSS,
                    spec::Trigger::VictoryCheck => VICTORY_CHECK,
                }),
                _ => None,
            })
            .collect();
        assert_eq!(react_kinds(&def), expected, "{}", spec.name);
        count += expected.len();
    }
    assert!(count >= 10);
    // A reaction costs something real: turning the Healer.
    let heiler = def("Heiler");
    let react = heiler.abilities.iter().find(|a| a.trigger == Trigger::React(LIFE_LOSS)).unwrap();
    assert_eq!(react.cost, Some(Cost::Exhaust));
    assert_eq!(react.effect, Effect::Ignore);
}

fn life_loss(def: &CardDef) -> &gamelib::effect::Defense {
    def.abilities
        .iter()
        .find_map(|a| match &a.effect {
            Effect::Affect {
                who: Who::Rivals,
                defense,
                effect,
            } if matches!(**effect, Effect::Lose { res: LIFE, .. }) => Some(defense),
            _ => None,
        })
        .unwrap_or_else(|| panic!("{} has no life loss power", def.name))
}

#[test]
fn dragons_defend_with_both_life_loss_kinds() {
    let mut dragons = 0;
    for spec in specs().iter().filter(|s| s.is(CardType::Dragon)) {
        let def = def(&spec.name);
        let defense = life_loss(&def);
        assert_eq!(
            defense.react,
            [LIFE_LOSS, DRAGON_LIFE_LOSS],
            "{}",
            spec.name
        );
        assert!(defense.skip_passed);
        // The escape printed on the card.
        let printed = spec.abilities.iter().find_map(|a| match a {
            spec::Ability::Power { effect, .. } => effect.iter().find_map(|p| match p {
                spec::EffectPart::RivalsLoseLife { unless, .. } => Some(unless.clone()),
                _ => None,
            }),
            _ => None,
        });
        let expected = match printed.unwrap() {
            None => None,
            Some(RivalReact::Pay(a)) => match (a.what, a.count) {
                (Selector::Essence(e), Count::Fixed(n)) => Some(Escape::Pay(resource(e), n)),
                other => panic!("unexpected {other:?}"),
            },
            Some(RivalReact::DiscardCard) => Some(Escape::DiscardCard),
            Some(RivalReact::DestroyArtifact) => Some(Escape::Destroy(tag::ARTIFACT.into())),
        };
        assert_eq!(defense.escape, expected, "{}", spec.name);
        dragons += 1;
    }
    assert_eq!(dragons, 6);
}

#[test]
fn other_life_loss_only_offers_ordinary_life_loss_reactions() {
    let bow = def("Elfenbogen");
    let defense = life_loss(&bow);
    assert_eq!(defense.react, [LIFE_LOSS]);
    assert_eq!(defense.escape, None);
}

#[test]
fn life_loss_loses_two_other_essences_for_every_missing_life() {
    let bow = def("Elfenbogen");
    let Effect::Affect { effect, .. } = &bow.abilities[0].effect else {
        panic!("not an attack");
    };
    assert_eq!(
        **effect,
        Effect::Lose {
            res:       LIFE,
            n:         1,
            shortfall: compile::LIFE_SHORTFALL,
        }
    );
}

#[test]
fn discounts_and_on_claim_effects_are_not_abilities() {
    for spec in specs() {
        let Some(def) = compiled().by_name(&spec.name).cloned() else {
            continue;
        };
        let discounts: Vec<_> = spec
            .abilities
            .iter()
            .filter_map(|a| match a {
                spec::Ability::Discount { on, amount } => {
                    let Selector::Any { exclude } = &amount.what else {
                        panic!("a discount is any-essence");
                    };
                    let Count::Fixed(n) = amount.count else {
                        panic!("a discount is fixed");
                    };
                    Some(gamelib::card::Discount {
                        tag: Some(
                            match on {
                                spec::DiscountOn::Artifacts => tag::ARTIFACT,
                                spec::DiscountOn::Dragons => tag::DRAGON,
                            }
                            .into(),
                        ),
                        n,
                        exclude: exclude.iter().map(|e| resource(*e)).collect(),
                    })
                },
                _ => None,
            })
            .collect();
        assert_eq!(def.discounts, discounts, "{}", spec.name);
        let claims = spec.abilities.iter().any(|a| matches!(a, spec::Ability::OnClaim(_)));
        assert_eq!(def.on_gain.is_some(), claims, "{}", spec.name);
        let non_abilities = spec
            .abilities
            .iter()
            .filter(|a| {
                matches!(
                    a,
                    spec::Ability::Discount { .. } | spec::Ability::OnClaim(_)
                )
            })
            .count();
        assert_eq!(
            def.abilities.len() + non_abilities,
            spec.abilities.len(),
            "{}",
            spec.name
        );
    }
    // Dragon discounts only reduce Dragons.
    let lair = def("Drachenhort");
    assert_eq!(lair.discounts.len(), 1);
    assert_eq!(lair.discounts[0].tag.as_deref(), Some(tag::DRAGON));
}

#[test]
fn power_costs_are_compiled_part_by_part() {
    // Turn plus an essence.
    let pickaxe = def("Zwergen-Spitzhacke");
    assert_eq!(
        pickaxe.abilities[0].cost,
        Some(Cost::All(vec![
            Cost::Exhaust,
            Cost::Resources(vec![(ELAN, 1)])
        ]))
    );
    // Turning alone is a plain exhaust.
    assert_eq!(def("Elfenbogen").abilities[0].cost, Some(Cost::Exhaust));
    // Pay one of several essences.
    let mermaid = def("Meerjungfrau");
    let power = mermaid.abilities.iter().find(|a| a.trigger == Trigger::Activated).unwrap();
    let Some(Cost::All(parts)) = &power.cost else {
        panic!("turn plus pay-one-of");
    };
    assert_eq!(
        parts[1],
        Cost::Or(vec![
            Cost::Resources(vec![(CALM, 1)]),
            Cost::Resources(vec![(LIFE, 1)]),
            Cost::Resources(vec![(GOLD, 1)]),
        ])
    );
    // Destroy this card.
    let statuette = def("Juwelenstatuette");
    assert_eq!(
        statuette.abilities[1].cost,
        Some(Cost::Destroy {
            which: cost::Which::This,
            tag:   tag::ARTIFACT.into(),
        })
    );
    // Take essences off the card, and a variable number of identical ones.
    let athanor = def("Athanor");
    assert_eq!(
        athanor.abilities[1].cost,
        Some(Cost::All(vec![
            Cost::Exhaust,
            Cost::TakeFromCard { res: ELAN, n: 6 },
            Cost::Any {
                n:         cost::Count::Var,
                exclude:   vec![],
                identical: true,
            },
        ]))
    );
    assert!(matches!(
        athanor.abilities[1].effect,
        Effect::GainX { res: GOLD, .. }
    ));
    // Turn another creature / dragon.
    let grove = def("Heiliger Hain");
    let Some(Cost::All(parts)) = &grove.abilities[1].cost else {
        panic!("turn plus turn other");
    };
    assert_eq!(
        parts[1],
        Cost::TurnOther {
            tag: tag::CREATURE.into(),
        }
    );
}

#[test]
fn victory_check_is_a_composition_of_generic_effects() {
    let check = compile::victory_check(TEMP_VP);
    let Effect::Sequence(steps) = &check else {
        panic!("a sequence");
    };
    assert_eq!(steps.len(), 3);
    assert!(matches!(
        &steps[0],
        Effect::ForEachPlayer(inner)
            if matches!(**inner, Effect::ReactWindow { kind: VICTORY_CHECK, repeat: true, .. })
    ));
    assert_eq!(steps[1], Effect::CheckWin);
    assert_eq!(steps[2], Effect::ResetCounters { counter: TEMP_VP });
    // The Coral Castle's power is exactly that.
    let castle = def("Korallenschloss");
    assert_eq!(castle.abilities[0].effect, check);
}

#[test]
fn spot_checks_of_effects() {
    // Peek at the artifact deck or the monument deck.
    let seer = def("Seherin");
    let power = seer.abilities.iter().find(|a| a.trigger == Trigger::Activated).unwrap();
    let Effect::Choose(decks) = &power.effect else {
        panic!("a choice of decks");
    };
    assert_eq!(decks.len(), 2);
    assert!(
        decks.iter().any(|d| matches!(d, Effect::Reorder { zone, .. } if *zone == MONUMENT_DECK))
    );
    assert!(decks.iter().any(|d| matches!(d, Effect::Reorder { zone, .. } if *zone == DECK)));
    // Putting the essence on the card itself.
    let duellist = def("Duellant");
    let power = duellist.abilities.iter().find(|a| a.trigger == Trigger::Activated).unwrap();
    assert_eq!(
        power.effect,
        Effect::Gain {
            to:  Target::Source,
            res: GOLD,
            n:   1,
        }
    );
    // Temporary points go to their own counter.
    let statue = def("Goldene Statue");
    assert!(matches!(
        statue.abilities[0].effect,
        Effect::ModifyCounter {
            counter,
            delta: 3,
            ..
        } if counter == TEMP_VP
    ));
}

#[test]
fn a_collect_power_is_not_a_choice_when_it_has_one_option() {
    let pit = def("Katakomben der Toten");
    assert!(matches!(
        pit.abilities.iter().find(|a| is_collect(a)).unwrap().effect,
        Effect::Gain {
            res: DEATH,
            n: 1,
            ..
        }
    ));
}

// ---------------------------------------------------------------------------------------------------
// A minimal table to play compiled cards
// ---------------------------------------------------------------------------------------------------

#[derive(Clone)]
struct Table {
    cards:    Compiled,
    phases:   Vec<Phase>,
    game_end: Option<GameEndPolicy>,
    extra:    fn(&mut GameState, &Compiled),
}

impl Table {
    fn new(phases: &[(gamelib::id::PhaseId, bool)], extra: fn(&mut GameState, &Compiled)) -> Self {
        Self {
            cards: compiled(),
            phases: phases
                .iter()
                .map(|(id, interactive)| Phase {
                    id:          *id,
                    interactive: *interactive,
                })
                .collect(),
            game_end: None,
            extra,
        }
    }
}

impl GameRules for Table {
    fn setup(&self, players: &[PlayerId], rng: Rng) -> GameState {
        let mut s = GameState::new(rng);
        for res in [ELAN, LIFE, CALM, DEATH, GOLD] {
            s.bank.set_infinite(res);
        }
        for def in &self.cards.defs {
            s.add_def(def.clone());
        }
        for &p in players {
            s.add_player(p);
            let owner = Owner::Player(p);
            let deck = s.add_zone(DECK, owner.clone(), Order::Stack, Visibility::Hidden);
            s.add_zone(HAND, owner.clone(), Order::Unordered, Visibility::Owner);
            s.add_zone(DISCARD, owner.clone(), Order::Stack, Visibility::Public);
            let tableau = s.add_zone(TABLEAU, owner.clone(), Order::Unordered, Visibility::Public);
            s.zone_mut(tableau).unwrap().abilities_active = true;
            s.zone_mut(deck).unwrap().recycle_from = Some(DISCARD);
            s.add_counter(Counter::new(VP, owner.clone(), 0, 0, 100));
            s.add_counter(Counter::new(TEMP_VP, owner, 0, 0, 100));
        }
        s.add_zone(
            MONUMENT_DECK,
            Owner::Shared,
            Order::Stack,
            Visibility::Hidden,
        );
        s.card_rules = Some(CardRules {
            hand:       HAND,
            tableau:    TABLEAU,
            discard:    DISCARD,
            on_discard: None,
        });
        s.flow = Flow {
            phases: self.phases.clone(),
            turn_order: TurnOrder::new(players.to_vec(), TurnOrderPolicy::Clockwise),
            round_end: RoundEndPolicy::AllPassed,
            game_end: self.game_end.clone(),
            ..Flow::default()
        };
        (self.extra)(&mut s, &self.cards);
        s
    }

    fn phase_effects(&self, _state: &GameState, phase: gamelib::id::PhaseId) -> Vec<Effect> {
        // The collect step of the real rules: every player does their collect
        // abilities in the order they like.
        if phase == COLLECT {
            vec![Effect::ForEachPlayer(Box::new(Effect::ReactWindow {
                kind:    COLLECT_STEP,
                default: Box::new(Effect::Ignore),
                repeat:  true,
            }))]
        } else {
            Vec::new()
        }
    }
}

/// Does every collect ability on offer (the first one each time) until a
/// collect step has nothing left but ending it, which it then does.
fn do_collects(e: &mut Engine<Table>) {
    while let Some(pending) = e.pending() {
        if pending.options.first() != Some(&Effect::Ignore) {
            return;
        }
        let who = pending.players[0];
        let choice = usize::from(pending.options.len() > 1);
        e.apply(who, Action::Decide(choice)).expect("a collect step option");
    }
}

/// Puts a new copy of the card in a player's zone.
fn put(
    s: &mut GameState,
    cards: &Compiled,
    name: &str,
    player: PlayerId,
    kind: ZoneKind,
) -> EntityId {
    let zone = s.zone_id(&Owner::Player(player), kind).unwrap();
    let id = cards.id_of(name).unwrap_or_else(|| panic!("no card {name}"));
    s.create_entity(id, zone, true).unwrap()
}

fn give(s: &mut GameState, player: PlayerId, res: ResourceId, n: u32) {
    s.gain_player(player, res, n).unwrap();
}

fn game(table: Table) -> Engine<Table> {
    Engine::new(table, 2, 7).expect("the table sets up")
}

fn actions_phase() -> [(gamelib::id::PhaseId, bool); 1] {
    [(ACTIONS, true)]
}

fn pool(e: &Engine<Table>, player: PlayerId, res: ResourceId) -> u32 {
    e.state().player_pool(player).unwrap().finite(res).unwrap_or(0)
}

fn entity_of(e: &Engine<Table>, name: &str, player: PlayerId) -> EntityId {
    let id = compiled().id_of(name).unwrap();
    e.state()
        .entities()
        .find(|x| x.def == id && e.state().zone(x.zone).unwrap().owner.contains(player))
        .unwrap_or_else(|| panic!("no {name} for {player:?}"))
        .id
}

/// Index of the first ability of the card matching the predicate.
fn ability_index(name: &str, wanted: impl Fn(&Ability) -> bool) -> usize {
    def(name)
        .abilities
        .iter()
        .position(wanted)
        .unwrap_or_else(|| panic!("{name}: no such ability"))
}

fn activate(e: &mut Engine<Table>, player: PlayerId, entity: EntityId, ability: usize) {
    let action = e
        .legal_actions(player)
        .into_iter()
        .find(|a| matches!(a, Action::Activate { entity: x, ability: i, .. } if *x == entity && *i == ability))
        .unwrap_or_else(|| panic!("ability {ability} is not legal"));
    e.apply(player, action).expect("the power resolves");
}

fn power(name: &str) -> usize {
    ability_index(name, |a| a.trigger == Trigger::Activated)
}

// ---------------------------------------------------------------------------------------------------
// Playing compiled cards
// ---------------------------------------------------------------------------------------------------

#[test]
fn a_collect_ability_gives_its_essences_when_the_collect_phase_starts() {
    let table = Table::new(&[(COLLECT, false), (ACTIONS, true)], |s, c| {
        put(s, c, "Elementarquelle", P0, TABLEAU);
        put(s, c, "Elementarquelle", P1, TABLEAU);
        put(s, c, "Wachhund", P1, TABLEAU);
    });
    let mut e = game(table);
    // Nothing is collected until the players do it.
    assert_eq!(pool(&e, P0, ELAN), 0);
    do_collects(&mut e);
    // The printed collect amounts, read from the spec.
    let spec = spec_of("Elementarquelle");
    let printed = spec
        .abilities
        .iter()
        .find_map(|a| match a {
            spec::Ability::Collect(Options(groups)) => Some(groups[0].clone()),
            _ => None,
        })
        .unwrap();
    assert!(!printed.is_empty());
    for player in [P0, P1] {
        for amount in &printed {
            let Selector::Essence(ess) = amount.what else {
                panic!("fixed essences");
            };
            let Count::Fixed(n) = amount.count else {
                panic!("fixed count");
            };
            assert_eq!(pool(&e, player, resource(ess)), n);
        }
    }
    assert_eq!(e.current_player(), Some(P0));
    assert_eq!(e.state().flow.phase_id(), Some(ACTIONS));
}

#[test]
fn a_collect_choice_asks_the_owner() {
    let table = Table::new(&[(COLLECT, false), (ACTIONS, true)], |s, c| {
        put(s, c, "Heiler", P0, TABLEAU);
    });
    let mut e = game(table);
    let step = e.pending().expect("the collect step").clone();
    assert_eq!(
        step.options.len(),
        2,
        "end the step, or the Healer's collect"
    );
    e.apply(P0, Action::Decide(1)).unwrap();
    let pending = e.pending().expect("C or L").clone();
    assert_eq!(pending.players, [P0]);
    assert_eq!(pending.options.len(), 2);
    e.apply(P0, Action::Decide(1)).unwrap();
    assert_eq!(pool(&e, P0, LIFE), 1);
    assert_eq!(pool(&e, P0, CALM), 0);
}

#[test]
fn collect_grow_adds_to_every_kind_on_the_card() {
    let Some(spec::Ability::CollectGrow { per_kind }) = spec_of("Aufziehmann")
        .abilities
        .into_iter()
        .find(|a| matches!(a, spec::Ability::CollectGrow { .. }))
    else {
        panic!("the Windup Man grows");
    };
    let table = Table::new(&[(COLLECT, false), (ACTIONS, true)], |s, c| {
        let man = put(s, c, "Aufziehmann", P0, TABLEAU);
        s.gain_entity(man, ELAN, 1).unwrap();
        // An empty one does not grow.
        put(s, c, "Aufziehmann", P1, TABLEAU);
    });
    let e = game(table);
    let man = entity_of(&e, "Aufziehmann", P0);
    let on_card = |entity| e.state().entity(entity).unwrap().pool.finite(ELAN).unwrap_or(0);
    assert_eq!(on_card(man), 1 + per_kind);
    assert_eq!(on_card(entity_of(&e, "Aufziehmann", P1)), 0);
}

#[test]
fn a_power_pays_its_cost_and_gains_its_effect() {
    let table = Table::new(&actions_phase(), |s, c| {
        put(s, c, "Zwergen-Spitzhacke", P0, TABLEAU);
        give(s, P0, ELAN, 3);
    });
    let mut e = game(table);
    let pickaxe = entity_of(&e, "Zwergen-Spitzhacke", P0);
    activate(&mut e, P0, pickaxe, power("Zwergen-Spitzhacke"));
    assert_eq!(pool(&e, P0, ELAN), 2);
    assert_eq!(pool(&e, P0, GOLD), 1);
    assert!(e.state().entity(pickaxe).unwrap().exhausted);
    // Turned: the power is not offered again.
    assert!(
        !e.legal_actions(P0)
            .iter()
            .any(|a| matches!(a, Action::Activate { entity, .. } if *entity == pickaxe))
    );
}

#[test]
fn a_power_needs_its_cost() {
    let table = Table::new(&actions_phase(), |s, c| {
        put(s, c, "Zwergen-Spitzhacke", P0, TABLEAU);
    });
    let e = game(table);
    assert!(!e.legal_actions(P0).iter().any(|a| matches!(a, Action::Activate { .. })));
}

#[test]
fn draw_and_variable_cost_powers_run() {
    let table = Table::new(&actions_phase(), |s, c| {
        put(s, c, "Gelehrter", P0, TABLEAU);
        put(s, c, "Nachtigall", P0, DECK);
        give(s, P0, DEATH, 1);
    });
    let mut e = game(table);
    let mage = entity_of(&e, "Gelehrter", P0);
    activate(&mut e, P0, mage, power("Gelehrter"));
    let hand = e.state().zone_id(&Owner::Player(P0), HAND).unwrap();
    assert_eq!(e.state().zone(hand).unwrap().len(), 1);
    assert_eq!(pool(&e, P0, DEATH), 0);
}

#[test]
fn straighten_creature_picks_a_turned_creature() {
    let table = Table::new(&actions_phase(), |s, c| {
        put(s, c, "Druidin", P0, TABLEAU);
        let dog = put(s, c, "Wachhund", P0, TABLEAU);
        s.entity_mut(dog).unwrap().exhausted = true;
        put(s, c, "Zwergen-Spitzhacke", P0, TABLEAU);
    });
    let mut e = game(table);
    let druid = entity_of(&e, "Druidin", P0);
    activate(&mut e, P0, druid, power("Druidin"));
    let dog = entity_of(&e, "Wachhund", P0);
    assert!(!e.state().entity(dog).unwrap().exhausted);
    assert!(e.state().entity(druid).unwrap().exhausted);
}

#[test]
fn a_dragon_places_a_dragon_from_hand_for_free() {
    let table = Table::new(&actions_phase(), |s, c| {
        put(s, c, "Drachenzahn", P0, TABLEAU);
        put(s, c, "Knochendrache", P0, HAND);
        put(s, c, "Zwergen-Spitzhacke", P0, HAND);
        give(s, P0, ELAN, 3);
    });
    let mut e = game(table);
    let tooth = entity_of(&e, "Drachenzahn", P0);
    let place = ability_index("Drachenzahn", |a| {
        matches!(a.effect, Effect::PlaceCard { .. })
    });
    activate(&mut e, P0, tooth, place);
    // Only the Dragon qualifies, so the choice is that Dragon or nothing.
    let pending = e.pending().expect("place or not").clone();
    assert_eq!(pending.options.len(), 2);
    e.apply(P0, Action::Decide(0)).unwrap();
    let dragon = entity_of(&e, "Knochendrache", P0);
    let tableau = e.state().zone_id(&Owner::Player(P0), TABLEAU).unwrap();
    assert!(e.state().zone(tableau).unwrap().entities().contains(&dragon));
    assert_eq!(pool(&e, P0, ELAN), 0);
}

fn attack_table(setup: fn(&mut GameState, &Compiled)) -> Table {
    Table::new(&actions_phase(), setup)
}

#[test]
fn a_rival_can_react_to_life_loss_with_a_react_card() {
    let table = attack_table(|s, c| {
        put(s, c, "Knochendrache", P0, TABLEAU);
        put(s, c, "Heiler", P1, TABLEAU);
        give(s, P1, LIFE, 3);
    });
    let mut e = game(table);
    let dragon = entity_of(&e, "Knochendrache", P0);
    activate(&mut e, P0, dragon, power("Knochendrache"));
    let pending = e.pending().expect("the rival may react").clone();
    assert_eq!(pending.players, [P1]);
    // Take the loss, or use the Healer.
    assert_eq!(pending.options.len(), 2);
    assert!(matches!(
        pending.options[0],
        Effect::Lose {
            res: LIFE,
            n: 2,
            ..
        }
    ));
    let healer = entity_of(&e, "Heiler", P1);
    assert!(matches!(pending.options[1], Effect::Use { source, .. } if source == healer));
    e.apply(P1, Action::Decide(1)).unwrap();
    assert_eq!(pool(&e, P1, LIFE), 3);
    assert!(e.state().entity(healer).unwrap().exhausted);
    assert!(e.pending().is_none());
}

#[test]
fn declining_the_reaction_loses_the_life() {
    let table = attack_table(|s, c| {
        put(s, c, "Knochendrache", P0, TABLEAU);
        put(s, c, "Heiler", P1, TABLEAU);
        give(s, P1, LIFE, 3);
    });
    let mut e = game(table);
    let dragon = entity_of(&e, "Knochendrache", P0);
    activate(&mut e, P0, dragon, power("Knochendrache"));
    e.apply(P1, Action::Decide(0)).unwrap();
    assert_eq!(pool(&e, P1, LIFE), 1);
}

#[test]
fn a_dragon_card_offers_the_escape_printed_on_it() {
    let table = attack_table(|s, c| {
        put(s, c, "Knochendrache", P0, TABLEAU);
        give(s, P1, LIFE, 3);
        give(s, P1, DEATH, 1);
    });
    let mut e = game(table);
    let dragon = entity_of(&e, "Knochendrache", P0);
    activate(&mut e, P0, dragon, power("Knochendrache"));
    let pending = e.pending().expect("lose or pay").clone();
    assert_eq!(pending.options.len(), 2);
    let bone = def("Knochendrache");
    let Some(Escape::Pay(res, n)) = life_loss(&bone).escape.clone() else {
        panic!("the Bone Dragon lets rivals pay");
    };
    assert_eq!(pending.options[1], Effect::Pay { res, n });
    e.apply(P1, Action::Decide(1)).unwrap();
    assert_eq!(pool(&e, P1, LIFE), 3);
    assert_eq!(pool(&e, P1, DEATH), 0);
}

#[test]
fn the_sea_serpent_lets_a_rival_destroy_one_of_their_artifacts() {
    let table = attack_table(|s, c| {
        put(s, c, "Seeschlange", P0, TABLEAU);
        put(s, c, "Nachtigall", P1, TABLEAU);
        give(s, P1, LIFE, 3);
    });
    let mut e = game(table);
    let serpent = entity_of(&e, "Seeschlange", P0);
    activate(&mut e, P0, serpent, power("Seeschlange"));
    let pending = e.pending().expect("lose or destroy").clone();
    assert_eq!(pending.options.len(), 2);
    e.apply(P1, Action::Decide(1)).unwrap();
    assert_eq!(pool(&e, P1, LIFE), 3);
    let nightingale = entity_of(&e, "Nachtigall", P1);
    let discard = e.state().zone_id(&Owner::Player(P1), DISCARD).unwrap();
    assert!(e.state().zone(discard).unwrap().entities().contains(&nightingale));
}

#[test]
fn the_dragon_bridle_only_answers_dragons() {
    let table = attack_table(|s, c| {
        put(s, c, "Knochendrache", P0, TABLEAU);
        put(s, c, "Drachenzaum", P1, TABLEAU);
        give(s, P1, LIFE, 3);
    });
    let mut e = game(table);
    let dragon = entity_of(&e, "Knochendrache", P0);
    activate(&mut e, P0, dragon, power("Knochendrache"));
    let bridle = entity_of(&e, "Drachenzaum", P1);
    let pending = e.pending().expect("the bridle answers a dragon").clone();
    assert!(
        pending
            .options
            .iter()
            .any(|o| matches!(o, Effect::Use { source, .. } if *source == bridle))
    );

    // The same card does nothing against an ordinary life loss.
    let table = attack_table(|s, c| {
        put(s, c, "Elfenbogen", P0, TABLEAU);
        put(s, c, "Drachenzaum", P1, TABLEAU);
        give(s, P1, LIFE, 3);
    });
    let mut e = game(table);
    let bow = entity_of(&e, "Elfenbogen", P0);
    activate(&mut e, P0, bow, power("Elfenbogen"));
    assert!(e.pending().is_none());
    assert_eq!(pool(&e, P1, LIFE), 2);
}

#[test]
fn a_rival_without_life_loses_two_other_essences_per_missing_life() {
    let table = attack_table(|s, c| {
        put(s, c, "Elfenbogen", P0, TABLEAU);
        give(s, P1, CALM, 3);
    });
    let mut e = game(table);
    let bow = entity_of(&e, "Elfenbogen", P0);
    activate(&mut e, P0, bow, power("Elfenbogen"));
    // One Life is missing, so two other essences go (Calm is all there is).
    assert!(e.pending().is_none());
    assert_eq!(pool(&e, P1, CALM), 1);
}

#[test]
fn passed_rivals_ignore_life_loss() {
    let table = attack_table(|s, c| {
        put(s, c, "Elfenbogen", P0, TABLEAU);
        give(s, P1, LIFE, 3);
    });
    let mut e = game(table);
    e.apply(P0, Action::EndTurn).unwrap();
    assert_eq!(e.current_player(), Some(P1));
    e.apply(P1, Action::Pass).unwrap();
    assert_eq!(e.current_player(), Some(P0));
    let bow = entity_of(&e, "Elfenbogen", P0);
    activate(&mut e, P0, bow, power("Elfenbogen"));
    assert_eq!(pool(&e, P1, LIFE), 3);
}

#[test]
fn rivals_gain_essences_even_after_passing() {
    let table = attack_table(|s, c| {
        put(s, c, "Hand des Ruhmes", P0, TABLEAU);
    });
    let mut e = game(table);
    e.apply(P0, Action::EndTurn).unwrap();
    e.apply(P1, Action::Pass).unwrap();
    let hand = entity_of(&e, "Hand des Ruhmes", P0);
    activate(&mut e, P0, hand, power("Hand des Ruhmes"));
    assert_eq!(pool(&e, P0, DEATH), 2);
    assert_eq!(pool(&e, P1, DEATH), 1);
}

fn win_table(extra: fn(&mut GameState, &Compiled)) -> Table {
    let mut table = Table::new(&actions_phase(), extra);
    table.game_end = Some(GameEndPolicy {
        counter:  VP,
        target:   10,
        when:     EndCheck::RoundEnd,
        tiebreak: vec![],
    });
    table
}

fn set_vp(s: &mut GameState, player: PlayerId, value: i64) {
    s.add_to_counter(VP, &Owner::Player(player), value).unwrap();
}

#[test]
fn a_power_can_trigger_a_victory_check() {
    let table = win_table(|s, c| {
        put(s, c, "Korallenschloss", P0, TABLEAU);
        set_vp(s, P0, 10);
    });
    let mut e = game(table);
    assert_eq!(e.winners(), None);
    let castle = entity_of(&e, "Korallenschloss", P0);
    activate(&mut e, P0, castle, power("Korallenschloss"));
    assert_eq!(e.winners(), Some(&[P0][..]));

    let table = win_table(|s, c| {
        put(s, c, "Korallenschloss", P0, TABLEAU);
        set_vp(s, P0, 9);
    });
    let mut e = game(table);
    let castle = entity_of(&e, "Korallenschloss", P0);
    activate(&mut e, P0, castle, power("Korallenschloss"));
    assert_eq!(e.winners(), None);
}

#[test]
fn the_golden_statue_adds_temporary_points_during_a_victory_check() {
    let table = win_table(|s, c| {
        put(s, c, "Korallenschloss", P0, TABLEAU);
        put(s, c, "Goldene Statue", P1, TABLEAU);
        give(s, P1, GOLD, 3);
    });
    let mut e = game(table);
    let castle = entity_of(&e, "Korallenschloss", P0);
    activate(&mut e, P0, castle, power("Korallenschloss"));
    let pending = e.pending().expect("the statue may react").clone();
    assert_eq!(pending.players, [P1]);
    assert_eq!(pending.options.len(), 2);
    e.apply(P1, Action::Decide(1)).unwrap();
    assert!(e.pending().is_none());
    assert_eq!(pool(&e, P1, GOLD), 0);
    let points = e.events().iter().any(|ev| {
        matches!(
            ev,
            Event::CounterChanged { counter, owner, delta: 3 }
                if *counter == TEMP_VP && *owner == Owner::Player(P1)
        )
    });
    assert!(points, "the temporary points were scored");
    // They are forgotten once the check is done.
    assert_eq!(
        e.state().counter(TEMP_VP, &Owner::Player(P1)).unwrap().value,
        0
    );
}

#[test]
fn claiming_a_monument_with_an_on_claim_effect_is_data_only() {
    // The Obelisk gives six any-essence (not Gold) when claimed.
    let obelisk = def("Obelisk");
    let Some(Effect::GainChoose {
        to: Target::Controller,
        n: cost::Count::Fixed(6),
        exclude,
        identical: false,
    }) = obelisk.on_gain
    else {
        panic!("a gain-choose on claim");
    };
    assert_eq!(exclude, [GOLD]);
}

#[test]
fn compiled_definitions_can_be_cloned_and_compared() {
    let a = compiled();
    let b = compiled();
    assert_eq!(a, b);
    assert!(a.by_name("Seeschlange").is_some());
    assert!(a.by_name("Sea Serpent").is_some());
    assert_eq!(a.by_name("Sea Serpent"), a.by_name("Seeschlange"));
}

#[test]
fn the_cards_that_need_card_cost_gains_or_the_discard_pile_compile_to_generic_effects() {
    use gamelib::{cost::Count as EngineCount, effect::Effect};
    // Crypt: place an artifact from the discard pile at a discount.
    let crypt = def("Krypta");
    assert!(crypt.abilities.iter().any(|a| matches!(
        &a.effect,
        Effect::PlaceCard { from: Some(zone), .. } if *zone == DISCARD
    )));
    // Sacrificial Dagger: its second power sizes the gain by the discarded
    // card, not the destroyed one.
    let dagger = def("Opferdolch");
    assert!(dagger.abilities.iter().any(|a| matches!(
        &a.effect,
        Effect::GainChoose {
            n: EngineCount::CostOf {
                discarded: true,
                bonus:     0,
            },
            ..
        }
    )));
    // Corrupt Altar and Fiery Whip: the destroyed card, plus 2.
    for name in ["Verdorbener Altar", "Flammenpeitsche"] {
        assert!(
            def(name).abilities.iter().any(|a| matches!(
                &a.effect,
                Effect::GainChoose {
                    n: EngineCount::CostOf {
                        discarded: false,
                        bonus:     2,
                    },
                    ..
                }
            )),
            "{name}"
        );
    }
}
