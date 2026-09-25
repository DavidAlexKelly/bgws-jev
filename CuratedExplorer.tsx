/**
 * CuratedExplorer — the real equipment catalogue, browsed and filtered.
 *
 * WHY THIS IS SEPARATE FROM THE L5 EXPLORER
 * ----------------------------------------
 * ⚠ THE TWO CATALOGUES DO NOT SHARE IDENTIFIERS. L5 assets are
 * `tankmodels/uk_challenger_2_bn`; curated rows are `var_11_default`, and
 * `l6_asset_id` is empty for every one. Joining them by DISPLAY NAME is the
 * obvious shortcut and the one thing this must not do: that exercise, done
 * carefully and by hand, matched 10 of 133 and produced "M2 Bradley" to an M2
 * Browning machine gun and "Challenger 3" to a Challenger 1, because the
 * catalogue does not call vehicles by their names.
 *
 * ONE LIST, FOUR TYPES
 * --------------------
 * Vehicles, aircraft, naval and infantry are browsed together and filtered by
 * type, because that is how somebody looks for equipment. They do not come
 * from one table: vehicles/aircraft/naval are the platform profile and
 * infantry is the section profile, which has loadouts instead of armour. The
 * merge happens in ./data/curatedFilters, which is pure and tested — the
 * predicates are the part that fails silently.
 *
 * Aircraft and warships are in the catalogue and NOT playable. The profile
 * says which with `bgws_playable`, and the "playable only" filter is how a
 * scenario author asks for the fieldable subset.
 */

import { useEffect, useMemo, useRef, useState } from "react";

import { SqlError } from "@/shared/lib/sqlClient";

import { ArmourFacings } from "./components/ArmourFacings";
import { PenetrationCurve } from "./components/PenetrationCurve";
import {
  curveFalloff,
  hasCurve,
  penetrationCurve,
  type CuratedCapability,
  type CuratedMunition,
  type CuratedPlatform,
  type CuratedSection,
  type CuratedSupport,
} from "./data/curatedAssets";
import {
  loadCuratedCapabilities,
  loadCuratedMunitions,
  loadCuratedPlatforms,
  loadCuratedSections,
  loadCuratedSupport,
} from "./data/curatedAssetsClient";
import {
  activeCuratedCount,
  applyCurated,
  facetCounts,
  rowsFromPlatforms,
  rowsFromSections,
  toggleCurated,
  type CuratedFilters,
  type EquipmentRow,
  type FacetEntry,
} from "./data/curatedFilters";
import { resolveMildataImage, type ResolvedImage } from "./data/unitImages";

export type CuratedMode = "curated" | "munitions";

const DATASET_NAME: Record<CuratedMode, string> = {
  curated: "[SIM] L7 bgws_platform_profile and bgws_section_profile",
  munitions: "[SIM] L7 bgws_munition_profile",
};

const FACETS = [
  ["types", "type"],
  ["subclasses", "class"],
  ["nations", "nation"],
  ["capabilities", "capability"],
] as const;

