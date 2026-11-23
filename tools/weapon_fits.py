#!/usr/bin/env python3
"""Weapon fits for the imported platforms, from the asset CSVs.

Reads:
  data/catalogue/imported.json                       the importer's platforms and sourced munition rows
  data/catalogue/csv/[SIM] L7 bgws_capability_profile.csv   each platform's weapons, by name
  data/catalogue/*.json                              the engine munitions used as templates, and the
                                                     parents' stores (to drop rounds a fit no longer fires)
Writes:
  data/catalogue/weapon-fits.json                    engine munitions built from the sourced rows, and a
                                                     `fits` overlay per platform (mounts and stores)

How a fit is made:
  1. Each weapon a platform's capability rows name is matched to the CSV's weapon id
     (WEAPONS below: the names are display text, so the match is a table).
  2. Each sourced munition row of that weapon (those in imported.json; rows the
     importer rejected are not used) becomes an engine munition: an engine
     template for its weapon class and round type (guidance, flight, pk, CEP,
     targets, cost, family) with the row's own name, penetration against range,
     tandem and top-attack flags, warhead and lethal radius. Speed and maximum
     range come from the platform's capability row when that row names this one
     weapon only; otherwise from the template.
  3. A weapon with no sourced round fires the engine's own round for it
     (FALLBACK), so it is still armed.
  4. Mount figures (salvo, ready rounds, reload) and rounds carried per vehicle
     come from CLASSES: [default], one set per weapon class.

Unmatched weapon names are an error (`--check` too), so a new weapon in the
CSV is a decision, not a silent gap.

  python3 tools/weapon_fits.py            write the file
  python3 tools/weapon_fits.py --check    fail if the committed file is out of date
"""

import csv
import json
import re
import sys
from collections import OrderedDict
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CAT = ROOT / "data" / "catalogue"
OUT = CAT / "weapon-fits.json"
CAPS = CAT / "csv" / "[SIM] L7 bgws_capability_profile.csv"

# Weapon class -> mount kind and [default] mount figures. `carried` is rounds per
# vehicle across all its munitions; the first munition gets half, the rest share
# the other half. `ready` is the mount's magazine (each munition gets at least that).
CLASSES = {
    "tankGun":    dict(kind="directFire", salvo=1, ready=10, reloadS=8, carried=40, priority=0),
    "lowPressure": dict(kind="directFire", salvo=1, ready=10, reloadS=10, carried=40, priority=1),
    "autocannon": dict(kind="cannon", salvo=3, ready=100, reloadS=2, carried=400, priority=2),
    "atgm":       dict(kind="atgm", salvo=1, ready=1, reloadS=30, carried=4, priority=3),
    "agl":        dict(kind="cannon", salvo=3, ready=50, reloadS=3, carried=300, priority=4),
    "hmg":        dict(kind="smallArms", salvo=1, ready=100, reloadS=10, carried=600, priority=5),
    "mg":         dict(kind="smallArms", salvo=1, ready=250, reloadS=10, carried=2000, priority=6),
    "howitzer":   dict(kind="howitzer", salvo=4, ready=10, reloadS=12, carried=40, priority=7),
    "mortar":     dict(kind="mortar", salvo=4, ready=10, reloadS=10, carried=40, priority=8),
    "samShort":   dict(kind="samShort", salvo=1, ready=4, reloadS=6, carried=8, priority=9),
    "gunAd":      dict(kind="gunAd", salvo=1, ready=40, reloadS=3, carried=600, priority=10),
}

