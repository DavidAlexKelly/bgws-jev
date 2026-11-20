// A building's ghost outline: the footprint swept along the lean. Run: npm test.

import assert from "node:assert/strict";
import test from "node:test";
import { area, ghostOutline } from "./ghost.ts";

test("a box swept along a shift covers its footprint, the shifted one and the band between, once", () => {
  // 40 × 12, swept 6 east and 9 north (y south): 480 + 6·12 + 9·40.
  const g = ghostOutline(
    [
      [0, 0],
      [40, 0],
      [40, 12],
      [0, 12],
    ],
    [6, -9],
  );
  assert.equal(g.length, 1, "one shape");
  assert.equal(g[0].length, 1, "no holes");
  assert.ok(Math.abs(area(g) - (480 + 72 + 360)) < 1e-6, `area ${area(g)}`);
});

test("an L-shaped block keeps its inner corner where the sweep does not reach it", () => {
  // An L: 30 × 30 less its 20 × 20 north-east corner; swept 2 east. Its notch stays open.
  const l: [number, number][] = [
    [0, 0],
    [10, 0],
    [10, 20],
    [30, 20],
    [30, 30],
    [0, 30],
  ];
  const g = ghostOutline(l, [2, 0]);
  // Footprint 500, plus a 2-wide band along each east-facing side: 2·20 (the notch's west side) + 2·10 (the east end).
  assert.ok(Math.abs(area(g) - (500 + 40 + 20)) < 1e-6, `area ${area(g)}`);
});

test("no shift, no ghost", () => {
  assert.deepEqual(ghostOutline([[0, 0], [1, 0], [1, 1]], [0, 0]), []);
});
