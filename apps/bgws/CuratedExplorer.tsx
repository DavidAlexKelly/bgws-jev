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
 *
 * Layout and styling: ./explorer.css and ./components/explorerUi.
 */

import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { SqlError } from "@/shared/lib/sqlClient";

import { ArmourFacings } from "./components/ArmourFacings";
import {
  DetailFrame,
  DetailPlaceholder,
  ExplorerLayout,
  FilterSection,
  FiltersButton,
  FiltersPanel,
  Group,
  ImageFrame,
  Kpis,
  ListSkeleton,
  Row,
  SearchInput,
} from "./components/explorerUi";
import { fmt } from "./components/explorerFormat";
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
  type EquipmentType,
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

const TYPE_GLYPH: Record<EquipmentType, string> = {
  vehicle: "▣",
  aircraft: "✈",
  naval: "⚓",
  infantry: "⚑",
};

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
  const [filtersOpen, setFiltersOpen] = useState(false);
  const closeFilters = useCallback(() => setFiltersOpen(false), []);

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

  if (mode === "munitions") {
    if (error) return <div className="ax-error" style={{ flex: 1, alignSelf: "flex-start" }}>{error}</div>;
    const needle = munitionSearch.trim().toLowerCase();
    const shown = munitions.filter((m) =>
      !needle || `${m.name} ${m.kind ?? ""}`.toLowerCase().includes(needle),
    );
    return (
      <MunitionList
        munitions={shown}
        loading={loading}
        search={munitionSearch}
        onSearch={setMunitionSearch}
      />
    );
  }

  return (
    <ExplorerLayout
      filtersOpen={filtersOpen}
      hasSelection={selected !== null}
      onCloseFilters={closeFilters}
      filters={
        <FiltersPanel activeCount={activeCount} onClear={() => setFilters({})} onClose={closeFilters}>
          <FilterSection title="Show">
            <div className="ax-chips">
              <Toggle
                label="Playable only"
                hint="Only what BGWS can field: land, with a move type the game resolves"
                on={filters.playableOnly === true}
                onClick={() =>
                  setFilters({ ...filters, playableOnly: filters.playableOnly ? undefined : true })
                }
              />
              <Toggle
                label="Has armour"
                hint="Only rows with a protection model — excludes aircraft, ships and sections"
                on={filters.armouredOnly === true}
                onClick={() =>
                  setFilters({ ...filters, armouredOnly: filters.armouredOnly ? undefined : true })
                }
              />
            </div>
          </FilterSection>

          {/* One open at a time. Four expanded panels in a narrow column is a
              wall rather than a filter, and closing the previous one keeps the
              facet you are working in at a predictable place on screen. */}
          <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
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
        </FiltersPanel>
      }
      list={
        <section className="ax-list" aria-label="Equipment">
          <div className="ax-list-toolbar">
            <div className="ax-list-toolbar-row">
              <SearchInput
                value={filters.search ?? ""}
                onChange={(search) => setFilters({ ...filters, search })}
                placeholder={loading ? "Search…" : `Search ${rows.length} items…`}
                label="Search equipment"
              />
              <FiltersButton activeCount={activeCount} onClick={() => setFiltersOpen(true)} />
            </div>
            <div className="ax-meta" aria-live="polite">
              {loading
                ? "Loading…"
                : visible.length === rows.length
                  ? `${rows.length} items`
                  : `${visible.length} of ${rows.length} items`}
            </div>
          </div>

          {error && <div className="ax-error">{error}</div>}

          <div className="ax-list-scroll">
            {loading ? (
              <ListSkeleton />
            ) : (
              visible.map((row) => (
                <button
                  key={row.id}
                  type="button"
                  className="ax-item"
                  aria-current={selectedId === row.id}
                  onClick={() => setSelectedId(row.id)}
                >
                  <span className="ax-item-glyph" aria-hidden="true" title={row.type}>
                    {TYPE_GLYPH[row.type] ?? "▣"}
                  </span>
                  <span className="ax-item-main">
                    <span className="ax-item-name">{row.name}</span>
                    <span className="ax-item-sub">
                      {[row.subclassLabel ?? row.subclass, row.nation].filter(Boolean).join(" · ") ||
                        row.type}
                    </span>
                  </span>
                  <span className="ax-item-end">
                    {row.csIndex != null && (
                      <span className="ax-pill" title="Combat Strength">
                        CS {row.csIndex}
                      </span>
                    )}
                    {!row.playable && (
                      <span
                        className="ax-dot"
                        style={{ border: "1px solid var(--ax-faint)" }}
                        title="Not fieldable in BGWS"
                        aria-label="Not fieldable in BGWS"
                      />
                    )}
                  </span>
                </button>
              ))
            )}
            {!loading && !error && visible.length === 0 && (
              <div className="ax-empty">
                Nothing matches these filters.
                {activeCount > 0 && (
                  <div style={{ marginTop: 10 }}>
                    <button type="button" className="ax-btn ax-btn-sm" onClick={() => setFilters({})}>
                      Clear filters
                    </button>
                  </div>
                )}
              </div>
            )}
          </div>
        </section>
      }
      detail={
        <section className="ax-detail" aria-label="Equipment detail">
          {selected?.platform ? (
            <DetailFrame onBack={() => setSelectedId(null)}>
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
            </DetailFrame>
          ) : selected?.section ? (
            <DetailFrame onBack={() => setSelectedId(null)}>
              <SectionDetail section={selected.section} />
            </DetailFrame>
          ) : (
            <DetailPlaceholder>Select an item to see its protection, weapons and reach.</DetailPlaceholder>
          )}
        </section>
      }
    />
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
      ? "Any"
      : selected.length === 1
        ? selected[0].label
        : `${selected.length} selected`;

  return (
    <div ref={containerRef} className="ax-facet">
      <button
        type="button"
        className="ax-facet-button"
        data-active={selected.length > 0}
        onClick={() => {
          setNeedle("");
          onOpenChange(!open);
        }}
        aria-expanded={open}
        aria-label={`${label} filter, ${summary}`}
      >
        <span className="ax-facet-label">{label}</span>
        <span className="ax-facet-summary">{summary}</span>
        <span className="ax-facet-caret" aria-hidden="true">
          {open ? "\u25b2" : "\u25bc"}
        </span>
      </button>

      {open && (
        <div className="ax-facet-panel">
          {/* Only where the list is long enough to need it. Nation runs to
              dozens of entries; type has four. */}
          {entries.length > 8 && (
            <input
              type="search"
              className="ax-input ax-input-sm"
              value={needle}
              onChange={(event) => setNeedle(event.target.value)}
              placeholder={`Filter ${entries.length} ${label}s…`}
              aria-label={`Filter ${label} options`}
              autoFocus
              style={{ marginBottom: 6 }}
            />
          )}

          <div className="ax-facet-options">
            {shown.map((entry) => (
              <button
                key={entry.value}
                type="button"
                className="ax-option"
                onClick={() => onToggle(entry.value)}
                role="checkbox"
                aria-checked={entry.selected}
              >
                {/* A drawn box rather than a real checkbox: the row is the
                    hit target, and a nested input would swallow its clicks. */}
                <span className="ax-check" aria-hidden="true">
                  {entry.selected ? "\u2713" : ""}
                </span>
                <span className="ax-option-label">{entry.label}</span>
                {/* Zero is shown, not hidden: a selected value that now
                    matches nothing is information, and facetCounts keeps it
                    in the list precisely so it can be unselected. */}
                <span className="ax-option-count">{entry.count}</span>
              </button>
            ))}
            {shown.length === 0 && <div className="ax-empty" style={{ padding: 8 }}>Nothing matches</div>}
          </div>

          {selected.length > 0 && (
            <button
              type="button"
              className="ax-btn ax-btn-sm ax-btn-ghost ax-btn-danger"
              onClick={onClear}
              style={{ width: "100%", marginTop: 4 }}
            >
              Clear {label}
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
      type="button"
      className="ax-chip"
      data-state={on ? "on" : "off"}
      aria-pressed={on}
      onClick={onClick}
      title={hint}
    >
      {on && <span aria-hidden="true">✓</span>}
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
    <>
      <div className="ax-hero">
        <ImageFrame
          image={image}
          alt={platform.displayName}
          missing={
            platform.imageFilename
              ? "No image in the set for this platform."
              : "No photograph recorded for this platform."
          }
          failureHint="Check the Mildata Images media set is a Resource on this app."
          caption={image?.path}
        />
        <div className="ax-hero-title">
          <h2>{platform.displayName}</h2>
          <div className="ax-hero-id">
            {platform.assetId}
            {platform.mfId && ` · Military Factory id ${platform.mfId}`}
          </div>
          <div className="ax-badges">
            {platform.domain && <span className="ax-badge ax-badge-accent">{platform.domain}</span>}
            {platform.subclass && <span className="ax-badge">{platform.subclass}</span>}
            {platform.nation && <span className="ax-badge ax-badge-muted">{platform.nation}</span>}
            {platform.playable ? (
              <span className="ax-badge ax-badge-good">Fieldable in BGWS</span>
            ) : (
              <span className="ax-badge ax-badge-muted">Not fieldable in BGWS</span>
            )}
          </div>
          <Kpis
            items={[
              { label: "Combat strength", value: platform.csIndex != null ? fmt(platform.csIndex) : "—" },
              { label: "Mass", value: fmt(platform.massT, " t") },
              { label: "Road speed", value: fmt(platform.speedKmh, " km/h") },
              { label: "Crew", value: fmt(platform.crewSize) },
            ]}
          />
        </div>
      </div>

      <div className="ax-groups">
        {hasArmour ? (
          <Group title="Protection by aspect">
            <ArmourFacings facings={platform.facings} />
            <div className="ax-note">
              {platform.eraFit && platform.eraFit !== "none" && `ERA: ${platform.eraFit}. `}
              {platform.apsFit && platform.apsFit !== "none" && `APS: ${platform.apsFit}. `}
              {platform.protectionConfidence && `Confidence ${platform.protectionConfidence}.`}
            </div>
            {platform.protectionSourceRef && (
              <div className="ax-note ax-note-source">{platform.protectionSourceRef}</div>
            )}
          </Group>
        ) : (
          <Group title="Protection by aspect">
            {/* Aircraft and warships have no protection rows in the curated
                tables. That is an absence of a model, not armour of zero. */}
            <div className="ax-note" style={{ marginTop: 0 }}>
              No armour model for this platform. The curated tables carry
              protection for land equipment only.
            </div>
          </Group>
        )}

        <Group title="Capabilities and reach">
          {capabilities.length === 0 && (
            <div className="ax-note" style={{ marginTop: 0 }}>No capability rows.</div>
          )}
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
              <div key={capability.capability} className="ax-cap">
                <div className="ax-subhead">
                  {capability.capability.toUpperCase()}
                  {capability.hasTopAttack && (
                    <span className="ax-badge ax-badge-info" style={{ marginLeft: 8 }}>
                      top attack
                    </span>
                  )}
                </div>
                <div className="ax-meta">
                  {fmt(capability.maxRangeM, " m")} max
                  {capability.minRangeM ? ` · ${capability.minRangeM} m min` : ""}
                  {capability.shortRangeM
                    ? ` · short under ${Math.round(capability.shortRangeM)} m`
                    : ""}
                </div>
                {capability.weapons.length > 0 && (
                  <div className="ax-meta">{capability.weapons.join(", ")}</div>
                )}
                {points.length >= 2 && (
                  <div className="ax-chart">
                    <PenetrationCurve points={points} />
                  </div>
                )}
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
              <div key={mission.assetId + mission.missionType} className="ax-cap">
                <div className="ax-subhead">
                  {(mission.missionType ?? "—").replace(/_/g, " ")}
                  <span className="ax-meta" style={{ marginLeft: 6, fontWeight: 400 }}>
                    {mission.supportRole === "excluded"
                      ? "· beyond a battlegroup action"
                      : `· ${mission.supportRole ?? "unclassified"}`}
                  </span>
                </div>
                <div className="ax-meta">
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
            <div className="ax-note">
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
            <div className="ax-rows">
              <Row label="Band" value={platform.survivabilityBand} />
              <Row
                label="Threatened by"
                value={(platform.threatVulnerability ?? "—").replace(/_/g, " ")}
              />
            </div>
            <div className="ax-note">
              Not armour. This asset has no frontal RHA figure and never will —
              the band says what weapon class can engage it, which is the
              equivalent question for something that is not a ground vehicle.
            </div>
          </Group>
        )}

        <Group title="As a counter">
          <div className="ax-rows">
            <Row
              label="Combat Strength"
              value={platform.csIndex != null ? fmt(platform.csIndex) : "not rated"}
            />
            <Row label="Move type" value={platform.moveType ?? "—"} />
            <Row label="Target class" value={platform.targetClass ?? "—"} />
            <Row label="Protection band" value={platform.protectionBand ?? "—"} />
            <Row label="Dismounts" value={fmt(platform.dismountsCarried)} />
            <Row label="Optics class" value={fmt(platform.opticsClass)} />
            <Row label="Signature class" value={fmt(platform.signatureClass)} />
            {platform.requiresPrimeMover && (
              <Row label="Towed" value="needs a prime mover — mapped to Wheeled" />
            )}
          </div>
          {/* The absence is deliberate and worth saying out loud, or a reader
              assumes the pipeline failed to compute something. */}
          {platform.csIndex == null ? (
            <div className="ax-note">
              Combat Strength rates a LAND counter: penetration at 1,000 m against
              frontal armour, on fire columns calibrated for 1–5 km. Scoring an
              aircraft or a warship on it would produce a number that looks like a
              measurement and is not, so this is left unrated.
            </div>
          ) : (
            <div className="ax-note">
              Combat Strength is DERIVED, not sourced: from {fmt(platform.csInputPenMm, " mm")}{" "}
              penetration, {fmt(platform.csInputArmourMm, " mm")} frontal armour and{" "}
              {fmt(platform.csInputRangeM, " m")} reach.
            </div>
          )}
        </Group>

        <Group title="Platform">
          <div className="ax-rows">
            <Row label="Domain" value={platform.domain ?? "—"} />
            <Row label="Class" value={platform.subclass ?? "—"} />
            <Row label="Nation" value={platform.nation ?? "—"} />
            <Row label="Mass" value={fmt(platform.massT, " t")} />
            <Row label="Road speed" value={fmt(platform.speedKmh, " km/h")} />
            <Row label="Crew" value={fmt(platform.crewSize)} />
            <Row label="Weapons" value={fmt(platform.weaponCount)} />
          </div>
        </Group>
      </div>
    </>
  );
}

// ─── Section detail ────────────────────────────────────────────────────────

function SectionDetail({ section }: { section: CuratedSection }) {
  return (
    <>
      <div className="ax-hero-title" style={{ marginBottom: 20 }}>
        <h2>{section.displayName}</h2>
        <div className="ax-hero-id">{section.assetId}</div>
        <div className="ax-badges">
          <span className="ax-badge ax-badge-accent">infantry section</span>
          {section.defaultCarrier && (
            <span className="ax-badge ax-badge-muted">carried by {section.defaultCarrier}</span>
          )}
        </div>
        <Kpis
          items={[
            { label: "Strength", value: fmt(section.strength, " men") },
            { label: "Fireteams", value: fmt(section.fireteams) },
            { label: "Combat strength", value: fmt(section.csIndex) },
            { label: "Optics class", value: fmt(section.opticsClass) },
          ]}
        />
      </div>

      <div className="ax-groups">
        <Group title="Reach, per capability">
          {/* ⚠ PER CAPABILITY, NOT ONE FIGURE. A Kornet detachment's rifles
              reach 400 m and its missile 5,500. A single "range" would be a lie
              about whichever weapon it was not describing. */}
          <div className="ax-rows">
            <ReachRow label="APERS" range={section.apersMaxRangeM} />
            <ReachRow label="ATK" range={section.atkMaxRangeM} pen={section.atkPenMm1000m} />
            <ReachRow label="ATM" range={section.atmMaxRangeM} pen={section.atmPenMm1000m} />
            <ReachRow label="AA" range={section.aaMaxRangeM} />
            <ReachRow label="IDF" range={section.idfMaxRangeM} />
          </div>
        </Group>

        <Group title="Weapons carried">
          {section.weapons.length === 0 ? (
            <div className="ax-note" style={{ marginTop: 0 }}>No loadout recorded.</div>
          ) : (
            <div className="ax-badges">
              {section.weapons.map((weapon) => (
                <span key={weapon} className="ax-badge">
                  {weapon}
                </span>
              ))}
            </div>
          )}
          <div className="ax-note">
            Penetration is the round the section CARRIES, not the best its
            launcher can fire.
          </div>
        </Group>

        {section.sourceRef && (
          <Group title="Source">
            <div className="ax-note ax-note-source" style={{ marginTop: 0 }}>
              {section.sourceRef}
              {section.sourceConfidence && ` · confidence ${section.sourceConfidence}`}
            </div>
          </Group>
        )}
      </div>
    </>
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
    <Row
      label={label}
      value={`${Math.round(range)} m`}
      note={pen != null ? `${Math.round(pen)} mm at 1 km` : undefined}
    />
  );
}

// ─── Munitions ─────────────────────────────────────────────────────────────

function MunitionList({
  munitions,
  loading,
  search,
  onSearch,
}: {
  munitions: CuratedMunition[];
  loading: boolean;
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
    <div className="ax-munitions">
      <div className="ax-munitions-head">
        <SearchInput
          value={search}
          onChange={onSearch}
          placeholder="Search munitions…"
          label="Search munitions"
        />
        <span className="ax-meta" aria-live="polite">
          {loading
            ? "Loading…"
            : `${counts.total} rounds · ${counts.curves} with a plottable curve · ${counts.flat} of those flat with range because they are shaped charges`}
        </span>
      </div>

      {loading ? (
        <div className="ax-card-grid">
          {Array.from({ length: 9 }, (_, index) => (
            <div key={index} className="ax-skeleton" style={{ height: 190, margin: 0 }} />
          ))}
        </div>
      ) : munitions.length === 0 ? (
        <div className="ax-empty">No munitions match.</div>
      ) : (
        <div className="ax-card-grid">
          {munitions.map((munition) => {
            const falloff = curveFalloff(munition);
            return (
              <article key={munition.munitionId} className="ax-card">
                <div className="ax-card-title">{munition.name}</div>
                <div className="ax-badges" style={{ marginTop: 6 }}>
                  <span className="ax-badge">{munition.kind ?? "?"}</span>
                  {munition.topAttack && <span className="ax-badge ax-badge-info">top attack</span>}
                  {munition.defeatsEraKe && <span className="ax-badge ax-badge-accent">defeats ERA</span>}
                  {munition.sourceConfidence && (
                    <span className="ax-badge ax-badge-muted">confidence {munition.sourceConfidence}</span>
                  )}
                </div>

                {hasCurve(munition) ? (
                  <>
                    <div className="ax-chart">
                      <PenetrationCurve
                        points={penetrationCurve(munition)}
                        flat={munition.isShapedCharge}
                        label={`${munition.name} penetration against range`}
                      />
                    </div>
                    <div className="ax-meta">
                      {falloff != null && falloff < 0.01
                        ? "Flat with range — the jet is formed on impact"
                        : falloff != null
                          ? `${Math.round(falloff * 100)}% lost over the plotted range`
                          : ""}
                    </div>
                  </>
                ) : (
                  <div className="ax-note">
                    No anti-armour penetration to plot
                    {munition.lethalRadiusM
                      ? ` · ${munition.lethalRadiusM} m lethal radius`
                      : ""}
                    {munition.heFillKg ? ` · ${munition.heFillKg} kg fill` : ""}
                  </div>
                )}

                {munition.sourceRef && (
                  <div className="ax-note ax-note-source">{munition.sourceRef}</div>
                )}
              </article>
            );
          })}
        </div>
      )}
    </div>
  );
}
