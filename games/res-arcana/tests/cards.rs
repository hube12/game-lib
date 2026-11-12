//! Checks the embedded card file against what the rulebook states.

use std::collections::HashSet;

use gamelib_res_arcana::{
    CardKind,
    CardSpec,
    Essence,
    load,
    spec::{
        Ability,
        Amount,
        CardType,
        CostPart,
        Count,
        DiscountOn,
        EffectPart,
        Options,
        Price,
        RivalReact,
        Selector,
        Straighten,
        Trigger,
        Vp,
        Which,
        Zone,
    },
};

fn cards() -> Vec<CardSpec> {
    load().expect("the card file parses")
}

fn card(name: &str) -> CardSpec {
    cards().into_iter().find(|c| c.name == name).unwrap_or_else(|| panic!("no card {name}"))
}

fn of(kind: CardKind) -> Vec<CardSpec> {
    cards().into_iter().filter(|c| c.kind == kind).collect()
}

fn powers(c: &CardSpec) -> Vec<(&Vec<CostPart>, &Vec<EffectPart>)> {
    c.abilities
        .iter()
        .filter_map(|a| match a {
            Ability::Power { cost, effect } => Some((cost, effect)),
            _ => None,
        })
        .collect()
}

fn essence(n: u32, e: Essence) -> Amount {
    Amount::of(n, e)
}

fn any(n: u32, exclude: &[Essence]) -> Amount {
    Amount {
        count:     Count::Fixed(n),
        what:      Selector::Any {
            exclude: exclude.to_vec(),
        },
        identical: false,
    }
}

// ---- the rulebook's component list (page 2)
// ---------------------------------------------------------------

#[test]
fn component_counts_match_the_rulebook() {
    assert_eq!(of(CardKind::Artifact).len(), 40);
    assert_eq!(of(CardKind::Mage).len(), 10);
    assert_eq!(of(CardKind::Monument).len(), 10);
    assert_eq!(of(CardKind::Place).len(), 5);
    assert_eq!(of(CardKind::Item).len(), 8);
}

#[test]
fn names_are_unique() {
    let all = cards();
    let names: HashSet<_> = all.iter().map(|c| &c.name).collect();
    assert_eq!(names.len(), all.len());
}

#[test]
fn every_card_other_than_pure_vp_cards_does_something() {
    for c in cards() {
        if c.abilities.is_empty() {
            assert!(c.vp.base > 0, "{} has neither abilities nor points", c.name);
        }
    }
}

// ---- costs
// ---------------------------------------------------------------------------------------------------

#[test]
fn all_monuments_cost_four_gold() {
    for m in of(CardKind::Monument) {
        assert_eq!(m.cost, Some(vec![essence(4, Essence::Gold)]), "{}", m.name);
        assert!(m.vp.base >= 1, "{} is worth points", m.name);
    }
}

#[test]
fn only_magical_shard_and_prism_cost_zero() {
    let zero: HashSet<_> = of(CardKind::Artifact)
        .into_iter()
        .filter(|a| a.cost == Some(vec![]))
        .map(|a| a.name)
        .collect();
    assert_eq!(
        zero,
        HashSet::from(["Magische Scherbe".to_string(), "Prisma".to_string()])
    );
}

#[test]
fn mages_and_items_have_no_placement_cost_but_artifacts_and_places_do() {
    for c in cards() {
        match c.kind {
            CardKind::Mage | CardKind::Item => assert_eq!(c.cost, None, "{}", c.name),
            _ => assert!(c.cost.is_some(), "{} needs a placement cost", c.name),
        }
    }
}

#[test]
fn spot_checked_placement_costs() {
    // Wind Dragon is drawn on rulebook page 6: 4 Calm and 4 of any essence.
    assert_eq!(
        card("Winddrache").cost,
        Some(vec![essence(4, Essence::Calm), any(4, &[])])
    );
    assert_eq!(
        card("Seeschlange").cost,
        Some(vec![essence(6, Essence::Calm), essence(3, Essence::Life)])
    );
    assert_eq!(
        card("Katakomben der Toten").cost,
        Some(vec![essence(9, Essence::Death)])
    );
    assert_eq!(card("Drachenzaum").cost.unwrap().len(), 4);
}

// ---- types and dragons
// -------------------------------------------------------------------------------------

#[test]
fn six_dragons_each_with_a_life_loss_power_and_a_rival_reaction() {
    let dragons: Vec<_> =
        of(CardKind::Artifact).into_iter().filter(|a| a.is(CardType::Dragon)).collect();
    assert_eq!(dragons.len(), 6);
    for d in &dragons {
        assert_eq!(d.vp.base, 1, "{}", d.name);
        let attacks = powers(d)
            .into_iter()
            .flat_map(|(_, effect)| effect.iter())
            .filter(|e| {
                matches!(
                    e,
                    EffectPart::RivalsLoseLife {
                        amount: 2,
                        unless: Some(_),
                    }
                )
            })
            .count();
        assert_eq!(
            attacks, 1,
            "{} should attack for 2 Life with a reaction",
            d.name
        );
    }
}

