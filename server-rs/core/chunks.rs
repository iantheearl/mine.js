#![allow(dead_code)]

// use rayon::prelude::*;
use std::collections::{HashMap, VecDeque};

use log::info;

use crate::{
    libs::types::{Block, Coords2, Coords3, MeshType, UV},
    utils::convert::{
        get_chunk_name, get_position_name, get_voxel_name, map_voxel_to_chunk, map_world_to_voxel,
    },
};

use super::{
    chunk::{Chunk, Meshes},
    constants::{
        BlockFace, CornerData, CornerSimplified, PlantFace, AO_TABLE, BLOCK_FACES,
        CHUNK_HORIZONTAL_NEIGHBORS, CHUNK_NEIGHBORS, PLANT_FACES, VOXEL_NEIGHBORS,
    },
    registry::{get_texture_type, Registry},
    world::WorldMetrics,
};

/// Node of a light propagation queue
struct LightNode {
    voxel: Coords3<i32>,
    level: u32,
}

/// Light data of a single vertex
struct VertexLight {
    count: u32,
    torch_light: u32,
    sunlight: u32,
}

/// A wrapper around all the chunks
#[derive(Debug)]
pub struct Chunks {
    pub metrics: WorldMetrics,
    max_loaded_chunks: i32,
    chunks: HashMap<String, Chunk>,
    registry: Registry,
}

/**
 * THIS CODE IS REALLY REALLY BAD
 * NEED REFACTOR ASAP
 */
impl Chunks {
    pub fn new(metrics: WorldMetrics, max_loaded_chunks: i32, registry: Registry) -> Self {
        Chunks {
            metrics,
            max_loaded_chunks,
            chunks: HashMap::new(),
            registry,
        }
    }

    pub fn len(&self) -> usize {
        self.chunks.len()
    }

    /// Return all chunks as raw
    pub fn all(&self) -> Vec<&Chunk> {
        self.chunks.values().collect()
    }

    /// Return a mutable chunk regardless initialization
    pub fn raw(&mut self, coords: &Coords2<i32>) -> Option<&mut Chunk> {
        self.get_chunk_mut(coords)
    }

    /// Return a chunk references only if chunk is fully initialized (generated and decorated)
    pub fn get(&mut self, coords: &Coords2<i32>) -> Option<&Chunk> {
        let chunk = self.get_chunk(coords);
        let neighbors = self.neighbors(coords);

        match chunk {
            None => {
                return None;
            }
            Some(chunk) => {
                if chunk.needs_terrain
                    || chunk.needs_decoration
                    || neighbors.iter().any(|&c| c.is_none())
                    || neighbors.iter().any(|&c| c.unwrap().needs_decoration)
                {
                    return None;
                }
                chunk
            }
        };

        self.remesh_chunk(coords);

        return self.get_chunk(coords);
    }

    /// To preload chunks surrounding 0,0
    pub fn preload(&mut self, width: i16) {
        self.load(Coords2(0, 0), width);
    }

    /// Generate chunks around a certain coordinate
    pub fn generate(&mut self, coords: Coords2<i32>, render_radius: i16) {
        info!(
            "Generating chunks surrounding {:?} with radius {}",
            coords, render_radius
        );

        self.load(coords, render_radius);
    }

    /// Unload chunks when too many chunks are loaded.
    pub fn unload() {
        todo!();
    }

    /// Remesh a chunk, propagating itself and its neighbors then mesh.
    pub fn remesh_chunk(&mut self, coords: &Coords2<i32>) {
        // propagate light first
        let chunk = self.get_chunk(coords).unwrap();

        if !chunk.is_dirty {
            return;
        }

        if chunk.needs_propagation {
            self.propagate_chunk(coords);
        }

        // propagate neighboring chunks too
        for [ox, oz] in CHUNK_NEIGHBORS.iter() {
            let n_coords = Coords2(coords.0 + ox, coords.1 + oz);
            if self.get_chunk(&n_coords).unwrap().needs_propagation {
                self.propagate_chunk(&n_coords);
            }
        }

        // TODO: MESH HERE (AND SUB MESHES)
        let opaque = self.mesh_chunk(coords, false);
        let transparent = self.mesh_chunk(coords, true);

        let chunk = self.get_chunk_mut(coords).unwrap();
        chunk.meshes = Meshes {
            opaque,
            transparent,
        };

        chunk.is_dirty = false
    }

