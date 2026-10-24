// ── behaviour/walk.ts ────────────────────────────────────────────────────
// Resolve a template against its parents, evaluate conditions, and walk the
// graph from a trigger to the decision Jev is asked (or the drill that runs).
//
// Pure and deterministic: no randomness, no clock, no I/O. Facts come from
// the caller's FactTable, so the same template, facts and trigger always
// give the same result, and replays stay exact.

import type {
  Bias,
  Condition,
  FactTable,
  FactValue,
  GraphNode,
  ResolvedTemplate,
  Template,
  WalkResult,
} from "./types";

/** A runaway graph (a branch cycle) stops here rather than hanging the tick. */
export const MAX_WALK_STEPS = 32;

/**
 * Merge a template with its ancestors. Own triggers and nodes override the
 * parent's of the same name; removals and bias overrides accumulate down the
 * chain, child winning on bias.
 */
export function resolveTemplate(id: string, registry: ReadonlyMap<string, Template>): ResolvedTemplate {
  const chain: Template[] = [];
  const seen = new Set<string>();
  for (let next: string | undefined = id; next; ) {
    if (seen.has(next)) throw new Error(`Template inheritance loop at "${next}" (from "${id}")`);
    seen.add(next);
    const t = registry.get(next);
    if (!t) throw new Error(`Unknown template "${next}"${next === id ? "" : ` (parent in "${id}")`}`);
    chain.push(t);
    next = t.parent;
  }

  const resolved: ResolvedTemplate = {
    id,
    lineage: chain.map((t) => t.id),
    doctrine: [],
    triggers: {},
    nodes: {},
    remove: new Set(),
    bias: {},
    when: {},
  };
  // Root first, so each descendant overrides what it inherits.
  for (const t of [...chain].reverse()) {
    if (t.doctrine) resolved.doctrine.push(t.doctrine);
    Object.assign(resolved.triggers, t.triggers ?? {});
    Object.assign(resolved.nodes, t.nodes ?? {});
    for (const r of t.remove ?? []) resolved.remove.add(r);
    Object.assign(resolved.bias, t.bias ?? {});
    Object.assign(resolved.when, t.when ?? {});
  }
  return resolved;
}

function compare(actual: FactValue, op: string, expected: FactValue): boolean {
  switch (op) {
    case "==":
      return actual === expected;
    case "!=":
      return actual !== expected;
  }
  // Ordering comparisons only make sense between numbers; anything else
  // (including a missing fact, null) is false rather than coerced.
  if (typeof actual !== "number" || typeof expected !== "number") return false;
  switch (op) {
    case ">":
      return actual > expected;
    case ">=":
      return actual >= expected;
    case "<":
      return actual < expected;
    case "<=":
      return actual <= expected;
  }
  return false;
}

/** Evaluate a condition. A fact missing from the table is an error, not false. */
export function evaluate<Ctx>(cond: Condition, facts: FactTable<Ctx>, ctx: Ctx): boolean {
  if ("all" in cond) return cond.all.every((c) => evaluate(c, facts, ctx));
  if ("any" in cond) return cond.any.some((c) => evaluate(c, facts, ctx));
  if ("not" in cond) return !evaluate(cond.not, facts, ctx);
  const fact = facts[cond.fact];
  if (!fact) throw new Error(`Unknown fact "${cond.fact}"`);
  return compare(fact(ctx), cond.op, cond.value);
}

/**
 * Walk from the node `trigger` points at to the first `choose` or `do` node.
 * Returns `none` when the template has no node for this trigger, which lets
 * the caller fall back to the default template.
 */
export function walk<Ctx>(
  template: ResolvedTemplate,
  trigger: string,
  facts: FactTable<Ctx>,
  ctx: Ctx,
): WalkResult {
  const start = template.triggers[trigger];
  const path: string[] = [];
  if (!start) return { kind: "none", path };

  let nodeId = start;
  for (let step = 0; step < MAX_WALK_STEPS; step++) {
    const node: GraphNode | undefined = template.nodes[nodeId];
    if (!node) throw new Error(`Template "${template.id}": no node "${nodeId}"`);
    path.push(nodeId);

    if (node.kind === "branch") {
      const hit = node.cases.find((c) => evaluate(c.when, facts, ctx));
      nodeId = hit ? hit.next : node.else;
      continue;
    }
    if (node.kind === "do") {
      return { kind: "do", nodeId, action: node.action, params: node.params ?? {}, path };
    }

    const dropped: { id: string; action: string; why: "removed" | "condition" }[] = [];
    const options = node.options
      .filter((o) => {
        if (template.remove.has(o.id)) {
          dropped.push({ id: o.id, action: o.action, why: "removed" });
          return false;
        }
        if ((o.when && !evaluate(o.when, facts, ctx)) || (template.when[o.id] && !evaluate(template.when[o.id], facts, ctx))) {
          dropped.push({ id: o.id, action: o.action, why: "condition" });
          return false;
        }
        return true;
      })
      .map((o) => ({ ...o, bias: (template.bias[o.id] ?? o.bias ?? 0) as Bias }));
    const fallback = options.some((o) => o.id === node.fallback)
      ? node.fallback
      : (options[0]?.id ?? null);
    return {
      kind: "choose",
      nodeId,
      title: node.title,
      question: node.question,
      doctrine: template.doctrine,
      options,
      fallback,
      dropped,
      path,
    };
  }
  throw new Error(
    `Template "${template.id}": walk from trigger "${trigger}" exceeded ${MAX_WALK_STEPS} steps (cycle?): ${path.slice(-6).join(" → ")}`,
  );
}
