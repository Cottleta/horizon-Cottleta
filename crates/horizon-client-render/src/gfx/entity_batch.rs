//! Stable entity material grouping with renderer-owned, reusable CPU scratch.
use super::Vertex;

/// Entity materials by a quad's overlay (`Vertex::layer`), in draw order:
/// ordinary entities, world text drawn with a depth bias (35,
/// `RenderPipelines.TEXT_POLYGON_OFFSET`), breaking cracks (11), translucent
/// breaking cracks (12), the boat water mask (13, `RenderPipelines.WATER_MASK`),
/// energy swirls (34, `RenderPipelines.ENERGY_SWIRL`), entity shadows (14,
/// `RenderPipelines.ENTITY_SHADOW`), translucent-layer particles (15, drawn
/// after translucent terrain: `SubmitNodeCollection.submitQuadParticleGroup`)
/// and translucent item quads (32, drawn without depth writes).
///
/// With improved transparency on, quads tagged `OIT_ENTITY` are a tenth
/// material at the end: the render types vanilla gives an `OitPipelineSet`
/// (translucent, emissive, eyes and breeze wind entity models). Off, the tag is
/// ignored and they stay in the first one, in their order.
/// Sorted still-fluid geometry follows, then the two native world-canvas text materials,
/// NORMAL and POLYGON_OFFSET, then the native terrain models. Material storage order is
/// independent of their draw stages.
const MATERIALS: usize = 16;
/// The material the eyes moved out of `additive` join (improved transparency's entities).
const EYES: usize = 9;
/// The first sorted material (still fluid, then the world-canvas text): drawn after the
/// moved eyes, each sorted far to near.
const TEXT: usize = 10;
/// The first native terrain model material: depth writing, in submission order.
const NATIVE: usize = 13;
pub const TERRAIN_MODEL_SOLID_OVERLAY: u32 = 42;
pub const TERRAIN_MODEL_CUTOUT_OVERLAY: u32 = 43;
pub const TERRAIN_MODEL_CUTOUT_MIPPED_OVERLAY: u32 = 44;
/// The overlay of a resource pack's Polytone custom particle drawn additively
/// (`additive_translucent`): fog tints its colour as vanilla's particle shader does, which the
/// other additive geometry does not take.
pub const ADDITIVE_PARTICLE_OVERLAY: u32 = 45;
/// Native terrain-array UV/payloads must never enter a shared-atlas shader-pack pass.
pub fn is_native_terrain(layer: u32) -> bool {
    matches!(
        layer & 0xffff,
        FLUID_OVERLAY
            | TERRAIN_MODEL_SOLID_OVERLAY
            | TERRAIN_MODEL_CUTOUT_OVERLAY
            | TERRAIN_MODEL_CUTOUT_MIPPED_OVERLAY
    )
}
/// Sorted depth-writing still fluid, sampling terrain sprite arrays rather than the shared atlas.
pub const FLUID_OVERLAY: u32 = 41;
/// A bit of `Vertex::layer`, above the overlay (low 16 bits) and the HDR tier
/// (bits 16 to 23): the quad's render type has an `OitPipelineSet`
/// (`RenderTypes.entityTranslucent`, `entityTranslucentEmissive`, `eyes`,
/// `breezeWind`). Only the improved transparency path reads it.
pub const OIT_ENTITY: u32 = 1 << 24;
/// `RenderPipelines.DRAGON_RAYS`: untextured colour, additive with depth writes.
pub const DRAGON_RAYS_OVERLAY: u32 = 37;
/// `RenderPipelines.ENTITY_CUTOUT_DISSOLVE` (the dying dragon): a pixel is dropped where the
/// vertex alpha is below the mask's, whose atlas UV rides in `Vertex::glint_uv`; the rest is
/// drawn opaque. No overlay.
pub const DISSOLVE_OVERLAY: u32 = 38;
/// Tags quads as drawn by a render type vanilla gives an `OitPipelineSet`
/// (`OIT_ENTITY`): improved transparency draws them in its stages.
pub fn mark_oit(quads: &mut [Vertex]) {
    for vertex in quads {
        if !is_native_terrain(vertex.layer) {
            vertex.layer |= OIT_ENTITY;
        }
    }
}
/// The overlay of depth-only quads that hide water (`RenderTypes.waterMask`).
pub const WATER_MASK_OVERLAY: u32 = 13;
/// The overlay of energy swirl quads (`RenderTypes.energySwirl`: the charged creeper's and the
/// powered wither's armour). The light bytes carry the sprite's origin and the colour bytes its
/// size (`breaking::stage_light`), the UV the scrolled position in the sprite.
pub const ENERGY_SWIRL_OVERLAY: u32 = 34;
/// The overlay of entity shadow quads (`RenderTypes.entityShadow`).
pub const SHADOW_OVERLAY: u32 = 14;
/// The overlay of translucent-layer particles (`SingleQuadParticle.Layer.translucent`).
pub const TRANSLUCENT_PARTICLE_OVERLAY: u32 = 15;
/// The overlay of item quads with translucent pixels (`Sheets.translucentItemSheet`,
/// `RenderPipelines.ITEM_TRANSLUCENT`); past the creeper swell range (16 to 31).
pub const TRANSLUCENT_ITEM_OVERLAY: u32 = 32;
/// A translucent item with an enchantment glint.
pub const TRANSLUCENT_GLINT_ITEM_OVERLAY: u32 = 33;
/// The overlay of text in `Font.DisplayMode.POLYGON_OFFSET` (text displays):
/// drawn like an ordinary entity, pulled toward the camera.
pub const TEXT_POLYGON_OFFSET_OVERLAY: u32 = 35;
/// A native world canvas, drawn after particles with normal text depth/culling.
pub const WORLD_TEXT_OVERLAY: u32 = 39;
/// The front of its eight-offset outlined text, using POLYGON_OFFSET.
pub const WORLD_TEXT_POLYGON_OFFSET_OVERLAY: u32 = 40;