export function CuratedExplorer({ mode }: { mode: CuratedMode }) {
  const [rows, setRows] = useState<EquipmentRow[]>([]);
  const [munitions, setMunitions] = useState<CuratedMunition[]>([]);
  /** platform_id -> the missions it can be called on for. See the load below. */
  const [supportByPlatform, setSupportByPlatform] = useState<
    Map<string, CuratedSupport[]>
  >(new Map());
  const [filters, setFilters] = useState<CuratedFilters>({});
  /** Which facet dropdown is expanded, or null. One at a time — see below. */
  const [openFacet, setOpenFacet] = useState<string | null>(null);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [munitionSearch, setMunitionSearch] = useState("");
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const controller = new AbortController();
    setLoading(true);
    setError(null);
    setSelectedId(null);

    const load =
      mode === "curated"
        ? Promise.all([
            loadCuratedPlatforms(controller.signal),
            loadCuratedSections(controller.signal),
            loadCuratedSupport(controller.signal),
          ]).then(([platforms, sections, support]) => {
            setRows([...rowsFromPlatforms(platforms), ...rowsFromSections(sections)]);
            // Indexed by platform, because the detail pane asks "what can
            // this airframe be called on to do" and one platform flies
            // several missions. Loaded with the list rather than per
            // selection: 350 rows is one small query, and doing it per click
            // would put a network round trip between a reader and an answer
            // the app already has.
            const byPlatform = new Map<string, CuratedSupport[]>();
            for (const mission of support) {
              if (!mission.platformId) continue;
              const existing = byPlatform.get(mission.platformId);
              if (existing) existing.push(mission);
              else byPlatform.set(mission.platformId, [mission]);
            }
            setSupportByPlatform(byPlatform);
          })
        : loadCuratedMunitions(controller.signal).then(setMunitions);

    load
      .then(() => setLoading(false))
      .catch((err: unknown) => {
        if (controller.signal.aborted) return;
        setLoading(false);
        setError(
          err instanceof SqlError && err.permissionDenied
            ? `No access to ${DATASET_NAME[mode]}. Add it as a Resource on this app ` +
              "in Developer Console — the SQL scopes alone are not enough."
            : err instanceof Error
              ? err.message
              : "Query failed.",
        );
      });
    return () => controller.abort();
  }, [mode]);

  const visible = useMemo(() => applyCurated(rows, filters), [rows, filters]);
  const activeCount = activeCuratedCount(filters);
  const selected = visible.find((row) => row.id === selectedId) ?? null;

  if (error) return <div style={{ padding: 16, ...warn }}>{error}</div>;
  if (loading) return <div style={{ padding: 16, ...subtle }}>loading…</div>;

  if (mode === "munitions") {
    const needle = munitionSearch.trim().toLowerCase();
    const shown = munitions.filter((m) =>
      !needle || `${m.name} ${m.kind ?? ""}`.toLowerCase().includes(needle),
    );
    return <MunitionList munitions={shown} search={munitionSearch} onSearch={setMunitionSearch} />;
  }

  return (
    <>
      <div style={filterPane}>
        <input
          value={filters.search ?? ""}
          onChange={(event) => setFilters({ ...filters, search: event.target.value })}
          placeholder={`search ${rows.length} items…`}
          style={input}
        />

        <div style={{ display: "flex", gap: 6, marginBottom: 10, flexWrap: "wrap" }}>
          <Toggle
            label="playable only"
            hint="Only what BGWS can field: land, with a move type the game resolves"
            on={filters.playableOnly === true}
            onClick={() =>
              setFilters({ ...filters, playableOnly: filters.playableOnly ? undefined : true })
            }
          />
          <Toggle
            label="has armour"
            hint="Only rows with a protection model — excludes aircraft, ships and sections"
            on={filters.armouredOnly === true}
            onClick={() =>
              setFilters({ ...filters, armouredOnly: filters.armouredOnly ? undefined : true })
            }
          />
          {activeCount > 0 && (
            <button onClick={() => setFilters({})} style={{ ...chip, color: "#e07a5f" }}>
              clear {activeCount}
            </button>
          )}
        </div>

        {/* One open at a time. Four expanded panels in a 260px column is a
            wall rather than a filter, and closing the previous one keeps the
            facet you are working in at a predictable place on screen. */}
        {FACETS.map(([facet, label]) => (
          <FacetSelect
            key={facet}
            label={label}
            entries={facetCounts(rows, filters, facet)}
            open={openFacet === facet}
            onOpenChange={(next) => setOpenFacet(next ? facet : null)}
            onToggle={(value) => setFilters(toggleCurated(filters, facet, value))}
            onClear={() => setFilters({ ...filters, [facet]: undefined })}
          />
        ))}
      </div>

      <div style={listPane}>
        <div style={{ ...subtle, marginBottom: 6 }}>
          {visible.length === rows.length
            ? `${rows.length} items`
            : `${visible.length} of ${rows.length}`}
        </div>
        <div style={{ overflowY: "auto", minHeight: 0 }}>
          {visible.map((row) => (
            <button
              key={row.id}
              onClick={() => setSelectedId(row.id)}
              style={{
                ...listItem,
                background: selectedId === row.id ? "rgba(232,197,71,0.12)" : "transparent",
                borderColor: selectedId === row.id ? "#e8c547" : "transparent",
              }}
            >
              <div style={{ color: "#e9ecfb", fontSize: 12 }}>
                {row.name}
                {!row.playable && (
                  <span style={{ ...subtle, marginLeft: 5 }} title="not fieldable in BGWS">
                    ○
                  </span>
                )}
              </div>
              <div style={subtle}>
                {[row.type, row.subclass, row.nation].filter(Boolean).join(" · ")}
                {row.csIndex != null && ` · CS ${row.csIndex}`}
              </div>
            </button>
          ))}
          {visible.length === 0 && <div style={subtle}>Nothing matches.</div>}
        </div>
      </div>

      <div style={detailPane}>
        {selected?.platform ? (
          <PlatformDetail
            platform={selected.platform}
            // Keyed on the PLATFORM, not the asset: a variant inherits the
            // airframe's missions, and support is recorded per platform.
            support={
              supportByPlatform.get(
                selected.platform.baseAssetId ?? selected.platform.assetId,
              ) ?? []
            }
          />
        ) : selected?.section ? (
          <SectionDetail section={selected.section} />
        ) : (
          <div style={subtle}>Select an item.</div>
        )}
      </div>
    </>
  );
}