    /// Load in chunks in two steps:
    ///
    /// 1. Generate the terrain within `terrain_radius`
    /// 2. Populate the terrains within `decorate_radius` with decoration
    ///
    /// Note: `decorate_radius` should always be less than `terrain_radius`
    fn load(&mut self, coords: Coords2<i32>, render_radius: i16) {
        let Coords2(cx, cz) = coords;

        let mut to_generate: Vec<Chunk> = Vec::new();
        let mut to_decorate: Vec<Coords2<i32>> = Vec::new();

        let terrain_radius = render_radius + 4;
        let decorate_radius = render_radius;

        for x in -terrain_radius..=terrain_radius {
            for z in -terrain_radius..=terrain_radius {
                let dist = x * x + z * z;

                if dist >= terrain_radius * terrain_radius {
                    continue;
                }

                let coords = Coords2(cx + x as i32, cz + z as i32);
                let chunk = self.get_chunk(&coords);

                if chunk.is_none() {
                    let mut new_chunk = Chunk::new(
                        coords.to_owned(),
                        self.metrics.chunk_size,
                        self.metrics.max_height,
                        self.metrics.dimension,
                    );
                    self.generate_chunk(&mut new_chunk);
                    to_generate.push(new_chunk);
                }

                if dist <= decorate_radius * decorate_radius {
                    to_decorate.push(coords.to_owned());
                }
            }
        }

        for chunk in to_generate {
            self.chunks.insert(chunk.name.to_owned(), chunk);
        }

        for coords in to_decorate.iter() {
            self.decorate_chunk(coords);
        }

        for coords in to_decorate.iter() {
            // ?
            self.generate_chunk_height_map(coords);
        }
    }

    /// Populate a chunk with preset decorations.
    fn decorate_chunk(&mut self, coords: &Coords2<i32>) {
        let chunk = self
            .get_chunk_mut(&coords)
            .expect(format!("Chunk not found {:?}", coords).as_str());

        if !chunk.needs_decoration {
            return;
        }

        chunk.needs_decoration = false;

        let Coords3(min_x, min_y, min_z) = chunk.min;

        self.set_voxel_by_voxel(min_x, min_y, min_z, 1);
        self.set_voxel_by_voxel(min_x - 1, min_y, min_z - 1, 2);
    }

    /// Centered around a coordinate, return 3x3 chunks neighboring the coordinate (not inclusive).
    fn neighbors(&self, Coords2(cx, cz): &Coords2<i32>) -> Vec<Option<&Chunk>> {
        let mut neighbors = Vec::new();

        for x in -1..=1 {
            for z in -1..1 {
                if x == 0 && z == 0 {
                    continue;
                }

                neighbors.push(self.get_chunk(&Coords2(cx + x, cz + z)));
            }
        }

        neighbors
    }

    /// Get a chunk reference from a coordinate
    fn get_chunk(&self, coords: &Coords2<i32>) -> Option<&Chunk> {
        let name = get_chunk_name(&coords);
        self.chunks.get(&name)
    }

    /// Get a mutable chunk reference from a coordinate
    fn get_chunk_mut(&mut self, coords: &Coords2<i32>) -> Option<&mut Chunk> {
        let name = get_chunk_name(&coords);
        self.chunks.get_mut(&name)
    }

    /// Get a chunk reference from a voxel coordinate
    fn get_chunk_by_voxel(&self, vx: i32, vy: i32, vz: i32) -> Option<&Chunk> {
        let coords = map_voxel_to_chunk(&Coords3(vx, vy, vz), self.metrics.chunk_size);
        self.get_chunk(&coords)
    }

    /// Get a mutable chunk reference from a voxel coordinate
    fn get_chunk_by_voxel_mut(&mut self, vx: i32, vy: i32, vz: i32) -> Option<&mut Chunk> {
        let coords = map_voxel_to_chunk(&Coords3(vx, vy, vz), self.metrics.chunk_size);
        self.get_chunk_mut(&coords)
    }

    /// Get the voxel type at a voxel coordinate
    fn get_voxel_by_voxel(&self, vx: i32, vy: i32, vz: i32) -> u32 {
        let chunk = self
            .get_chunk_by_voxel(vx, vy, vz)
            .expect("Chunk not found.");
        chunk.get_voxel(vx, vy, vz)
    }

    /// Get the voxel type at a world coordinate
    fn get_voxel_by_world(&self, wx: f32, wy: f32, wz: f32) -> u32 {
        let Coords3(vx, vy, vz) = map_world_to_voxel(&Coords3(wx, wy, wz), self.metrics.dimension);
        self.get_voxel_by_voxel(vx, vy, vz)
    }

    /// Set the voxel type for a voxel coordinate
    fn set_voxel_by_voxel(&mut self, vx: i32, vy: i32, vz: i32, id: u32) {
        let chunk = self
            .get_chunk_by_voxel_mut(vx, vy, vz)
            .expect("Chunk not found.");
        chunk.set_voxel(vx, vy, vz, id);
        chunk.is_dirty = true;
    }

    /// Get the sunlight level at a voxel coordinate
    fn get_sunlight(&self, vx: i32, vy: i32, vz: i32) -> u32 {
        let chunk = self
            .get_chunk_by_voxel(vx, vy, vz)
            .expect("Chunk not found.");
        chunk.get_sunlight(vx, vy, vz)
    }

    /// Set the sunlight level for a voxel coordinate
    fn set_sunlight(&mut self, vx: i32, vy: i32, vz: i32, level: u32) {
        let chunk = self
            .get_chunk_by_voxel_mut(vx, vy, vz)
            .expect("Chunk not found.");
        chunk.set_sunlight(vx, vy, vz, level);
    }

    /// Get the torch light level at a voxel coordinate
    fn get_torch_light(&self, vx: i32, vy: i32, vz: i32) -> u32 {
        let chunk = self
            .get_chunk_by_voxel(vx, vy, vz)
            .expect("Chunk not found.");
        chunk.get_torch_light(vx, vy, vz)
    }

