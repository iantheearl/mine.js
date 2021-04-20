import fs from 'fs';
import path from 'path';
import zlib from 'zlib';

import vec3 from 'gl-vec3';
import ndarray from 'ndarray';

import { Coords2, Coords3, Helper } from '../../shared';

import { World } from '.';

type ChunkOptionsType = {
  size: number;
  maxHeight: number;
  dimension: number;
};

class Chunk {
  public voxels: ndarray;
  public lights: ndarray;
  public heightMap: ndarray;

  public min: Coords3;
  public max: Coords3;

  public needsSaving = false;

  constructor(public coords: Coords2, public world: World, public options: ChunkOptionsType) {
    const { size, maxHeight } = options;
    const [cx, cz] = coords;
    const coords3 = [cx, 0, cz];

    this.voxels = ndarray(new Uint8Array(size * maxHeight * size), [size, maxHeight, size]);
    this.lights = ndarray(new Uint8Array(size * maxHeight * size), [size, maxHeight, size]);
    this.heightMap = ndarray(new Uint8Array(size * size), [size, size]);

    vec3.copy(this.min, coords3);
    vec3.copy(this.max, coords3);
    vec3.scale(this.min, this.min, size);
    vec3.add(this.max, this.max, [1, 0, 1]);
    vec3.scale(this.max, this.max, size);
  }

  getVoxel = (voxel: Coords3) => {
    return this.voxels.get(...voxel);
  };

  setVoxel = (voxel: Coords3, type: number) => {
    return this.voxels.set(...voxel, type);
  };

  getLight = (voxel: Coords3) => {
    return this.lights.get(...voxel);
  };

  setLight = (voxel: Coords3, level: number) => {
    return this.lights.set(...voxel, level);
  };

  getMaxHeight = (column: Coords2) => {
    return this.heightMap.get(...column);
  };

  setMaxHeight = (column: Coords2, height: number) => {
    return this.heightMap.set(...column, height);
  };

  contains = (voxel: Coords3, padding = 0) => {
    const { size, maxHeight } = this.options;
    const [lx, ly, lz] = this.toLocal(voxel);

    return lx >= -padding && lx < size + padding && ly >= 0 && ly < maxHeight && lz >= -padding && lz < size + padding;
  };

  load = () => {
    // load from existing files

    const {
      options: { storage },
    } = this.world;

    const fileBuffer = fs.readFileSync(path.join(storage, `${Helper.getChunkName(this.coords)}.json`), 'utf8');
    const { voxels, lights } = JSON.parse(fileBuffer);
    this.needsSaving = false;
    this.voxels.data = zlib.inflateSync(Buffer.from(voxels, 'base64'));
    this.lights.data = zlib.inflateSync(Buffer.from(lights, 'base64'));
    this.generateHeightMap();
  };

  save = () => {
    // save to file system
    const {
      options: { storage },
    } = this.world;

    fs.writeFileSync(
      path.join(storage, `${Helper.getChunkName(this.coords)}.json`),
      JSON.stringify({
        voxels: zlib.deflateSync(this.voxels.data as Uint8Array).toString('base64'),
        lights: zlib.deflateSync(this.lights.data as Uint8Array).toString('base64'),
      }),
    );

    this.needsSaving = false;
  };

  generate = () => {
    // generate terrain, height map, and mesh
  };

  generateHeightMap = () => {
    // generate 2d height map for lighting
    const { size, maxHeight } = this.options;

    for (let lx = 0; lx < size; lx++) {
      for (let lz = 0; lz < size; lz++) {
        for (let ly = maxHeight - 1; ly >= 0; ly--) {
          // TODO: air check
          if (ly === 0 || this.voxels.get(lx, ly, lz) !== 0) {
            this.heightMap.set(lx, lz, ly);
            break;
          }
        }
      }
    }
  };

  propagate = () => {
    // light propagation
  };

  floodLight = () => {
    // flood light from source
  };

  removeLight = () => {
    // remove light and back-propagate
  };

  update = () => {
    // update a voxel and rebuild mesh
  };

  remesh = () => {
    // rebuild mesh
  };

  private toLocal = (voxel: Coords3) => {
    const [vx, vy, vz] = voxel;
    return vec3.sub([0, 0, 0], [vx, vy, vz], this.min);
  };
}

export { Chunk };