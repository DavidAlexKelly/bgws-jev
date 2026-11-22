#!/usr/bin/env python3
"""Penetration against range for kinetic-energy (APFSDS) rounds, per round.

Replaces the catalogue's single fall-off curve (every round keeping 97%,
92% and 86% of its muzzle penetration at 1, 2 and 3 km) with one worked
out from each round's own velocity and drag:

1. Velocity at range: v(x) = v0 * exp(-x / lam). The decay length lam
   comes from two measured points and scales with the square root of
   calibre between them:
     - 25 mm sub-calibre: lam = 11.2 km, fitted to the Oerlikon KBA's
       flight times to 1, 1.5 and 2 km (Rheinmetall D867e0125;
       docs/sources/notes/rheinmetall-datasheets-medium-calibre.md);
     - 120 mm APFSDS: lam = 24.5 km, from the published velocity losses of
       M829A1 (62 m/s at 1 km, 123 at 2 km) and DM33 (75, 120)
       (reference tables, en.wikipedia.org/wiki/120x570mm_NATO; grade C).
     lam(cal) = 11.2 km * (cal / 25)^0.5 reproduces both.
2. Penetration at that velocity: the SwRI database's fit for long rods
   into semi-infinite armour steel (Anderson, Morris and Littlefield 1992,
   eq. 7, p. 67; docs/sources/notes/anderson-1992-penetration-mechanics-database.md):
     P/L = 1.121 (rho_p/rho_t)^0.3704 tanh[0.01750 (L/D)^-0.3020 (rho_p V^2 / S_t)^1.137]
   with RHA at BHN 270 (S_t = 0.90 GPa by the report's eq. 1).
3. Anchoring: the curve is scaled so that it passes through the round's
   published penetration at its published range. Only the shape comes
   from the model; the level stays with the source.

Usage, from the repository root:

    python3 tools/penetration_model.py --check     # validate the model against its sources
    python3 tools/penetration_model.py             # show old and new tables
    python3 tools/penetration_model.py --apply     # write them into the catalogue

Inputs per round are in data/catalogue/ke_rounds.csv.
"""
import csv
import glob
import io
import json
import math
import os
import sys

ROOT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..")
INPUTS = os.path.join(ROOT, "data", "catalogue", "ke_rounds.csv")
SIM = os.path.join(ROOT, "data", "catalogue", "munitions.json")
CSV_MUNITIONS = glob.glob(os.path.join(ROOT, "data", "catalogue", "csv", "*bgws_munition_profile.csv"))[0]

LAM_25_M = 11200.0
LAM_EXP = 0.5
RHO_T = 7850.0
S_T = (270 - 11.24) / 1.979 * 6.894757e6  # BHN 270 -> UTS (Pa), report eq. 1
DENSITY = {"W": 17600.0, "DU": 18600.0, "WC-steel": 14000.0}  # WC-steel: steel body with a carbide core (older Soviet rounds), assumed
NOTE = "range fall-off: tools/penetration_model.py (v = v0 exp(-x/lam), lam = 11.2 km (cal/25)^0.5; P/L by Anderson et al. 1992 eq. 7), anchored at the published point"


def decay_length_m(calibre_mm):
    return LAM_25_M * (calibre_mm / 25.0) ** LAM_EXP


def velocity(v0, calibre_mm, x_m):
    return v0 * math.exp(-x_m / decay_length_m(calibre_mm))


def p_over_l(v_m_s, l_over_d, rho_p):
    g = rho_p * v_m_s**2 / S_T
    return 1.121 * (rho_p / RHO_T) ** 0.3704 * math.tanh(0.01750 * l_over_d**-0.3020 * g**1.137)


def table(r, ranges):
    """Penetration (mm) at each range for one round, through its anchor."""
    rho = DENSITY[r["rod_material"]]
    ld, cal, v0 = float(r["rod_l_over_d"]), float(r["calibre_mm"]), float(r["v0_m_s"])
    shape = lambda x: p_over_l(velocity(v0, cal, x), ld, rho)
    k = float(r["anchor_mm"]) / shape(float(r["anchor_range_m"]))
    return [round(k * shape(x)) for x in ranges]


def load_inputs():
    with open(INPUTS, newline="") as f:
        return [r for r in csv.DictReader(f) if not r["id"].startswith("#")]


