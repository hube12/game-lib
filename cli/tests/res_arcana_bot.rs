//! The Res Arcana heuristic bot against the random bot.

use gamelib::engine::Engine;
use gamelib_cli::{
    bot::{Bot, RandomBot},
    runner::{RunError, run},
};
use gamelib_res_arcana::{bot::HeuristicBot, rules::ResArcanaRules};

/// Plays one 2 player game; `heuristic_seat` gets the heuristic bot, the other
/// a random one.
fn game(seed: u64, heuristic_seat: usize) -> Option<usize> {
    let mut engine = Engine::new(ResArcanaRules::new(), 2, seed).expect("sets up");
    let mut bots: Vec<Box<dyn Bot>> = (0..2)
        .map(|i| -> Box<dyn Bot> {
            if i == heuristic_seat {
                Box::new(HeuristicBot::new())
            } else {
                Box::new(RandomBot::new(seed.wrapping_mul(7).wrapping_add(i as u64)))
            }
        })
        .collect();
    match run(&mut engine, &mut bots, 20_000) {
        Ok(outcome) => (outcome.winners.len() == 1).then(|| usize::from(outcome.winners[0].0)),
        Err(RunError::TooLong(_)) => None,
        Err(e) => panic!("seed {seed}: {e:?}"),
    }
}

#[test]
fn the_heuristic_bot_beats_the_random_bot_in_either_seat() {
    let (mut wins, mut games) = (0, 0);
    for seed in 0..80 {
        for seat in [0, 1] {
            if let Some(winner) = game(seed, seat) {
                games += 1;
                wins += usize::from(winner == seat);
            }
        }
    }
    assert!(games >= 120, "most games finish ({games} did)");
    assert!(
        wins * 100 >= games * 55,
        "the heuristic bot won only {wins} of {games}"
    );
}