// ─── Filters ───────────────────────────────────────────────────────────────

/**
 * One facet as a multi-select dropdown.
 *
 * ⚠ THE PANEL IS IN FLOW, NOT FLOATING, AND THAT IS NOT LAZINESS.
 *
 * The filter pane is a narrow column with `overflowY: auto`. An absolutely
 * positioned menu inside a scrolling ancestor is clipped by it — the dropdown
 * would be cut off at the bottom of the pane, which is exactly where the last
 * facet sits. The alternatives are a portal with hand-computed coordinates
 * that must be recomputed on every scroll, or expanding in place. In a
 * sidebar this narrow, expanding in place is both simpler and better: nothing
 * can be clipped, and the list scrolls itself when it is long.
 *
 * WHAT THE COUNTS ARE, because they are the reason this is not a plain
 * `<select multiple>`. Each value carries the number of rows it would match
 * given every OTHER facet's selection — see facetCounts. A native multi-select
 * cannot show them, cannot show a checked state clearly, and needs ctrl-click
 * to add a second value, which people discover by accident or never.
 */
function FacetSelect({
  label,
  entries,
  open,
  onOpenChange,
  onToggle,
  onClear,
}: {
  label: string;
  entries: FacetEntry[];
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onToggle: (value: string) => void;
  onClear: () => void;
}) {
  const [needle, setNeedle] = useState("");
  const containerRef = useRef<HTMLDivElement | null>(null);

  // Close on a click elsewhere or on Escape. Registered only while open, so
  // four collapsed facets cost no document listeners at all.
  useEffect(() => {
    if (!open) return;
    const onPointerDown = (event: MouseEvent) => {
      if (!containerRef.current?.contains(event.target as Node)) onOpenChange(false);
    };
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") onOpenChange(false);
    };
    document.addEventListener("mousedown", onPointerDown);
    document.addEventListener("keydown", onKeyDown);
    return () => {
      document.removeEventListener("mousedown", onPointerDown);
      document.removeEventListener("keydown", onKeyDown);
    };
  }, [open, onOpenChange]);

  if (entries.length === 0) return null;

  const selected = entries.filter((entry) => entry.selected);
  const trimmed = needle.trim().toLowerCase();
  const shown = trimmed
    ? entries.filter((entry) => entry.label.toLowerCase().includes(trimmed))
    : entries;

  // The closed state has to say what is filtering the list without being
  // opened, or a reader cannot tell a narrowed list from a short one.
  const summary =
    selected.length === 0
      ? "any"
      : selected.length === 1
        ? selected[0].label
        : `${selected.length} selected`;

  return (
    <div ref={containerRef} style={{ marginBottom: 8, position: "relative" }}>
      <button
        onClick={() => {
          setNeedle("");
          onOpenChange(!open);
        }}
        aria-expanded={open}
        aria-label={`${label} filter, ${summary}`}
        style={{
          ...selectButton,
          borderColor: selected.length > 0 ? "#e8c547" : "rgba(255,255,255,0.12)",
        }}
      >
        <span style={{ ...subtle, textTransform: "uppercase", letterSpacing: "0.1em" }}>
          {label}
        </span>
        <span
          style={{
            flex: 1,
            textAlign: "left",
            marginLeft: 8,
            color: selected.length > 0 ? "#e8c547" : "#8a91a8",
            overflow: "hidden",
            textOverflow: "ellipsis",
            whiteSpace: "nowrap",
          }}
        >
          {summary}
        </span>
        <span style={{ ...subtle, marginLeft: 6 }}>{open ? "\u25b2" : "\u25bc"}</span>
      </button>

      {open && (
        <div style={selectPanel}>
          {/* Only where the list is long enough to need it. Nation runs to
              dozens of entries; type has four. */}
          {entries.length > 8 && (
            <input
              value={needle}
              onChange={(event) => setNeedle(event.target.value)}
              placeholder={`filter ${entries.length} ${label}s…`}
              autoFocus
              style={{ ...input, marginBottom: 4, fontSize: 11, padding: "4px 6px" }}
            />
          )}

          <div style={{ maxHeight: 220, overflowY: "auto" }}>
            {shown.map((entry) => (
              <button
                key={entry.value}
                onClick={() => onToggle(entry.value)}
                role="checkbox"
                aria-checked={entry.selected}
                style={{
                  ...optionRow,
                  color: entry.selected ? "#e8c547" : "#c7cce0",
                }}
              >
                {/* A drawn box rather than a real checkbox: the row is the
                    hit target, and a nested input would swallow its clicks. */}
                <span style={{ width: 12, flexShrink: 0 }}>
                  {entry.selected ? "\u2713" : ""}
                </span>
                <span
                  style={{
                    flex: 1,
                    overflow: "hidden",
                    textOverflow: "ellipsis",
                    whiteSpace: "nowrap",
                  }}
                >
                  {entry.label}
                </span>
                {/* Zero is shown, not hidden: a selected value that now
                    matches nothing is information, and facetCounts keeps it
                    in the list precisely so it can be unselected. */}
                <span style={{ ...subtle, marginLeft: 6 }}>{entry.count}</span>
              </button>
            ))}
            {shown.length === 0 && (
              <div style={{ ...subtle, padding: "4px 6px" }}>nothing matches</div>
            )}
          </div>

          {selected.length > 0 && (
            <button
              onClick={onClear}
              style={{ ...chip, color: "#e07a5f", marginTop: 4, width: "100%" }}
            >
              clear {label}
            </button>
          )}
        </div>
      )}
    </div>
  );
}