/// The material of a quad by its overlay (`MATERIALS` order): the overlay is the layer's low 16
/// bits, as entity.wgsl reads it; HDR output's tier rides in the high ones.
fn material(layer: u32, oit: bool) -> usize {
    match layer & 0xffff {
        TERRAIN_MODEL_SOLID_OVERLAY => return 13,
        TERRAIN_MODEL_CUTOUT_OVERLAY => return 14,
        TERRAIN_MODEL_CUTOUT_MIPPED_OVERLAY => return 15,
        _ => {}
    }
    if layer & 0xffff == FLUID_OVERLAY {
        return 10;
    }
    if oit && layer & OIT_ENTITY != 0 {
        return 9;
    }
    match layer & 0xffff {
        WORLD_TEXT_OVERLAY => 11,
        WORLD_TEXT_POLYGON_OFFSET_OVERLAY => 12,
        TEXT_POLYGON_OFFSET_OVERLAY => 1,
        11 => 2,
        12 => 3,
        WATER_MASK_OVERLAY => 4,
        ENERGY_SWIRL_OVERLAY => 5,
        SHADOW_OVERLAY => 6,
        TRANSLUCENT_PARTICLE_OVERLAY => 7,
        TRANSLUCENT_ITEM_OVERLAY | TRANSLUCENT_GLINT_ITEM_OVERLAY => 8,
        _ => 0,
    }
}

/// The quads a held quad stands for (`parts::PLACEHOLDER_LIGHT` in its first vertex's
/// light), in its first vertex's `glint_uv[0]` bits: the compute pass writes them.
fn held_quads(quad: &[Vertex; 4]) -> Option<usize> {
    (quad[0].light == super::parts::PLACEHOLDER_LIGHT)
        .then(|| quad[0].glint_uv[0].to_bits() as usize)
}