#[test]
fn the_sea_serpent_is_both_dragon_and_creature() {
    let s = card("Seeschlange");
    assert!(s.is(CardType::Dragon) && s.is(CardType::Creature));
    let creatures = of(CardKind::Artifact).into_iter().filter(|a| a.is(CardType::Creature)).count();
    assert_eq!(creatures, 7);
}

#[test]
fn rival_reactions_differ_per_dragon() {
    let reaction = |name: &str| {
        powers(&card(name))
            .into_iter()
            .flat_map(|(_, e)| e.to_vec())
            .find_map(|e| match e {
                EffectPart::RivalsLoseLife { unless, .. } => unless,
                _ => None,
            })
            .unwrap()
    };
    assert_eq!(reaction("Winddrache"), RivalReact::DiscardCard); // rulebook page 6/7
    assert_eq!(reaction("Seeschlange"), RivalReact::DestroyArtifact);
    assert_eq!(
        reaction("Feuerdrache"),
        RivalReact::Pay(essence(1, Essence::Calm))
    );
    assert_eq!(
        reaction("Wasserdrache"),
        RivalReact::Pay(essence(1, Essence::Elan))
    );
    assert_eq!(
        reaction("Erddrache"),
        RivalReact::Pay(essence(1, Essence::Gold))
    );
    assert_eq!(
        reaction("Knochendrache"),
        RivalReact::Pay(essence(1, Essence::Death))
    );
}

// ---- victory points
// ----------------------------------------------------------------------------------------

#[test]
fn places_of_power_score_for_essences_on_them() {
    let vp = |name: &str| card(name).vp;
    assert_eq!(
        vp("Katakomben der Toten"),
        Vp {
            base:        0,
            per_essence: Some((1, Essence::Death)),
        }
    ); // page 7
    assert_eq!(
        vp("Heiliger Hain"),
        Vp {
            base:        2,
            per_essence: Some((1, Essence::Life)),
        }
    );
    assert_eq!(
        vp("Verfluchte Schmiede"),
        Vp {
            base:        1,
            per_essence: Some((1, Essence::Gold)),
        }
    );
    assert_eq!(
        vp("Drachenhort"),
        Vp {
            base:        0,
            per_essence: Some((1, Essence::Gold)),
        }
    );
    assert_eq!(
        vp("Korallenschloss"),
        Vp {
            base:        3,
            per_essence: None,
        }
    );
}

#[test]
fn only_a_few_artifacts_are_worth_points() {
    let scoring: Vec<_> = of(CardKind::Artifact).into_iter().filter(|a| a.vp.base > 0).collect();
    assert_eq!(scoring.len(), 12);
    assert!(scoring.iter().all(|a| a.vp.base == 1));
}

#[test]
fn the_great_pyramid_is_worth_three_and_does_nothing_else() {
    let p = card("Große Pyramide");
    assert_eq!((p.vp.base, p.abilities.len()), (3, 0));
}

// ---- rulebook fine points (pages 5, 8, 10)
// -----------------------------------------------------------------

#[test]
fn vault_pays_interest_only_if_gold_is_left_on_it() {
    let vault = card("Tresor");
    let Ability::CollectIfLeft {
        essence: left,
        gain,
    } = &vault.abilities[0]
    else {
        panic!("Vault starts with its collect ability");
    };
    assert_eq!(*left, Essence::Gold);
    assert_eq!(gain, &Options(vec![vec![any(2, &[Essence::Gold])]]));
}

#[test]
fn windup_man_adds_two_of_each_color_left_on_it() {
    assert!(card("Aufziehmann").abilities.contains(&Ability::CollectGrow { per_kind: 2 }));
}

#[test]
fn cursed_forge_asks_for_a_death_or_turning_during_collect() {
    assert_eq!(
        card("Verfluchte Schmiede").abilities[0],
        Ability::CollectCurse {
            pay: essence(1, Essence::Death),
        }
    );
}

#[test]
fn sacrificial_dagger_needs_both_destroying_itself_and_discarding_a_card() {
    let dagger = card("Opferdolch");
    let (cost, _) = powers(&dagger)[1];
    assert!(cost.contains(&CostPart::Destroy(Which::This)));
    assert!(cost.contains(&CostPart::Discard(1)));
}

#[test]
fn corrupt_altar_may_destroy_itself_but_fiery_whip_may_not() {
    let altar = card("Verdorbener Altar");
    assert!(powers(&altar)[1].0.contains(&CostPart::Destroy(Which::Own)));
    let whip = card("Flammenpeitsche");
    assert!(powers(&whip)[1].0.contains(&CostPart::Destroy(Which::Other)));
}

