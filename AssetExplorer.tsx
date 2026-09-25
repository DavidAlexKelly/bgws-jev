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
 * ⚠ Needs [SIM] L5 sim_asset_card, [SIM] L6 unit_image_index and the Unit
 * Images media set added as Resources on this app in Developer Console.
 * Without the first the list is empty and says so; without the others images
 * are missing and say that too.
 */

import { useEffect, useMemo, useState } from "react";
import { Link } from "react-router-dom";

import { AppSwitcher } from "@/components/AppSwitcher";
import { SqlError } from "@/shared/lib/sqlClient";
import { CuratedExplorer, type CuratedMode } from "./CuratedExplorer";

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
    label: "equipment",
    hint:
      "[SIM] L7 — 203 platforms across land, air and sea plus 20 infantry sections, " +
      "filterable by type, class, nation and capability",
  },
  {
    id: "munitions",
    label: "munitions",
    hint: "[SIM] L7 bgws_munition_profile — 150 rounds with penetration curves",
  },
  // Last, and named for what it is. The simulator export is a tank game's
  // asset library: 1,239 platforms sharing 72 statlines, with a Sturmtiger, a
  // Maus and an esports livery in it. It is kept for lineage and for the
  // images, not because anybody should field from it.
  {
    id: "simulator",
    label: "simulator (legacy)",
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
  const [filters, setFilters] = useState<AssetFilters>({});
  const [facetCounts, setFacetCounts] = useState<FacetCount[]>([]);
  const [cards, setCards] = useState<AssetCard[]>([]);
  const [matchCount, setMatchCount] = useState<number | null>(null);
  const [imageCount, setImageCount] = useState<number | null>(null);
  const [images, setImages] = useState<Map<string, UnitImageRow>>(new Map());
  const [selected, setSelected] = useState<AssetCard | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [showFilters, setShowFilters] = useState(true);
  // Which catalogue is on screen. The simulator export and the curated tables
  // share no identifiers, so they are browsed separately rather than joined
  // by name -- see the note at the top of CuratedExplorer.
  const [mode, setMode] = useState<ExplorerMode>("curated");

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

  return (
    <div style={page}>
      <div style={header}>
        <Link to="/bgws" style={{ ...subtle, color: "#e8c547", textDecoration: "none" }}>
          ← BGWS
        </Link>
        <span style={{ fontWeight: 700, letterSpacing: "0.12em" }}>ASSET EXPLORER</span>
        <span style={subtle}>
          {loading
            ? "loading…"
            : matchCount === null
              ? `${shown} shown`
              : capped
                ? `${shown} of ${matchCount} matches (capped)`
                : `${matchCount} ${matchCount === 1 ? "match" : "matches"}`}
        </span>
        <span style={{ flex: 1 }} />
        {MODES.map((entry) => (
          <button
            key={entry.id}
            onClick={() => setMode(entry.id)}
            title={entry.hint}
            style={{
              ...chip,
              borderColor: mode === entry.id ? "#e8c547" : "rgba(255,255,255,0.12)",
              color: mode === entry.id ? "#e8c547" : "#8a91a8",
            }}
          >
            {entry.label}
          </button>
        ))}
        {mode === "simulator" && (
        <button onClick={() => setShowFilters((open) => !open)} style={chip}>
          {showFilters ? "hide filters" : "filters"}
          {activeCount > 0 && ` · ${activeCount}`}
        </button>
        )}
        <AppSwitcher />
      </div>

      <div style={body}>
        {mode !== "simulator" ? (
          <CuratedExplorer mode={mode} />
        ) : (
        <>
        {showFilters && (
          <FilterPane
            filters={filters}
            facets={facets}
            onChange={setFilters}
            activeCount={activeCount}
            imageCount={imageCount}
          />
        )}

        <div style={listPane}>
          <input
            value={filters.search ?? ""}
            onChange={(e) => setFilters({ ...filters, search: e.target.value })}
            placeholder="Search name or id…"
            style={input}
          />

          <div style={{ display: "flex", alignItems: "center", gap: 6, marginBottom: 8 }}>
            <span style={subtle}>sort</span>
            <select
              value={filters.sort ?? "name"}
              onChange={(e) => setFilters({ ...filters, sort: e.target.value as SortKey })}
              style={select}
            >
              {SORT_KEYS.map((key) => (
                <option key={key} value={key}>
                  {SORT_LABEL[key]}
                </option>
              ))}
            </select>
          </div>

          {error && <div style={{ ...subtle, color: "#e07a5f", lineHeight: 1.6 }}>{error}</div>}

          <div style={{ overflowY: "auto", flex: 1 }}>
            {cards.map((card) => (
              <button
                key={card.assetId}
                onClick={() => setSelected(card)}
                style={{
                  ...row,
                  background:
                    selected?.assetId === card.assetId ? "rgba(232,197,71,0.12)" : "transparent",
                  borderColor: selected?.assetId === card.assetId ? "#e8c547" : "transparent",
                }}
              >
                <div style={{ color: "#e9ecfb" }}>
                  {card.displayName}
                  {/* A dot, not a word: the list is dense and this is a hint,
                      not a fact worth a column. */}
                  {images.has(card.assetId) && (
                    <span style={{ color: "#5a8f5a", marginLeft: 5 }} title="has a photograph">
                      ●
                    </span>
                  )}
                </div>
                <div style={subtle}>
                  {[card.domain, card.subclass, card.nation].filter(Boolean).join(" · ")}
                </div>
              </button>
            ))}
            {!loading && !error && cards.length === 0 && (
              <div style={subtle}>Nothing matches.</div>
            )}
          </div>
        </div>

        <div style={detailPane}>
          {selected ? (
            <AssetDetail card={selected} indexed={images.get(selected.assetId) ?? null} />
          ) : (
            <div style={subtle}>Select an asset.</div>
          )}
        </div>
        </>
        )}
      </div>
    </div>
  );
}

