/**
 * AssetExplorer — every asset in the catalogue, with its stats and photograph.
 *
 * Reads [SIM] L5 sim_asset_card (all 2,258 assets, every domain) rather than
 * the L6 wargame profiles, because an explorer should show what the source
 * actually holds, not the land-only subset a wargame needs.
 *
 * ONE OPINION THROUGHOUT: a figure is shown with its PROVENANCE. `speed_source`
 * and `range_source` are on screen next to the numbers they qualify, because
 * this source's top speed is a copy-pasted 75 km/h for almost every vehicle and
 * a reader who does not know that will believe it.
 *
 * The filter model lives in ./data/assetFilters, which is pure and tested. Only
 * the rendering is here. That split matters: a wrong predicate does not throw,
 * it quietly returns the wrong assets, and a list of tanks looks equally
 * plausible either way.
 *
 * Layout and styling: ./explorer.css and ./components/explorerUi — three panes
 * on a wide screen, a filter drawer below 1280px, one pane at a time on a
 * phone.
 *
 * ⚠ Needs [SIM] L5 sim_asset_card, [SIM] L6 unit_image_index and the Unit
 * Images media set added as Resources on this app in Developer Console.
 * Without the first the list is empty and says so; without the others images
 * are missing and say that too.
 */

import { useCallback, useEffect, useMemo, useState } from "react";
import { Link } from "react-router-dom";

import { AppSwitcher } from "@/components/AppSwitcher";
import { SqlError } from "@/shared/lib/sqlClient";
import { CuratedExplorer, type CuratedMode } from "./CuratedExplorer";
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
import "./explorer.css";

import { kmh, type AssetCard } from "./data/assetCards";
import {
  activeFilterCount,
  cycleExists,
  cycleFlag,
  EXISTS_KEYS,
  EXISTS_LABEL,
  FACET_KEYS,
  FACET_LABEL,
  FLAG_KEYS,
  FLAG_LABEL,
  groupFacetCounts,
  NULL_FACET,
  RANGE_KEYS,
  RANGE_LABEL,
  RANGE_WARNING,
  setRange,
  SORT_LABEL,
  toggleFacetValue,
  type AssetFilters,
  type FacetCount,
  type FacetKey,
  type SortKey,
} from "./data/assetFilters";
import {
  loadAssetCards,
  loadFacetCounts,
  loadImageCount,
  loadImageIndex,
  loadMatchCount,
} from "./data/assetCardsClient";
import { describePlayability, playabilityOfProfile } from "./data/playability";
import type { PlatformProfile } from "./data/profiles";
import { loadPlatformProfile } from "./data/profilesClient";
import type { UnitImageRow } from "./data/unitImageIndex";
import { resolveUnitImage, resolveUnitImageByPath, type ResolvedImage } from "./data/unitImages";

type ExplorerMode = "simulator" | CuratedMode;

/**
 * The catalogues this explorer can show.
 *
 * ⚠ SEPARATE MODES, NOT ONE MERGED LIST. The simulator export and the curated
 * tables describe overlapping equipment with disjoint identifiers, and the
 * only way to merge them today would be to match display names -- the exercise
 * that matched 10 of 133 vehicles by hand and produced "M2 Bradley" to an M2
 * Browning machine gun along the way. The reader is told which catalogue they
 * are in instead.
 */
const MODES: { id: ExplorerMode; label: string; hint: string }[] = [
  {
    id: "curated",
    label: "Equipment",
    hint:
      "[SIM] L7 — 203 platforms across land, air and sea plus 20 infantry sections, " +
      "filterable by type, class, nation and capability",
  },
  {
    id: "munitions",
    label: "Munitions",
    hint: "[SIM] L7 bgws_munition_profile — 150 rounds with penetration curves",
  },
  // Last, and named for what it is. The simulator export is a tank game's
  // asset library: 1,239 platforms sharing 72 statlines, with a Sturmtiger, a
  // Maus and an esports livery in it. It is kept for lineage and for the
  // images, not because anybody should field from it.
  {
    id: "simulator",
    label: "Simulator (legacy)",
    hint: "[SIM] L5 sim_asset_card — the game-engine export, 2,258 rows, kept for lineage",
  },
];

const PAGE_LIMIT = 300;

