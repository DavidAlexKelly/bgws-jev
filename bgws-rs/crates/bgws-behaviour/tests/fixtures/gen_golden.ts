import { readFileSync, readdirSync, writeFileSync, mkdirSync } from "node:fs";
// Writes tests/golden/*.json from the TypeScript walker, so the Rust walk is
// checked against the real thing. Run from this folder:
//   node --experimental-strip-types gen_golden.ts ../../../../data/templates ../golden
import { resolveTemplate, walk } from "../../../../../apps/bgws/realtime/behaviour/walk.ts";
const dir = process.argv[2], out = process.argv[3];
const reg = new Map<string, any>();
for (const f of readdirSync(`${dir}/core`).sort()) { const t = JSON.parse(readFileSync(`${dir}/core/${f}`, "utf8")); reg.set(t.id, t); }
for (const t of JSON.parse(readFileSync("custom.json", "utf8"))) reg.set(t.id, t);
const resolved = (id: string) => resolveTemplate(id, reg);
// world/questions.ts templateForTrigger, with the candidates given directly.
function templateForTrigger(cands: string[], trigger: string) {
  const own = cands.filter((id) => reg.has(id)).map(resolved);
  const answering = own.find((t) => t.triggers[trigger]);
  if (answering) return answering;
  if (!reg.has("core/world")) return null;
  const world = resolved("core/world");
  if (!world.triggers[trigger]) return null;
  const mine = own[0];
  if (!mine) return world;
  return { ...world, id: mine.id, lineage: [...mine.lineage, ...world.lineage.filter((id) => !mine.lineage.includes(id))],
    doctrine: [...world.doctrine, ...mine.doctrine.filter((d) => !world.doctrine.includes(d))],
    remove: new Set([...world.remove, ...mine.remove]), bias: { ...world.bias, ...mine.bias }, when: { ...world.when, ...mine.when } };
}
mkdirSync(out, { recursive: true });
for (const s of JSON.parse(readFileSync("scenes.json", "utf8"))) {
  const t = templateForTrigger(s.candidates, s.trigger);
  const facts = Object.fromEntries(Object.entries(s.facts).map(([k, v]) => [k, () => v]));
  const result = t ? { template: t.id, lineage: t.lineage, walk: walk(t, s.trigger, facts as any, null) } : { template: null, lineage: [], walk: { kind: "none", path: [] } };
  writeFileSync(`${out}/${s.name}.json`, JSON.stringify({ scene: s, ...result }, null, 2) + "\n");
}
console.log("ok");
