//! M21 Rotation Level A — user-set starting 5 per team.
//!
//! `Starters` is purely positional: one optional `PlayerId` per canonical
//! NBA position. The bench (positions 6-13) and minutes split stay
//! auto-built — Level A only lets the GM lock who starts. The struct
//! ships `Default` (all `None`) so a fresh team round-trips through the
//! store as "no override → auto rotation".

use crate::{PlayerId, Position};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Starters {
    pub pg: Option<PlayerId>,
    pub sg: Option<PlayerId>,
    pub sf: Option<PlayerId>,
    pub pf: Option<PlayerId>,
    pub c: Option<PlayerId>,
}

impl Starters {
    /// True iff every positional slot has a player assigned. The sim hook
    /// only honors a user override when this returns true — partial
    /// lineups fall through to the auto-builder.
    pub fn is_complete(&self) -> bool {
        self.pg.is_some()
            && self.sg.is_some()
            && self.sf.is_some()
            && self.pf.is_some()
            && self.c.is_some()
    }

    pub fn slot(&self, pos: Position) -> Option<PlayerId> {
        match pos {
            Position::PG => self.pg,
            Position::SG => self.sg,
            Position::SF => self.sf,
            Position::PF => self.pf,
            Position::C => self.c,
        }
    }

    pub fn set_slot(&mut self, pos: Position, player: Option<PlayerId>) {
        match pos {
            Position::PG => self.pg = player,
            Position::SG => self.sg = player,
            Position::SF => self.sf = player,
            Position::PF => self.pf = player,
            Position::C => self.c = player,
        }
    }

    /// Iterate `(Position, PlayerId)` for every assigned slot, in canonical
    /// PG→C order. Empty slots are skipped.
    pub fn iter_assigned(&self) -> impl Iterator<Item = (Position, PlayerId)> + '_ {
        Position::all()
            .into_iter()
            .filter_map(|pos| self.slot(pos).map(|pid| (pos, pid)))
    }
}