const SORT_KEYS: SortKey[] = [
  "name",
  "massDesc",
  "armourDesc",
  "calibreDesc",
  "completenessDesc",
];

export default function AssetExplorer() {
  // Which catalogue is on screen. The simulator export and the curated tables
  // share no identifiers, so they are browsed separately rather than joined
  // by name -- see the note at the top of CuratedExplorer.
  const [mode, setMode] = useState<ExplorerMode>("curated");
  const current = MODES.find((entry) => entry.id === mode);

  return (
    <div className="ax-root">
      <header className="ax-topbar">
        <Link to="/bgws" className="ax-back">
          ‹ Main menu
        </Link>
        <div className="ax-title">
          <h1>Asset Explorer</h1>
          <span className="ax-subtitle" title={current?.hint}>
            {current?.hint}
          </span>
        </div>
        <span className="ax-topbar-spacer" />
        <div className="ax-segmented" role="group" aria-label="Catalogue">
          {MODES.map((entry) => (
            <button
              key={entry.id}
              type="button"
              onClick={() => setMode(entry.id)}
              aria-pressed={mode === entry.id}
              title={entry.hint}
            >
              {entry.label}
            </button>
          ))}
        </div>
        <AppSwitcher />
      </header>

      <div className="ax-body">
        {mode === "simulator" ? <SimulatorExplorer /> : <CuratedExplorer mode={mode} />}
      </div>
    </div>
  );
}

// ─── Simulator (legacy) catalogue ──────────────────────────────────────────