#[derive(Default)]
pub(super) struct EntityBatch {
    // Runs of consecutive quads of one material (first quad, quad count, held): a
    // model's quads share one, so whole runs are copied at once. A held quad is a run
    // of its own that stands for `held` quads nothing is copied into. Cleared each
    // frame, keeping their storage.
    runs: [Vec<(usize, usize, usize)>; MATERIALS],
    /// Vertices the runs lay out.
    vertices: usize,
    /// Additive quads tagged `OIT_ENTITY` that the last `update` moved into the
    /// last material (improved transparency on): the dragon's eyes, which share
    /// the additive buffer with its rays.
    pub(super) moved_additive: u32,
    /// Whether the last `update` moved them.
    oit: bool,
}

impl EntityBatch {
    /// Group `entities`' quads by material; `write` then lays them out in
    /// material order, followed by `additive`, `hand`, `see_through` and `tail`.
    pub(super) fn update(
        &mut self,
        entities: &[Vertex],
        additive: &[Vertex],
        oit: bool,
    ) -> [u32; MATERIALS] {
        let quads = entities.as_chunks::<4>().0;
        for runs in &mut self.runs {
            runs.clear();
        }
        let mut counts = [0usize; MATERIALS];
        let mut start = 0;
        while start < quads.len() {
            let kind = material(quads[start][0].layer, oit);
            if let Some(held) = held_quads(&quads[start]) {
                self.runs[kind].push((start, 1, held));
                counts[kind] += held;
                start += 1;
                continue;
            }
            let end = quads[start + 1..]
                .iter()
                .position(|quad| material(quad[0].layer, oit) != kind || held_quads(quad).is_some())
                .map_or(quads.len(), |n| start + 1 + n);
            self.runs[kind].push((start, end - start, 0));
            counts[kind] += end - start;
            start = end;
        }
        self.vertices = counts.iter().sum::<usize>() * 4;
        // `RenderTypes.eyes` is blended like the other entities, not added: with
        // improved transparency on, tagged additive quads join the last
        // material, drawn without the additive overlay and the baked shading.
        self.oit = oit;
        self.moved_additive = if oit {
            additive
                .as_chunks::<4>()
                .0
                .iter()
                .filter(|quad| quad[0].layer & OIT_ENTITY != 0)
                .count() as u32
        } else {
            0
        };
        counts[EYES] += self.moved_additive as usize;
        counts.map(|count| count as u32)
    }

    /// The vertex count `write` lays out for `update`'s entities and these slices.
    pub(super) fn len(
        &self,
        additive: &[Vertex],
        hand: &[Vertex],
        see_through: &[Vertex],
    ) -> usize {
        self.vertices + additive.len() + hand.len() + see_through.len()
    }

    /// Where `write` puts each run: (first entity vertex, vertices, buffer vertex), by
    /// entity vertex, into `places` (cleared). A held quad's place is where the
    /// vertices it stands for start.
    pub(super) fn places(&self, places: &mut Vec<(u32, u32, u32)>) {
        places.clear();
        let mut at = 0;
        for (material, runs) in self.runs.iter().enumerate() {
            if material == TEXT {
                // The eyes moved out of `additive` sit between (`write`).
                at += self.moved_additive * 4;
            }
            for &(first, count, held) in runs {
                places.push(((first * 4) as u32, (count * 4) as u32, at));
                at += (count.max(held) * 4) as u32;
            }
        }
        // Sorted runs, one per material: the stable sort merges them.
        places.sort_by_key(|p| p.0);
    }

    /// The buffer vertex of entity vertex `vertex`, by `places`. `run` is where the last
    /// lookup landed: jobs come in entity order, nearly all in that run or the next.
    pub(super) fn place(places: &[(u32, u32, u32)], run: &mut usize, vertex: u32) -> Option<u32> {
        let holds = |i: usize| {
            places
                .get(i)
                .is_some_and(|p| p.0 <= vertex && vertex < p.0 + p.1)
        };
        if !holds(*run) {
            *run = if holds(*run + 1) {
                *run + 1
            } else {
                places.partition_point(|p| p.0 <= vertex).checked_sub(1)?
            };
        }
        let (first, count, at) = places[*run];
        (vertex < first + count).then_some(at + vertex - first)
    }