function Toggle({
  label,
  hint,
  on,
  onClick,
}: {
  label: string;
  hint: string;
  on: boolean;
  onClick: () => void;
}) {
  return (
    <button
      onClick={onClick}
      title={hint}
      style={{ ...chip, borderColor: on ? "#e8c547" : "transparent", color: on ? "#e8c547" : "#8a91a8" }}
    >
      {label}
    </button>
  );
}

// ─── Platform detail ───────────────────────────────────────────────────────

function PlatformDetail({
  platform,
  support,
}: {
  platform: CuratedPlatform;
  support: CuratedSupport[];
}) {
  const [capabilities, setCapabilities] = useState<CuratedCapability[]>([]);
  const [image, setImage] = useState<ResolvedImage | null>(null);

  // Addressed exactly from image_filename, so a platform gets its own
  // photograph or none -- never somebody else's on a near-miss.
  useEffect(() => {
    let cancelled = false;
    setImage(null);
    if (!platform.imageFilename) {
      setImage({ url: null });
      return;
    }
    resolveMildataImage(platform.imageFilename).then((resolved) => {
      if (!cancelled) setImage(resolved);
    });
    return () => {
      cancelled = true;
    };
  }, [platform.imageFilename]);

  useEffect(() => {
    const controller = new AbortController();
    setCapabilities([]);
    loadCuratedCapabilities(platform.assetId, controller.signal)
      .then(setCapabilities)
      .catch(() => setCapabilities([]));
    return () => controller.abort();
  }, [platform.assetId]);

  const hasArmour = platform.facings.some((f) => f.keMm != null || f.ceMm != null);

  return (
    <div style={{ overflowY: "auto", height: "100%" }}>
      <div style={{ fontSize: 16, fontWeight: 700, color: "#e9ecfb" }}>
        {platform.displayName}
      </div>
      <div style={{ ...subtle, marginBottom: 12 }}>
        {platform.assetId}
        {platform.mfId && ` · Military Factory id ${platform.mfId}`}
        {!platform.playable && " · not fieldable in BGWS"}
      </div>

      <div style={imageFrame}>
        {image === null && <span style={subtle}>loading image…</span>}
        {image?.url && (
          <img
            src={image.url}
            alt={platform.displayName}
            style={{ width: "100%", height: "100%", objectFit: "contain" }}
          />
        )}
        {image !== null && !image.url && !image.error && (
          <span style={subtle}>
            {platform.imageFilename
              ? "no image in the set for this platform"
              : "no photograph recorded for this platform"}
          </span>
        )}
        {image?.error && (
          <span style={{ ...warn, padding: 12, textAlign: "center" }}>
            image lookup failed — {image.error}
            <br />
            Check the Mildata Images media set is a Resource on this app.
          </span>
        )}
      </div>
      {image?.path && <div style={{ ...subtle, marginBottom: 14 }}>image: {image.path}</div>}

      {hasArmour ? (
        <Group title="Protection by aspect">
          <ArmourFacings facings={platform.facings} />
          <div style={{ ...subtle, marginTop: 6 }}>
            {platform.eraFit && platform.eraFit !== "none" && `ERA: ${platform.eraFit}. `}
            {platform.apsFit && platform.apsFit !== "none" && `APS: ${platform.apsFit}. `}
            {platform.protectionConfidence && `confidence ${platform.protectionConfidence}`}
          </div>
          {platform.protectionSourceRef && (
            <div style={{ ...subtle, marginTop: 4, fontStyle: "italic" }}>
              {platform.protectionSourceRef}
            </div>
          )}
        </Group>
      ) : (
        <Group title="Protection by aspect">
          {/* Aircraft and warships have no protection rows in the curated
              tables. That is an absence of a model, not armour of zero. */}
          <div style={subtle}>
            No armour model for this platform. The curated tables carry
            protection for land equipment only.
          </div>
        </Group>
      )}

      <Group title="Capabilities and reach">
        {capabilities.length === 0 && <div style={subtle}>no capability rows</div>}
        {capabilities.map((capability) => {
          const points = [
            capability.penMm0m == null ? null : { rangeM: 0, penMm: capability.penMm0m },
            capability.penMm1000m == null
              ? null
              : { rangeM: 1000, penMm: capability.penMm1000m },
            capability.penMm2000m == null
              ? null
              : { rangeM: 2000, penMm: capability.penMm2000m },
          ].filter((point): point is { rangeM: number; penMm: number } => point !== null);

          return (
            <div key={capability.capability} style={{ marginBottom: 10 }}>
              <div style={{ color: "#e9ecfb", fontSize: 12 }}>
                {capability.capability.toUpperCase()}
                {capability.hasTopAttack && (
                  <span style={{ color: "#7ec9a3" }}> · top attack</span>
                )}
              </div>
              <div style={subtle}>
                {fmt(capability.maxRangeM, " m")} max
                {capability.minRangeM ? ` · ${capability.minRangeM} m min` : ""}
                {capability.shortRangeM
                  ? ` · short under ${Math.round(capability.shortRangeM)} m`
                  : ""}
              </div>
              {capability.weapons.length > 0 && (
                <div style={subtle}>{capability.weapons.join(", ")}</div>
              )}
              {points.length >= 2 && <PenetrationCurve points={points} />}
            </div>
          );
        })}
      </Group>

      {/* ⚠ THE ANSWER TO "WHY IS THIS NOT PLAYABLE". An aircraft or warship
          reads as an empty counter -- no move type, no armour, no Combat
          Strength -- and without this the reader concludes the data is
          missing. It is not: this is how the asset takes part. Called for,
          arrives after a delay, stays for a while, leaves. */}
      {support.length > 0 && (
        <Group title="As called-for support">
          {support.map((mission) => (
            <div
              key={mission.assetId + mission.missionType}
              style={{ marginBottom: 8 }}
            >
              <div style={{ color: "#e9ecfb" }}>
                {(mission.missionType ?? "—").replace(/_/g, " ")}
                <span style={{ ...subtle, marginLeft: 6 }}>
                  {mission.supportRole === "excluded"
                    ? "· beyond a battlegroup action"
                    : `· ${mission.supportRole ?? "unclassified"}`}
                </span>
              </div>
              <div style={subtle}>
                {fmt(mission.responseTurns)} turn
                {mission.responseTurns === 1 ? "" : "s"} to arrive
                {mission.loiterTurns != null &&
                  mission.loiterTurns > 0 &&
                  `, ${mission.loiterTurns} on station`}
                {mission.sortiesPerDay != null &&
                  `, ${mission.sortiesPerDay}/day`}
                {mission.liftCapacity != null &&
                  mission.liftCapacity > 0 &&
                  `, lifts ${mission.liftCapacity}`}
              </div>
            </div>
          ))}
          <div style={{ ...subtle, marginTop: 6, lineHeight: 1.5 }}>
            Aircraft and warships are not counters on a 10 km land board — the
            game has no move type for them and its fire columns stop at 30 km.
            They take part through response and loiter instead, which is what
            these figures are.
          </div>
        </Group>
      )}

      {/* ⚠ SURVIVABILITY IS NOT ARMOUR, AND IS SHOWN SEPARATELY BECAUSE OF IT.
          An aircraft or warship has no STANAG facing, so "Protection by
          aspect" and the protection band below are empty for them by design.
          What IS known is what can reach them, and putting it in the armour
          group would read as a substitute for millimetres rather than a
          different kind of statement. */}
      {platform.survivabilityBand && (
        <Group title="Survivability">
          <Row label="Band" value={platform.survivabilityBand} />
          <Row
            label="Threatened by"
            value={(platform.threatVulnerability ?? "—").replace(/_/g, " ")}
          />
          <div style={{ ...subtle, marginTop: 6, lineHeight: 1.5 }}>
            Not armour. This asset has no frontal RHA figure and never will —
            the band says what weapon class can engage it, which is the
            equivalent question for something that is not a ground vehicle.
          </div>
        </Group>
      )}

      <Group title="As a counter">
        <Row
          label="Combat Strength"
          value={platform.csIndex != null ? fmt(platform.csIndex) : "not rated"}
        />
        <Row label="Move type" value={platform.moveType ?? "—"} />
        <Row label="Target class" value={platform.targetClass ?? "—"} />
        <Row label="Protection band" value={platform.protectionBand ?? "—"} />
        {/* The absence is deliberate and worth saying out loud, or a reader
            assumes the pipeline failed to compute something. */}
        {platform.csIndex == null && (
          <div style={{ ...subtle, marginTop: 6, lineHeight: 1.5 }}>
            Combat Strength rates a LAND counter: penetration at 1,000 m against
            frontal armour, on fire columns calibrated for 1–5 km. Scoring an
            aircraft or a warship on it would produce a number that looks like a
            measurement and is not, so this is left unrated.
          </div>
        )}
        <Row label="Dismounts" value={fmt(platform.dismountsCarried)} />
        <Row label="Optics class" value={fmt(platform.opticsClass)} />
        <Row label="Signature class" value={fmt(platform.signatureClass)} />
        {platform.requiresPrimeMover && (
          <Row label="Towed" value="needs a prime mover — mapped to Wheeled" />
        )}
        {platform.csIndex != null && (
          <div style={{ ...subtle, marginTop: 6 }}>
            Combat Strength is DERIVED, not sourced: from {fmt(platform.csInputPenMm, " mm")}{" "}
            penetration, {fmt(platform.csInputArmourMm, " mm")} frontal armour and{" "}
            {fmt(platform.csInputRangeM, " m")} reach.
          </div>
        )}
      </Group>

      <Group title="Platform">
        <Row label="Domain" value={platform.domain ?? "—"} />
        <Row label="Class" value={platform.subclass ?? "—"} />
        <Row label="Nation" value={platform.nation ?? "—"} />
        <Row label="Mass" value={fmt(platform.massT, " t")} />
        <Row label="Road speed" value={fmt(platform.speedKmh, " km/h")} />
        <Row label="Crew" value={fmt(platform.crewSize)} />
        <Row label="Weapons" value={fmt(platform.weaponCount)} />
      </Group>
    </div>
  );
}