    /// Set the torch light level at a voxel coordinate
    fn set_torch_light(&mut self, vx: i32, vy: i32, vz: i32, level: u32) {
        let chunk = self
            .get_chunk_by_voxel_mut(vx, vy, vz)
            .expect("Chunk not found.");
        chunk.set_torch_light(vx, vy, vz, level);
    }

    /// Get a block type from a voxel coordinate
    fn get_block_by_voxel(&self, vx: i32, vy: i32, vz: i32) -> &Block {
        let voxel = self.get_voxel_by_voxel(vx, vy, vz);
        self.registry.get_block_by_id(voxel)
    }

    /// Get a block type from a voxel id
    fn get_block_by_id(&self, id: u32) -> &Block {
        self.registry.get_block_by_id(id)
    }

    /// Get the max height at a voxel column coordinate
    fn get_max_height(&self, vx: i32, vz: i32) -> i32 {
        let chunk = self
            .get_chunk_by_voxel(vx, 0, vz)
            .expect("Chunk not found.");
        chunk.get_max_height(vx, vz)
    }

    /// Set the max height at a voxel column coordinate
    fn set_max_height(&mut self, vx: i32, vz: i32, height: i32) {
        let chunk = self
            .get_chunk_by_voxel_mut(vx, 0, vz)
            .expect("Chunk not found.");
        chunk.set_max_height(vx, vz, height)
    }

    /// Mark a chunk for saving from a voxel coordinate
    fn mark_saving_from_voxel(&mut self, vx: i32, vy: i32, vz: i32) {
        self.get_chunk_by_voxel_mut(vx, vy, vz)
            .unwrap()
            .needs_saving = true;
    }

    /// Generate terrain for a chunk
    fn generate_chunk(&mut self, chunk: &mut Chunk) {
        let Coords3(start_x, start_y, start_z) = chunk.min;
        let Coords3(end_x, end_y, end_z) = chunk.max;

        let types = self.registry.get_type_map(vec!["Stone", "Dirt"]);
        let stone = types.get("Stone").unwrap();
        let dirt = types.get("Dirt").unwrap();

        let is_empty = true;

        for vx in start_x..end_x {
            for vz in start_z..end_z {
                for vy in start_y..end_y {
                    if vy == 10 {
                        chunk.set_voxel(vx, vy, vz, *dirt);
                    } else if vy < 10 {
                        chunk.set_voxel(vx, vy, vz, *stone)
                    }
                }
            }
        }

        chunk.is_empty = is_empty;
        chunk.needs_terrain = false;
    }

    /// Generate chunk's height map
    ///
    /// Note: the chunk should already be initialized with voxel data
    fn generate_chunk_height_map(&mut self, coords: &Coords2<i32>) {
        let size = self.metrics.chunk_size;
        let max_height = self.metrics.chunk_size;

        let registry = self.registry.clone(); // there must be better way
        let chunk = self.get_chunk_mut(coords).expect("Chunk not found.");

        for lx in 0..size {
            for lz in 0..size {
                for ly in (0..max_height).rev() {
                    let id = chunk.voxels[&[lx, ly, lz]];
                    let ly_i32 = ly as i32;

                    // TODO: CHECK FROM REGISTRY &&&&& PLANTS
                    if ly == 0 || (!registry.is_air(id) && !registry.is_plant(id)) {
                        if chunk.top_y < ly_i32 {
                            chunk.top_y = ly_i32 + 3;
                        }

                        chunk.height_map[&[lx, lz]] = ly_i32;
                        break;
                    }
                }
            }
        }
    }

    /// Propagate light on a chunk. Things this function does:
    ///
    /// 1. Spread sunlight from the very top of the chunk
    /// 2. Recognize the torch lights and flood-fill them as well
    fn propagate_chunk(&mut self, coords: &Coords2<i32>) {
        let chunk = self.get_chunk_mut(coords).expect("Chunk not found");

        let Coords3(start_x, start_y, start_z) = chunk.min;
        let Coords3(end_x, end_y, end_z) = chunk.max;

        chunk.needs_propagation = false;
        chunk.needs_saving = true;

        let max_light_level = self.metrics.max_light_level;

        let mut light_queue = VecDeque::<LightNode>::new();
        let mut sunlight_queue = VecDeque::<LightNode>::new();

        for vz in start_z..end_z {
            for vx in start_x..end_x {
                let h = self.get_max_height(vx, vz);

                for vy in (start_y..end_y).rev() {
                    let &Block {
                        is_transparent,
                        is_light,
                        light_level,
                        ..
                    } = self.get_block_by_voxel(vx, vy, vz);

                    if vy > h && is_transparent {
                        self.set_sunlight(vx, vy, vz, max_light_level);

                        for [ox, oz] in CHUNK_HORIZONTAL_NEIGHBORS.iter() {
                            let neighbor_block = self.get_block_by_voxel(vx + ox, vy, vz + oz);

                            if !neighbor_block.is_transparent {
                                continue;
                            }

                            if self.get_max_height(vx + ox, vz + oz) > vy {
                                // means sunlight should propagate here horizontally
                                if !sunlight_queue.iter().any(|LightNode { voxel, .. }| {
                                    voxel.0 == vx && voxel.1 == vy && voxel.2 == vz
                                }) {
                                    sunlight_queue.push_back(LightNode {
                                        level: max_light_level,
                                        voxel: Coords3(vx, vy, vz),
                                    })
                                }
                            }
                        }
                    }

                    // ? might be erroneous here, but this is for lights on voxels like plants
                    if is_light {
                        self.set_torch_light(vx, vy, vz, light_level);
                        light_queue.push_back(LightNode {
                            level: light_level,
                            voxel: Coords3(vx, vy, vz),
                        })
                    }
                }
            }
        }

        self.flood_light(light_queue, false);
        self.flood_light(sunlight_queue, true);
    }

