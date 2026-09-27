// ── bgws/realtime/overlayLayers.ts ─────────────────────────────────────────
// How the overlays (overlays.ts) look on the map: their sources and layers.
//
// Plain MapLibre style objects, with no map or React in sight, so the screen
// and a test page draw them exactly the same way. Bottom to top: areas, then
// lines, then points. A line's dash pattern cannot follow the data, so each
// pattern is its own layer.

export const SOURCE = {
  fire: "rt-fire",
  impact: "rt-impact",
  wreck: "rt-wreck",
  spot: "rt-spot",
  pop: "rt-pop",
  wedge: "rt-wedge",
  radio: "rt-radio",
  pulse: "rt-pulse",
  plan: "rt-plan",
  planZone: "rt-plan-zone",
} as const;

export const OVERLAY_SOURCES: string[] = Object.values(SOURCE);

/** What a volley did: red a knock-out, orange through the armour, blue intercepted, pale a bounce. */
const OUTCOME_COLOUR = ["match", ["get", "outcome"], "knockedOut", "#ff4d4d", "penetrated", "#ff9f43", "intercepted", "#7fb8ff", "#d7dbe6"];
/** What was said: teal a contact, orange under fire, magenta a request, lilac a reply, grey a situation report. */
const MESSAGE_COLOUR = ["match", ["get", "kind"], "contact", "#5fd4c0", "underFire", "#ff9f43", "request", "#e879f9", "reply", "#c4b5fd", "#6b7a90"];