// ─── Section detail ────────────────────────────────────────────────────────

function SectionDetail({ section }: { section: CuratedSection }) {
  return (
    <div style={{ overflowY: "auto", height: "100%" }}>
      <div style={{ fontSize: 16, fontWeight: 700, color: "#e9ecfb" }}>
        {section.displayName}
      </div>
      <div style={{ ...subtle, marginBottom: 12 }}>{section.assetId}</div>

      <Group title="Organisation">
        <Row label="Strength" value={fmt(section.strength, " men")} />
        <Row label="Fireteams" value={fmt(section.fireteams)} />
        <Row label="Combat Strength" value={fmt(section.csIndex)} />
        <Row label="Optics class" value={fmt(section.opticsClass)} />
        <Row label="Carried by" value={section.defaultCarrier ?? "—"} />
      </Group>

      <Group title="Reach, per capability">
        {/* ⚠ PER CAPABILITY, NOT ONE FIGURE. A Kornet detachment's rifles
            reach 400 m and its missile 5,500. A single "range" would be a lie
            about whichever weapon it was not describing. */}
        <ReachRow label="apers" range={section.apersMaxRangeM} />
        <ReachRow label="atk" range={section.atkMaxRangeM} pen={section.atkPenMm1000m} />
        <ReachRow label="atm" range={section.atmMaxRangeM} pen={section.atmPenMm1000m} />
        <ReachRow label="aa" range={section.aaMaxRangeM} />
        <ReachRow label="idf" range={section.idfMaxRangeM} />
      </Group>

      <Group title="Weapons carried">
        {section.weapons.length === 0 ? (
          <div style={subtle}>no loadout recorded</div>
        ) : (
          section.weapons.map((weapon) => (
            <div key={weapon} style={{ color: "#e9ecfb", fontSize: 12, padding: "1px 0" }}>
              {weapon}
            </div>
          ))
        )}
        <div style={{ ...subtle, marginTop: 6 }}>
          Penetration is the round the section CARRIES, not the best its
          launcher can fire.
        </div>
      </Group>

      {section.sourceRef && (
        <Group title="Source">
          <div style={{ ...subtle, fontStyle: "italic" }}>
            {section.sourceRef}
            {section.sourceConfidence && ` · confidence ${section.sourceConfidence}`}
          </div>
        </Group>
      )}
    </div>
  );
}