#[test]
fn crypt_places_from_the_discard_pile_with_a_non_gold_discount() {
    let crypt = card("Krypta");
    let (_, effect) = powers(&crypt)[1];
    assert_eq!(
        effect[0],
        EffectPart::Place {
            only:  None,
            from:  Zone::Discard,
            price: Price::Discount(any(2, &[Essence::Gold])),
        }
    );
}

#[test]
fn straightening_rules() {
    let straightens = |name: &str| {
        let c = card(name);
        c.abilities
            .iter()
            .filter_map(|a| match a {
                Ability::Power { effect, .. } => Some(effect.clone()),
                _ => None,
            })
            .flatten()
            .find_map(|e| match e {
                EffectPart::Straighten(s) => Some(s),
                _ => None,
            })
    };
    assert_eq!(straightens("Druidin"), Some(Straighten::Creature));
    assert_eq!(straightens("Hexe"), Some(Straighten::Any));
    assert_eq!(straightens("Flammenkelch"), Some(Straighten::Any));
    assert_eq!(straightens("Wiederbelebung"), Some(Straighten::Any));
    assert_eq!(straightens("Wachhund"), Some(Straighten::This));
}

#[test]
fn guard_dog_can_straighten_itself_without_turning() {
    let dog = card("Wachhund");
    assert!(!powers(&dog)[0].0.contains(&CostPart::Turn));
}

#[test]
fn places_need_a_second_component_turned() {
    let grove = card("Heiliger Hain");
    assert!(powers(&grove)[1].0.contains(&CostPart::TurnOther(CardType::Creature)));
    let lair = card("Drachenhort");
    assert!(powers(&lair)[1].0.contains(&CostPart::TurnOther(CardType::Dragon)));
}

#[test]
fn mermaid_pays_one_of_calm_life_or_gold() {
    let mermaid = card("Meerjungfrau");
    let (cost, effect) = powers(&mermaid)[0];
    assert_eq!(
        cost[1],
        CostPart::PayOneOf(vec![
            essence(1, Essence::Calm),
            essence(1, Essence::Life),
            essence(1, Essence::Gold)
        ])
    );
    assert!(matches!(effect[0], EffectPart::PutPaid(_)));
}

#[test]
fn athanor_takes_six_elan_off_itself_to_convert_identical_essences_to_gold() {
    let athanor = card("Athanor");
    let (cost, effect) = powers(&athanor)[1];
    assert!(cost.contains(&CostPart::TakeFromCard(essence(6, Essence::Elan))));
    assert!(cost.iter().any(|c| matches!(
        c,
        CostPart::Pay(Amount {
            count: Count::Var,
            identical: true,
            ..
        })
    )));
    assert!(matches!(&effect[0], EffectPart::Gain { options, .. }
        if options.0[0][0] == Amount { count: Count::Var, what: Selector::Essence(Essence::Gold), identical: false }));
}

#[test]
fn discounts_are_cumulative_abilities() {
    let discount = |name: &str| {
        card(name).abilities.into_iter().find_map(|a| match a {
            Ability::Discount { on, amount } => Some((on, amount)),
            _ => None,
        })
    };
    assert_eq!(
        discount("Feuerwerker"),
        Some((DiscountOn::Artifacts, any(1, &[Essence::Gold])))
    );
    assert_eq!(
        discount("Drachenzaum"),
        Some((DiscountOn::Dragons, any(3, &[])))
    );
    assert_eq!(
        discount("Drachenhort"),
        Some((DiscountOn::Dragons, any(3, &[])))
    );
}

#[test]
fn victory_check_cards() {
    let statue = card("Goldene Statue");
    assert!(matches!(
        &statue.abilities[0],
        Ability::React { trigger: Trigger::VictoryCheck, effect, .. }
            if effect == &vec![EffectPart::Vp { points: 3, temporary: true }]
    ));
    let castle = card("Korallenschloss");
    assert!(powers(&castle)[0].1.contains(&EffectPart::VictoryCheck));
}

#[test]
fn hawk_oracle_and_seer_look_at_three_cards() {
    for name in ["Falke", "Orakel", "Seherin"] {
        let c = card(name);
        let peeks = powers(&c)
            .into_iter()
            .flat_map(|(_, e)| e.iter())
            .any(|e| matches!(e, EffectPart::Peek { count: 3, decks } if decks.len() == 2));
        assert!(peeks, "{name}");
    }
}

