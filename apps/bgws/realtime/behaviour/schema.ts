// ── behaviour/schema.ts ──────────────────────────────────────────────────
// Validate behaviour templates. Run over every JSON file at load and in the
// tests, so a broken template fails CI, never a game.
//
// Two levels:
//   validateTemplate   one file on its own: shape, ids, internal references,
//                      and (when given) facts/actions against the vocabulary.
//   validateRegistry   all files together: parents exist, no inheritance
//                      loops, and no child removes an option its inherited
//                      fallback depends on.
// Both return every problem found, each prefixed with where it is.

import { COMPARISONS, type Template } from "./types";
import { resolveTemplate } from "./walk";

export interface Vocabulary {
  facts?: ReadonlySet<string>;
  actions?: ReadonlySet<string>;
}

const isObj = (v: unknown): v is Record<string, unknown> =>
  typeof v === "object" && v !== null && !Array.isArray(v);
const isStr = (v: unknown): v is string => typeof v === "string" && v.length > 0;
const isFactValue = (v: unknown) =>
  v === null || typeof v === "number" || typeof v === "string" || typeof v === "boolean";
const BIASES = new Set([-2, -1, 0, 1, 2]);

function checkCondition(c: unknown, at: string, vocab: Vocabulary, errors: string[]): void {
  if (!isObj(c)) return void errors.push(`${at}: condition must be an object`);
  if ("all" in c || "any" in c) {
    const list = c.all ?? c.any;
    if (!Array.isArray(list) || list.length === 0)
      return void errors.push(`${at}: "all"/"any" needs a non-empty list`);
    list.forEach((x, i) => checkCondition(x, `${at}.${"all" in c ? "all" : "any"}[${i}]`, vocab, errors));
    return;
  }
  if ("not" in c) return checkCondition(c.not, `${at}.not`, vocab, errors);
  if (!isStr(c.fact)) errors.push(`${at}: needs "fact", "all", "any" or "not"`);
  else if (vocab.facts && !vocab.facts.has(c.fact)) errors.push(`${at}: unknown fact "${c.fact}"`);
  if (!COMPARISONS.includes(c.op as never)) errors.push(`${at}: "op" must be one of ${COMPARISONS.join(" ")}`);
  if (!isFactValue(c.value)) errors.push(`${at}: "value" must be a number, string, boolean or null`);
}

function checkAction(action: unknown, at: string, vocab: Vocabulary, errors: string[]): void {
  if (!isStr(action)) errors.push(`${at}: "action" must be a non-empty string`);
  else if (vocab.actions && !vocab.actions.has(action)) errors.push(`${at}: unknown action "${action}"`);
}