function FilterPane({
  filters,
  facets,
  onChange,
  activeCount,
  imageCount,
}: {
  filters: AssetFilters;
  facets: Record<FacetKey, FacetCount[]>;
  onChange: (next: AssetFilters) => void;
  activeCount: number;
  imageCount: number | null;
}) {
  return (
    <div style={filterPane}>
      <div style={{ display: "flex", alignItems: "center", marginBottom: 10 }}>
        <span style={{ ...groupTitle, border: "none", margin: 0, padding: 0, flex: 1 }}>
          Filters
        </span>
        {activeCount > 0 && (
          <button onClick={() => onChange({ sort: filters.sort })} style={chip}>
            clear {activeCount}
          </button>
        )}
      </div>

      <div style={{ marginBottom: 14 }}>
        <div style={groupTitle}>Presence</div>
        {EXISTS_KEYS.map((key) => (
          <TriStateChip
            key={key}
            label={EXISTS_LABEL[key]}
            value={filters.exists?.[key]}
            onClick={() => onChange(cycleExists(filters, key))}
          />
        ))}
        {imageCount !== null ? (
          <div style={{ ...subtle, marginTop: 4, lineHeight: 1.5 }}>
            {imageCount} assets have a photograph.
          </div>
        ) : (
          <div style={{ ...subtle, marginTop: 4, lineHeight: 1.5 }}>
            The image index is unavailable — it may not be built on this branch
            yet, in which case &ldquo;has image&rdquo; will match nothing.
          </div>
        )}
      </div>

      <div style={{ marginBottom: 14 }}>
        <div style={groupTitle}>Flags</div>
        {FLAG_KEYS.map((key) => (
          <TriStateChip
            key={key}
            label={FLAG_LABEL[key]}
            value={filters.flags?.[key]}
            onClick={() => onChange(cycleFlag(filters, key))}
          />
        ))}
      </div>

      {FACET_KEYS.map((facet) => {
        const values = facets[facet];
        if (!values.length) return null;
        const selected = filters.facets?.[facet] ?? [];

        return (
          <div key={facet} style={{ marginBottom: 14 }}>
            <div style={groupTitle}>{FACET_LABEL[facet]}</div>
            <div style={{ display: "flex", flexWrap: "wrap", gap: 4 }}>
              {values.map((entry) => (
                <button
                  key={entry.value}
                  onClick={() => onChange(toggleFacetValue(filters, facet, entry.value))}
                  style={{
                    ...chip,
                    borderColor: selected.includes(entry.value) ? "#e8c547" : "transparent",
                    color: selected.includes(entry.value) ? "#e8c547" : "#8a91a8",
                    // The null bucket is a real answer about the data, not a
                    // missing chip, so it is shown — just muted.
                    fontStyle: entry.value === NULL_FACET ? "italic" : "normal",
                  }}
                >
                  {entry.value} {entry.count}
                </button>
              ))}
            </div>
            {facet === "nation" && (
              <div style={{ ...subtle, marginTop: 4, lineHeight: 1.5 }}>
                The source carries both `united_kingdom` and `britain`, and both
                `united_states` and `usa`. These are folded together here.
              </div>
            )}
          </div>
        );
      })}

      <div style={{ marginBottom: 14 }}>
        <div style={groupTitle}>Ranges</div>
        {RANGE_KEYS.map((key) => {
          const range = filters.ranges?.[key];
          const warning = RANGE_WARNING[key];
          const active = range?.min !== undefined || range?.max !== undefined;

          return (
            <div key={key} style={{ padding: "2px 0" }}>
              <div style={{ display: "flex", alignItems: "center", gap: 4 }}>
                <span style={{ ...subtle, width: 84, flexShrink: 0 }}>
                  {RANGE_LABEL[key]}
                  {warning && <span style={{ color: "#e8c547" }}> ⚠</span>}
                </span>
                <input
                  type="number"
                  value={range?.min ?? ""}
                  placeholder="min"
                  onChange={(e) =>
                    onChange(
                      setRange(filters, key, {
                        ...range,
                        min: e.target.value === "" ? undefined : Number(e.target.value),
                      }),
                    )
                  }
                  style={numberInput}
                />
                <input
                  type="number"
                  value={range?.max ?? ""}
                  placeholder="max"
                  onChange={(e) =>
                    onChange(
                      setRange(filters, key, {
                        ...range,
                        max: e.target.value === "" ? undefined : Number(e.target.value),
                      }),
                    )
                  }
                  style={numberInput}
                />
              </div>
              {/* Shown only once the filter is in use. A panel of permanent
                  warnings is a panel nobody reads; one that appears at the
                  moment the trap is stepped in gets seen. */}
              {warning && active && (
                <div
                  style={{ ...subtle, color: "#e8c547", lineHeight: 1.5, padding: "2px 0 4px" }}
                >
                  {warning}
                </div>
              )}
            </div>
          );
        })}
        <div style={{ ...subtle, marginTop: 4, lineHeight: 1.5 }}>
          A range excludes assets with no value at all. Only 1,132 of 2,258 have
          a mass; 234 have a year.
        </div>
      </div>
    </div>
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
  const colour = value === undefined ? "#8a91a8" : value ? "#5a8f5a" : "#e07a5f";
  const mark = value === undefined ? "" : value ? " ✓" : " ✕";

  return (
    <button
      onClick={onClick}
      style={{
        ...chip,
        display: "block",
        width: "100%",
        textAlign: "left",
        marginBottom: 2,
        color: colour,
        borderColor: value === undefined ? "transparent" : colour,
      }}
    >
      {label}
      {mark}
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
    <div style={{ overflowY: "auto", height: "100%" }}>
      <div style={{ fontSize: 16, fontWeight: 700, color: "#e9ecfb" }}>{card.displayName}</div>
      <div style={{ ...subtle, marginBottom: 12 }}>{card.assetId}</div>

      <div style={imageFrame}>
        {image === null && <span style={subtle}>loading image…</span>}
        {image?.url && (
          <img
            src={image.url}
            alt={card.displayName}
            style={{ width: "100%", height: "100%", objectFit: "contain" }}
          />
        )}
        {image !== null && !image.url && !image.error && (
          <span style={subtle}>no image in the set for this asset</span>
        )}
        {image?.error && (
          <span style={{ ...subtle, color: "#e07a5f", padding: 12, textAlign: "center" }}>
            image lookup failed — {image.error}
            <br />
            Check the Unit Images media set is a Resource on this app.
          </span>
        )}
      </div>
      {image?.path && (
        <div style={{ ...subtle, marginBottom: 14 }}>
          image: {image.path}
          {/* Which convention matched is exactly the question asked when a
              new batch of images does not appear. */}
          {indexed?.matchMethod && ` (matched by ${indexed.matchMethod})`}
        </div>
      )}

      {groups.map((group) => (
        <div key={group.title} style={{ marginBottom: 14 }}>
          <div style={groupTitle}>{group.title}</div>
          {group.rows.map(([label, value, note]) => (
            <div key={label} style={statRow}>
              <span style={{ ...subtle, width: 130, flexShrink: 0 }}>{label}</span>
              <span style={{ color: value === "—" ? "#6a7292" : "#e9ecfb" }}>{value}</span>
              {note && <span style={{ ...subtle, marginLeft: 6 }}>({note})</span>}
            </div>
          ))}
        </div>
      ))}
    </div>
  );
}

type StatRow = [label: string, value: string, note?: string];

function statGroups(
  card: AssetCard,
  profile: PlatformProfile | null,
): { title: string; rows: StatRow[] }[] {
  const show = (value: number | string | null | undefined, suffix = ""): string =>
    value == null || value === "" ? "—" : `${value}${suffix}`;

  // Derived figures come FIRST when they exist, because the source's own
  // mobility numbers are not trustworthy and a reader scans from the top.
  const derived: { title: string; rows: StatRow[] }[] = profile
    ? [
        {
          title: "Derived · mobility (L6)",
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

const page: React.CSSProperties = {
  height: "100vh",
  background: "#060d18",
  color: "#e9ecfb",
  font: "11px/1.5 var(--font-mono, monospace)",
  display: "flex",
  flexDirection: "column",
};

const header: React.CSSProperties = {
  height: 40,
  display: "flex",
  alignItems: "center",
  gap: 12,
  padding: "0 12px",
  borderBottom: "1px solid #191e37",
  flexShrink: 0,
};

const body: React.CSSProperties = { flex: 1, display: "flex", minHeight: 0 };

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

const subtle: React.CSSProperties = { fontSize: 10, color: "#6a7292" };

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

const statRow: React.CSSProperties = { display: "flex", padding: "2px 0" };

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

const input: React.CSSProperties = {
  width: "100%",
  boxSizing: "border-box",
  padding: "6px 8px",
  marginBottom: 8,
  background: "rgba(255,255,255,0.04)",
  border: "1px solid rgba(255,255,255,0.12)",
  borderRadius: 3,
  color: "#e9ecfb",
  font: "inherit",
};

const numberInput: React.CSSProperties = {
  width: 54,
  boxSizing: "border-box",
  padding: "3px 5px",
  background: "rgba(255,255,255,0.04)",
  border: "1px solid rgba(255,255,255,0.12)",
  borderRadius: 3,
  color: "#e9ecfb",
  font: "inherit",
  fontSize: 10,
};

const select: React.CSSProperties = {
  flex: 1,
  padding: "3px 5px",
  background: "rgba(255,255,255,0.04)",
  border: "1px solid rgba(255,255,255,0.12)",
  borderRadius: 3,
  color: "#e9ecfb",
  font: "inherit",
  fontSize: 10,
};

const chip: React.CSSProperties = {
  padding: "3px 7px",
  background: "rgba(255,255,255,0.04)",
  border: "1px solid",
  borderColor: "transparent",
  borderRadius: 3,
  color: "#8a91a8",
  cursor: "pointer",
  font: "inherit",
  fontSize: 10,
};

const row: React.CSSProperties = {
  display: "block",
  width: "100%",
  textAlign: "left",
  padding: "5px 7px",
  marginBottom: 2,
  border: "1px solid transparent",
  borderRadius: 3,
  cursor: "pointer",
  font: "inherit",
  background: "transparent",
};