def check():
    """The model against the figures it was built from."""
    print("Velocity decay")
    for x, t in [(1000, 0.75), (1500, 1.15), (2000, 1.57)]:
        lam, v0 = decay_length_m(25), 1395.0
        t_model = lam / v0 * (math.exp(x / lam) - 1)
        print(f"  25 mm KBA, {x} m: flight time {t_model:.2f} s (sheet {t}s)")
    for name, v0, pts in [("M829A1", 1575, [(1000, 62), (2000, 123)]), ("DM33", 1650, [(1000, 75), (2000, 120)])]:
        for x, dv in pts:
            print(f"  120 mm {name}, {x} m: loses {v0 - velocity(v0, 120, x):.0f} m/s (table {dv})")
    print("Penetration fit against the SwRI tungsten rows (measured / model P/L)")
    rows = list(csv.DictReader(open(os.path.join(ROOT, "docs", "sources", "data", "anderson-1992-penetration-mechanics-database--tungsten-rods-semi-infinite-rha.csv"))))
    bands = {}
    for row in rows:
        try:
            ld, v, pl, rho = float(row["l_over_d"]), float(row["impact_velocity_km_s"]) * 1000, float(row["p_over_l"]), float(row["penetrator_density_g_cc"]) * 1000
        except ValueError:
            continue
        b = "10-12" if ld < 13 else "13-23" if ld < 24 else "24-33"
        bands.setdefault(b, []).append(pl / p_over_l(v, ld, rho))
    for b, v in sorted(bands.items()):
        m = sum(v) / len(v)
        sd = (sum((x - m) ** 2 for x in v) / len(v)) ** 0.5
        print(f"  L/D {b}: {len(v)} shots, mean {m:.2f}, SD {sd:.2f}")


def main():
    if "--check" in sys.argv:
        check()
        return
    apply = "--apply" in sys.argv
    inputs = load_inputs()
    sim_raw = open(SIM, encoding="utf-8").read()
    sim = json.loads(sim_raw)
    by_id = {m["id"]: m for m in sim["munitions"]}
    csv_text = open(CSV_MUNITIONS, encoding="utf-8-sig", newline="").read()
    rows = list(csv.reader(io.StringIO(csv_text)))
    head, body = rows[0], rows[1:]
    col = {c: i for i, c in enumerate(head)}
    csv_by_id = {r[col["munition_id"]]: r for r in body}
    for r in inputs:
        if r["catalogue"] == "sim":
            m = by_id[r["id"]]
            top = max(3000, int(m.get("maxRangeM", 3000)))
            ranges = [x for x in (0, 1000, 2000, 3000, 4000) if x <= top]
            new = table(r, ranges)
            old = [(p["rangeM"], p["mm"]) for p in m["penetration"]]
            print(f"sim  {r['id']:<14} old {old}")
            print(f"     {'':<14} new {list(zip(ranges, new))}")
            if apply:
                m["penetration"] = [{"rangeM": x, "mm": p} for x, p in zip(ranges, new)]
                m["speedMs"] = int(float(r["v0_m_s"]))
                src = m.get("source", "")
                if NOTE not in src:
                    m["source"] = (src + "; " if src else "") + f"{r['anchor_source']}; {NOTE}"
                if r.get("name"):
                    m["name"] = r["name"]
        else:
            row = csv_by_id[r["id"]]
            ranges = [0, 1000, 2000, 3000]
            new = table(r, ranges)
            cols = ["pen_mm_0m", "pen_mm_1000m", "pen_mm_2000m", "pen_mm_3000m"]
            old = [row[col[c]] for c in cols]
            print(f"csv  {r['id']:<14} old {old} new {new}")
            if apply:
                for c, p in zip(cols, new):
                    row[col[c]] = f"{float(p):.1f}"
                if r.get("name"):
                    row[col["name"]] = r["name"]
                if r.get("source_confidence"):
                    row[col["source_confidence"]] = r["source_confidence"]
                src = row[col["source_ref"]]
                if "penetration_model.py" not in src:
                    row[col["source_ref"]] = f"{r['anchor_source']}; {NOTE}" if r.get("replace_source") == "yes" else f"{src}; {NOTE}"
    if apply:
        with open(SIM, "w", encoding="utf-8") as f:
            f.write(json.dumps(sim, indent=1, ensure_ascii=False) + "\n")
        out = io.StringIO()
        csv.writer(out, lineterminator="\r\n" if "\r\n" in csv_text else "\n").writerows([head] + body)
        with open(CSV_MUNITIONS, "w", encoding="utf-8-sig", newline="") as f:
            f.write(out.getvalue())
        print("written")


if __name__ == "__main__":
    main()