# Display name in the capability CSV -> (weapon ids in the munition CSV, class, guidance for missiles).
WEAPONS = {
    "2A28 Grom 73mm smoothbore": (["gun_73_2a28"], "lowPressure", None),
    "2A33 152mm (2S3)": (["how_152_2a33"], "howitzer", None),
    "2A36 / 2A37 Giatsint 152mm gun": (["gun_152_2a36"], "howitzer", None),
    "2A42 30mm": (["ac_30_2a42"], "autocannon", None),
    "2A46M 125mm smoothbore": (["gun_125_2a46"], "tankGun", None),
    "2A60 / 2A80 120mm gun-mortar": (["mor_120_2a60"], "mortar", None),
    "2A64 / 2A65 152mm (2S19 / MSTA-B)": (["how_152_2a64"], "howitzer", None),
    "2A70 100mm rifled gun-launcher": (["gun_100_2a70"], "lowPressure", None),
    "2A72 30mm": (["ac_30_2a72"], "autocannon", None),
    "2A75 125mm smoothbore (Sprut-SD)": (["gun_125_2a46"], "tankGun", None),
    "2A82-1M 125mm smoothbore": (["gun_125_2a82", "gun_125_2a46"], "tankGun", None),
    "2A88 152mm (2S35)": (["how_152_2a88"], "howitzer", None),
    "9K113 Konkurs (AT-5)": (["at_konkurs"], "atgm", "wire"),
    "9K135 Kornet / Kornet-EM (AT-14)": (["at_kornet"], "atgm", "laser"),
    "9M117 Bastion (gun-launched, 100mm)": (["at_bastion"], "atgm", "laser"),
    "9M120 Ataka (AT-9)": (["at_ataka"], "atgm", "operator"),
    "9M123 Khrizantema (AT-15)": (["at_khrizantema"], "atgm", "radar"),
    "9M14 Malyutka (AT-3 Sagger)": (["at_sagger"], "atgm", "wire"),
    "AGS-17/AGS-30 30mm AGL": (["agl_30_ags17"], "agl", None),
    "AS-90 155mm L31 (39 cal)": (["how_155_l31"], "howitzer", None),
    "BGM-71 TOW-2A/2B": (["at_tow2"], "atgm", "wire"),
    "CT40 40mm cased-telescoped cannon": (["ac_40_ct40"], "autocannon", None),
    "Cockerill Mk3 90mm": (["gun_90_ckm"], "lowPressure", None),
    "D-10T 100mm rifled": (["gun_100_d10"], "tankGun", None),
    "D-30 / 2A18 122mm": (["how_122_2a18"], "howitzer", None),
    "KPVT 14.5mm": (["hmg_145_kpvt"], "hmg", None),
    "Kord / NSVT 12.7mm": (["hmg_127_kord"], "hmg", None),
    "L/44 120mm smoothbore (M256/Rh120)": (["gun_120_l44"], "tankGun", None),
    "L21A1 RARDEN 30mm": (["ac_30_rarden"], "autocannon", None),
    "L30A1 120mm rifled gun": (["gun_120_l30"], "tankGun", None),
    "L55A1 120mm smoothbore": (["gun_120_l55a1"], "tankGun", None),
    "L7A2 GPMG / FN MAG 7.62mm": (["mg_762_gpmg"], "mg", None),
    "L94A1 EX-34 7.62mm chain gun": (["mg_762_l94"], "mg", None),
    "M240 7.62mm": (["mg_762_m240"], "mg", None),
    "M242 Bushmaster 25mm": (["ac_25_m242"], "autocannon", None),
    "M284 155mm (M109A6)": (["how_155_m284"], "howitzer", None),
    "M2HB / M3M 12.7mm": (["hmg_127_m2"], "hmg", None),
    "M35 105mm (M68 derivative)": (["gun_105_m35"], "tankGun", None),
    "M68A1 105mm rifled": (["gun_105_m68"], "tankGun", None),
    "Mauser MK30-2/ABM 30mm": (["ac_30_mk30"], "autocannon", None),
    "Minimi / M249 / L110 5.56mm": (["mg_556_lmg"], "mg", None),
    "Mk 19 / HK GMG 40mm AGL": (["agl_40_mk19"], "agl", None),
    "Oerlikon KDA 35mm twin (Gepard)": (["ac_35_kda"], "gunAd", None),
    "PKM/PKP 7.62mm": (["mg_762_pkm"], "mg", None),
    "PKT/PKTM 7.62mm": (["mg_762_pkt"], "mg", None),
    "Rheinmetall Rh 202 20mm": (["ac_20_rh202"], "autocannon", None),
    "Rheinmetall Wotan 35 (Lance 2.0)": (["ac_35_wotan"], "autocannon", None),
    "Spike-LR / LR2 (MELLS)": (["at_spike_lr"], "atgm", "fibre"),
    "Starstreak HVM": (["sam_starstreak"], "samShort", "laser"),
    "U-5TS / D-68 115mm smoothbore": (["gun_115_u5ts"], "tankGun", None),
}