function SimulatorExplorer() {
  const [filters, setFilters] = useState<AssetFilters>({});
  const [facetCounts, setFacetCounts] = useState<FacetCount[]>([]);
  const [cards, setCards] = useState<AssetCard[]>([]);
  const [matchCount, setMatchCount] = useState<number | null>(null);
  const [imageCount, setImageCount] = useState<number | null>(null);
  const [images, setImages] = useState<Map<string, UnitImageRow>>(new Map());
  const [selected, setSelected] = useState<AssetCard | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [filtersOpen, setFiltersOpen] = useState(false);
  const closeFilters = useCallback(() => setFiltersOpen(false), []);

  const facets = useMemo(() => groupFacetCounts(facetCounts), [facetCounts]);
  const activeCount = activeFilterCount(filters);

  useEffect(() => {
    const controller = new AbortController();
    loadFacetCounts(controller.signal)
      .then(setFacetCounts)
      .catch(() => setFacetCounts([]));
    // Counted rather than written into the copy. The figure moves whenever
    // someone adds images to the media set, and a hardcoded "799 of 2,258"
    // would be wrong by the second batch while still looking authoritative.
    loadImageCount(controller.signal)
      .then(setImageCount)
      .catch(() => setImageCount(null));
    return () => controller.abort();
  }, []);

  useEffect(() => {
    const controller = new AbortController();
    setLoading(true);
    setError(null);

    // Debounced: a search box that fires a SQL query per keystroke is a
    // search box that spends its life cancelling itself.
    const timer = setTimeout(() => {
      const query = { ...filters, limit: PAGE_LIMIT };

      loadAssetCards(query, controller.signal)
        .then((rows) => {
          setCards(rows);
          setLoading(false);

          // Images for this page only, and on a SEPARATE failure path. The
          // index makes this one query for the whole page instead of up to
          // nine calls per asset — but it is decoration, and an explorer
          // that refuses to list 2,258 assets because it could not decorate
          // them is worse than one with no dots. Notably the index does not
          // exist on master until its pipeline PR is merged and built, so
          // this path WILL fail for a while.
          loadImageIndex(
            rows.map((row) => row.assetId),
            controller.signal,
          )
            .then(setImages)
            .catch(() => setImages(new Map()));
        })
        .catch((err: unknown) => {
          if (controller.signal.aborted) return;
          setLoading(false);
          if (err instanceof SqlError && err.permissionDenied) {
            setError(
              "No access to [SIM] L5 sim_asset_card. Add it as a Resource on this " +
                "app in Developer Console — the SQL scopes alone return 403.",
            );
          } else {
            setError(err instanceof Error ? err.message : String(err));
          }
        });

      // Deliberately not chained onto the list: the count is over the whole
      // catalogue and should not hold up rendering the rows we already have.
      loadMatchCount(query, controller.signal)
        .then(setMatchCount)
        .catch(() => setMatchCount(null));
    }, 250);

    return () => {
      controller.abort();
      clearTimeout(timer);
    };
  }, [filters]);

  const shown = cards.length;
  const capped = shown === PAGE_LIMIT;
  const countLabel = loading
    ? "Loading…"
    : matchCount === null
      ? `${shown} shown`
      : capped
        ? `${shown} of ${matchCount} matches (capped)`
        : `${matchCount} ${matchCount === 1 ? "match" : "matches"}`;

  return (
    <ExplorerLayout
      filtersOpen={filtersOpen}
      hasSelection={selected !== null}
      onCloseFilters={closeFilters}
      filters={
        <FiltersPanel
          activeCount={activeCount}
          onClear={() => setFilters({ sort: filters.sort })}
          onClose={closeFilters}
        >
          <SimulatorFilters
            filters={filters}
            facets={facets}
            onChange={setFilters}
            imageCount={imageCount}
          />
        </FiltersPanel>
      }
      list={
        <section className="ax-list" aria-label="Assets">
          <div className="ax-list-toolbar">
            <div className="ax-list-toolbar-row">
              <SearchInput
                value={filters.search ?? ""}
                onChange={(search) => setFilters({ ...filters, search })}
                placeholder="Search name or id…"
                label="Search assets"
              />
              <FiltersButton activeCount={activeCount} onClick={() => setFiltersOpen(true)} />
            </div>
            <div className="ax-list-toolbar-row">
              <span className="ax-meta" style={{ flex: 1 }} aria-live="polite">
                {countLabel}
              </span>
              <label className="ax-meta" htmlFor="ax-sim-sort">
                Sort
              </label>
              <select
                id="ax-sim-sort"
                className="ax-select"
                value={filters.sort ?? "name"}
                onChange={(e) => setFilters({ ...filters, sort: e.target.value as SortKey })}
              >
                {SORT_KEYS.map((key) => (
                  <option key={key} value={key}>
                    {SORT_LABEL[key]}
                  </option>
                ))}
              </select>
            </div>
          </div>

          {error && <div className="ax-error">{error}</div>}

          <div className="ax-list-scroll">
            {loading && cards.length === 0 ? (
              <ListSkeleton />
            ) : (
              cards.map((card) => (
                <button
                  key={card.assetId}
                  type="button"
                  className="ax-item"
                  aria-current={selected?.assetId === card.assetId}
                  onClick={() => setSelected(card)}
                >
                  <span className="ax-item-glyph" aria-hidden="true">
                    {domainGlyph(card.domain)}
                  </span>
                  <span className="ax-item-main">
                    <span className="ax-item-name">{card.displayName}</span>
                    <span className="ax-item-sub">
                      {[card.domain, card.subclass, card.nation].filter(Boolean).join(" · ")}
                    </span>
                  </span>
                  {/* A dot, not a word: the list is dense and this is a hint,
                      not a fact worth a column. */}
                  {images.has(card.assetId) && (
                    <span className="ax-item-end" title="Has a photograph">
                      <span className="ax-dot" style={{ background: "var(--ax-good)" }} />
                    </span>
                  )}
                </button>
              ))
            )}
            {!loading && !error && cards.length === 0 && (
              <div className="ax-empty">Nothing matches these filters.</div>
            )}
          </div>
        </section>
      }
      detail={
        <section className="ax-detail" aria-label="Asset detail">
          {selected ? (
            <DetailFrame onBack={() => setSelected(null)}>
              <AssetDetail card={selected} indexed={images.get(selected.assetId) ?? null} />
            </DetailFrame>
          ) : (
            <DetailPlaceholder>Select an asset to see its stats and photograph.</DetailPlaceholder>
          )}
        </section>
      }
    />
  );
}

function domainGlyph(domain: string | null | undefined): string {
  const value = (domain ?? "").toLowerCase();
  if (value.includes("air") || value.includes("heli")) return "✈";
  if (value.includes("naval") || value.includes("sea") || value.includes("ship")) return "⚓";
  if (value.includes("inf")) return "⚑";
  return "▣";
}

