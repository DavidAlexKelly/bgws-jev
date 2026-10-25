// Templates shared through Foundry (DFG step 9): round trip, versions and conflicts.
import { describe, expect, it } from "vitest";
import { foundryTemplateRemote, TEMPLATE_ACTIONS, TEMPLATE_OBJECT_TYPE } from "../../data/templateStore";
import { memoryRemote, publish, pull, type SharedTemplate } from "./shared";
import type { Template } from "./types";

const standFast: Template = { id: "custom/tank-stand-fast", version: 1, name: "Tank: stand fast", parent: "core/default", remove: ["pullBack"] };
const child: Template = { id: "custom/tank-stand-fast-night", version: 1, name: "…at night", parent: "custom/tank-stand-fast" };

describe("shared templates", () => {
  it("round-trip: published, then pulled into another browser as the same template", async () => {
    const remote = memoryRemote();
    const r = await publish(remote, [standFast, child], [standFast, child], "a.user");
    expect(r.created).toEqual([standFast.id, child.id]);
    const elsewhere = await pull(remote, []);
    expect(elsewhere.templates).toEqual([standFast, child]);
    expect(elsewhere.added.length).toBe(2);
    expect(remote.objects.get(child.id)!.parentVersion).toBe(1);
  });

  it("a newer shared version is never overwritten; pulling takes it", async () => {
    const remote = memoryRemote();
    await publish(remote, [{ ...standFast, version: 3 }], [standFast]);
    const r = await publish(remote, [standFast], [standFast]);
    expect(r.conflicts).toEqual([{ id: standFast.id, mine: 1, theirs: 3 }]);
    const p = await pull(remote, [standFast]);
    expect(p.updated).toEqual([standFast.id]);
    expect(p.templates[0].version).toBe(3);
  });

  it("flags a child built on a parent version that has moved on", async () => {
    const remote = memoryRemote();
    await publish(remote, [standFast, child], [standFast, child]);
    await publish(remote, [{ ...standFast, version: 2 }], [standFast, child]);
    expect((await pull(remote, [])).staleParents).toEqual([child.id]);
  });

  it("the Foundry remote lists objects and applies the actions through the Ontology API", async () => {
    const calls: { method: string; url: string; body?: unknown }[] = [];
    const store: SharedTemplate[] = [];
    const fake = async (url: string, init: { method: string; body?: string }) => {
      calls.push({ method: init.method, url, ...(init.body ? { body: JSON.parse(init.body) } : {}) });
      if (init.method === "POST") store.push((JSON.parse(init.body!) as { parameters: SharedTemplate }).parameters);
      return { ok: true, status: 200, json: async () => (init.method === "GET" ? { data: store } : {}), text: async () => "" };
    };
    const remote = foundryTemplateRemote({ baseUrl: "https://foundry.example/", tokenProvider: async () => "t" }, "ri.ontology.x", fake);
    await publish(remote, [standFast], [standFast]);
    const back = await pull(remote, []);
    expect(back.templates).toEqual([standFast]);
    expect(calls[0].url).toBe(`https://foundry.example/api/v2/ontologies/ri.ontology.x/objects/${TEMPLATE_OBJECT_TYPE}?pageSize=500`);
    expect(calls[1].url).toBe(`https://foundry.example/api/v2/ontologies/ri.ontology.x/actions/${TEMPLATE_ACTIONS.create}/apply`);
  });
});