#[test]
fn life_loss_is_reactable_by_the_cards_that_say_so() {
    let reacts: HashSet<_> = cards()
        .into_iter()
        .filter(|c| {
            c.abilities.iter().any(|a| {
                matches!(
                    a,
                    Ability::React {
                        trigger: Trigger::LifeLoss,
                        ..
                    }
                )
            })
        })
        .map(|c| c.name)
        .collect();
    for name in [
        "Heiler",
        "Tempel",
        "Schutz",
        "Korallenschloss",
        "Wachhund",
        "Baum des Lebens",
        "Elementarquelle",
    ] {
        assert!(reacts.contains(name), "{name}");
    }
}

// ---- internal consistency
// ---------------------------------------------------------------------------------

fn all_amounts(c: &CardSpec) -> Vec<Amount> {
    let mut out: Vec<Amount> = c.cost.clone().unwrap_or_default();
    let options = |o: &Options, out: &mut Vec<Amount>| out.extend(o.0.iter().flatten().cloned());
    let cost_parts = |parts: &Vec<CostPart>, out: &mut Vec<Amount>| {
        for p in parts {
            match p {
                CostPart::Pay(a) | CostPart::TakeFromCard(a) => out.push(a.clone()),
                CostPart::PayOneOf(v) => out.extend(v.iter().cloned()),
                _ => {},
            }
        }
    };
    let effect_parts = |parts: &Vec<EffectPart>, out: &mut Vec<Amount>| {
        for p in parts {
            match p {
                EffectPart::Gain { options: o, .. } => out.extend(o.0.iter().flatten().cloned()),
                EffectPart::RivalsGain(a) => out.push(a.clone()),
                EffectPart::RivalsLoseLife {
                    unless: Some(RivalReact::Pay(a)),
                    ..
                } => out.push(a.clone()),
                EffectPart::Place {
                    price: Price::Discount(a),
                    ..
                } => out.push(a.clone()),
                _ => {},
            }
        }
    };
    for a in &c.abilities {
        match a {
            Ability::Collect(o) | Ability::OnClaim(o) => options(o, &mut out),
            Ability::CollectIfLeft { gain, .. } => options(gain, &mut out),
            Ability::CollectCurse { pay } | Ability::Discount { amount: pay, .. } => {
                out.push(pay.clone())
            },
            Ability::CollectGrow { .. } => {},
            Ability::Power { cost, effect } | Ability::React { cost, effect, .. } => {
                cost_parts(cost, &mut out);
                effect_parts(effect, &mut out);
            },
        }
    }
    out
}

#[test]
fn every_amount_survives_a_round_trip_through_the_symbol_notation() {
    let mut seen = 0;
    for c in cards() {
        for a in all_amounts(&c) {
            let text = a.to_string();
            let again =
                gamelib_res_arcana::parse::parse(&format!("mage \"x\"\n  collect {text}\n"))
                    .unwrap();
            let Ability::Collect(Options(groups)) = &again[0].abilities[0] else {
                panic!("collect expected");
            };
            assert_eq!(groups[0][0], a, "{} / {text}", c.name);
            seen += 1;
        }
    }
    assert!(seen > 100, "only {seen} amounts were checked");
}

#[test]
fn a_variable_number_in_an_effect_is_bound_by_the_cost() {
    for c in cards() {
        for (cost, effect) in powers(&c) {
            let uses_n_in_effect = effect.iter().any(|e| match e {
                EffectPart::Gain { options, .. } => {
                    options.0.iter().flatten().any(|a| a.count == Count::Var)
                },
                _ => false,
            });
            let binds_n_in_cost = cost.iter().any(|p| {
                matches!(
                    p,
                    CostPart::Pay(Amount {
                        count: Count::Var,
                        ..
                    })
                )
            });
            assert!(
                !uses_n_in_effect || binds_n_in_cost,
                "{}: N is not bound by the cost",
                c.name
            );
        }
    }
}

#[test]
fn essence_symbols_match_the_rulebook() {
    let table: Vec<_> = Essence::ALL.iter().map(|e| (e.symbol(), e.name(), e.color())).collect();
    assert_eq!(
        table,
        [
            ('E', "Elan", "red"),
            ('L', "Life", "green"),
            ('C', "Calm", "blue"),
            ('D', "Death", "black"),
            ('G', "Gold", "yellow"),
        ]
    );
    for e in Essence::ALL {
        assert_eq!(Essence::from_symbol(e.symbol()), Some(e));
    }
    assert_eq!(Essence::from_symbol('X'), None);
}

#[test]
fn english_names_are_only_given_where_the_rulebook_names_the_card() {
    let named: HashSet<_> = cards().into_iter().filter_map(|c| c.english).collect();
    for expected in [
        "Artificer",
        "Transmuter",
        "Vault",
        "Windup Man",
        "Corrupt Altar",
        "Crypt",
        "Guard Dog",
    ] {
        assert!(named.contains(expected), "{expected}");
    }
}
