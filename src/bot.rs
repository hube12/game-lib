//! Bots: something that picks one of the legal actions for a player.

use std::collections::VecDeque;

use crate::{engine::Action, rng::Rng, state::GameState, view::PlayerView};

/// Chooses an action for a player.
pub trait Bot {
    /// Returns the index into `legal` of the action to play. `legal` is never
    /// empty.
    fn choose(&mut self, view: &PlayerView, legal: &[Action]) -> usize;

    /// The same choice for a bot that tries moves out on a copy of the game.
    /// Such a bot gets the whole state and is trusted not to read what its
    /// player could not know (it should shuffle the hidden zones of its copy).
    /// The default ignores the state.
    fn choose_with_state(
        &mut self,
        _state: &GameState,
        view: &PlayerView,
        legal: &[Action],
    ) -> usize {
        self.choose(view, legal)
    }

    /// Why the last choice was made, for game logs. `None` when the bot has
    /// nothing to say.
    fn explain(&self) -> Option<String> {
        None
    }
}

/// Plays a uniformly random legal action.
#[derive(Debug)]
pub struct RandomBot {
    rng: Rng,
}

impl RandomBot {
    /// A random bot with its own deterministic stream.
    pub fn new(seed: u64) -> Self {
        Self {
            rng: Rng::from_seed(seed),
        }
    }
}

impl Bot for RandomBot {
    fn choose(&mut self, _view: &PlayerView, legal: &[Action]) -> usize {
        self.rng.below(legal.len())
    }
}

/// Plays a fixed list of actions in order, skipping the ones that are not legal
/// at that moment. When the script is used up (or nothing in it applies) it
/// falls back to the first legal action.
#[derive(Debug)]
pub struct ScriptedBot {
    script: VecDeque<Action>,
}

impl ScriptedBot {
    /// A bot following the script.
    pub fn new(script: impl IntoIterator<Item = Action>) -> Self {
        Self {
            script: script.into_iter().collect(),
        }
    }
}

impl Bot for ScriptedBot {
    fn choose(&mut self, _view: &PlayerView, legal: &[Action]) -> usize {
        while let Some(next) = self.script.pop_front() {
            if let Some(i) = legal.iter().position(|a| *a == next) {
                return i;
            }
        }
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{id::PlayerId, state::GameState};

    fn view() -> PlayerView {
        let mut state = GameState::new(Rng::from_seed(1));
        state.add_player(PlayerId(0));
        PlayerView::new(&state, PlayerId(0))
    }

    #[test]
    fn random_bot_is_deterministic_and_in_range() {
        let legal = [Action::EndTurn, Action::Pass, Action::Decide(0)];
        let (mut a, mut b) = (RandomBot::new(4), RandomBot::new(4));
        let v = view();
        for _ in 0..50 {
            let i = a.choose(&v, &legal);
            assert!(i < legal.len());
            assert_eq!(i, b.choose(&v, &legal));
        }
    }

    #[test]
    fn scripted_bot_follows_the_script_and_skips_illegal_steps() {
        let mut bot = ScriptedBot::new([Action::Decide(3), Action::Pass, Action::EndTurn]);
        let v = view();
        assert_eq!(
            bot.choose(&v, &[Action::EndTurn, Action::Pass]),
            1,
            "Decide(3) is skipped, Pass is played"
        );
        assert_eq!(bot.choose(&v, &[Action::EndTurn, Action::Pass]), 0);
        assert_eq!(
            bot.choose(&v, &[Action::Pass, Action::EndTurn]),
            0,
            "script used up: first legal action"
        );
    }
}