    /// Flood fill light from a queue
    fn flood_light(&mut self, mut queue: VecDeque<LightNode>, is_sunlight: bool) {
        let max_height = self.metrics.max_height as i32;
        let max_light_level = self.metrics.max_light_level;

        while queue.len() != 0 {
            let LightNode { voxel, level } = queue.pop_front().unwrap();
            let Coords3(vx, vy, vz) = voxel;

            for [ox, oy, oz] in VOXEL_NEIGHBORS.iter() {
                let nvy = vy + oy;

                if nvy < 0 || nvy > max_height {
                    continue;
                }

                let nvx = vx + ox;
                let nvz = vz + oz;
                let sd = is_sunlight && *oy == -1 && level == max_light_level;
                let nl = level - if sd { 0 } else { 1 };
                let n_voxel = Coords3(nvx, nvy, nvz);
                let block_type = self.get_block_by_voxel(nvx, nvy, nvz);

                if !block_type.is_transparent
                    || (if is_sunlight {
                        self.get_sunlight(nvx, nvy, nvz)
                    } else {
                        self.get_torch_light(nvx, nvy, nvz)
                    } >= nl)
                {
                    continue;
                }

                if is_sunlight {
                    self.set_sunlight(nvx, nvy, nvz, nl);
                } else {
                    self.set_torch_light(nvx, nvy, nvz, nl);
                }

                self.mark_saving_from_voxel(nvx, nvy, nvz);

                queue.push_back(LightNode {
                    voxel: n_voxel,
                    level: nl,
                })
            }
        }
    }

    /// Remove a light source. Steps:
    ///
    /// 1. Remove the existing lights in a flood-fill fashion
    /// 2. If external light source exists, flood fill them back
    fn remove_light(&mut self, vx: i32, vy: i32, vz: i32, is_sunlight: bool) {
        let max_height = self.metrics.max_height as i32;
        let max_light_level = self.metrics.max_light_level;

        let mut fill = VecDeque::<LightNode>::new();
        let mut queue = VecDeque::<LightNode>::new();

        queue.push_back(LightNode {
            voxel: Coords3(vx, vy, vz),
            level: if is_sunlight {
                self.get_sunlight(vx, vy, vz)
            } else {
                self.get_torch_light(vx, vy, vz)
            },
        });

        if is_sunlight {
            self.set_sunlight(vx, vy, vz, 0);
        } else {
            self.set_torch_light(vx, vy, vz, 0);
        }

        self.mark_saving_from_voxel(vx, vy, vz);

        while queue.len() != 0 {
            let LightNode { voxel, level } = queue.pop_front().unwrap();
            let Coords3(vx, vy, vz) = voxel;

            for [ox, oy, oz] in VOXEL_NEIGHBORS.iter() {
                let nvy = vy + oy;

                if nvy < 0 || nvy >= max_height {
                    continue;
                }

                let nvx = vx + ox;
                let nvz = vz + oz;
                let n_voxel = Coords3(nvx, nvy, nvz);

                let nl = if is_sunlight {
                    self.get_sunlight(nvx, nvy, nvz)
                } else {
                    self.get_torch_light(nvx, nvy, nvz)
                };

                if nl == 0 {
                    continue;
                }

                // if level is less, or if sunlight is propagating downwards without stopping
                if nl < level
                    || (is_sunlight
                        && *oy == -1
                        && level == max_light_level
                        && nl == max_light_level)
                {
                    queue.push_back(LightNode {
                        voxel: n_voxel,
                        level: nl,
                    });

                    if is_sunlight {
                        self.set_sunlight(nvx, nvy, nvz, 0);
                    } else {
                        self.set_torch_light(nvx, nvy, nvz, 0);
                    }

                    self.mark_saving_from_voxel(nvx, nvy, nvz);
                } else if nl >= level {
                    if !is_sunlight || *oy != -1 || nl > level {
                        fill.push_back(LightNode {
                            voxel: n_voxel,
                            level: nl,
                        })
                    }
                }
            }
        }

        self.flood_light(fill, is_sunlight);
    }