function SimulatorFilters({
  filters,
  facets,
  onChange,
  imageCount,
}: {
  filters: AssetFilters;
  facets: Record<FacetKey, FacetCount[]>;
  onChange: (next: AssetFilters) => void;
  imageCount: number | null;
}) {
  return (
    <>
      <FilterSection title="Presence">
        <div className="ax-chips">
          {EXISTS_KEYS.map((key) => (
            <TriStateChip
              key={key}
              label={EXISTS_LABEL[key]}
              value={filters.exists?.[key]}
              onClick={() => onChange(cycleExists(filters, key))}
            />
          ))}
        </div>
        <div className="ax-note">
          {imageCount !== null ? (
            <>{imageCount} assets have a photograph.</>
          ) : (
            <>
              The image index is unavailable — it may not be built on this branch
              yet, in which case &ldquo;has image&rdquo; will match nothing.
            </>
          )}
        </div>
      </FilterSection>

      <FilterSection title="Flags">
        <div className="ax-chips">
          {FLAG_KEYS.map((key) => (
            <TriStateChip
              key={key}
              label={FLAG_LABEL[key]}
              value={filters.flags?.[key]}
              onClick={() => onChange(cycleFlag(filters, key))}
            />
          ))}
        </div>
        <div className="ax-note">Click once to require, twice to exclude, again to clear.</div>
      </FilterSection>

      {FACET_KEYS.map((facet) => {
        const values = facets[facet];
        if (!values.length) return null;
        const selected = filters.facets?.[facet] ?? [];

        return (
          <FilterSection key={facet} title={FACET_LABEL[facet]}>
            <div className="ax-chips">
              {values.map((entry) => (
                <button
                  key={entry.value}
                  type="button"
                  className="ax-chip"
                  data-state={selected.includes(entry.value) ? "on" : "off"}
                  aria-pressed={selected.includes(entry.value)}
                  onClick={() => onChange(toggleFacetValue(filters, facet, entry.value))}
                  // The null bucket is a real answer about the data, not a
                  // missing chip, so it is shown — just muted.
                  style={entry.value === NULL_FACET ? { fontStyle: "italic" } : undefined}
                >
                  {entry.value}
                  <span className="ax-option-count">{entry.count}</span>
                </button>
              ))}
            </div>
            {facet === "nation" && (
              <div className="ax-note">
                The source carries both `united_kingdom` and `britain`, and both
                `united_states` and `usa`. These are folded together here.
              </div>
            )}
          </FilterSection>
        );
      })}

      <FilterSection title="Ranges">
        <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
          {RANGE_KEYS.map((key) => {
            const range = filters.ranges?.[key];
            const warning = RANGE_WARNING[key];
            const active = range?.min !== undefined || range?.max !== undefined;

            return (
              <div key={key}>
                <div style={{ display: "flex", alignItems: "center", gap: 6 }}>
                  <span className="ax-row-label" style={{ flex: 1, minWidth: 0 }}>
                    {RANGE_LABEL[key]}
                    {warning && <span style={{ color: "var(--ax-warn)" }}> ⚠</span>}
                  </span>
                  <input
                    type="number"
                    className="ax-input ax-input-num"
                    value={range?.min ?? ""}
                    placeholder="min"
                    aria-label={`${RANGE_LABEL[key]} minimum`}
                    onChange={(e) =>
                      onChange(
                        setRange(filters, key, {
                          ...range,
                          min: e.target.value === "" ? undefined : Number(e.target.value),
                        }),
                      )
                    }
                  />
                  <input
                    type="number"
                    className="ax-input ax-input-num"
                    value={range?.max ?? ""}
                    placeholder="max"
                    aria-label={`${RANGE_LABEL[key]} maximum`}
                    onChange={(e) =>
                      onChange(
                        setRange(filters, key, {
                          ...range,
                          max: e.target.value === "" ? undefined : Number(e.target.value),
                        }),
                      )
                    }
                  />
                </div>
                {/* Shown only once the filter is in use. A panel of permanent
                    warnings is a panel nobody reads; one that appears at the
                    moment the trap is stepped in gets seen. */}
                {warning && active && (
                  <div className="ax-note" style={{ color: "var(--ax-warn)", marginTop: 4 }}>
                    {warning}
                  </div>
                )}
              </div>
            );
          })}
        </div>
        <div className="ax-note">
          A range excludes assets with no value at all. Only 1,132 of 2,258 have
          a mass; 234 have a year.
        </div>
      </FilterSection>
    </>
  );
}