    /// Hand `put` each piece of the combined buffer with its vertex offset:
    /// the material runs, then `additive`, `hand` and `see_through`. Copying
    /// straight into the upload's staging memory spares a combined copy.
    pub(super) fn write(
        &self,
        entities: &[Vertex],
        additive: &[Vertex],
        hand: &[Vertex],
        see_through: &[Vertex],
        tail: &[Vertex],
        mut put: impl FnMut(usize, &[Vertex]),
    ) {
        let mut at = 0;
        for runs in &self.runs[..TEXT] {
            for &(first, count, held) in runs {
                if held > 0 {
                    // Left to the compute pass.
                    at += held * 4;
                } else {
                    put(at, &entities[first * 4..(first + count) * 4]);
                    at += count * 4;
                }
            }
        }
        if self.moved_additive > 0 {
            for quad in additive.as_chunks::<4>().0 {
                if quad[0].layer & OIT_ENTITY != 0 {
                    put(at, &quad.map(|v| Vertex {
                        layer: v.layer & !0xffff,
                        normal: [0; 4],
                        ..v
                    }));
                    at += 4;
                }
            }
        }
        // Fluid and RenderType.text sort their own alpha-blended quads far to near, after
        // the other materials (and the eyes moved out of additive); the front text uses
        // polygon offset.
        for runs in &self.runs[TEXT..NATIVE] {
            let mut quads: Vec<[Vertex; 4]> = Vec::new();
            for &(first, count, held) in runs {
                if held > 0 {
                    at += held * 4;
                    continue;
                }
                quads.extend_from_slice(entities[first * 4..(first + count) * 4].as_chunks::<4>().0);
            }
            if quads.is_empty() {
                continue;
            }
            let distance = |q: &[Vertex; 4]| {
                (0..3)
                    .map(|i| ((q[0].pos[i] + q[2].pos[i]) * 0.5).powi(2))
                    .sum::<f32>()
            };
            quads.sort_by(|a, b| distance(b).total_cmp(&distance(a)));
            put(at, quads.as_flattened());
            at += quads.len() * 4;
        }
        // Native solid/cutout models are depth writing and keep their submission order.
        for runs in &self.runs[NATIVE..] {
            for &(first, count, held) in runs {
                if held > 0 {
                    at += held * 4;
                } else {
                    put(at, &entities[first * 4..(first + count) * 4]);
                    at += count * 4;
                }
            }
        }
        if self.moved_additive > 0 {
            for quad in additive.as_chunks::<4>().0 {
                if quad[0].layer & OIT_ENTITY == 0 {
                    put(at, quad);
                    at += 4;
                }
            }
        } else if !additive.is_empty() {
            put(at, additive);
            at += additive.len();
        }
        for vertices in [hand, see_through, tail] {
            if !vertices.is_empty() {
                put(at, vertices);
                at += vertices.len();
            }
        }
    }

