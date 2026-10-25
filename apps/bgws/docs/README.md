# BGWS documents

| Document | What it is |
|---|---|
| [BGWS - Rulebook.pdf](BGWS%20-%20Rulebook.pdf) | The BGWS core rulebook: what the turn game implements. |
| [JEV_INTEGRATION_PLAN.md](JEV_INTEGRATION_PLAN.md) | How Jev makes decisions in the turn game. |
| [REALTIME_MODE_PLAN.md](REALTIME_MODE_PLAN.md) | The real-time mode: a separate game on one clock, sharing the rules. |
| [REALTIME_REALISM.md](REALTIME_REALISM.md) | Making the real-time mode behave realistically: suppression and nerve, drills, the combat model from the data, spotting. |
| [REALTIME_COMMAND_DESIGN.html](REALTIME_COMMAND_DESIGN.html) | The real-time command design: who decides what (Claude, Jev, game logic), what each unit knows, the decision points. Open in a browser. |
| [REALTIME_COMMAND.md](REALTIME_COMMAND.md) | How that design is built: modules, declared figures, calibration, what is not done yet. |
| [REALTIME_COMMS.md](REALTIME_COMMS.md) | What each unit knows, and how units talk: per-unit pictures, the radio net, asking friends for help (D9, D12). |
| [EVENT_STREAM.md](EVENT_STREAM.md) | The real-time feed streamed to Foundry: the schema, and how it is sent. |
| [plan/MULTI_DOMAIN_PLAN.md](plan/MULTI_DOMAIN_PLAN.md) | The plan from proof of concept to a multi-domain simulation: the end goal, target architecture, the Claude planner hierarchy and commander seats, scenario scopes and eight preset scenario types, fifteen workstreams (land, C2 and comms, drones, EW, air, naval, space and cyber, logistics, the joint kill web, synthetic data, human command) and the milestones to build them in. Built to M9; its "Where it stands" says where each milestone lives. |
| [plan/WORKER_AND_SCHEDULER.md](plan/WORKER_AND_SCHEDULER.md) | Moving the engine into a Web Worker (deciders stay on the main thread) and an event scheduler for scale: measure first, config as data, worker host, frame view, scheduler in slices. |
| [plan/DECISION_FLOW_GRAPHS.md](plan/DECISION_FLOW_GRAPHS.md) | Editable decision flow graphs: every unit's default behaviour template from what it can do, templates derived by patches, swapping them, and the editor. |
| [plan/RUST_ENGINE.md](plan/RUST_ENGINE.md) | A separate Rust engine and desktop app: local, GPU-assisted, physics-first models for every domain, Jev and Claude as deciders, calibration, and a phased roadmap. A plan. |
| [plan/SIDE_PICTURE.md](plan/SIDE_PICTURE.md) | One record of reports per side instead of one picture per unit: what each unit was told, read through `picture.ts`. Built. |
| [research/MODERN_WARFARE_REFERENCE.md](research/MODERN_WARFARE_REFERENCE.md) | The research the plan is built on: how modern war is planned, commanded, communicated and fought in every domain, with sourced figures separated from unverified defaults. Sections are numbered R1–R8 and parameter rows P1–P32; the plan cites them as **[ref R4.3, P12]**. |