/**
 * Unset → require true → require false → unset.
 *
 * Three states rather than a checkbox because "not armed" is a question worth
 * asking of this catalogue, and a checkbox can only ask one of the two.
 */
function TriStateChip({
  label,
  value,
  onClick,
}: {
  label: string;
  value: boolean | undefined;
  onClick: () => void;
}) {
  const state = value === undefined ? "off" : value ? "yes" : "no";
  const mark = value === undefined ? "" : value ? "✓" : "✕";
  const spoken = value === undefined ? "any" : value ? "required" : "excluded";

  return (
    <button
      type="button"
      className="ax-chip"
      data-state={state}
      onClick={onClick}
      aria-label={`${label}: ${spoken}`}
    >
      {mark && <span aria-hidden="true">{mark}</span>}
      {label}
    </button>
  );
}

function AssetDetail({
  card,
  indexed,
}: {
  card: AssetCard;
  indexed: UnitImageRow | null;
}) {
  const [image, setImage] = useState<ResolvedImage | null>(null);
  const [profile, setProfile] = useState<PlatformProfile | null>(null);

  useEffect(() => {
    let cancelled = false;
    setImage(null);

    // The index knows the path, so fetch exactly that. Only fall back to
    // guessing nine filenames when the index has no row for this asset —
    // which also keeps the pane working before the index is built.
    const lookup = indexed
      ? resolveUnitImageByPath(indexed.imagePath)
      : resolveUnitImage(card.assetId, card.displayName);

    lookup.then((resolved) => {
      if (!cancelled) setImage(resolved);
    });
    return () => {
      cancelled = true;
    };
  }, [card.assetId, card.displayName, indexed]);

  // The derived L6 profile, where there is one. Land assets only.
  useEffect(() => {
    const controller = new AbortController();
    setProfile(null);
    loadPlatformProfile(card.assetId, controller.signal)
      .then(setProfile)
      .catch(() => setProfile(null));
    return () => controller.abort();
  }, [card.assetId]);

  const groups = useMemo(() => statGroups(card, profile), [card, profile]);

  return (
    <>
      <div className="ax-hero">
        <ImageFrame
          image={image}
          alt={card.displayName}
          missing="No image in the set for this asset."
          failureHint="Check the Unit Images media set is a Resource on this app."
          caption={
            image?.path && (
              <>
                {image.path}
                {/* Which convention matched is exactly the question asked when a
                    new batch of images does not appear. */}
                {indexed?.matchMethod && ` · matched by ${indexed.matchMethod}`}
              </>
            )
          }
        />
        <div className="ax-hero-title">
          <h2>{card.displayName}</h2>
          <div className="ax-hero-id">{card.assetId}</div>
          <div className="ax-badges">
            {card.domain && <span className="ax-badge ax-badge-accent">{card.domain}</span>}
            {card.subclass && <span className="ax-badge">{card.subclass}</span>}
            {card.nation && <span className="ax-badge ax-badge-muted">{card.nation}</span>}
            <span className="ax-badge ax-badge-muted">legacy simulator export</span>
          </div>
          <Kpis
            items={[
              { label: "Mass", value: fmt(profile?.massCombatT ?? card.massT, " t") },
              { label: "Armour max", value: fmt(card.armourMaxMm, " mm") },
              { label: "Calibre", value: fmt(card.primaryCalibreMm, " mm") },
              { label: "Crew", value: fmt(card.crewSize) },
            ]}
          />
        </div>
      </div>

      <div className="ax-groups">
        {groups.map((group) => (
          <Group key={group.title} title={group.title} wide={group.wide}>
            <div className="ax-rows">
              {group.rows.map(([label, value, note]) => (
                <Row key={label} label={label} value={value} note={note} />
              ))}
            </div>
          </Group>
        ))}
      </div>
    </>
  );
}

type StatRow = [label: string, value: string, note?: string];

