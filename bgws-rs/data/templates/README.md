# Behaviour templates

One JSON file per template. `core/` holds the shipped defaults (one per unit
archetype); `custom/` holds variants that inherit from them. A template's `"id"`
must match its path: `core/tank.json` → `"id": "core/tank"`.

Every file is checked on load and in CI (`../schema.ts`), so a broken template
fails the build, never a game. Format: `../types.ts`. Design and rationale:
`docs/plan/DECISION_FLOW_GRAPHS.md`.

```json
{
  "id": "core/tank", "version": 1, "archetype": "tank",
  "doctrine": "A tank crew fights from cover and keeps its distance from infantry.",
  "triggers": { "sighted": "contact" },
  "nodes": {
    "contact": {
      "kind": "choose", "title": "Enemy in sight", "question": "What does the crew do?",
      "options": [
        { "id": "fire", "action": "fire", "when": { "fact": "ammo.main", "op": ">", "value": 0 } },
        { "id": "hull-down", "action": "takeHullDown" },
        { "id": "pull-back", "action": "withdraw" }
      ],
      "fallback": "hull-down"
    }
  }
}
```

A variant only states what differs:

```json
{ "id": "custom/tank-stand-fast", "version": 1, "parent": "core/tank", "remove": ["pull-back"] }
```

- **`choose`** asks Jev; only options whose `when` holds, and that no template in the chain `remove`s, are offered. `fallback` is used if Jev can't answer, and can't be removed.
- **`branch`** is a deterministic rule split (`cases` in order, then `else`).
- **`do`** is a fixed drill: no question asked.
- **`bias`** (−2…2) tells Jev an option is discouraged/preferred; it never removes it.

Facts and actions are a closed vocabulary supplied by the engine; naming one that doesn't exist fails validation.