export function overlayLayerSpecs(sideColour: { blue: string; red: string }): Record<string, unknown>[] {
  const bySide = ["match", ["get", "side"], "blue", sideColour.blue, sideColour.red];
  const planLine = (kind: string, paint: Record<string, unknown>) => ({
    id: `${SOURCE.plan}-${kind}`,
    type: "line",
    source: SOURCE.plan,
    filter: ["==", ["get", "kind"], kind],
    paint,
  });
  return [
    // Areas: bearings to shooters it cannot see; the selected unit's trigger.
    { id: `${SOURCE.wedge}-fill`, type: "fill", source: SOURCE.wedge, paint: { "fill-color": "#ff9f43", "fill-opacity": ["*", 0.14, ["get", "opacity"]] } },
    {
      id: `${SOURCE.wedge}-edge`,
      type: "line",
      source: SOURCE.wedge,
      paint: { "line-color": "#ff9f43", "line-width": 1, "line-opacity": ["*", 0.6, ["get", "opacity"]] },
    },
    {
      id: `${SOURCE.planZone}-fill`,
      type: "fill",
      source: SOURCE.planZone,
      paint: { "fill-color": ["case", ["get", "met"], "#ff6b6b", "#e8c547"], "fill-opacity": 0.07 },
    },
    {
      id: `${SOURCE.planZone}-edge`,
      type: "line",
      source: SOURCE.planZone,
      paint: { "line-color": ["case", ["get", "met"], "#ff6b6b", "#e8c547"], "line-width": 1.5, "line-dasharray": [3, 2] },
    },
    // Wrecks: smoke for a minute, then a dark mark.
    {
      id: `${SOURCE.wreck}-smoke`,
      type: "circle",
      source: SOURCE.wreck,
      paint: { "circle-radius": 16, "circle-color": "#8a8f99", "circle-blur": 1, "circle-opacity": ["*", 0.55, ["get", "smoke"]] },
    },
    {
      id: `${SOURCE.wreck}-mark`,
      type: "circle",
      source: SOURCE.wreck,
      paint: {
        "circle-radius": 4,
        "circle-color": "#1a1c22",
        "circle-stroke-width": 1.5,
        "circle-stroke-color": bySide,
        "circle-stroke-opacity": 0.6,
      },
    },
    // The selected unit's plan.
    planLine("current", { "line-color": "#ffffff", "line-width": 2.5, "line-opacity": 0.9 }),
    planLine("later", { "line-color": "#ffffff", "line-width": 2, "line-opacity": 0.7, "line-dasharray": [2, 2] }),
    planLine("boundary", { "line-color": "#ff6b6b", "line-width": 2, "line-opacity": 0.8, "line-dasharray": [4, 3] }),
    planLine("support", { "line-color": "#c4b5fd", "line-width": 1.5, "line-opacity": 0.8, "line-dasharray": [1, 2] }),
    planLine("watching", { "line-color": "#e8c547", "line-width": 1, "line-opacity": 0.8, "line-dasharray": [1, 3] }),
    // Fire: dashed for a miss; solid for a volley that struck, thicker the worse it was.
    {
      id: `${SOURCE.fire}-miss`,
      type: "line",
      source: SOURCE.fire,
      filter: ["==", ["get", "hit"], 0],
      paint: { "line-color": bySide, "line-width": 1.2, "line-opacity": ["*", 0.8, ["get", "opacity"]], "line-dasharray": [2, 2] },
    },
    {
      id: `${SOURCE.fire}-hit`,
      type: "line",
      source: SOURCE.fire,
      filter: ["==", ["get", "hit"], 1],
      paint: {
        "line-color": bySide,
        "line-width": ["match", ["get", "outcome"], "knockedOut", 3.5, "penetrated", 2.5, 2],
        "line-opacity": ["get", "opacity"],
      },
    },
    // Spotting: a dotted line to what it saw — red for a shooter it found.
    {
      id: `${SOURCE.spot}-line`,
      type: "line",
      source: SOURCE.spot,
      paint: {
        "line-color": ["match", ["get", "kind"], "located", "#ff6b6b", "#e8c547"],
        "line-width": 1.5,
        "line-opacity": ["get", "opacity"],
        "line-dasharray": [0.5, 2],
      },
    },
    // Radio: an arc from the sender to each hearer.
    {
      id: `${SOURCE.radio}-line`,
      type: "line",
      source: SOURCE.radio,
      paint: {
        "line-color": MESSAGE_COLOUR,
        "line-width": ["case", ["==", ["get", "broadcast"], 1], 1, 2.2],
        "line-opacity": ["*", ["case", ["==", ["get", "broadcast"], 1], 0.3, 0.95], ["get", "opacity"]],
        "line-dasharray": [3, 1.5],
      },
    },
    // Points on top: bursts where rounds struck, spotting pops, radio pulses.
    {
      id: `${SOURCE.impact}-burst`,
      type: "circle",
      source: SOURCE.impact,
      paint: {
        "circle-radius": [
          "+",
          ["match", ["get", "outcome"], "knockedOut", 9, "penetrated", 6, 4],
          ["*", ["get", "grow"], ["match", ["get", "outcome"], "knockedOut", 22, 10]],
        ],
        "circle-color": OUTCOME_COLOUR,
        "circle-opacity": ["*", 0.45, ["get", "opacity"]],
        "circle-stroke-width": ["match", ["get", "outcome"], "knockedOut", 2.5, 1.5],
        "circle-stroke-color": OUTCOME_COLOUR,
        "circle-stroke-opacity": ["get", "opacity"],
      },
    },
    {
      id: `${SOURCE.pop}-ring`,
      type: "circle",
      source: SOURCE.pop,
      paint: {
        "circle-radius": ["+", 6, ["*", ["get", "grow"], 16]],
        "circle-color": "rgba(0,0,0,0)",
        "circle-stroke-width": 2,
        "circle-stroke-color": "#e8c547",
        "circle-stroke-opacity": ["get", "opacity"],
      },
    },
    {
      id: `${SOURCE.pulse}-ring`,
      type: "circle",
      source: SOURCE.pulse,
      paint: {
        "circle-radius": ["+", 8, ["*", ["get", "grow"], 26]],
        "circle-color": "rgba(0,0,0,0)",
        "circle-stroke-width": 1.5,
        "circle-stroke-color": MESSAGE_COLOUR,
        "circle-stroke-opacity": ["get", "opacity"],
      },
    },
  ];
}