function statGroups(
  card: AssetCard,
  profile: PlatformProfile | null,
): { title: string; rows: StatRow[]; wide?: boolean }[] {
  const show = (value: number | string | null | undefined, suffix = ""): string =>
    value == null || value === "" ? "—" : `${value}${suffix}`;

  // Derived figures come FIRST when they exist, because the source's own
  // mobility numbers are not trustworthy and a reader scans from the top.
  const derived: { title: string; rows: StatRow[]; wide?: boolean }[] = profile
    ? [
        {
          title: "Derived · mobility (L6)",
          wide: true,
          rows: [
            [
              "Combat mass",
              show(profile.massCombatT, " t"),
              profile.massSource === "combat_mass_takeoff"
                ? "VehiclePhys.Mass.TakeOff"
                : "placeholder — the source's own mass field",
            ],
            ["Engine", show(profile.engineHp, " hp")],
            [
              "Power to weight",
              show(profile.hpPerTonne, " hp/t"),
              profile.hpPerTonneMethod ?? undefined,
            ],
            ["Mobility class", show(profile.mobilityClass)],
            [
              "Top speed",
              show(profile.gearLimitedKmh, " km/h"),
              profile.gearLimitedConfidence === "drivetrain_with_gearbox"
                ? "engine speed through top gear, main and side drive"
                : profile.gearLimitedConfidence === "no_gear_table_assumed_direct_drive"
                  ? "no gearbox in the source — direct drive assumed, treat as a ceiling"
                  : (profile.gearLimitedConfidence ?? undefined),
            ],
            ["Move type", show(profile.moveType)],
            ["Target class", show(profile.targetClass)],
            [
              // The Explorer browses the whole catalogue; the game can only
              // field 819 of its 1,239 rows. Saying WHY a platform cannot be
              // fielded turns a puzzling absence into a fact about the source.
              "In the wargame",
              describePlayability(playabilityOfProfile(profile)),
            ],
            [
              "Capabilities",
              profile.capabilities.length ? profile.capabilities.join(", ") : "—",
            ],
            ["Best penetration", show(profile.bestPenMm1000m, " mm at 1 km")],
          ],
        },
      ]
    : [];

  return [
    ...derived,
    {
      title: "Identity",
      rows: [
        ["Domain", show(card.domain)],
        ["Subclass", show(card.subclass)],
        ["Nation", show(card.nation), "source tech tree, not an army"],
        ["Years", card.yearFrom ? `${card.yearFrom}–${card.yearTo ?? ""}` : "—"],
        ["Variant of", show(card.baseAssetId)],
        ["Completeness", show(card.completeness)],
      ],
    },
    {
      title: "As the source states it",
      rows: [
        ["Mass", show(card.massT, " t"), profile?.massCombatT ? "placeholder" : undefined],
        [
          "Max speed",
          show(kmh(card.maxSpeedMps), " km/h"),
          // Naming the field it came from was not enough: a reader has to be
          // told the number means nothing. 1,067 of 1,160 ground vehicles in
          // this source carry exactly this figure.
          profile?.statcardSpeedIsTemplate
            ? "TEMPLATE CONSTANT — shared by ~92% of vehicles, not a performance figure"
            : (card.speedSource ?? undefined),
        ],
        ["Mobile", card.isMobile ? "yes" : "no"],
        ["Crew", show(card.crewSize)],
      ],
    },
    {
      title: "Protection",
      rows: [
        ["Armour (max)", show(card.armourMaxMm, " mm")],
        ["Armour class", show(card.armourClass)],
        ["Hit points", show(card.hpTotal)],
      ],
    },
    {
      title: "Armament",
      rows: [
        ["Armed", card.isArmed ? "yes" : "no"],
        ["Weapons", show(card.weaponCount)],
        ["Primary", show(card.primaryWeaponRef)],
        ["Calibre", show(card.primaryCalibreMm, " mm")],
        ["Rate of fire", show(card.primaryRateOfFireRpm, " rpm")],
        ["Muzzle velocity", show(card.primaryMuzzleVelocityMps, " m/s")],
        ["Explosive mass", show(card.primaryExplosiveMassKg, " kg")],
        ["Munitions", card.munitionFamilies.length ? card.munitionFamilies.join(", ") : "—"],
        [
          "Engagement range",
          show(card.maxEngagementRangeM, " m"),
          card.rangeSource ?? undefined,
        ],
        ["Engages", [card.engagesGround && "ground", card.engagesAir && "air"].filter(Boolean).join(", ") || "—"],
        ["Detection", show(card.detectDistanceM, " m")],
      ],
    },
    {
      title: "Provenance",
      rows: [["Source file", show(card.sourcePath)]],
    },
  ];
}
