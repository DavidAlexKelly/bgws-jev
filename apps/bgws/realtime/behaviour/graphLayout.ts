// ── behaviour/graphLayout.ts ─────────────────────────────────────────────
// The graph view's layout (DECISION_FLOW_GRAPHS.md §5.1, step 10): a resolved
// template as positioned boxes and edges, triggers on the left, each node in
// the column of its shortest path from a trigger, choose nodes tall enough
// for their options as ports. Pure: the canvas (BehaviourGraph.tsx) draws it.

import type { ResolvedTemplate } from "./types";

export interface LaidNode {
  id: string;
  kind: "trigger" | "choose" | "branch" | "do";
  label: string;
  x: number;
  y: number;
  w: number;
  h: number;
  /** Choose nodes: one row per option, with whether the template removes it. */
  options?: { id: string; action: string; removed: boolean; bias: number; conditional: boolean }[];
  /** Set by this template itself (not inherited). */
  own?: boolean;
}

export interface LaidEdge {
  from: string;
  to: string;
  label?: string;
  /** From an option row (choose) or a case (branch): which row it leaves. */
  fromRow?: number;
}

export const COL_W = 260;
export const ROW_H = 18;
const GAP = 24;

export function layoutTemplate(t: ResolvedTemplate, own: ReadonlySet<string> = new Set()): { nodes: LaidNode[]; edges: LaidEdge[]; width: number; height: number } {
  const depth = new Map<string, number>();
  const edges: LaidEdge[] = [];
  const queue: string[] = [];
  for (const [trigger, node] of Object.entries(t.triggers).sort()) {
    edges.push({ from: `trigger:${trigger}`, to: node });
    if (!depth.has(node)) {
      depth.set(node, 1);
      queue.push(node);
    }
  }
  while (queue.length) {
    const id = queue.shift()!;
    const n = t.nodes[id];
    if (!n || n.kind !== "branch") continue;
    const nexts = [...n.cases.map((c, i) => ({ to: c.next, row: i, label: "when" })), { to: n.else, row: n.cases.length, label: "else" }];
    for (const e of nexts) {
      edges.push({ from: id, to: e.to, label: e.label, fromRow: e.row });
      if (!depth.has(e.to)) {
        depth.set(e.to, depth.get(id)! + 1);
        queue.push(e.to);
      }
    }
  }
  // Nodes no trigger reaches still get a place, in the last column.
  const maxDepth = Math.max(1, ...depth.values());
  for (const id of Object.keys(t.nodes)) if (!depth.has(id)) depth.set(id, maxDepth + 1);

  const columns = new Map<number, string[]>();
  columns.set(0, Object.keys(t.triggers).sort().map((k) => `trigger:${k}`));
  for (const [id, d] of [...depth.entries()].sort((a, b) => a[1] - b[1] || a[0].localeCompare(b[0]))) columns.set(d, [...(columns.get(d) ?? []), id]);

  const nodes: LaidNode[] = [];
  let height = 0;
  for (const [col, ids] of [...columns.entries()].sort((a, b) => a[0] - b[0])) {
    let y = GAP;
    for (const id of ids) {
      let node: LaidNode;
      if (id.startsWith("trigger:")) {
        node = { id, kind: "trigger", label: id.slice(8), x: col * COL_W + GAP, y, w: COL_W - 2 * GAP, h: ROW_H * 1.5 };
      } else {
        const n = t.nodes[id];
        if (!n) continue;
        if (n.kind === "choose") {
          const options = n.options.map((o) => ({ id: o.id, action: o.action, removed: t.remove.has(o.id), bias: t.bias[o.id] ?? o.bias ?? 0, conditional: o.when != null || t.when[o.id] != null }));
          node = { id, kind: "choose", label: n.title, x: col * COL_W + GAP, y, w: COL_W - 2 * GAP, h: ROW_H * (2 + options.length), options, own: own.has(id) };
        } else if (n.kind === "branch") {
          node = { id, kind: "branch", label: `${id}: ${n.cases.length} case${n.cases.length === 1 ? "" : "s"} + else`, x: col * COL_W + GAP, y, w: COL_W - 2 * GAP, h: ROW_H * (2 + n.cases.length), own: own.has(id) };
        } else {
          node = { id, kind: "do", label: `do ${n.action}`, x: col * COL_W + GAP, y, w: COL_W - 2 * GAP, h: ROW_H * 1.5, own: own.has(id) };
        }
      }
      nodes.push(node);
      y += node.h + GAP;
    }
    height = Math.max(height, y);
  }
  return { nodes, edges, width: (Math.max(...columns.keys()) + 1) * COL_W, height };
}
