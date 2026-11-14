//! The JSON protocol between a frontend and a game, independent of the
//! transport (a Tauri command, an HTTP request, a test).

use std::path::PathBuf;

use gamelib::id::PlayerId;
use serde::{Deserialize, Serialize};

use crate::{
    description::GameDescription,
    session::{GameUi, NewGame, Session, Snapshot},
};

/// What a frontend can ask for.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Request {
    /// The game's names and card text.
    Describe,
    /// Start a game (replaces the running one).
    NewGame(NewGame),
    /// The screen of one seat.
    Snapshot {
        /// Whose screen.
        viewer: u8,
    },
    /// Play one of the actions of a snapshot.
    Act {
        /// Who acts.
        viewer:   u8,
        /// The revision of the snapshot the action came from.
        revision: u64,
        /// The action's `index`.
        action:   usize,
    },
    /// Start planning: the viewer's moves are tried out in a what-if copy of
    /// the game and listed, without being played.
    PlanStart {
        /// Who plans.
        viewer: u8,
    },
    /// Take the last planned move back.
    PlanUndo {
        /// Who plans.
        viewer: u8,
    },
    /// Put the last undone move back.
    PlanRedo {
        /// Who plans.
        viewer: u8,
    },
    /// Play the planned moves for real, as far as they are still possible.
    PlanRun {
        /// Who plans.
        viewer: u8,
    },
    /// Drop the plan and show the real game again.
    PlanStop {
        /// Who plans.
        viewer: u8,
    },
    /// Turn "auto next player" on or off.
    SetAuto {
        /// Whether the next player moves by itself after a move.
        auto:   bool,
        /// Whose screen to answer with.
        viewer: u8,
    },
    /// Let the next player move (when auto is off).
    Next {
        /// Whose screen to answer with.
        viewer: u8,
    },
    /// Take back what the viewer did this turn (when auto is off).
    Undo {
        /// Whose screen to answer with.
        viewer: u8,
    },
}

/// What the game answers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Response {
    /// Answer to `Describe`.
    Description(GameDescription),
    /// Answer to everything that changes or reads the game.
    Snapshot(Box<Snapshot>),
    /// Something was refused.
    Error {
        /// Why.
        message: String,
    },
}

/// Holds the game and answers requests.
pub struct Backend<G: GameUi> {
    ui:          G,
    description: GameDescription,
    session:     Option<Session<G>>,
    log_dir:     Option<PathBuf>,
    auto:        bool,
}

impl<G: GameUi> Backend<G> {
    /// A backend without a running game.
    pub fn new(ui: G) -> Self {
        let description = ui.description();
        Self {
            ui,
            description,
            session: None,
            log_dir: None,
            auto: true,
        }
    }

    /// Writes every game started from now on to its own file in this
    /// directory (see [`crate::record`]).
    #[must_use]
    pub fn with_log_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.log_dir = Some(dir.into());
        self
    }

    /// Where games are written, if they are.
    pub fn log_dir(&self) -> Option<&std::path::Path> {
        self.log_dir.as_deref()
    }

    /// The description of the game.
    pub const fn description(&self) -> &GameDescription {
        &self.description
    }

    /// The running game, if any.
    pub const fn session(&self) -> Option<&Session<G>> {
        self.session.as_ref()
    }

    /// Answers one request.
    pub fn handle(&mut self, request: Request) -> Response {
        match self.try_handle(request) {
            Ok(response) => response,
            Err(message) => Response::Error { message },
        }
    }

    fn try_handle(&mut self, request: Request) -> Result<Response, String> {
        match request {
            Request::Describe => Ok(Response::Description(self.description.clone())),
            Request::NewGame(game) => {
                let session = Session::with_log(
                    self.ui.clone(),
                    &game,
                    &self.description,
                    self.log_dir.as_deref(),
                )?;
                let mut session = session;
                session.set_auto(self.auto)?;
                let viewer = game.humans.as_ref().and_then(|h| h.first().copied()).unwrap_or(0);
                let snapshot = session.snapshot(PlayerId(viewer));
                self.session = Some(session);
                Ok(Response::Snapshot(Box::new(snapshot)))
            },
            Request::Snapshot { viewer } => {
                let session = self.session.as_ref().ok_or("no game is running")?;
                Ok(Response::Snapshot(Box::new(
                    session.snapshot(PlayerId(viewer)),
                )))
            },
            Request::Act {
                viewer,
                revision,
                action,
            } => {
                let session = self.session.as_mut().ok_or("no game is running")?;
                session.act(PlayerId(viewer), revision, action)?;
                Ok(Response::Snapshot(Box::new(
                    session.snapshot(PlayerId(viewer)),
                )))
            },
            Request::SetAuto { auto, viewer } => {
                self.auto = auto;
                let session = self.session.as_mut().ok_or("no game is running")?;
                session.set_auto(auto)?;
                Ok(Response::Snapshot(Box::new(
                    session.snapshot(PlayerId(viewer)),
                )))
            },
            Request::Next { viewer } => {
                let session = self.session.as_mut().ok_or("no game is running")?;
                session.next()?;
                Ok(Response::Snapshot(Box::new(
                    session.snapshot(PlayerId(viewer)),
                )))
            },
            Request::Undo { viewer } => {
                let session = self.session.as_mut().ok_or("no game is running")?;
                session.undo()?;
                Ok(Response::Snapshot(Box::new(
                    session.snapshot(PlayerId(viewer)),
                )))
            },
            Request::PlanStart { viewer }
            | Request::PlanUndo { viewer }
            | Request::PlanRedo { viewer }
            | Request::PlanRun { viewer }
            | Request::PlanStop { viewer } => {
                let session = self.session.as_mut().ok_or("no game is running")?;
                let player = PlayerId(viewer);
                match request {
                    Request::PlanStart { .. } => session.plan_start(player)?,
                    Request::PlanUndo { .. } => session.plan_undo()?,
                    Request::PlanRedo { .. } => session.plan_redo()?,
                    Request::PlanRun { .. } => session.plan_run(player)?,
                    _ => session.plan_stop(),
                }
                Ok(Response::Snapshot(Box::new(session.snapshot(player))))
            },
        }
    }

    /// Answers a request given as JSON with a response as JSON.
    pub fn handle_json(&mut self, request: &str) -> String {
        let response = match serde_json::from_str::<Request>(request) {
            Ok(request) => self.handle(request),
            Err(e) => Response::Error {
                message: format!("bad request: {e}"),
            },
        };
        serde_json::to_string(&response).unwrap_or_else(|e| {
            format!("{{\"type\":\"error\",\"message\":\"cannot encode the answer: {e}\"}}")
        })
    }
}