// ─── Munitions ─────────────────────────────────────────────────────────────

function MunitionList({
  munitions,
  search,
  onSearch,
}: {
  munitions: CuratedMunition[];
  search: string;
  onSearch: (value: string) => void;
}) {
  const counts = useMemo(() => {
    const withCurve = munitions.filter(hasCurve);
    return {
      total: munitions.length,
      curves: withCurve.length,
      flat: withCurve.filter((m) => m.isShapedCharge).length,
    };
  }, [munitions]);

  return (
    <div style={{ ...detailPane, overflowY: "auto" }}>
      <input
        value={search}
        onChange={(event) => onSearch(event.target.value)}
        placeholder="search munitions…"
        style={{ ...input, maxWidth: 320 }}
      />
      <div style={{ ...subtle, marginBottom: 12 }}>
        {counts.total} rounds · {counts.curves} with a plottable curve ·{" "}
        {counts.flat} of those flat with range because they are shaped charges
      </div>

      <div style={{ display: "flex", flexWrap: "wrap", gap: 14 }}>
        {munitions.map((munition) => {
          const falloff = curveFalloff(munition);
          return (
            <div key={munition.munitionId} style={munitionCard}>
              <div style={{ color: "#e9ecfb", fontSize: 12, fontWeight: 600 }}>
                {munition.name}
              </div>
              <div style={subtle}>
                {munition.kind ?? "?"}
                {munition.topAttack && <span style={{ color: "#7ec9a3" }}> · top attack</span>}
                {munition.defeatsEraKe && " · defeats ERA"}
                {munition.sourceConfidence && ` · confidence ${munition.sourceConfidence}`}
              </div>

              {hasCurve(munition) ? (
                <>
                  <PenetrationCurve
                    points={penetrationCurve(munition)}
                    flat={munition.isShapedCharge}
                    label={`${munition.name} penetration against range`}
                  />
                  <div style={subtle}>
                    {falloff != null && falloff < 0.01
                      ? "flat with range — the jet is formed on impact"
                      : falloff != null
                        ? `${Math.round(falloff * 100)}% lost over the plotted range`
                        : ""}
                  </div>
                </>
              ) : (
                <div style={{ ...subtle, marginTop: 8 }}>
                  no anti-armour penetration to plot
                  {munition.lethalRadiusM
                    ? ` · ${munition.lethalRadiusM} m lethal radius`
                    : ""}
                  {munition.heFillKg ? ` · ${munition.heFillKg} kg fill` : ""}
                </div>
              )}

              {munition.sourceRef && (
                <div style={{ ...subtle, marginTop: 4, fontStyle: "italic" }}>
                  {munition.sourceRef}
                </div>
              )}
            </div>
          );
        })}
      </div>
    </div>
  );
}

