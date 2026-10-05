import { describe, expect, it } from "vitest";

import { chunkLinks, DIRECTIONS, moveKind, offRoadBlocked, onRoad, roadLinked, type LinksAt } from "./links";

// Written by the Python builder (links.py), south-first, 8 x 8: a river
// running east-west between rows 3 and 4, and a road from the south edge
// at x = 2 crossing it at x = 3 and ending at (5, 7).
const W = 8;
const H = 8;
// prettier-ignore
const FIXTURE = new Uint8Array([
  0, 0, 8, 0, 0, 0, 0, 0,
  0, 0, 8, 0, 0, 0, 0, 0,
  0, 0, 4, 8, 0, 0, 0, 0,
  2, 2, 2, 10, 2, 2, 2, 2,
  0, 0, 0, 4, 8, 0, 0, 0,
  0, 0, 0, 0, 8, 0, 0, 0,
  0, 0, 0, 0, 16, 0, 0, 0,
  0, 0, 0, 0, 0, 0, 0, 0,
]);
const at = chunkLinks(FIXTURE, W, H);

function reach(links: LinksAt, start: [number, number], roads: boolean): Set<string> {
  const seen = new Set([start.join()]);
  const queue = [start];
  while (queue.length) {
    const [x, y] = queue.shift()!;
    for (const [dx, dy] of DIRECTIONS) {
      const nx = x + dx;
      const ny = y + dy;
      if (nx < 0 || nx >= W || ny < 0 || ny >= H || seen.has(`${nx},${ny}`)) continue;
      const kind = moveKind(links, x, y, dx, dy);
      if (kind === "offRoad" || (roads && kind === "road")) {
        seen.add(`${nx},${ny}`);
        queue.push([nx, ny]);
      }
    }
  }
  return seen;
}

describe("links.bin", () => {
  it("walls the river off-road, straight and diagonal, and not along it", () => {
    expect(offRoadBlocked(at, 1, 3, 0, 1)).toBe(true);
    expect(offRoadBlocked(at, 1, 4, 0, -1)).toBe(true);
    expect(offRoadBlocked(at, 1, 3, 1, 1)).toBe(true);
    expect(offRoadBlocked(at, 1, 4, -1, -1)).toBe(true);
    expect(offRoadBlocked(at, 1, 3, 1, 0)).toBe(false);
    expect(offRoadBlocked(at, 1, 4, -1, 0)).toBe(false);
  });

  it("reads each road link the same from both ends", () => {
    for (let x = 0; x < W; x += 1) {
      for (let y = 0; y < H; y += 1) {
        for (const [dx, dy] of DIRECTIONS) {
          expect(roadLinked(at, x, y, dx, dy)).toBe(roadLinked(at, x + dx, y + dy, -dx, -dy));
        }
      }
    }
  });

  it("crosses only by the bridge", () => {
    expect(moveKind(at, 3, 3, 0, 1)).toBe("road");
    expect(moveKind(at, 4, 3, 0, 1)).toBe("blocked");
    expect(reach(at, [0, 0], false).has("0,7")).toBe(false);
    expect(reach(at, [0, 0], true).has("0,7")).toBe(true);
  });

  it("knows which pixels the road passes", () => {
    const road = ["2,0", "2,1", "2,2", "3,2", "3,3", "3,4", "4,4", "4,5", "4,6", "5,7"];
    for (let x = 0; x < W; x += 1) {
      for (let y = 0; y < H; y += 1) {
        expect(onRoad(at, x, y)).toBe(road.includes(`${x},${y}`));
      }
    }
  });

  it("refuses a file of the wrong size", () => {
    expect(() => chunkLinks(new Uint8Array(10), W, H)).toThrow();
  });
});
