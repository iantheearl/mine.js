import fs from 'fs';
import path from 'path';

import { Coords2, Coords3, Helper } from '../../shared';

import { ClientType, Network, NetworkOptionsType, Chunk, GeneratorTypes, Registry } from '.';

const chunkNeighbors = [
  { x: -1, z: -1 },
  { x: 0, z: -1 },
  { x: 1, z: -1 },
  { x: -1, z: 0 },
  { x: 1, z: 0 },
  { x: -1, z: 1 },
  { x: 0, z: 1 },
  { x: 1, z: 1 },
];

type WorldOptionsType = NetworkOptionsType & {
  storage: string;
  preload: number;
  chunkSize: number;
  dimension: number;
  maxHeight: number;
  renderRadius: number;
  maxLightLevel: number;
  useSmoothLighting: boolean;
  generation: GeneratorTypes;
};

class World extends Network {
  public registry: Registry;

  public chunks: Map<string, Chunk> = new Map();

  constructor(public options: WorldOptionsType) {
    super(options);

    this.registry = new Registry({ basePath: path.join(__dirname, '..', 'blocks') });

    this.initStorage();
    this.setupRoutes();
    this.preloadChunks();
  }

  initStorage = () => {
    // if storage doesn't exist, make directory
    const { storage } = this.options;

    if (!fs.existsSync(storage)) {
      fs.mkdirSync(storage);
    }
  };

  setupRoutes = () => {
    // texture atlas
    this.app.get('/atlas', (_, res) => {
      res.setHeader('Content-Type', 'image/png');
      this.registry.textureAtlas.canvas.createPNGStream().pipe(res);
    });
  };

  preloadChunks = () => {
    const { preload } = this.options;
    console.log(`Preloading ${(preload * 2 + 1) ** 2} chunks...`);
    for (let x = -preload; x <= preload; x++) {
      for (let z = -preload; z <= preload; z++) {
        this.getChunkByCPos([x, z]).remesh();
      }
    }
    console.log(`Preloaded ${(preload * 2 + 1) ** 2} chunks.`);
  };

  getChunkByCPos = (cCoords: Coords2) => {
    return this.getChunkByName(Helper.getChunkName(cCoords));
  };

  getChunkByName = (chunkName: string) => {
    let chunk = this.chunks.get(chunkName);
    if (!chunk) {
      const { chunkSize, dimension, maxHeight } = this.options;
      const coords = Helper.parseChunkName(chunkName) as Coords2;
      chunk = new Chunk(coords, this, {
        dimension,
        maxHeight,
        size: chunkSize,
      });
      this.chunks.set(chunkName, chunk);
    }
    return chunk;
  };

  getChunkByVoxel = (vCoords: Coords3) => {
    const { chunkSize } = this.options;
    const chunkCoords = Helper.mapVoxelPosToChunkPos(vCoords, chunkSize);
    return this.getChunkByCPos(chunkCoords);
  };

  getNeighborChunks = (coords: Coords2) => {
    const [cx, cz] = coords;
    const chunks: Chunk[] = [];
    chunkNeighbors.forEach((offset) => {
      chunks.push(this.getChunkByCPos([cx + offset.x, cz + offset.z]));
    });
    return chunks;
  };

  getVoxelByVoxel = (vCoords: Coords3) => {
    const chunk = this.getChunkByVoxel(vCoords);
    return chunk ? chunk.getVoxel(vCoords) : 0;
  };

  getVoxelByWorld = (wCoords: Coords3) => {
    const vCoords = Helper.mapWorldPosToVoxelPos(wCoords, this.options.dimension);
    return this.getVoxelByVoxel(vCoords);
  };

  getTorchLight(vCoords: Coords3) {
    const chunk = this.getChunkByVoxel(vCoords);
    return chunk.getTorchLight(vCoords) || 0;
  }

  setTorchLight(vCoords: Coords3, level: number) {
    const chunk = this.getChunkByVoxel(vCoords);
    chunk.setTorchLight(vCoords, level);
  }

  getSunlight(vCoords: Coords3) {
    const chunk = this.getChunkByVoxel(vCoords);
    return chunk.getSunlight(vCoords);
  }

  setSunlight(vCoords: Coords3, level: number) {
    const chunk = this.getChunkByVoxel(vCoords);
    return chunk.setSunlight(vCoords, level);
  }

  getMaxHeight = (column: Coords2) => {
    const chunk = this.getChunkByVoxel([column[0], 0, column[1]]);
    return chunk.getMaxHeight(column);
  };

  getSolidityByVoxel = (vCoords: Coords3) => {};

  getFluidityByVoxel = (vCoords: Coords3) => {
    return false;
  };

  getSolidityByWorld = () => {};

  getFluidityByWorld = (wCoords: Coords3) => {
    const vCoords = Helper.mapWorldPosToVoxelPos(wCoords, this.options.dimension);
    return this.getFluidityByVoxel(vCoords);
  };

  setChunk = (chunk: Chunk) => {
    return this.chunks.set(chunk.name, chunk);
  };

  setVoxel = () => {};

  onRequest = (client: ClientType, request) => {
    switch (request.type) {
      case 'REQUEST': {
        const { x, z } = request.json;
        // TODO: check x z validity
        const chunk = this.getChunkByCPos([x, z]);
        if (chunk.mesh) chunk.remesh();
        console.log(`requested: ${x} ${z}`);
        this.sendChunks(client, [chunk]);
        break;
      }
      default:
        break;
    }
  };

  onInit = (client: ClientType) => {
    const test = 2;
    const chunks: Chunk[] = [];

    for (let x = -test; x <= test; x++) {
      for (let z = -test; z <= test; z++) {
        chunks.push(this.getChunkByCPos([x, z]));
      }
    }

    client.send(
      Network.encode({
        type: 'INIT',
        json: {
          spawn: [0, this.getMaxHeight([0, 0]), 0],
        },
      }),
    );
  };

  sendChunks = (client: ClientType, chunks: Chunk[], type = 'LOAD') => {
    client.send(
      Network.encode({
        type,
        chunks: chunks.map((c) => c.protocol),
      }),
    );
  };
}

export { World };