// ─── Shared bits ───────────────────────────────────────────────────────────

function Group({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <div style={{ marginBottom: 14 }}>
      <div style={groupTitle}>{title}</div>
      {children}
    </div>
  );
}

function Row({ label, value }: { label: string; value: string }) {
  return (
    <div style={{ display: "flex", padding: "2px 0" }}>
      <span style={{ ...subtle, width: 130, flexShrink: 0 }}>{label}</span>
      <span style={{ color: value === "—" ? "#6a7292" : "#e9ecfb" }}>{value}</span>
    </div>
  );
}

function ReachRow({
  label,
  range,
  pen,
}: {
  label: string;
  range: number | null;
  pen?: number | null;
}) {
  if (range == null) return null;
  return (
    <div style={{ display: "flex", fontSize: 11, padding: "1px 0" }}>
      <span style={{ ...subtle, width: 44, flexShrink: 0 }}>{label}</span>
      <span style={{ color: "#e9ecfb" }}>{Math.round(range)} m</span>
      {pen != null && (
        <span style={{ ...subtle, marginLeft: 6 }}>({Math.round(pen)} mm at 1 km)</span>
      )}
    </div>
  );
}

function fmt(value: number | null | undefined, suffix = ""): string {
  return value == null ? "—" : `${value}${suffix}`;
}