# The engine's own round for a weapon class, when the weapon has no sourced round.
FALLBACK = {
    "tankGun": ["apfsds125"],
    "lowPressure": ["heat120"],
    "autocannon": ["apfsds30", "he30"],
    "atgm": ["atgmTandem"],
    "agl": ["he30"],
    "hmg": ["ball762"],
    "mg": ["ball762"],
    "howitzer": ["he155", "smoke155"],
    "mortar": ["he120Mortar", "smoke120Mortar"],
    "samShort": ["samShort"],
    "gunAd": ["adShell30"],
}
# Better fallbacks for particular weapons (the engine has these rounds).
FALLBACK_BY_WEAPON = {
    "at_kornet": ["kornet"],
    "at_tow2": ["atgmTow"],
    "gun_120_l44": ["m829", "heat120"],
    "gun_120_l30": ["charm3"],
    "ac_30_rarden": ["apds30Rarden"],
    "gun_105_m68": ["apfsds100"],
    "gun_100_2a70": ["he30"],
    "mg_556_lmg": ["ball556"],
}
# Plain HE every gun or mortar keeps, when its sourced rounds are all guided.
PLAIN_HE = {"howitzer": "he155", "mortar": "he120Mortar"}
# Smoke rounds every gun or mortar battery keeps (the engine fires smoke from these).
SMOKE = {"howitzer": "smoke155", "mortar": "smoke120Mortar"}

# (class, CSV round kind) -> (engine kind, template engine munition). Kinds not listed are left out.
ROUNDS = {
    ("tankGun", "apfsds"): ("ke", "apfsds125"),
    ("tankGun", "apds"): ("ke", "apfsds125"),
    ("tankGun", "ap"): ("ke", "apfsds125"),
    ("tankGun", "heat"): ("heat", "heat120"),
    ("tankGun", "heat_tandem"): ("heat", "heat120"),
    ("tankGun", "hesh"): ("heat", "heat120"),
    ("lowPressure", "heat"): ("heat", "heat120"),
    ("autocannon", "apfsds"): ("ke", "apfsds30"),
    ("autocannon", "apds"): ("ke", "apfsds30"),
    ("autocannon", "api"): ("ke", "apfsds30"),
    ("autocannon", "he"): ("he", "he30"),
    ("autocannon", "airburst"): ("he", "he30"),
    ("agl", "he"): ("he", "he30"),
    ("agl", "hefrag"): ("he", "he30"),
    ("agl", "heat"): ("heat", "he30"),
    ("hmg", "api"): ("smallArms", "ball762"),
    ("mg", "api"): ("smallArms", "ball762"),
    ("atgm", "heat"): ("atgm", "kornet"),
    ("atgm", "heat_tandem"): ("atgm", "kornet"),
    ("howitzer", "he"): ("shell", "he155"),
    ("mortar", "he"): ("mortar", "he120Mortar"),
    ("samShort", "sam"): ("sam", "samShort"),
    ("gunAd", "he"): ("shell", "adShell30"),
}
# Guided artillery rounds, by munition id.
GUIDED_SHELLS = {"m_excalibur": "gnss", "m_krasnopol": "laser"}

CSV_KIND = {}  # munition id -> the CSV's round kind (apfsds, heat_tandem, ...), read below


def load_json(p):
    return json.loads(p.read_text(encoding="utf-8"), object_pairs_hook=OrderedDict)


def split_weapons(cell):
    """`[A, B (x, y), C]` -> ['A', 'B (x, y)', 'C']: split on commas outside brackets."""
    s = cell.strip().strip("[]")
    out, depth, cur = [], 0, ""
    for ch in s:
        if ch == "(":
            depth += 1
        elif ch == ")":
            depth -= 1
        if ch == "," and depth == 0:
            out.append(cur.strip())
            cur = ""
        else:
            cur += ch
    if cur.strip():
        out.append(cur.strip())
    return out


def num(x):
    try:
        v = float(x)
    except (TypeError, ValueError):
        return None
    return v


def clean(v):
    """Integers stay integers in the JSON."""
    return int(v) if isinstance(v, float) and v.is_integer() else v


