import { describe, expect, it } from "vitest";

import { boardBounds, distanceM, isOnBoard } from "../lib/board";
import { DEFAULT_ORIGIN, placedFromList } from "../lib/forceBuilder";
import { SYMMETRIC_CONTROL_V1 } from "../rules/forceList";
import { moveBoard, onBoard } from "./board";

describe("moving placed units with the board", () => {
  const elsewhere = { lat: 54.7, lng: 20.5 };

  it("puts a force list on a board that has moved, keeping its layout", () => {
    const list = placedFromList(SYMMETRIC_CONTROL_V1);
    const moved = onBoard(list, elsewhere);
    for (const element of moved) expect(isOnBoard(element.position, boardBounds(elsewhere))).toBe(true);
    // Same spacing as before.
    expect(distanceM(moved[0].position, moved[1].position)).toBeCloseTo(
      distanceM(list[0].position, list[1].position),
      -1,
    );
  });

  it("brings units back when the board moves back", () => {
    const list = placedFromList(SYMMETRIC_CONTROL_V1);
    const there = moveBoard(list, DEFAULT_ORIGIN, elsewhere);
    const back = moveBoard(there, elsewhere, DEFAULT_ORIGIN);
    for (const [index, element] of back.entries()) {
      expect(distanceM(element.position, list[index].position)).toBeLessThan(5);
    }
  });

  it("leaves units alone when the board has not moved", () => {
    const list = placedFromList(SYMMETRIC_CONTROL_V1);
    expect(moveBoard(list, DEFAULT_ORIGIN, DEFAULT_ORIGIN)).toBe(list);
  });
});
