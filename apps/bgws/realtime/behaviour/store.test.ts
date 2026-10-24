import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  allTemplates,
  assignTemplate,
  assignedCount,
  customTemplates,
  deleteCustomTemplate,
  freeCustomId,
  isAssigned,
  saveCustomTemplate,
  subscribe,
  templateFor,
  DEFAULT_TEMPLATE_ID,
  assignSide,
  behaviourSnapshot,
  resolveBehaviour,
  sideUsesOf,
} from "./store";

function memoryStorage(): Storage {
  const data = new Map<string, string>();
  return {
    get length() { return data.size; },
    clear: () => data.clear(),
    getItem: (k) => data.get(k) ?? null,
    key: (i) => [...data.keys()][i] ?? null,
    removeItem: (k) => void data.delete(k),
    setItem: (k, v) => void data.set(k, String(v)),
  };
}

beforeEach(() => {
  vi.stubGlobal("localStorage", memoryStorage());
});

const standFast = { id: "custom/stand-fast", version: 1, name: "Stand fast", parent: DEFAULT_TEMPLATE_ID, remove: ["pullBack"] };

describe("behaviour store", () => {
  it("lists the built-in templates, and saves a valid variant", () => {
    expect(allTemplates().map((t) => t.id)).toContain(DEFAULT_TEMPLATE_ID);
    const notified = vi.fn();
    const off = subscribe(notified);
    expect(saveCustomTemplate(standFast)).toEqual([]);
    off();
    expect(notified).toHaveBeenCalled();
    expect(customTemplates().map((t) => t.id)).toEqual(["custom/stand-fast"]);
  });

  it("refuses built-in ids and invalid templates, storing nothing", () => {
    expect(saveCustomTemplate({ ...standFast, id: "core/default" })[0]).toMatch(/built-in/);
    // "keep" is D1's fallback: removing it must be refused.
    expect(saveCustomTemplate({ ...standFast, remove: ["keep"] }).join("\n")).toMatch(/fallback/);
    expect(saveCustomTemplate({ ...standFast, parent: "custom/nope" }).join("\n")).toMatch(/Unknown template/);
    expect(customTemplates()).toEqual([]);
  });

  it("assigns per asset, falls back to the default, and counts uses", () => {
    expect(templateFor("m1a2")).toBe(DEFAULT_TEMPLATE_ID);
    saveCustomTemplate(standFast);
    assignTemplate("m1a2", "custom/stand-fast");
    expect(templateFor("m1a2")).toBe("custom/stand-fast");
    expect(isAssigned("m1a2")).toBe(true);
    expect(assignedCount("custom/stand-fast")).toBe(1);
    assignTemplate("m1a2", null);
    expect(templateFor("m1a2")).toBe(DEFAULT_TEMPLATE_ID);
  });

  it("moves assignments and children when a template is renamed", () => {
    saveCustomTemplate(standFast);
    saveCustomTemplate({ id: "custom/child", version: 1, parent: "custom/stand-fast" });
    assignTemplate("m1a2", "custom/stand-fast");
    expect(saveCustomTemplate({ ...standFast, id: "custom/hold-fast" }, "custom/stand-fast")).toEqual([]);
    expect(templateFor("m1a2")).toBe("custom/hold-fast");
    expect(customTemplates().find((t) => t.id === "custom/child")?.parent).toBe("custom/hold-fast");
  });

  it("deletes, sending assets back to their default — but not while others build on it", () => {
    saveCustomTemplate(standFast);
    saveCustomTemplate({ id: "custom/child", version: 1, parent: "custom/stand-fast" });
    assignTemplate("m1a2", "custom/stand-fast");
    expect(deleteCustomTemplate("custom/stand-fast")[0]).toMatch(/build on it/);
    expect(deleteCustomTemplate("custom/child")).toEqual([]);
    expect(deleteCustomTemplate("custom/stand-fast")).toEqual([]);
    expect(templateFor("m1a2")).toBe(DEFAULT_TEMPLATE_ID);
    expect(deleteCustomTemplate(DEFAULT_TEMPLATE_ID)[0]).toMatch(/built-in/);
  });

  it("finds a free id for a name", () => {
    expect(freeCustomId("Stand Fast!")).toBe("custom/stand-fast");
    saveCustomTemplate(standFast);
    expect(freeCustomId("Stand fast")).toBe("custom/stand-fast-2");
  });

  it("side assignments: Red armour uses Stand fast; most specific wins", () => {
    saveCustomTemplate(standFast);
    saveCustomTemplate({ id: "custom/cautious", version: 1, parent: DEFAULT_TEMPLATE_ID, bias: { pullBack: 1 } });
    assignTemplate("m1a2", "custom/cautious", "armoured_vehicle");
    assignSide("red", "armoured_vehicle", "custom/stand-fast");
    const tank = (side: "blue" | "red") => ({ platform: "m1a2", side, targetClass: "armoured_vehicle" });
    // Red's side assignment beats the type's; Blue keeps the type's.
    expect(resolveBehaviour(tank("red"))).toEqual({ templateId: "custom/stand-fast", why: "side" });
    expect(resolveBehaviour(tank("blue"))).toEqual({ templateId: "custom/cautious", why: "asset" });
    // Infantry on Red is not armour: its kind's default.
    expect(resolveBehaviour({ platform: "rifles", side: "red", targetClass: "foot" })).toEqual({ templateId: "core/infantry", why: "default" });
    expect(sideUsesOf("custom/stand-fast")).toEqual([{ side: "red", kind: "armoured_vehicle" }]);

    const snap = behaviourSnapshot([
      { id: "R1", ...tank("red") },
      { id: "B1", ...tank("blue") },
      { id: "R2", platform: "rifles", side: "red", targetClass: "foot" },
    ]);
    expect(snap.behaviours).toEqual({ R1: "custom/stand-fast", B1: "custom/cautious" });
    expect(snap.why).toEqual({ R1: "side", B1: "asset" });

    // Cleared: back to the type's.
    assignSide("red", "armoured_vehicle", null);
    expect(resolveBehaviour(tank("red")).why).toBe("asset");
  });
});