def build():
    imported = load_json(CAT / "imported.json")
    engine = {}
    parents = {}
    for f in sorted(CAT.glob("*.json")):
        if f.name in ("imported.json", OUT.name):
            continue
        d = load_json(f)
        for m in d.get("munitions", []):
            engine[m["id"]] = m
        for p in d.get("platforms", []):
            parents[p["id"]] = p
    mun_csv = {}
    with open(CAT / "csv" / "[SIM] L7 bgws_munition_profile.csv", encoding="utf-8-sig") as fh:
        for r in csv.DictReader(fh):
            mun_csv[r["munition_id"]] = r
    sourced = {}  # weapon id -> sourced rows, in CSV order
    for m in imported["munitions"]:
        sourced.setdefault(m["family"], []).append(m)
    caps = {}
    with open(CAPS, encoding="utf-8-sig") as fh:
        for r in csv.DictReader(fh):
            caps.setdefault(r["asset_id"], []).append(r)

    problems = []
    munitions = OrderedDict()
    fits = OrderedDict()
    for plat in imported["platforms"]:
        pid = plat["id"]
        bare = pid.split("/", 1)[1]
        rows = caps.get(bare, [])
        # Each weapon once, with the row that names it alone (for its range and speed).
        weapons = OrderedDict()
        for r in rows:
            names = split_weapons(r["weapons"])
            for n in names:
                if n not in WEAPONS:
                    problems.append(f"{pid}: weapon `{n}` is not in WEAPONS")
                    continue
                own = len(set(names)) == 1
                prev = weapons.get(n)
                if prev is None or (own and not prev[1]) or (own and prev[1] and num(r["bgws_max_range_m"]) > num(prev[0]["bgws_max_range_m"])):
                    weapons[n] = (r, own)
        if not weapons:
            continue
        mounts, ammo = [], OrderedDict()
        ordered = sorted(weapons.items(), key=lambda kv: (CLASSES[WEAPONS[kv[0]][1]]["priority"], kv[0]))
        used_ids = set()
        for name, (row, own) in ordered:
            wids, cls, guidance = WEAPONS[name]
            c = CLASSES[cls]
            built_rounds = []
            for wid in wids:
                for m in sourced.get(wid, []):
                    built = make_munition(m, cls, guidance, row if own else None, engine, mun_csv)
                    if built is not None and all(b["id"] != built["id"] for b in built_rounds):
                        built_rounds.append(built)
            rounds = []
            for b in pick_rounds(built_rounds, row if own else None):
                munitions[b["id"]] = b
                rounds.append(b["id"])
            if not rounds:
                rounds = list(FALLBACK_BY_WEAPON.get(wids[0], FALLBACK[cls]))
            if cls in PLAIN_HE and not any(engine.get(r, munitions.get(r, {})).get("guidance") == "none" for r in rounds):
                rounds.insert(0, PLAIN_HE[cls])
            if cls in SMOKE and SMOKE[cls] not in rounds:
                rounds.append(SMOKE[cls])
            mid = wids[0]
            if mid in used_ids:
                continue
            used_ids.add(mid)
            mount = OrderedDict(id=mid, kind=c["kind"], munitions=rounds, salvo=c["salvo"], magazine=c["ready"], reloadS=c["reloadS"])
            rof = num(row.get("rof_sustained"))
            if own and rof and rof > 0:
                mount["rofRpm"] = clean(rof)
            mounts.append(mount)
            share = [0.5] + [0.5 / (len(rounds) - 1)] * (len(rounds) - 1) if len(rounds) > 1 else [1.0]
            for mu, sh in zip(rounds, share):
                ammo[mu] = ammo.get(mu, 0) + max(int(round(c["carried"] * sh)), c["ready"])
        # Rounds the parent carries that this fit does not fire are dropped.
        for k in sorted(parent_ammo(plat["parent"], parents)):
            if k not in ammo:
                ammo[k] = None
        fits[pid] = OrderedDict(mounts=mounts, stores=OrderedDict(ammo=ammo))
    out = OrderedDict()
    out["$comment"] = (
        "GENERATED by tools/weapon_fits.py from imported.json and the capability CSV; do not edit by hand. "
        "`munitions`: engine munitions built from the importer's sourced munition rows on an engine template "
        "for their weapon class (the `source` of each says which). `fits`: each imported platform's mounts and "
        "stores, from the weapons its capability rows name. Mount figures and rounds carried are [default] per "
        "weapon class (CLASSES in the tool)."
    )
    out["munitions"] = list(munitions.values())
    out["fits"] = fits
    return out, problems


KIND_ORDER = ["ke", "atgm", "heat", "smallArms", "shell", "mortar", "he", "sam"]


def pen_at(m, r):
    pts = m.get("penetration") or []
    for a, b in zip(pts, pts[1:]):
        if a["rangeM"] <= r <= b["rangeM"]:
            f = (r - a["rangeM"]) / (b["rangeM"] - a["rangeM"]) if b["rangeM"] > a["rangeM"] else 1.0
            return a["mm"] + f * (b["mm"] - a["mm"])
    return pts[-1]["mm"] if pts else 0.0


def pick_rounds(built, row):
    """One round of each engine kind, armour-piercing first. A gun's rounds are
    every round of its calibre in the CSV, not what this vehicle carries; the
    capability row's best penetration at 1 km is this vehicle's, so the KE round
    nearest it (not above it, where one is) is the one it fires. Other kinds:
    the first sourced row."""
    by_kind = OrderedDict()
    for b in built:
        by_kind.setdefault(b["kind"], []).append(b)
    out = []
    for kind in sorted(by_kind, key=lambda k: KIND_ORDER.index(k) if k in KIND_ORDER else len(KIND_ORDER)):
        cands = by_kind[kind]
        if kind == "ke" and len(cands) > 1:
            best = num(row.get("best_pen_mm_1000m")) if row else None
            if best:
                under = [c for c in cands if pen_at(c, 1000) <= best + 1]
                pool = under or cands
                out.append(min(pool, key=lambda c: (abs(pen_at(c, 1000) - best), c["id"])))
            else:
                out.append(max(cands, key=lambda c: (pen_at(c, 1000), c["id"])))
        else:
            out.append(cands[0])
    return out


