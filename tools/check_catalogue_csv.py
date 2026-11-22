#!/usr/bin/env python3
"""Checks on the asset catalogue CSVs (data/catalogue/csv): completeness,
provenance, internal consistency, cross-file links and the signs of blanket
calculations. Run it after changing the CSVs, from the repository root:

    python3 tools/check_catalogue_csv.py

It reads only; it prints what it finds and exits 1 if a hard check fails
(a broken link between files, a duplicate id, a value that contradicts its
own stated formula, penetration rising with range).
"""
import glob
import json
import os
import sys

import pandas as pd

DIR = os.path.join(os.path.dirname(__file__), "..", "data", "catalogue", "csv")


def load(stem):
    return pd.read_csv(glob.glob(os.path.join(DIR, f"*{stem}.csv"))[0], encoding="utf-8-sig")


P = load("bgws_platform_profile")
C = load("bgws_capability_profile")
M = load("bgws_munition_profile")
S = load("bgws_support_profile")
SEC = load("bgws_section_profile")
I = load("mildata_image_index")

hard = []


def check(ok, what):
    print(("  ok    " if ok else "  FAIL  ") + what)
    if not ok:
        hard.append(what)


def note(what):
    print("  note  " + what)


print("Links and ids")
check(P.asset_id.is_unique, f"platform asset_id unique ({P.asset_id.duplicated().sum()} duplicates)")
check(M.munition_id.is_unique, "munition_id unique")
check(set(C.asset_id) <= set(P.asset_id) | set(P.base_asset_id.dropna()), "every capability row names a platform")
ids = set(M.munition_id)
missing = set()
for p in S.payload.dropna():
    for item in json.loads(p):
        if item.get("munition_id") not in ids:
            missing.add(item.get("munition_id"))
check(not missing, f"every support payload names a munition ({sorted(missing)[:5]})")
dups = P[P.display_name.duplicated(keep=False)][["asset_id", "display_name"]].values.tolist()
if dups:
    note(f"display names used twice: {dups}")
idx = set(I.image_path.str.replace(".png", "", regex=False))
lost = P.image_filename.dropna()[~P.image_filename.dropna().isin(idx)].tolist()
if lost:
    note(f"platform images not in the image index: {lost}")

print("Stated calculations")
x = P.dropna(subset=["engine_hp", "mass_combat_t", "hp_per_tonne"])
bad = (x.engine_hp / x.mass_combat_t - x.hp_per_tonne).abs() > 0.01 * x.hp_per_tonne
check(not bad.any(), f"hp_per_tonne = engine_hp / mass_combat_t ({int(bad.sum())} of {len(x)} off)")
for name, df, cols in [
    ("munitions", M, ["pen_mm_0m", "pen_mm_1000m", "pen_mm_2000m", "pen_mm_3000m"]),
    ("capabilities", C, ["best_pen_mm_0m", "best_pen_mm_1000m", "best_pen_mm_2000m"]),
    ("platforms", P, ["best_pen_mm_0m", "best_pen_mm_1000m", "best_pen_mm_2000m"]),
]:
    d = df.dropna(subset=cols)
    rising = (d[cols].diff(axis=1).iloc[:, 1:] > 0.5).any(axis=1)
    check(not rising.any(), f"{name}: penetration never rises with range ({int(rising.sum())} rows)")
sc = M[M.is_shaped_charge == True].dropna(subset=["pen_mm_0m", "pen_mm_2000m"])  # noqa: E712
check((sc.pen_mm_0m == sc.pen_mm_2000m).all(), "shaped charges: penetration does not change with range")

print("Signs of blanket calculations")
ke = M[M.kind == "apfsds"].dropna(subset=["pen_mm_0m", "pen_mm_2000m"])
r2 = (ke.pen_mm_2000m / ke.pen_mm_0m).round(2)
note(f"APFSDS 2 km / muzzle penetration ratios: {sorted(r2.unique().tolist())} over {len(ke)} rounds" + (" (one curve for every calibre: a blanket calculation)" if r2.nunique() == 1 else " (per round: tools/penetration_model.py)"))
land = P[P.domain == "land"]
shared = 0
total = 0
for _, r in land.iterrows():
    sib = land[(land.base_asset_id == r.base_asset_id) & (land.asset_id != r.asset_id)]
    if len(sib):
        total += 1
        shared += int(((sib.mass_combat_t == r.mass_combat_t) & (sib.engine_hp == r.engine_hp)).any())
note(f"land rows with a sibling variant: {total}; of those with the same mass and engine as a sibling: {shared}")
fires = S[S.bgws_support_role == "fires"]
top = fires.groupby(["response_turns", "loiter_turns", "sorties_per_day"]).size().sort_values(ascending=False)
note(f"fire support rows sharing the most common (response, loiter, sorties) {top.index[0]}: {top.iloc[0]} of {len(fires)}")

print("Provenance")
for name, df, col in [("munitions", M, "source_confidence"), ("support", S, "confidence"), ("sections", SEC, "source_confidence"), ("platform protection", P, "protection_confidence")]:
    note(f"{name} {col}: {df[col].value_counts(dropna=False).to_dict()}")
generic = M.source_ref.str.contains("see README", na=False).sum()
note(f"munitions citing only 'published reference tables ... (see README sources)': {generic} of {len(M)}")
note(f"platform mass_source values: {P.mass_source.value_counts().to_dict()}")
class_typical = S.source_ref.str.contains("class-typical|class-default", na=False).sum()
note(f"support rows with class-typical or class-default loadouts: {class_typical} of {len(S)}")

print("Empty or unused columns")
for col in P.columns:
    if P[col].isna().all():
        note(f"platform column {col} is empty")
if (P.has_smoke == False).all():  # noqa: E712
    note("has_smoke is false on every platform")

print(f"\n{len(hard)} hard check(s) failed")
sys.exit(1 if hard else 0)