    /// Update a voxel to a new type
    fn update(&mut self, vx: i32, vy: i32, vz: i32, id: u32) {
        // TODO: fix this code (might have better way)
        self.get_chunk_by_voxel_mut(vx, vy, vz)
            .unwrap()
            .needs_saving = true;
        let needs_propagation = self
            .get_chunk_by_voxel(vx, vy, vz)
            .unwrap()
            .needs_propagation;

        let max_height = self.metrics.max_height as i32;
        let max_light_level = self.metrics.max_light_level;

        let height = self.get_max_height(vx, vz);

        // TODO: better way? RefCell?
        let current_type = self.get_block_by_voxel(vx, vy, vz).clone();
        let updated_type = self.get_block_by_id(id).clone();

        let voxel = Coords3(vx, vy, vz);

        // updating the new block
        self.set_voxel_by_voxel(vx, vy, vz, id);

        // updating the height map
        if self.registry.is_air(id) {
            if vy == height {
                // on max height, should set max height to lower
                for y in (0..vy).rev() {
                    if y == 0 || !self.registry.is_air(self.get_voxel_by_voxel(vx, y, vz)) {
                        self.set_max_height(vx, vz, y);
                        break;
                    }
                }
            }
        } else if height < vy {
            self.set_max_height(vx, vz, vy);
        }

        // update light levels
        if !needs_propagation {
            if current_type.is_light {
                // remove leftover light
                self.remove_light(vx, vy, vz, false);
            } else if current_type.is_transparent && !updated_type.is_transparent {
                // remove light if solid block is placed
                [false, true].iter().for_each(|&is_sunlight| {
                    let level = if is_sunlight {
                        self.get_sunlight(vx, vy, vz)
                    } else {
                        self.get_torch_light(vx, vy, vz)
                    };
                    if level != 0 {
                        self.remove_light(vx, vy, vz, is_sunlight);
                    }
                });
            }

            if updated_type.is_light {
                // placing a light
                self.set_torch_light(vx, vy, vz, updated_type.light_level);
                self.flood_light(
                    VecDeque::from(vec![LightNode {
                        voxel: voxel.clone(),
                        level: updated_type.light_level,
                    }]),
                    false,
                );
            } else if updated_type.is_transparent && !current_type.is_transparent {
                // solid block removed
                [false, true].iter().for_each(|&is_sunlight| {
                    let mut queue = VecDeque::<LightNode>::new();

                    if is_sunlight && vy == max_height - 1 {
                        // propagate sunlight down
                        self.set_sunlight(vx, vy, vz, max_light_level);
                        queue.push_back(LightNode {
                            voxel: voxel.clone(),
                            level: max_light_level,
                        })
                    } else {
                        for [ox, oy, oz] in VOXEL_NEIGHBORS.iter() {
                            let nvy = vy + oy;

                            if nvy < 0 || nvy >= max_height {
                                return;
                            }

                            let nvx = vx + ox;
                            let nvz = vz + oz;
                            let n_voxel = Coords3(nvx, nvy, nvz);
                            let &Block {
                                is_light,
                                is_transparent,
                                ..
                            } = self.get_block_by_voxel(nvx, nvy, nvz);

                            // need propagation after solid block removed
                            let level = if is_sunlight {
                                self.get_sunlight(nvx, nvy, nvz)
                            } else {
                                self.get_torch_light(nvx, nvy, nvz)
                            };
                            if level != 0 && (is_transparent || (is_light && !is_sunlight)) {
                                queue.push_back(LightNode {
                                    voxel: n_voxel,
                                    level,
                                })
                            }
                        }
                    }
                    self.flood_light(queue, is_sunlight);
                })
            }
        }
    }

