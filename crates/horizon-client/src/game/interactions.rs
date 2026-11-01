//! Interaction entities on a server's client (`Interaction`): invisible boxes a
//! server puts where a click should count (buttons, NPCs, a duel's ready gate).
//! The client only aims at them (`Interaction.isPickable`) and sends the attack
//! or the use (`ServerboundAttackPacket`, `ServerboundInteractPacket`); the
//! server decides what they mean.
use super::{Game, ray_box};
use horizon_core::geometry::{Aabb, Vec3};
use horizon_protocol::entity_data::Value;

pub(super) struct Interaction {
    position: [f64; 3],
    /// `DATA_WIDTH_ID` and `DATA_HEIGHT_ID`.
    width: f32,
    height: f32,
    /// `DATA_RESPONSE_ID`: a use is `InteractionResult.SUCCESS` (the arm
    /// swings), else `CONSUME`.
    pub response: bool,
}

impl Interaction {
    pub(super) fn new(position: [f64; 3]) -> Self {
        Self {
            position,
            width: 1.0,
            height: 1.0,
            response: false,
        }
    }

    /// `EntityDimensions.makeBoundingBox`: the width around the position, the
    /// height up from it.
    fn bounds(&self) -> Aabb {
        let [x, y, z] = self.position;
        let half = f64::from(self.width) / 2.0;
        Aabb::new(
            Vec3::new(x - half, y, z - half),
            Vec3::new(x + half, y + f64::from(self.height), z + half),
        )
    }
}

impl Game<'_> {
    /// The nearest interaction box the view enters within `reach`: its id, how
    /// far, and its position.
    pub(super) fn interaction_hit(
        &self,
        eye: [f64; 3],
        dir: [f64; 3],
        reach: f64,
    ) -> Option<(i32, f64, [f64; 3])> {
        self.interactions
            .iter()
            .filter_map(|(&id, i)| Some((id, ray_box(eye, dir, reach, i.bounds())?, i.position)))
            .min_by(|a, b| a.1.total_cmp(&b.1))
    }

    /// `Interaction.onSyncedDataUpdated`: a new size or response.
    pub(super) fn interaction_data(&mut self, id: i32, values: &[(u8, i32, Value)]) {
        let Some(i) = self.interactions.get_mut(&id) else {
            return;
        };
        for (index, _, value) in values {
            match (index, value) {
                (8, Value::Float(w)) => i.width = *w,
                (9, Value::Float(h)) => i.height = *h,
                (10, Value::Bool(r)) => i.response = *r,
                _ => {}
            }
        }
    }

    /// Entity's NO_OP InterpolationHandler: an interaction box is where the
    /// server puts it at once.
    pub(super) fn move_remote_interaction(&mut self, id: i32, position: [f64; 3]) {
        if let Some(i) = self.interactions.get_mut(&id) {
            i.position = position;
        }
    }
}