const subtle: React.CSSProperties = { fontSize: 10, color: "#6a7292" };
const warn: React.CSSProperties = { fontSize: 12, color: "#e07a5f" };

const filterPane: React.CSSProperties = {
  width: 260,
  borderRight: "1px solid #191e37",
  padding: 10,
  overflowY: "auto",
  flexShrink: 0,
};

const listPane: React.CSSProperties = {
  width: 320,
  borderRight: "1px solid #191e37",
  padding: 10,
  display: "flex",
  flexDirection: "column",
  minHeight: 0,
  flexShrink: 0,
};

const detailPane: React.CSSProperties = { flex: 1, padding: 16, minHeight: 0 };

const listItem: React.CSSProperties = {
  display: "block",
  width: "100%",
  textAlign: "left",
  padding: "6px 8px",
  marginBottom: 2,
  border: "1px solid transparent",
  borderRadius: 3,
  cursor: "pointer",
};

const chip: React.CSSProperties = {
  padding: "3px 7px",
  fontSize: 10,
  background: "rgba(255,255,255,0.04)",
  border: "1px solid transparent",
  borderRadius: 3,
  cursor: "pointer",
};

const selectButton: React.CSSProperties = {
  display: "flex",
  alignItems: "center",
  width: "100%",
  boxSizing: "border-box",
  padding: "5px 7px",
  background: "rgba(255,255,255,0.04)",
  border: "1px solid",
  borderRadius: 3,
  cursor: "pointer",
  font: "inherit",
  fontSize: 11,
};

const selectPanel: React.CSSProperties = {
  marginTop: 3,
  padding: 4,
  background: "#11141d",
  border: "1px solid #2a3040",
  borderRadius: 3,
};

const optionRow: React.CSSProperties = {
  display: "flex",
  alignItems: "center",
  gap: 6,
  width: "100%",
  boxSizing: "border-box",
  padding: "3px 6px",
  background: "transparent",
  border: "none",
  borderRadius: 2,
  cursor: "pointer",
  font: "inherit",
  fontSize: 11,
  textAlign: "left",
};

const groupTitle: React.CSSProperties = {
  fontSize: 10,
  fontWeight: 700,
  letterSpacing: "0.12em",
  textTransform: "uppercase",
  color: "rgba(255,255,255,0.45)",
  borderBottom: "1px solid #191e37",
  paddingBottom: 4,
  marginBottom: 6,
};

const input: React.CSSProperties = {
  width: "100%",
  boxSizing: "border-box",
  padding: "6px 8px",
  marginBottom: 8,
  background: "rgba(255,255,255,0.04)",
  border: "1px solid rgba(255,255,255,0.12)",
  borderRadius: 3,
  color: "#e9ecfb",
};

const imageFrame: React.CSSProperties = {
  width: "100%",
  maxWidth: 420,
  height: 220,
  marginBottom: 6,
  display: "flex",
  alignItems: "center",
  justifyContent: "center",
  background: "rgba(255,255,255,0.03)",
  border: "1px solid #191e37",
  borderRadius: 4,
  overflow: "hidden",
};

const munitionCard: React.CSSProperties = {
  width: 280,
  padding: 10,
  border: "1px solid #191e37",
  borderRadius: 4,
  background: "rgba(255,255,255,0.02)",
};
