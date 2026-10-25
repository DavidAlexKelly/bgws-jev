import { createElement } from "react";
import { renderToString } from "react-dom/server";
import { describe, expect, it } from "vitest";
import BehaviourGraph from "./BehaviourGraph";
import { bundledTemplateFiles } from "./realtime/behaviour/load";
import type { Template } from "./realtime/behaviour/types";

describe("the behaviour graph view", () => {
  it("draws every choose node's options, and says a built-in template is read-only", () => {
    const all = Object.values(bundledTemplateFiles()) as Template[];
    const t = all.find((x) => x.id === "core/default")!;
    const html = renderToString(createElement(BehaviourGraph, { template: t, templates: all }));
    expect(html).toMatch(/<svg/);
    for (const [id, n] of Object.entries(t.nodes ?? {})) if (n.kind === "choose") expect(html, id).toContain(n.options[0].id);
    expect(html).toContain("read-only");
  });
});