    /// Meshing a chunk. Poorly written. Needs refactor.
    fn mesh_chunk(&self, coords: &Coords2<i32>, transparent: bool) -> Option<MeshType> {
        let Chunk {
            min,
            max,
            top_y,
            dimension,
            ..
        } = self.get_chunk(coords).unwrap();

        let mut positions = Vec::<f32>::new();
        let mut indices = Vec::<i32>::new();
        let mut uvs = Vec::<f32>::new();
        let mut aos = Vec::<f32>::new();

        let mut smooth_sunlights_reps = Vec::<String>::new();
        let mut smooth_torch_light_reps = Vec::<String>::new();

        let &Coords3(start_x, start_y, start_z) = min;
        let &Coords3(end_x, end_y, end_z) = max;

        let mut vertex_to_light = HashMap::<String, VertexLight>::new();

        let vertex_ao = |side1: u32, side2: u32, corner: u32| -> usize {
            let num_s1 = self.registry.get_transparency_by_id(side1) as usize;
            let num_s2 = self.registry.get_transparency_by_id(side2) as usize;
            let num_c = self.registry.get_transparency_by_id(corner) as usize;

            if num_s1 == 1 && num_s2 == 1 {
                0
            } else {
                3 - (num_s1 + num_s2 + num_c)
            }
        };

        let plant_shrink = 0.6;

        for vx in start_x..end_x {
            for vy in start_y..(*top_y + 1) {
                for vz in start_z..end_z {
                    let voxel_id = self.get_voxel_by_voxel(vx, vy, vz);
                    let &Block {
                        is_solid,
                        is_transparent,
                        is_block,
                        is_plant,
                        ..
                    } = self.get_block_by_id(voxel_id);

                    // TODO: simplify this logic
                    if (is_solid || is_plant)
                        && (if transparent {
                            is_transparent
                        } else {
                            !is_transparent
                        })
                    {
                        if is_plant {
                            let [dx, dz] = [0, 0];

                            let torch_light_level = self.get_torch_light(vx, vy, vz);
                            let sunlight_level = self.get_sunlight(vx, vy, vz);

                            for PlantFace { corners, .. } in PLANT_FACES.iter() {
                                for &CornerSimplified { pos, .. } in corners.iter() {
                                    let offset = (1.0 - plant_shrink) / 2.0;
                                    let pos_x =
                                        pos[0] as f32 * plant_shrink + offset + (vx + dx) as f32;
                                    let pos_y = (pos[1] + vy) as f32;
                                    let pos_z =
                                        pos[2] as f32 * plant_shrink + offset + (vz + dz) as f32;

                                    let rep = get_position_name(&Coords3(
                                        pos_x * *dimension as f32,
                                        pos_y * *dimension as f32,
                                        pos_z * *dimension as f32,
                                    ));

                                    if vertex_to_light.contains_key(&rep) {
                                        let &VertexLight {
                                            count,
                                            torch_light,
                                            sunlight,
                                        } = vertex_to_light.get(&rep).unwrap();

                                        vertex_to_light.insert(
                                            rep.to_owned(),
                                            VertexLight {
                                                count: count + 1,
                                                torch_light: torch_light + torch_light_level,
                                                sunlight: sunlight + sunlight_level,
                                            },
                                        );
                                    } else {
                                        vertex_to_light.insert(
                                            rep.to_owned(),
                                            VertexLight {
                                                count: 1,
                                                torch_light: torch_light_level,
                                                sunlight: sunlight_level,
                                            },
                                        );
                                    }

                                    smooth_sunlights_reps.push(rep.to_owned());
                                    smooth_torch_light_reps.push(rep.to_owned());
                                }
                            }
                        } else if is_block {
                            for BlockFace { dir, corners, .. } in BLOCK_FACES.iter() {
                                let nvx = vx + dir[0];
                                let nvy = vy + dir[1];
                                let nvz = vz + dir[2];

                                let neighbor_id = self.get_voxel_by_voxel(nvx, nvy, nvz);
                                let n_block_type = self.get_block_by_id(neighbor_id);

                                if n_block_type.is_transparent
                                    && (!transparent
                                        || n_block_type.is_empty
                                        || neighbor_id != voxel_id
                                        || (n_block_type.transparent_standalone
                                            && dir[0] + dir[1] + dir[2] >= 1))
                                {
                                    let torch_light_level = self.get_torch_light(nvx, nvy, nvz);
                                    let sunlight_level = self.get_sunlight(nvx, nvy, nvz);

                                    for CornerData { pos, .. } in corners {
                                        let pos_x = pos[0] + vx;
                                        let pos_y = pos[1] + vy;
                                        let pos_z = pos[2] + vz;

                                        let rep = get_voxel_name(&Coords3(
                                            pos_x * *dimension as i32,
                                            pos_y * *dimension as i32,
                                            pos_z * *dimension as i32,
                                        ));

                                        if vertex_to_light.contains_key(&rep) {
                                            let &VertexLight {
                                                count,
                                                torch_light,
                                                sunlight,
                                            } = vertex_to_light.get(&rep).unwrap();

                                            vertex_to_light.insert(
                                                rep.to_owned(),
                                                VertexLight {
                                                    count: count + 1,
                                                    torch_light: torch_light + torch_light_level,
                                                    sunlight: sunlight + sunlight_level,
                                                },
                                            );
                                        } else {
                                            vertex_to_light.insert(
                                                rep.to_owned(),
                                                VertexLight {
                                                    count: 1,
                                                    torch_light: torch_light_level,
                                                    sunlight: sunlight_level,
                                                },
                                            );
                                        }

                                        let test_conditions = [
                                            pos_x == start_x,
                                            pos_y == start_y,
                                            pos_z == start_z,
                                            // position can be voxel + 1, thus can reach end
                                            pos_x == end_x,
                                            pos_y == end_y,
                                            pos_z == end_z,
                                            // edges
                                            pos_x == start_x && pos_y == start_y,
                                            pos_x == start_x && pos_z == start_z,
                                            pos_x == start_x && pos_y == end_y,
                                            pos_x == start_x && pos_z == end_z,
                                            pos_x == end_x && pos_y == start_y,
                                            pos_x == end_x && pos_z == start_z,
                                            pos_x == end_x && pos_y == end_y,
                                            pos_x == end_x && pos_z == end_z,
                                            pos_y == start_y && pos_z == start_z,
                                            pos_y == end_y && pos_z == start_z,
                                            pos_y == start_y && pos_z == end_z,
                                            pos_y == end_y && pos_z == end_z,
                                            // corners
                                            pos_x == start_x
                                                && pos_y == start_y
                                                && pos_z == start_z,
                                            pos_x == start_x && pos_y == start_y && pos_z == end_z,
                                            pos_x == start_x && pos_y == end_y && pos_z == start_z,
                                            pos_x == start_x && pos_y == end_y && pos_z == end_z,
                                            pos_x == end_x && pos_y == start_y && pos_z == start_z,
                                            pos_x == end_x && pos_y == start_y && pos_z == end_z,
                                            pos_x == end_x && pos_y == end_y && pos_z == start_z,
                                            pos_x == end_x && pos_y == end_y && pos_z == end_z,
                                        ];

                                        let test_offsets = [
                                            [-1, 0, 0],
                                            [0, -1, 0],
                                            [0, 0, -1],
                                            // position can be voxel + 1, thus can reach end
                                            [1, 0, 0],
                                            [0, 1, 0],
                                            [0, 0, 1],
                                            // edges
                                            [-1, -1, 0],
                                            [-1, 0, -1],
                                            [-1, 1, 0],
                                            [-1, 0, 1],
                                            [1, -1, 0],
                                            [1, 0, -1],
                                            [1, 1, 0],
                                            [1, 0, 1],
                                            [0, -1, -1],
                                            [0, 1, -1],
                                            [0, -1, 1],
                                            [0, 1, 1],
                                            // corners
                                            [-1, -1, -1],
                                            [-1, -1, 1],
                                            [-1, 1, -1],
                                            [-1, 1, 1],
                                            [1, -1, -1],
                                            [1, -1, 1],
                                            [1, 1, -1],
                                            [1, 1, 1],
                                        ];

                                        for (&check, [a, b, c]) in
                                            test_conditions.iter().zip(test_offsets.iter())
                                        {
                                            if check
                                                && self
                                                    .get_block_by_voxel(
                                                        nvx + *a,
                                                        nvy + *b,
                                                        nvz + *c,
                                                    )
                                                    .is_transparent
                                            {
                                                let torch_light_level_n = self.get_torch_light(
                                                    nvx + *a,
                                                    nvy + *b,
                                                    nvz + *c,
                                                );
                                                let sunlight_level_n =
                                                    self.get_sunlight(nvx + *a, nvy + *b, nvz + *c);
                                                let VertexLight {
                                                    count,
                                                    torch_light,
                                                    sunlight,
                                                } = vertex_to_light.remove(&rep).unwrap();

                                                vertex_to_light.insert(
                                                    rep.to_owned(),
                                                    VertexLight {
                                                        count: count + 1,
                                                        torch_light: torch_light
                                                            + torch_light_level_n,
                                                        sunlight: sunlight + sunlight_level_n,
                                                    },
                                                );
                                            }
                                        }

                                        smooth_sunlights_reps.push(rep.to_owned());
                                        smooth_torch_light_reps.push(rep.to_owned());
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        let sunlight_levels: Vec<i32> = smooth_sunlights_reps
            .iter()
            .map(|rep| {
                let VertexLight {
                    sunlight, count, ..
                } = vertex_to_light.get(rep).unwrap();
                (*sunlight as f32 / *count as f32) as i32
            })
            .collect();

        let torch_light_levels: Vec<i32> = smooth_torch_light_reps
            .iter()
            .map(|rep| {
                let VertexLight {
                    torch_light, count, ..
                } = vertex_to_light.get(rep).unwrap();
                (*torch_light as f32 / *count as f32) as i32
            })
            .collect();

        let mut i = 0;
        for vx in start_x..end_x {
            for vy in start_y..(*top_y + 1) {
                for vz in start_z..end_z {
                    let voxel_id = self.get_voxel_by_voxel(vx, vy, vz);
                    let &Block {
                        is_solid,
                        is_transparent,
                        is_block,
                        is_plant,
                        ..
                    } = self.get_block_by_id(voxel_id);

                    // TODO: simplify this logic
                    if (is_solid || is_plant)
                        && (if transparent {
                            is_transparent
                        } else {
                            !is_transparent
                        })
                    {
                        let texture = self.registry.get_texture_by_id(voxel_id);
                        let texture_type = get_texture_type(texture);
                        let uv_map = self.registry.get_uv_by_id(voxel_id);

                        if is_plant {
                            let [dx, dz] = [0, 0];

                            for PlantFace { corners, mat } in PLANT_FACES.iter() {
                                let UV {
                                    start_u,
                                    end_u,
                                    start_v,
                                    end_v,
                                } = uv_map.get(texture.get(*mat).unwrap()).unwrap();
                                let ndx = (positions.len() / 3) as i32;

                                for &CornerSimplified { pos, uv } in corners.iter() {
                                    let offset = (1.0 - plant_shrink) / 2.0;
                                    let pos_x =
                                        pos[0] as f32 * plant_shrink + offset + (vx + dx) as f32;
                                    let pos_y = (pos[1] + vy) as f32;
                                    let pos_z =
                                        pos[2] as f32 * plant_shrink + offset + (vz + dz) as f32;

                                    positions.push(pos_x * *dimension as f32);
                                    positions.push(pos_y * *dimension as f32);
                                    positions.push(pos_z * *dimension as f32);

                                    uvs.push(uv[0] as f32 * (end_u - start_u) + start_u);
                                    uvs.push(uv[1] as f32 * (start_v - end_v) + end_v);

                                    aos.push(1.0);
                                }

                                indices.push(ndx);
                                indices.push(ndx + 1);
                                indices.push(ndx + 2);
                                indices.push(ndx + 2);
                                indices.push(ndx + 1);
                                indices.push(ndx + 3);

                                i += 4;
                            }
                        } else if is_block {
                            let is_mat_1 = texture_type == "mat1";
                            let is_mat_3 = texture_type == "mat3";

                            for BlockFace {
                                dir,
                                mat3,
                                mat6,
                                corners,
                                neighbors,
                            } in BLOCK_FACES.iter()
                            {
                                let nvx = vx + dir[0];
                                let nvy = vy + dir[1];
                                let nvz = vz + dir[2];

                                let neighbor_id = self.get_voxel_by_voxel(nvx, nvy, nvz);
                                let n_block_type = self.get_block_by_id(neighbor_id);

                                if n_block_type.is_transparent
                                    && (!transparent
                                        || n_block_type.is_empty
                                        || neighbor_id != voxel_id
                                        || (n_block_type.transparent_standalone
                                            && dir[0] + dir[1] + dir[2] >= 1))
                                {
                                    let near_voxels: Vec<u32> = neighbors
                                        .iter()
                                        .map(|[a, b, c]| {
                                            self.get_voxel_by_voxel(vx + a, vy + b, vz + c)
                                        })
                                        .collect();

                                    let UV {
                                        start_u,
                                        end_u,
                                        start_v,
                                        end_v,
                                    } = if is_mat_1 {
                                        uv_map.get(texture.get("all").unwrap()).unwrap()
                                    } else {
                                        if is_mat_3 {
                                            uv_map.get(texture.get(*mat3).unwrap()).unwrap()
                                        } else {
                                            uv_map.get(texture.get(*mat6).unwrap()).unwrap()
                                        }
                                    };

                                    let ndx = (positions.len() / 3) as i32;
                                    let mut face_aos = vec![];

                                    for CornerData {
                                        pos,
                                        uv,
                                        side1,
                                        side2,
                                        corner,
                                    } in corners.iter()
                                    {
                                        let pos_x = pos[0] + vx;
                                        let pos_y = pos[1] + vy;
                                        let pos_z = pos[2] + vz;

                                        positions.push(pos_x as f32 * *dimension as f32);
                                        positions.push(pos_y as f32 * *dimension as f32);
                                        positions.push(pos_z as f32 * *dimension as f32);

                                        uvs.push(uv[0] as f32 * (end_u - start_u) + start_u);
                                        uvs.push(uv[1] as f32 * (start_v - end_v) + end_v);
                                        face_aos.push(
                                            AO_TABLE[vertex_ao(
                                                near_voxels[*side1 as usize],
                                                near_voxels[*side2 as usize],
                                                near_voxels[*corner as usize],
                                            )] / 255.0,
                                        );
                                    }

                                    let a_t = torch_light_levels[i + 0];
                                    let b_t = torch_light_levels[i + 1];
                                    let c_t = torch_light_levels[i + 2];
                                    let d_t = torch_light_levels[i + 3];

                                    let threshold = 0;

                                    /* -------------------------------------------------------------------------- */
                                    /*                     I KNOW THIS IS UGLY, BUT IT WORKS!                     */
                                    /* -------------------------------------------------------------------------- */
                                    // at least one zero
                                    let one_t0 = a_t <= threshold
                                        || b_t <= threshold
                                        || c_t <= threshold
                                        || d_t <= threshold;
                                    // one is zero, and ao rule, but only for zero AO's
                                    let ozao = a_t + d_t < b_t + c_t
                                        && face_aos[0] + face_aos[3] == face_aos[1] + face_aos[2];
                                    // all not zero, 4 parts
                                    let anzp1 = (b_t as f32 > (a_t + d_t) as f32 / 2.0
                                        && (a_t + d_t) as f32 / 2.0 > c_t as f32)
                                        || (c_t as f32 > (a_t + d_t) as f32 / 2.0
                                            && (a_t + d_t) as f32 / 2.0 > b_t as f32);
                                    // fixed two light sources colliding
                                    let anz = one_t0 && anzp1;

                                    if face_aos[0] + face_aos[3] > face_aos[1] + face_aos[2]
                                        || ozao
                                        || anz
                                    {
                                        // generate flipped quad
                                        indices.push(ndx);
                                        indices.push(ndx + 1);
                                        indices.push(ndx + 3);
                                        indices.push(ndx + 3);
                                        indices.push(ndx + 2);
                                        indices.push(ndx);
                                    } else {
                                        indices.push(ndx);
                                        indices.push(ndx + 1);
                                        indices.push(ndx + 2);
                                        indices.push(ndx + 2);
                                        indices.push(ndx + 1);
                                        indices.push(ndx + 3);
                                    }

                                    i += 4;

                                    aos.push(face_aos[0]);
                                    aos.push(face_aos[1]);
                                    aos.push(face_aos[2]);
                                    aos.push(face_aos[3]);
                                }
                            }
                        }
                    }
                }
            }
        }

        if transparent && indices.len() == 0 {
            return None;
        }

        Some(MeshType {
            aos,
            indices,
            positions,
            sunlights: sunlight_levels,
            torch_lights: torch_light_levels,
            uvs,
        })
    }
}