def parent_ammo(pid, parents):
    keys = set()
    while pid in parents:
        p = parents[pid]
        keys |= set((p.get("stores") or {}).get("ammo") or {})
        pid = p.get("parent")
    return keys


def make_munition(m, cls, guidance, row, engine, mun_csv):
    csv_kind = mun_csv.get(m["id"].split("/", 1)[1], {}).get("kind", "")
    key = (cls, csv_kind)
    mid = m["id"].split("/", 1)[1]
    if cls == "howitzer" and mid in GUIDED_SHELLS:
        kind, tpl = "shell", "pgm155"
    elif key in ROUNDS:
        kind, tpl = ROUNDS[key]
    else:
        return None
    t = engine[tpl]
    out = OrderedDict()
    out["id"] = m["id"]
    out["name"] = m["name"]
    out["kind"] = kind
    out["family"] = t["family"]
    g = GUIDED_SHELLS.get(mid) or guidance or t["guidance"]
    out["guidance"] = g
    out["flight"] = t["flight"]
    speed, max_range = t["speedMs"], t["maxRangeM"]
    notes = []
    if row is not None:
        mv = num(row.get("muzzle_velocity_ms"))
        rng = num(row.get("bgws_max_range_m"))
        if mv and mv > 0 and cls not in ("howitzer", "mortar"):
            speed = mv
            notes.append("speed")
        if rng and rng > 0:
            max_range = rng
            notes.append("maximum range")
    out["speedMs"] = clean(speed)
    out["minRangeM"] = t["minRangeM"]
    out["maxRangeM"] = clean(max_range)
    out["pk"] = t["pk"]
    out["cepM"] = t["cepM"]
    wk = m.get("warheadKg")
    out["warheadKg"] = clean(wk) if wk else t.get("warheadKg", 0)
    lr = m.get("lethalRadiusM")
    out["lethalRadiusM"] = clean(lr) if lr else t.get("lethalRadiusM", 0)
    if m.get("penetration"):
        out["penetration"] = m["penetration"]
    out["topAttack"] = bool(m.get("topAttack"))
    out["tandem"] = bool(m.get("tandem")) or m.get("eraEffect") == "precursor_defeats_era"
    if m.get("penBasis") == "behind_era" and m.get("penetration"):
        out["penBasis"] = "behind_era"
    if m.get("eraEffect") == "defeats_era" and kind == "ke":
        out["defeatsEra"] = True
    out["targets"] = t["targets"]
    out["costUsd"] = t["costUsd"]
    out["structuralHits"] = t.get("structuralHits", 0)
    if "dis" in t:
        out["dis"] = t["dis"]
    taken = ", ".join(notes) if notes else "none"
    basis = m.get("penBasis")
    out["source"] = (
        f"L7 munition row {mid} (weapon {m['family']}, grade {m['grade']}): {m['source']}. "
        f"Engine fields from `{tpl}` [default]; from the capability row: {taken}."
        + (" Penetration measured behind ERA: the engine does not take ERA off it again." if basis == "behind_era" else "")
        + (" Penetration basis not stated: read as RHA." if basis == "unstated" else "")
        + (" Made against heavy ERA: heavy ERA does not degrade it." if m.get("eraEffect") == "defeats_era" and kind == "ke" else "")
    )
    return out


def main():
    data, problems = build()
    if problems:
        print("\n".join(problems), file=sys.stderr)
        sys.exit(1)
    text = json.dumps(data, indent=1, ensure_ascii=False) + "\n"
    if "--check" in sys.argv:
        if not OUT.exists() or OUT.read_text(encoding="utf-8") != text:
            print(f"{OUT.relative_to(ROOT)} is out of date: run python3 tools/weapon_fits.py", file=sys.stderr)
            sys.exit(1)
        print("weapon-fits.json is current")
        return
    OUT.write_text(text, encoding="utf-8")
    n_m = len(data["munitions"])
    n_f = len(data["fits"])
    print(f"wrote {OUT.relative_to(ROOT)}: {n_m} munitions, {n_f} fits")


if __name__ == "__main__":
    main()