    #[cfg(test)]
    pub(super) fn combined(
        &self,
        entities: &[Vertex],
        additive: &[Vertex],
        hand: &[Vertex],
        see_through: &[Vertex],
    ) -> Vec<Vertex> {
        let mut out = vec![bytemuck::Zeroable::zeroed(); self.len(additive, hand, see_through)];
        self.write(entities, additive, hand, see_through, &[], |at, src| {
            out[at..at + src.len()].copy_from_slice(src);
        });
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{hint::black_box, time::Instant};
    #[test]
    fn native_models_keep_opaque_order_and_do_not_move_existing_materials_or_tails() {
        let quad = |layer, x| {
            [Vertex {
                layer,
                pos: [x, 0.0, 0.0],
                ..geometry(1, false)[0]
            }; 4]
        };
        let entities: Vec<_> = [
            (FLUID_OVERLAY, 10.0),
            (WORLD_TEXT_OVERLAY, 11.0),
            (TERRAIN_MODEL_SOLID_OVERLAY | 279 << 16, 3.0),
            (TERRAIN_MODEL_SOLID_OVERLAY, 1.0),
            (TERRAIN_MODEL_CUTOUT_OVERLAY, 4.0),
            (TERRAIN_MODEL_CUTOUT_MIPPED_OVERLAY, 5.0),
            (OIT_ENTITY, 6.0),
            (0, 7.0),
        ]
        .into_iter()
        .flat_map(|(layer, x)| quad(layer, x))
        .collect();
        let mut batch = EntityBatch::default();
        let counts = batch.update(&entities, &quad(0, 8.0), true);
        assert_eq!(counts, [1, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 0, 2, 1, 1]);
        let combined = batch.combined(&entities, &quad(0, 8.0), &quad(0, 9.0), &quad(0, 12.0));
        let xs: Vec<_> = combined.iter().step_by(4).map(|v| v.pos[0]).collect();
        assert_eq!(
            xs,
            [7.0, 6.0, 10.0, 11.0, 3.0, 1.0, 4.0, 5.0, 8.0, 9.0, 12.0]
        );
        assert_eq!(
            combined[16].layer,
            TERRAIN_MODEL_SOLID_OVERLAY | 279 << 16
        );
    }
    #[test]
    fn tagging_oit_does_not_corrupt_a_native_models_sprite_index() {
        let mut vertices = [geometry(1, false)[0]; 2];
        vertices[0].layer = TERRAIN_MODEL_CUTOUT_OVERLAY | 17 << 16;
        vertices[1].layer = 3;
        mark_oit(&mut vertices);
        assert_eq!(vertices[0].layer, TERRAIN_MODEL_CUTOUT_OVERLAY | 17 << 16);
        assert_eq!(vertices[1].layer, 3 | OIT_ENTITY);
    }
    #[test]
    fn fluid_and_world_canvas_sort_after_oit_eyes_before_frame_tails() {
        let quad = |layer, x| {
            [Vertex {
                layer,
                pos: [x, 0.0, 0.0],
                ..geometry(1, false)[0]
            }; 4]
        };
        let entities: Vec<_> = [
            (0, 1.0),
            (35, 2.0),
            (OIT_ENTITY, 3.0),
            (WORLD_TEXT_OVERLAY, 4.0),
            (WORLD_TEXT_OVERLAY, 5.0),
            (WORLD_TEXT_POLYGON_OFFSET_OVERLAY, 6.0),
            (FLUID_OVERLAY, 11.0),
            (FLUID_OVERLAY | OIT_ENTITY, 12.0),
        ]
        .into_iter()
        .flat_map(|(layer, x)| quad(layer, x))
        .collect();
        let additive: Vec<_> = [(OIT_ENTITY | 2, 7.0), (2, 8.0)]
            .into_iter()
            .flat_map(|(layer, x)| quad(layer, x))
            .collect();
        let mut batch = EntityBatch::default();
        assert_eq!(
            batch.update(&entities, &additive, true),
            [1, 1, 0, 0, 0, 0, 0, 0, 0, 2, 2, 2, 1, 0, 0, 0]
        );
        let positions: Vec<_> = batch
            .combined(&entities, &additive, &quad(0, 9.0), &quad(0, 10.0))
            .iter()
            .step_by(4)
            .map(|v| v.pos[0])
            .collect();
        assert_eq!(
            positions,
            [
                1.0, 2.0, 3.0, 7.0, 12.0, 11.0, 5.0, 4.0, 6.0, 8.0, 9.0, 10.0
            ]
        );
    }

    // Retain the original implementation as the equivalence/benchmark oracle.
    fn baseline(
        entities: &[Vertex],
        additive: &[Vertex],
        hand: &[Vertex],
    ) -> (Vec<Vertex>, [u32; MATERIALS]) {
        let mut combined = Vec::with_capacity(entities.len() + additive.len() + hand.len());
        let mut counts = [0; MATERIALS];
        for (i, layer) in [0, 35, 11, 12, 13, 34, 14, 15].into_iter().enumerate() {
            for quad in entities.as_chunks::<4>().0 {
                let material = match quad[0].layer {
                    35 => 35,
                    11 => 11,
                    12 => 12,
                    13 => 13,
                    34 => 34,
                    14 => 14,
                    15 => 15,
                    _ => 0,
                };
                if material == layer {
                    combined.extend_from_slice(quad);
                    counts[i] += 1;
                }
            }
        }
        combined.extend_from_slice(additive);
        combined.extend_from_slice(hand);
        (combined, counts)
    }

    fn geometry(quads: usize, mixed: bool) -> Vec<Vertex> {
        (0..quads * 4)
            .map(|index| Vertex {
                pos: [index as f32, 2.0, 3.0],
                uv: [index as f32, 5.0],
                layer: if mixed {
                    [7, 11, 14, 35, 12, 4, 13, 15, 0][index / 4 % 9]
                } else {
                    0
                },
                color: index as u32,
                light: (index * 3) as u32,
                normal: [1, 2, 3, 4],

                glint_uv: [0.0, 0.0],
            })
            .collect()
    }

    fn equivalent(
        batch: &mut EntityBatch,
        entities: &[Vertex],
        additive: &[Vertex],
        hand: &[Vertex],
    ) {
        let (expected, counts) = baseline(entities, additive, hand);
        assert_eq!(batch.update(entities, additive, false), counts);
        let combined = batch.combined(entities, additive, hand, &[]);
        assert_eq!(
            bytemuck::cast_slice::<_, u8>(&combined),
            bytemuck::cast_slice::<_, u8>(&expected)
        );
    }

    #[test]
    fn a_tier_in_the_high_bits_keeps_the_material() {
        for layer in [0u32, 11, 12, 15, 32, 33, 35, OIT_ENTITY] {
            for oit in [false, true] {
                assert_eq!(material(layer | 4 << 16, oit), material(layer, oit));
            }
        }
    }

    #[test]
    fn oit_entity_quads_are_a_last_material_only_when_on() {
        let quad = |layer: u32, x: f32| {
            (0..4).map(move |_| Vertex {
                pos: [x, 0.0, 0.0],
                layer,
                ..geometry(1, false)[0]
            })
        };
        // Plain, tagged, plain, shadow, tagged with a hurt overlay.
        let entities: Vec<Vertex> = [
            (0, 0.0),
            (OIT_ENTITY | 3 << 16, 1.0),
            (1, 2.0),
            (SHADOW_OVERLAY, 3.0),
            (OIT_ENTITY | 1, 4.0),
        ]
        .into_iter()
        .flat_map(|(layer, x)| quad(layer, x))
        .collect();
        let mut batch = EntityBatch::default();
        // Off: the tag changes nothing, the quads keep their place.
        let counts = batch.update(&entities, &[], false);
        assert_eq!(counts, [4, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        let xs: Vec<f32> = batch.combined(&entities, &[], &[], &[]).iter().step_by(4).map(|v| v.pos[0]).collect();
        assert_eq!(xs, [0.0, 1.0, 2.0, 4.0, 3.0]);
        // On: the tagged quads come last, in order.
        let counts = batch.update(&entities, &[], true);
        assert_eq!(counts, [2, 0, 0, 0, 0, 0, 1, 0, 0, 2, 0, 0, 0, 0, 0, 0]);
        let xs: Vec<f32> = batch.combined(&entities, &[], &[], &[]).iter().step_by(4).map(|v| v.pos[0]).collect();
        assert_eq!(xs, [0.0, 2.0, 3.0, 1.0, 4.0]);
    }

    #[test]
    fn tagged_additive_quads_join_the_last_material_only_when_on() {
        let quad = |layer: u32, x: f32| {
            (0..4).map(move |_| Vertex {
                pos: [x, 0.0, 0.0],
                layer,
                normal: [1, 2, 3, 4],
                ..geometry(1, false)[0]
            })
        };
        // The dragon's eyes (tagged, overlay 2) and a ray (plain).
        let additive: Vec<Vertex> = [(2 | OIT_ENTITY | 3 << 16, 1.0), (2, 2.0)]
            .into_iter()
            .flat_map(|(layer, x)| quad(layer, x))
            .collect();
        let entities: Vec<Vertex> = quad(0, 0.0).collect();
        let mut batch = EntityBatch::default();
        // Off: untouched.
        let counts = batch.update(&entities, &additive, false);
        assert_eq!((counts[9], batch.moved_additive), (0, 0));
        assert_eq!(batch.combined(&entities, &additive, &[], &[]).len(), 12);
        assert_eq!(batch.combined(&entities, &additive, &[], &[])[4].layer, 2 | OIT_ENTITY | 3 << 16);
        // On: the eyes are an entity quad without the additive overlay, the ray stays.
        let counts = batch.update(&entities, &additive, true);
        assert_eq!((counts[0], counts[9], batch.moved_additive), (1, 1, 1));
        let xs: Vec<f32> = batch.combined(&entities, &additive, &[], &[]).iter().step_by(4).map(|v| v.pos[0]).collect();
        assert_eq!(xs, [0.0, 1.0, 2.0]);
        assert_eq!(batch.combined(&entities, &additive, &[], &[])[4].layer, OIT_ENTITY | 3 << 16);
        assert_eq!(batch.combined(&entities, &additive, &[], &[])[4].normal, [0; 4]);
        assert_eq!(batch.combined(&entities, &additive, &[], &[])[8].layer, 2);
    }

    #[test]
    fn stable_material_order_tails_and_reuse() {
        let mut batch = EntityBatch::default();
        let mut entities = geometry(4096, true);
        // Only the first vertex determines a quad's material.
        entities[1].layer = 12;
        let additive = geometry(3, true);
        let hand = geometry(2, false);
        // Warm every bucket to its maximum size.
        equivalent(&mut batch, &entities, &additive, &hand);
        let capacities = batch.runs.each_ref().map(|b| b.capacity());
        for length in [0, 1, 3, 4, 511, entities.len(), 17, 0, entities.len()] {
            equivalent(&mut batch, &entities[..length], &additive[..5], &hand[..3]);
            assert_eq!(batch.runs.each_ref().map(|b| b.capacity()), capacities);
        }
        equivalent(&mut batch, &[], &[], &[]);
        assert!(batch.combined(&[], &[], &[], &[]).is_empty());
        // Grow again, then confirm empty/small frames retain the new storage.
        let larger = geometry(8192, true);
        equivalent(&mut batch, &larger, &additive, &hand);
        let capacities = batch.runs.each_ref().map(|b| b.capacity());
        for entities in [&larger[..7], &larger[..], &[]] {
            equivalent(&mut batch, entities, &[], &[]);
            assert_eq!(batch.runs.each_ref().map(|b| b.capacity()), capacities);
        }
    }

    #[test]
    #[ignore = "manual production-helper microbenchmark"]
    fn benchmark_entity_batch() {
        let additive = geometry(16, false);
        let hand = geometry(8, false);
        for (name, mixed, quads) in [
            ("mixed", true, 65_536),
            ("single", false, 65_536),
            ("runs", true, 65_536),
            ("mixed", true, 262_144),
            ("single", false, 262_144),
        ] {
            let mut entities = geometry(quads, mixed);
            if name == "runs" {
                // Model/material runs: normal 16, glint 4, overlay 4, shadow 2.
                for (index, quad) in entities.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                    let layer = match index % 26 {
                        0..16 => 0,
                        16..20 => 11,
                        20..24 => 12,
                        _ => 4,
                    };
                    for vertex in quad {
                        vertex.layer = layer;
                    }
                }
            }
            let mut batch = EntityBatch::default();
            equivalent(&mut batch, &entities, &additive, &hand);
            for _ in 0..50 {
                black_box(baseline(black_box(&entities), &additive, &hand));
                black_box(batch.update(black_box(&entities), &additive, false));
                black_box(batch.combined(&entities, &additive, &hand, &[]));
            }
            let mut before = Vec::new();
            let mut after = Vec::new();
            for _ in 0..15 {
                let start = Instant::now();
                for _ in 0..100 {
                    black_box(baseline(black_box(&entities), &additive, &hand));
                }
                before.push(start.elapsed().as_nanos() / 100);
                let start = Instant::now();
                for _ in 0..100 {
                    black_box(batch.update(black_box(&entities), &additive, false));
                    black_box(batch.combined(&entities, &additive, &hand, &[]));
                }
                after.push(start.elapsed().as_nanos() / 100);
            }
            before.sort_unstable();
            after.sort_unstable();
            println!(
                "{name}: {quads} quads, baseline={} ns/frame optimized={} ns/frame speedup={:.2}x scratch={} bytes",
                before[7],
                after[7],
                before[7] as f64 / after[7] as f64,
                batch
                    .runs
                    .iter()
                    .map(|b| b.capacity() * std::mem::size_of::<(usize, usize, usize)>())
                    .sum::<usize>()
            );
        }
    }

    /// `place` finds every entity vertex where `write` put it.
    #[test]
    fn places_match_write() {
        let layers = [0, 11, 0, 0, 14, 35, 0, 11, 32, 0];
        let entities: Vec<Vertex> = layers
            .iter()
            .enumerate()
            .flat_map(|(q, &layer)| {
                (0..4).map(move |i| Vertex {
                    pos: [(q * 4 + i) as f32, 0.0, 0.0],
                    uv: [0.0; 2],
                    layer,
                    color: 0,
                    light: 0,
                    normal: [0; 4],
                    glint_uv: [0.0; 2],
                })
            })
            .collect();
        let mut batch = EntityBatch::default();
        batch.update(&entities, &[], false);
        let combined = batch.combined(&entities, &[], &[], &[]);
        let mut places = Vec::new();
        batch.places(&mut places);
        for (i, vertex) in entities.iter().enumerate() {
            let at = EntityBatch::place(&places, &mut 0, i as u32).expect("every vertex is laid out");
            assert_eq!(combined[at as usize].pos, vertex.pos);
        }
        assert_eq!(EntityBatch::place(&places, &mut 0, entities.len() as u32), None);
    }

    /// A held quad leaves room for the quads it stands for, copies nothing there, and
    /// `place` finds where that room starts.
    #[test]
    fn held_quads_leave_room() {
        let vertex = |x: f32, layer: u32| Vertex {
            pos: [x, 0.0, 0.0],
            uv: [0.0; 2],
            layer,
            color: 0,
            light: 0,
            normal: [0; 4],
            glint_uv: [0.0; 2],
        };
        let held = |layer: u32, quads: u32| Vertex {
            light: super::super::parts::PLACEHOLDER_LIGHT,
            glint_uv: [f32::from_bits(quads), 0.0],
            ..vertex(-1.0, layer)
        };
        // A plain quad, a held quad for 6 quads, a plain quad, a held translucent item.
        let mut entities = vec![vertex(1.0, 0); 4];
        entities.extend([held(0, 6); 4]);
        entities.extend(vec![vertex(2.0, 0); 4]);
        entities.extend([held(32, 3); 4]);
        let mut batch = EntityBatch::default();
        let counts = batch.update(&entities, &[], false);
        assert_eq!(counts[0], 1 + 6 + 1);
        assert_eq!(counts[8], 3);
        assert_eq!(batch.len(&[], &[], &[]), (8 + 3) * 4);
        let combined = batch.combined(&entities, &[], &[], &[]);
        assert_eq!(combined[0].pos[0], 1.0);
        // Room for the held quad's six, untouched; then the next plain quad.
        assert!(
            combined[4..28]
                .iter()
                .all(|v| v.pos[0] == 0.0 && v.light == 0)
        );
        assert_eq!(combined[28].pos[0], 2.0);
        let mut places = Vec::new();
        batch.places(&mut places);
        assert_eq!(EntityBatch::place(&places, &mut 0, 4), Some(4));
        assert_eq!(EntityBatch::place(&places, &mut 0, 8), Some(28));
        assert_eq!(EntityBatch::place(&places, &mut 0, 12), Some(32));
    }
}
