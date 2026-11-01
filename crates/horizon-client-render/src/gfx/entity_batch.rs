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
const MATERIALS: usize = 9;
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

/// The material of a quad by its overlay (`MATERIALS` order).
fn material(layer: u32) -> usize {
    match layer {
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
}

impl EntityBatch {
    /// Group `entities`' quads by material; `write` then lays them out in
    /// material order, followed by `additive`, `hand` and `see_through`.
    pub(super) fn update(&mut self, entities: &[Vertex]) -> [u32; MATERIALS] {
        let quads = entities.as_chunks::<4>().0;
        for runs in &mut self.runs {
            runs.clear();
        }
        let mut counts = [0usize; MATERIALS];
        let mut start = 0;
        while start < quads.len() {
            let kind = material(quads[start][0].layer);
            if let Some(held) = held_quads(&quads[start]) {
                self.runs[kind].push((start, 1, held));
                counts[kind] += held;
                start += 1;
                continue;
            }
            let end = quads[start + 1..]
                .iter()
                .position(|quad| material(quad[0].layer) != kind || held_quads(quad).is_some())
                .map_or(quads.len(), |n| start + 1 + n);
            self.runs[kind].push((start, end - start, 0));
            counts[kind] += end - start;
            start = end;
        }
        self.vertices = counts.iter().sum::<usize>() * 4;
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
        for runs in &self.runs {
            for &(first, count, held) in runs {
                places.push(((first * 4) as u32, (count * 4) as u32, at));
                at += (count.max(held) * 4) as u32;
            }
        }
        places.sort_unstable_by_key(|p| p.0);
    }

    /// The buffer vertex of entity vertex `vertex`, by `places`.
    pub(super) fn place(places: &[(u32, u32, u32)], vertex: u32) -> Option<u32> {
        let i = places.partition_point(|p| p.0 <= vertex).checked_sub(1)?;
        let (first, count, at) = places[i];
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
        mut put: impl FnMut(usize, &[Vertex]),
    ) {
        let mut at = 0;
        for runs in &self.runs {
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
        for vertices in [additive, hand, see_through] {
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
        self.write(entities, additive, hand, see_through, |at, src| {
            out[at..at + src.len()].copy_from_slice(src);
        });
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{hint::black_box, time::Instant};

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
        assert_eq!(batch.update(entities), counts);
        let combined = batch.combined(entities, additive, hand, &[]);
        assert_eq!(
            combined.len(),
            entities.len() / 4 * 4 + additive.len() + hand.len()
        );
        assert_eq!(
            bytemuck::cast_slice::<_, u8>(&combined),
            bytemuck::cast_slice::<_, u8>(&expected)
        );
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
                black_box(batch.update(black_box(&entities)));
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
                    black_box(batch.update(black_box(&entities)));
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
        batch.update(&entities);
        let combined = batch.combined(&entities, &[], &[], &[]);
        let mut places = Vec::new();
        batch.places(&mut places);
        for (i, vertex) in entities.iter().enumerate() {
            let at = EntityBatch::place(&places, i as u32).expect("every vertex is laid out");
            assert_eq!(combined[at as usize].pos, vertex.pos);
        }
        assert_eq!(EntityBatch::place(&places, entities.len() as u32), None);
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
        let counts = batch.update(&entities);
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
        assert_eq!(EntityBatch::place(&places, 4), Some(4));
        assert_eq!(EntityBatch::place(&places, 8), Some(28));
        assert_eq!(EntityBatch::place(&places, 12), Some(32));
    }
}
