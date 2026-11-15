//! The planner's difficulty dial: a lower setting plays worse, and games with
//! a weaker bot still replay identically.

use gamelib::engine::Engine;
use gamelib_cli::{
    bot::Bot,
    runner::{RunError, run},
};
use gamelib_res_arcana::{
    planner::{Difficulty, Params, PlannerBot},
    rules::ResArcanaRules,
};

/// Plays one 2 player game with a planner of `levels[seat]` in each seat and
/// returns the winner (`None` for a tie or a game that did not finish).
fn game(seed: u64, levels: [u8; 2]) -> Option<usize> {
    let rules = ResArcanaRules::new();
    let mut engine = Engine::new(rules.clone(), 2, seed).expect("sets up");
    let mut bots: Vec<Box<dyn Bot>> = levels
        .iter()
        .zip(0_u64..)
        .map(|(level, i)| -> Box<dyn Bot> {
            Box::new(PlannerBot::with_difficulty(
                Params::default(),
                Difficulty::new(*level),
                seed.wrapping_add(i),
            ))
        })
        .collect();
    let result = run(&mut engine, &mut bots, 20_000);
    if let Ok(outcome) = &result {
        let again = Engine::replay(rules, 2, seed, engine.log()).expect("the log replays");
        assert_eq!(
            again.state(),
            engine.state(),
            "seed {seed} replays identically"
        );
        return (outcome.winners.len() == 1).then(|| usize::from(outcome.winners[0].0));
    }
    match result {
        Err(RunError::TooLong(_)) => None,
        other => panic!("seed {seed}: {other:?}"),
    }
}

#[test]
fn the_dial_stays_between_one_and_five() {
    assert_eq!(Difficulty::default(), Difficulty::MAX);
    assert_eq!(Difficulty::new(0), Difficulty::MIN);
    assert_eq!(Difficulty::new(9), Difficulty::MAX);
    assert_eq!(
        (1..=5).map(|l| Difficulty::new(l).name()).collect::<Vec<_>>(),
        ["Novice", "Easy", "Medium", "Hard", "Expert"]
    );
}

#[test]
fn the_full_strength_planner_beats_a_novice() {
    let (mut wins, mut games) = (0, 0);
    for seed in 0..12 {
        for expert_seat in [0, 1] {
            let mut levels = [1, 1];
            levels[expert_seat] = 5;
            if let Some(winner) = game(seed, levels) {
                games += 1;
                wins += usize::from(winner == expert_seat);
            }
        }
    }
    assert!(games >= 18, "most games finish ({games} did)");
    assert!(
        wins * 100 >= games * 65,
        "the expert won only {wins} of {games}"
    );
}