/** Check one template on its own. Returns every problem found (empty = valid). */
export function validateTemplate(raw: unknown, vocab: Vocabulary = {}): string[] {
  const errors: string[] = [];
  if (!isObj(raw)) return ["template must be a JSON object"];
  const id = isStr(raw.id) ? raw.id : "(no id)";
  const at = (where: string) => `${id}${where}`;

  if (!isStr(raw.id)) errors.push(`${id}: "id" is required`);
  if (typeof raw.version !== "number") errors.push(at(`: "version" must be a number`));
  if (raw.parent !== undefined && !isStr(raw.parent)) errors.push(at(`: "parent" must be a template id`));

  const nodes = raw.nodes ?? {};
  if (!isObj(nodes)) errors.push(at(`: "nodes" must be an object`));
  const nodeIds = new Set(isObj(nodes) ? Object.keys(nodes) : []);
  // A child may point at nodes it inherits, so unresolved references are
  // only errors for root templates; validateRegistry re-checks resolved ones.
  const ownRefsOnly = raw.parent === undefined;
  const refOk = (ref: unknown) => isStr(ref) && (!ownRefsOnly || nodeIds.has(ref));

  if (raw.triggers !== undefined) {
    if (!isObj(raw.triggers)) errors.push(at(`: "triggers" must be an object`));
    else
      for (const [evt, target] of Object.entries(raw.triggers))
        if (!refOk(target)) errors.push(at(`.triggers.${evt}: no node "${String(target)}"`));
  }

  if (isObj(nodes))
    for (const [nid, node] of Object.entries(nodes)) {
      const where = `.nodes.${nid}`;
      if (!isObj(node)) {
        errors.push(at(`${where}: must be an object`));
        continue;
      }
      if (node.kind === "choose") {
        if (!isStr(node.title)) errors.push(at(`${where}: "title" is required`));
        if (!isStr(node.question)) errors.push(at(`${where}: "question" is required`));
        const options = Array.isArray(node.options) ? node.options : [];
        if (options.length === 0) errors.push(at(`${where}: needs at least one option`));
        const seen = new Set<string>();
        options.forEach((o: unknown, i) => {
          const ow = `${where}.options[${i}]`;
          if (!isObj(o)) return void errors.push(at(`${ow}: must be an object`));
          if (!isStr(o.id)) errors.push(at(`${ow}: "id" is required`));
          else if (seen.has(o.id)) errors.push(at(`${ow}: duplicate option id "${o.id}"`));
          else seen.add(o.id);
          checkAction(o.action, at(ow), vocab, errors);
          if (o.when !== undefined) checkCondition(o.when, at(`${ow}.when`), vocab, errors);
          if (o.bias !== undefined && !BIASES.has(o.bias as number))
            errors.push(at(`${ow}: "bias" must be -2, -1, 0, 1 or 2`));
        });
        if (!isStr(node.fallback)) errors.push(at(`${where}: "fallback" is required`));
        else if (!seen.has(node.fallback))
          errors.push(at(`${where}: fallback "${node.fallback}" is not one of its options`));
      } else if (node.kind === "branch") {
        const cases = Array.isArray(node.cases) ? node.cases : [];
        if (cases.length === 0) errors.push(at(`${where}: needs at least one case`));
        cases.forEach((c: unknown, i) => {
          const cw = `${where}.cases[${i}]`;
          if (!isObj(c)) return void errors.push(at(`${cw}: must be an object`));
          checkCondition(c.when, at(`${cw}.when`), vocab, errors);
          if (!refOk(c.next)) errors.push(at(`${cw}: no node "${String(c.next)}"`));
        });
        if (!refOk(node.else)) errors.push(at(`${where}: "else" → no node "${String(node.else)}"`));
      } else if (node.kind === "do") {
        checkAction(node.action, at(where), vocab, errors);
      } else {
        errors.push(at(`${where}: "kind" must be "choose", "branch" or "do"`));
      }
    }

  if (raw.remove !== undefined && !(Array.isArray(raw.remove) && raw.remove.every(isStr)))
    errors.push(at(`: "remove" must be a list of option ids`));
  if (raw.when !== undefined) {
    if (!isObj(raw.when)) errors.push(at(`: "when" must be an object of option id → condition`));
    else for (const [oid, c] of Object.entries(raw.when)) checkCondition(c, at(`.when.${oid}`), vocab, errors);
  }
  if (raw.bias !== undefined) {
    if (!isObj(raw.bias)) errors.push(at(`: "bias" must be an object`));
    else
      for (const [oid, b] of Object.entries(raw.bias))
        if (!BIASES.has(b as number)) errors.push(at(`.bias.${oid}: must be -2, -1, 0, 1 or 2`));
  }
  return errors;
}

/** Check all templates together: parents, inheritance loops, removed fallbacks, references. */
export function validateRegistry(registry: ReadonlyMap<string, Template>): string[] {
  const errors: string[] = [];
  for (const id of registry.keys()) {
    let resolved;
    try {
      resolved = resolveTemplate(id, registry);
    } catch (e) {
      errors.push(`${id}: ${e instanceof Error ? e.message : String(e)}`);
      continue;
    }
    for (const [evt, target] of Object.entries(resolved.triggers))
      if (!resolved.nodes[target]) errors.push(`${id}.triggers.${evt}: no node "${target}" (after inheritance)`);
    for (const [nid, node] of Object.entries(resolved.nodes)) {
      if (node.kind === "branch") {
        for (const c of node.cases)
          if (!resolved.nodes[c.next]) errors.push(`${id}.nodes.${nid}: no node "${c.next}" (after inheritance)`);
        if (!resolved.nodes[node.else]) errors.push(`${id}.nodes.${nid}: no node "${node.else}" (after inheritance)`);
      }
      if (node.kind === "choose" && resolved.remove.has(node.fallback))
        errors.push(
          `${id}.nodes.${nid}: removes "${node.fallback}", which is this decision's fallback — keep it, or override the node with another fallback`,
        );
      if (node.kind === "choose" && node.options.every((o) => resolved.remove.has(o.id)))
        errors.push(`${id}.nodes.${nid}: every option is removed`);
    }
  }
  return errors;
}
