//! Drives a game with one bot per player.

use gamelib::{
    engine::{Engine, GameRules, RuleError},
    id::PlayerId,
};

use crate::bot::Bot;

/// How a finished game went.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Outcome {
    /// The winners.
    pub winners: Vec<PlayerId>,
    /// Number of actions played.
    pub actions: usize,
    /// The round the game ended in.
    pub rounds:  u32,
}

/// Why a game could not be played to the end.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RunError {
    /// Nobody had a legal action although the game was not over.
    Stuck,
    /// The game did not finish within the action budget.
    TooLong(usize),
    /// The engine refused an action the bot picked from the legal list (an
    /// engine bug).
    Engine(RuleError),
}

/// Plays until the game ends, asking the bot of whichever player has a legal
/// action. Several players may be asked in turn while a simultaneous decision
/// is pending.
pub fn run<R: GameRules>(
    engine: &mut Engine<R>,
    bots: &mut [Box<dyn Bot>],
    max_actions: usize,
) -> Result<Outcome, RunError> {
    for played in 0..=max_actions {
        if let Some(winners) = engine.winners() {
            return Ok(Outcome {
                winners: winners.to_vec(),
                actions: played,
                rounds:  engine.state().flow.round,
            });
        }
        if played == max_actions {
            break;
        }
        let Some((player, legal)) = engine
            .players()
            .iter()
            .map(|p| (*p, engine.legal_actions(*p)))
            .find(|(_, legal)| !legal.is_empty())
        else {
            return Err(RunError::Stuck);
        };
        let choice = bots[usize::from(player.0)]
            .choose_with_state(engine.state(), &engine.view(player), &legal)
            .min(legal.len() - 1);
        engine.apply(player, legal[choice].clone()).map_err(RunError::Engine)?;
    }
    Err(RunError::TooLong(max_actions))
}

#[cfg(test)]
mod tests {
    use gamelib::engine::Action;
    use gamelib_toy::ToyRules;

    use super::*;
    use crate::bot::{RandomBot, ScriptedBot};

    fn random_bots(seed: u64, n: u64) -> Vec<Box<dyn Bot>> {
        (0..n).map(|i| Box::new(RandomBot::new(seed + i)) as Box<dyn Bot>).collect()
    }

    #[test]
    fn random_bots_play_the_toy_game_to_the_end() {
        let mut engine = Engine::new(ToyRules, 2, 5).unwrap();
        let outcome = run(&mut engine, &mut random_bots(5, 2), 20_000).unwrap();
        assert_eq!(outcome.winners.len().min(1), 1);
        assert_eq!(outcome.actions, engine.log().len());
        assert!(outcome.rounds >= 1);
    }

    #[test]
    fn a_tiny_budget_is_reported() {
        let mut engine = Engine::new(ToyRules, 2, 5).unwrap();
        assert_eq!(
            run(&mut engine, &mut random_bots(5, 2), 3),
            Err(RunError::TooLong(3))
        );
        assert_eq!(engine.log().len(), 3);
    }

    #[test]
    fn scripted_bots_are_followed() {
        let mut engine = Engine::new(ToyRules, 2, 5).unwrap();
        let mut bots: Vec<Box<dyn Bot>> = vec![
            Box::new(ScriptedBot::new([Action::EndTurn, Action::EndTurn])),
            Box::new(ScriptedBot::new([Action::Pass, Action::Pass])),
        ];
        assert_eq!(run(&mut engine, &mut bots, 4), Err(RunError::TooLong(4)));
        let log: Vec<_> = engine.log().iter().map(|(p, a)| (p.0, a.clone())).collect();
        assert_eq!(&log[..2], &[(0, Action::EndTurn), (1, Action::Pass)]);
    }
}
