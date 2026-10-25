#!/usr/bin/env python3
"""The synthetic catalogue's authoring source (RUST_ENGINE.md §14.1).

Writes data/catalogue/*.json. Every record is synthetic: plausible orders of
magnitude anchored on open sources (the research reference's P-rows where
they apply), never the real performance of a real system.

Run:  python3 tools/catalogue.py
"""
import json
import os

OUT = os.path.join(os.path.dirname(__file__), "..", "data", "catalogue")
KMH = 1 / 3.6
KT = 0.514444


def write(name, items):
    os.makedirs(OUT, exist_ok=True)
    with open(os.path.join(OUT, name), "w") as f:
        json.dump(items, f, indent=1, sort_keys=False)
        f.write("\n")


# ── sensors ────────────────────────────────────────────────────────────────
def eye(rng=4000, regard=360):
    return {"kind": "eye", "fovDeg": 60, "regardDeg": regard, "mradPerCycle": 0.6, "maxRangeM": rng}


def optic(mrad=0.15, rng=8000, fov=8, mast=0, regard=360):
    return {"kind": "optic", "fovDeg": fov, "regardDeg": regard, "mradPerCycle": mrad, "maxRangeM": rng, "mastM": mast}


def thermal(mrad=0.2, rng=6000, fov=6, mast=0, regard=360):
    return {"kind": "thermal", "fovDeg": fov, "regardDeg": regard, "mradPerCycle": mrad, "maxRangeM": rng, "mastM": mast}


def ii(mrad=0.8, rng=1500):
    return {"kind": "ii", "fovDeg": 40, "regardDeg": 360, "mradPerCycle": mrad, "maxRangeM": rng}


def radar(peak_w, gain_db, freq_ghz, rng, scan_s=4, mti=1.0, tracks=0, fov=2.0, mast=4, kind="radar", snr=13, bw=1.0, losses=6, duty=0.01):
    return {
        "kind": kind, "peakW": peak_w, "gainDb": gain_db, "freqGhz": freq_ghz, "maxRangeM": rng, "scanS": scan_s,
        "mtiMinMs": mti, "tracks": tracks, "fovDeg": fov, "regardDeg": 360, "mastM": mast, "snrDetectDb": snr,
        "bandwidthMhz": bw, "lossesDb": losses, "duty": duty,
    }


def esm(sens=-75, fmin=0.02, fmax=18, df=3, rng=60000, kind="esm"):
    return {"kind": kind, "sensitivityDbm": sens, "fminGhz": fmin, "fmaxGhz": fmax, "dfRmsDeg": df, "maxRangeM": rng, "regardDeg": 360}


def rf_detect(rng=5000):
    return {"kind": "rfDetect", "sensitivityDbm": -95, "fminGhz": 0.3, "fmaxGhz": 6.0, "dfRmsDeg": 8, "maxRangeM": rng, "regardDeg": 360}


def acoustic(rng=3000):
    return {"kind": "acoustic", "diDb": 6, "dtDb": 6, "maxRangeM": rng, "regardDeg": 360}


def sonar_p(di=20, dt=8, rng=60000, band=300):
    return {"kind": "sonarPassive", "diDb": di, "dtDb": dt, "maxRangeM": rng, "bandHz": band, "regardDeg": 360}


def sonar_a(sl=225, di=25, dt=12, rng=30000):
    return {"kind": "sonarActive", "sourceDb": sl, "diDb": di, "dtDb": dt, "maxRangeM": rng, "regardDeg": 360}


def irst(mrad=0.1, rng=60000):
    return {"kind": "irst", "fovDeg": 4, "regardDeg": 140, "mradPerCycle": mrad, "maxRangeM": rng}


# ── radios ─────────────────────────────────────────────────────────────────
def vhf(w=5, mast=2.5, hop=17):
    return {"bearer": "vhf", "freqMhz": 60, "powerW": w, "antennaM": mast, "bandwidthKhz": 25, "sensitivityDbm": -116, "rateKbps": 16, "hopGainDb": hop}


def uhf(w=10, mast=3):
    return {"bearer": "uhf", "freqMhz": 300, "powerW": w, "antennaM": mast, "bandwidthKhz": 25, "sensitivityDbm": -110, "rateKbps": 64, "hopGainDb": 10}


def hf(w=100):
    return {"bearer": "hf", "freqMhz": 6, "powerW": w, "antennaM": 6, "bandwidthKhz": 3, "sensitivityDbm": -120, "rateKbps": 2.4, "hopGainDb": 0}


def satcom_geo():
    return {"bearer": "satcomGeo", "freqMhz": 8000, "powerW": 50, "gainDb": 30, "antennaM": 2, "bandwidthKhz": 500, "sensitivityDbm": -120, "rateKbps": 512, "cryptoS": 1.0}


def satcom_leo():
    return {"bearer": "satcomLeo", "freqMhz": 12000, "powerW": 20, "gainDb": 33, "antennaM": 1, "bandwidthKhz": 50000, "sensitivityDbm": -100, "rateKbps": 10000, "cryptoS": 0.5}


def datalink(w=200):
    return {"bearer": "datalink", "freqMhz": 1000, "powerW": w, "antennaM": 4, "bandwidthKhz": 3000, "sensitivityDbm": -100, "rateKbps": 57.6, "hopGainDb": 20, "cryptoS": 0.0}


def mesh(w=5, mast=2):
    return {"bearer": "mesh", "freqMhz": 1400, "powerW": w, "antennaM": mast, "bandwidthKhz": 5000, "sensitivityDbm": -100, "rateKbps": 2000, "hopGainDb": 6, "cryptoS": 0.1}


def dronelink(freq=2400, w=1):
    return {"bearer": "droneLink", "freqMhz": freq, "powerW": w, "antennaM": 1.5, "bandwidthKhz": 10000, "sensitivityDbm": -95, "rateKbps": 4000, "hopGainDb": 0, "cryptoS": 0.0}


def mount(weapon, launchers=1, ready=1, **reserve):
    return {"weapon": weapon, "launchers": launchers, "ready": ready, "reserve": reserve}


def rec(id, name, archetype, domain, *, sidc="", mover=None, body=None, sig=None, sensors=(), mounts=(), radios=(), emitters=(), stores=None, carries=(), crew=None, cost=0, hq=False, echelon=None, note=""):
    r = {"id": id, "name": name, "archetype": archetype, "domain": domain, "sidc": sidc}
    if mover: r["mover"] = mover
    if body: r["body"] = body
    if sig: r["signature"] = sig
    if sensors: r["sensors"] = list(sensors)
    if mounts: r["mounts"] = list(mounts)
    if radios: r["radios"] = list(radios)
    if emitters: r["emitters"] = list(emitters)
    if stores: r["stores"] = stores
    if carries: r["carries"] = [{"record": c, "count": n} for c, n in carries]
    if crew: r["crew"] = crew
    if cost: r["costUsd"] = cost
    if hq: r["hq"] = True
    if echelon: r["echelon"] = echelon
    if note: r["note"] = note
    return r


def tracked(max_kmh=65, cross_kmh=35, vci=50, slope=31, ford=1.2, gap=2.7, mlc=70, l_km=3.9, idle=38):
    return {"kind": "tracked", "maxMs": max_kmh * KMH, "crossMs": cross_kmh * KMH, "vci": vci, "maxSlopeDeg": slope, "fordM": ford, "gapM": gap, "mlc": mlc, "fuelLPerKm": l_km, "fuelLPerHIdle": idle}


def wheeled(max_kmh=90, cross_kmh=30, vci=70, slope=30, ford=1.0, gap=0.8, mlc=20, l_km=0.6, idle=4):
    return {"kind": "wheeled", "maxMs": max_kmh * KMH, "crossMs": cross_kmh * KMH, "vci": vci, "maxSlopeDeg": slope, "fordM": ford, "gapM": gap, "mlc": mlc, "fuelLPerKm": l_km, "fuelLPerHIdle": idle}


def foot(kmh=4.5, cross=3.0):
    # Dismounted pace: 4 / 2–3 / 1–2 / <1 km/h by going [P26].
    return {"kind": "foot", "maxMs": kmh * KMH, "crossMs": cross * KMH, "vci": 5, "maxSlopeDeg": 45, "fordM": 1.0, "gapM": 2.0, "mlc": 0}


def body(count, crew, l, w, h, vul, **kw):
    b = {"count": count, "crewEach": crew, "lengthM": l, "widthM": w, "heightM": h, "vulnerability": vul}
    b.update(kw)
    return b


def armour(fke, fce, ske, sce, rear, top, era=0.0):
    return {"frontKe": fke, "frontCe": fce, "sideKe": ske, "sideCe": sce, "rear": rear, "top": top, "era": era}


def sig(contrast=0.5, irrun=10, ircold=0.5, rcs=(5, 10, 5, 10), ac=80, acpm=1.0, dust=False, mag=0.0):
    return {"visualContrast": contrast, "irRunningK": irrun, "irColdK": ircold, "rcsM2": list(rcs), "acousticDb": ac, "acousticDbPerMs": acpm, "dust": dust, "magnetic": mag}


land = [
    rec("mbt_plt", "MBT platoon", "armour", "land", sidc="10031000001211000000", echelon="platoon",
        mover=tracked(), body=body(4, 4, 7.9, 3.7, 2.4, "heavyArmour", armour=armour(600, 900, 80, 300, 40, 40),
                                    aps={"charges": 0, "pDefeat": 0.0}),
        sig=sig(0.6, 25, 0.5, (8, 20, 10, 25), 105, 0.6, True, 1.0),
        sensors=[thermal(0.15, 6000, 3), optic(0.1, 8000, 3), eye(4000)],
        mounts=[mount("gun_120", 4, 17, apfsds_120=14, heat_120=9), mount("mg_coax", 4, 1000, ball_762=3000)],
        radios=[vhf(50, 3)], stores={"fuelL": 4 * 1600, "foodDays": 3, "waterL": 160, "spares": 4},
        crew={"skill": 0.65, "cohesion": 0.65}, cost=4 * 8e6),
    rec("mbt_plt_era", "MBT platoon (ERA, APS)", "armour", "land", sidc="10031000001211000000", echelon="platoon",
        mover=tracked(70, 35, 55, 30, 1.2, 2.8, 50, 3.0, 25),
        body=body(4, 3, 6.9, 3.6, 2.2, "heavyArmour", armour=armour(550, 1000, 70, 280, 40, 50, 0.5),
                  aps={"charges": 2, "pDefeat": 0.7, "vsKe": False, "vsTop": False}),
        sig=sig(0.55, 25, 0.5, (7, 18, 9, 22), 105, 0.6, True, 1.0),
        sensors=[thermal(0.25, 4500, 3), optic(0.12, 7000, 3), eye(4000)],
        mounts=[mount("gun_125", 4, 22, apfsds_125=12, heat_125=10, atgm_gl=4), mount("mg_coax", 4, 1000, ball_762=2000)],
        radios=[vhf(30, 3)], stores={"fuelL": 4 * 1200, "foodDays": 3, "waterL": 120, "spares": 4},
        crew={"skill": 0.5, "cohesion": 0.55}, cost=4 * 4e6),
    rec("ifv_plt", "IFV platoon", "ifv", "land", sidc="10031000001211020000", echelon="platoon",
        mover=tracked(65, 35, 40, 31, 1.5, 2.5, 35, 2.0, 15),
        body=body(4, 3, 6.5, 3.4, 3.0, "lightArmour", armour=armour(60, 120, 30, 60, 20, 15)),
        sig=sig(0.6, 20, 0.5, (6, 15, 8, 18), 100, 0.6, True, 0.8),
        sensors=[thermal(0.25, 4000, 4), optic(0.2, 6000, 4), eye(4000)],
        mounts=[mount("gun_30", 4, 200, apfsds_30=400, he_30=600), mount("atgm_vehicle", 4, 2, atgm_heavy=4), mount("mg_coax", 4, 800, ball_762=2000)],
        radios=[vhf(50, 3)], carries=[("inf_section", 4)],
        stores={"fuelL": 4 * 700, "foodDays": 3, "waterL": 400, "spares": 3},
        crew={"skill": 0.6, "cohesion": 0.6}, cost=4 * 4e6),
    rec("apc_plt", "Wheeled APC platoon", "ifv", "land", sidc="10031000001211040000", echelon="platoon",
        mover=wheeled(100, 35, 60, 30, 1.2, 1.5, 25, 0.7, 5),
        body=body(4, 2, 7.0, 2.9, 2.7, "lightArmour", armour=armour(20, 40, 14, 30, 10, 8)),
        sig=sig(0.55, 15, 0.5, (5, 12, 6, 14), 92, 0.6, True, 0.6),
        sensors=[thermal(0.4, 3000, 8), optic(0.3, 5000, 8), eye(4000)],
        mounts=[mount("hmg", 4, 200, ball_127=1600)],
        radios=[vhf(50, 3)], carries=[("inf_section", 4)],
        stores={"fuelL": 4 * 400, "foodDays": 3, "waterL": 400, "spares": 3}, cost=4 * 1.5e6),
    rec("inf_section", "Infantry section", "infantry", "land", sidc="10031000001211000000", echelon="section",
        mover=foot(), body=body(1, 8, 20, 20, 1.0, "personnel"),
        sig=sig(0.35, 6, 6, (1, 1, 1, 1), 50, 0.5, False, 0.0),
        sensors=[eye(3000), optic(0.25, 3000, 10), ii(0.8, 1200), thermal(0.6, 1500, 12)],
        mounts=[mount("rifles", 1, 240, ball_556=1800), mount("lmg", 1, 200, ball_556=1000), mount("rpg", 1, 2, heat_rpg=4)],
        radios=[vhf(5, 1.5)], stores={"foodDays": 2, "waterL": 24, "batteries": 16, "medical": 8},
        crew={"skill": 0.6, "cohesion": 0.6}, cost=8 * 50e3),
    rec("inf_plt", "Infantry platoon (dismounted)", "infantry", "land", sidc="10031000001211000000", echelon="platoon",
        mover=foot(), body=body(3, 9, 40, 40, 1.0, "personnel"),
        sig=sig(0.35, 6, 6, (1, 1, 1, 1), 55, 0.5, False, 0.0),
        sensors=[eye(3000), optic(0.25, 3000, 10), ii(0.8, 1200), thermal(0.6, 1500, 12)],
        mounts=[mount("rifles", 3, 240, ball_556=5400), mount("lmg", 3, 200, ball_556=3000), mount("rpg", 3, 2, heat_rpg=9)],
        radios=[vhf(5, 1.5)], stores={"foodDays": 2, "waterL": 80, "batteries": 40, "medical": 25},
        crew={"skill": 0.6, "cohesion": 0.6}, cost=27 * 50e3),
    rec("atgm_team", "ATGM team (fire-and-forget)", "atgmTeam", "land", sidc="10031000001211000000", echelon="team",
        mover=foot(4.0, 2.5), body=body(2, 3, 10, 10, 0.8, "personnel"),
        sig=sig(0.3, 6, 6, (1, 1, 1, 1), 45, 0.5),
        sensors=[eye(3000), thermal(0.12, 4000, 4), optic(0.12, 4000, 4)],
        mounts=[mount("atgm_ff_launcher", 2, 1, atgm_ff=4)], radios=[vhf(5, 1.5)],
        stores={"foodDays": 2, "waterL": 18, "batteries": 8}, crew={"skill": 0.65, "cohesion": 0.6}, cost=2 * 300e3),
    rec("atgm_team_cmd", "ATGM team (command-guided)", "atgmTeam", "land", sidc="10031000001211000000", echelon="team",
        mover=foot(3.5, 2.0), body=body(2, 3, 10, 10, 0.8, "personnel"),
        sig=sig(0.3, 6, 6, (1, 1, 1, 1), 45, 0.5),
        sensors=[eye(3000), thermal(0.12, 5000, 4), optic(0.1, 6000, 4)],
        mounts=[mount("atgm_cmd_launcher", 2, 1, atgm_heavy=6)], radios=[vhf(5, 1.5)],
        stores={"foodDays": 2, "waterL": 18, "batteries": 8}, crew={"skill": 0.55, "cohesion": 0.55}, cost=2 * 150e3),
    rec("mortar_sec", "120 mm mortar section", "mortar", "land", sidc="10031000001311000000", echelon="section",
        mover=wheeled(80, 30, 65), body=body(4, 4, 6, 2.5, 2.5, "soft"),
        sig=sig(0.45, 12, 0.5, (4, 8, 4, 8), 88, 0.6, True, 0.3),
        sensors=[eye(2000)], mounts=[mount("mortar_120", 4, 1, he_120m=200, smoke_120m=30)],
        radios=[vhf(20, 3)], stores={"fuelL": 4 * 200, "foodDays": 3, "waterL": 100}, cost=4 * 400e3),
    rec("sph_bty", "155 mm SP howitzer battery", "artillery", "land", sidc="10031000001303000000", echelon="battery",
        mover=tracked(55, 30, 50, 30, 1.0, 2.0, 40, 3.0, 20),
        body=body(6, 5, 9.0, 3.4, 3.0, "lightArmour", armour=armour(30, 50, 20, 40, 15, 10)),
        sig=sig(0.6, 22, 0.5, (10, 25, 12, 30), 100, 0.6, True, 0.9),
        sensors=[eye(2000)],
        mounts=[mount("how_155", 6, 1, he_155=300, he_155_erfb=60, gnss_155=12, dpicm_155=40, smoke_155=30)],
        radios=[vhf(50, 3), datalink(50)], stores={"fuelL": 6 * 900, "foodDays": 3, "waterL": 200, "spares": 6},
        crew={"skill": 0.6, "cohesion": 0.6}, cost=6 * 7e6),
    rec("towed_bty", "152 mm towed howitzer battery", "artillery", "land", sidc="10031000001303000000", echelon="battery",
        mover=wheeled(60, 20, 75, 25, 0.8, 0.7, 20, 0.8, 6), body=body(6, 7, 8, 3, 2.5, "soft"),
        sig=sig(0.5, 15, 0.5, (8, 15, 8, 20), 95, 0.6, True, 0.8),
        sensors=[eye(2000)], mounts=[mount("how_152", 6, 1, he_152=400, he_152_rap=40)],
        radios=[vhf(20, 3)], stores={"fuelL": 6 * 250, "foodDays": 3, "waterL": 200},
        crew={"skill": 0.5, "cohesion": 0.55}, cost=6 * 1.2e6),
    rec("mlrs_bty", "Guided MLRS battery", "artillery", "land", sidc="10031000001304000000", echelon="battery",
        mover=wheeled(85, 25, 70, 30, 1.0, 1.0, 25, 0.8, 6), body=body(4, 3, 7, 2.4, 3.2, "soft"),
        sig=sig(0.55, 18, 0.5, (8, 18, 8, 20), 92, 0.6, True, 0.6),
        sensors=[eye(2000)], mounts=[mount("mlrs_launcher", 4, 6, gmlrs=24)],
        radios=[vhf(50, 3), datalink(50), satcom_leo()], stores={"fuelL": 4 * 300, "foodDays": 3, "waterL": 100},
        crew={"skill": 0.7, "cohesion": 0.65}, cost=4 * 5e6),
    rec("recce_sec", "Reconnaissance section", "recce", "land", sidc="10031000001213000000", echelon="section",
        mover=wheeled(100, 40, 55, 30, 1.0, 1.2, 15, 0.4, 3), body=body(2, 4, 5.5, 2.4, 2.2, "lightArmour", armour=armour(14, 30, 10, 20, 8, 8)),
        sig=sig(0.4, 12, 0.5, (4, 9, 5, 10), 85, 0.5, True, 0.4),
        sensors=[thermal(0.08, 12000, 2, 8), optic(0.06, 15000, 2, 8),
                 radar(30, 32, 10.0, 24000, 8, 0.3, 0, 2.0, 8, snr=12, bw=10), eye(4000), ii(0.6, 2000)],
        mounts=[mount("hmg", 2, 200, ball_127=800)], radios=[vhf(50, 6), satcom_leo(), mesh(5, 6)],
        carries=[("recon_quad", 4)], stores={"fuelL": 2 * 300, "foodDays": 5, "waterL": 80, "batteries": 40},
        crew={"skill": 0.7, "cohesion": 0.7}, cost=2 * 2e6),
    rec("eng_plt", "Combat engineer platoon", "engineer", "land", sidc="10031000001407000000", echelon="platoon",
        mover=tracked(60, 30, 50), body=body(4, 4, 7.5, 3.6, 2.6, "lightArmour", armour=armour(80, 150, 30, 60, 20, 15)),
        sig=sig(0.6, 20, 0.5, (8, 18, 9, 20), 100, 0.6, True, 0.9),
        sensors=[thermal(0.3, 3000, 8), eye(3000)],
        mounts=[mount("mg_coax", 4, 500, ball_762=2000), mount("line_charge", 2, 1, mclc=4)],
        radios=[vhf(50, 3)], stores={"fuelL": 4 * 800, "foodDays": 3, "waterL": 200, "cargoT": 8},
        crew={"skill": 0.65, "cohesion": 0.6}, cost=4 * 4e6, note="Breaching (plough, roller, line charge), obstacles, digging."),
    rec("bridge_sec", "Bridging section", "engineer", "land", sidc="10031000001407000000", echelon="section",
        mover=tracked(55, 25, 55), body=body(2, 3, 11, 4, 3.8, "lightArmour", armour=armour(20, 30, 15, 20, 10, 8)),
        sig=sig(0.7, 20, 0.5, (15, 30, 15, 35), 100, 0.6, True, 0.9),
        sensors=[eye(2000)], radios=[vhf(20, 3)], stores={"fuelL": 2 * 900},
        cost=2 * 6e6, note="Launches a 20 m, MLC 70 bridge in about 5 minutes."),
    rec("hq_coy", "Company command post", "hq", "land", sidc="10031000001211000000", echelon="company", hq=True,
        mover=tracked(65, 35, 45), body=body(2, 4, 6.5, 3.4, 3.0, "lightArmour", armour=armour(30, 60, 20, 40, 15, 10)),
        sig=sig(0.55, 20, 0.5, (6, 15, 8, 18), 100, 0.6, True, 0.8),
        sensors=[thermal(0.25, 4000, 6), eye(3000)], mounts=[mount("hmg", 2, 200, ball_127=800)],
        radios=[vhf(50, 6), vhf(50, 6), mesh(10, 6)], stores={"fuelL": 2 * 600, "foodDays": 3}, cost=2 * 3e6),
    rec("hq_bn", "Battalion command post", "hq", "land", sidc="10031000001211000000", echelon="battalion", hq=True,
        mover=wheeled(90, 25, 65), body=body(6, 4, 7, 2.5, 3.2, "soft"),
        sig=sig(0.6, 25, 2, (8, 18, 9, 20), 95, 0.6, True, 0.6),
        sensors=[eye(2000)], mounts=[mount("hmg", 2, 200, ball_127=800)],
        radios=[vhf(50, 10), vhf(50, 10), hf(400), satcom_geo(), satcom_leo(), datalink(200), mesh(10, 10)],
        stores={"fuelL": 6 * 300, "foodDays": 5, "waterL": 600}, crew={"skill": 0.65, "cohesion": 0.7}, cost=6 * 1.5e6),
    rec("hq_bde", "Brigade main command post", "hq", "land", sidc="10031000001211000000", echelon="brigade", hq=True,
        mover=wheeled(90, 20, 70), body=body(14, 5, 10, 2.5, 3.5, "soft"),
        sig=sig(0.7, 30, 3, (15, 30, 15, 40), 95, 0.6, True, 0.8),
        sensors=[eye(2000)], radios=[vhf(50, 12), hf(1000), satcom_geo(), satcom_leo(), datalink(200), mesh(20, 12)],
        stores={"fuelL": 14 * 300, "foodDays": 7, "waterL": 2000}, crew={"skill": 0.7, "cohesion": 0.7}, cost=14 * 1.5e6),
    rec("hq_div", "Division main command post", "hq", "land", sidc="10031000001211000000", echelon="division", hq=True,
        mover=wheeled(90, 20, 70), body=body(30, 6, 10, 2.5, 3.5, "soft"),
        sig=sig(0.8, 30, 3, (20, 40, 20, 50), 95, 0.6, True, 0.9),
        sensors=[eye(2000)], radios=[hf(1000), satcom_geo(), satcom_leo(), datalink(200), mesh(20, 15)],
        stores={"fuelL": 30 * 300, "foodDays": 7, "waterL": 5000}, crew={"skill": 0.7, "cohesion": 0.7}, cost=30 * 1.5e6),
    rec("hq_joint", "Joint headquarters", "hq", "node", sidc="10031000001211000000", echelon="joint", hq=True,
        mover={"kind": "static"}, body=body(1, 200, 100, 100, 10, "structure", missionKillPts=60, lossPts=150),
        sig=sig(0.8, 30, 10, (500, 500, 500, 800), 90, 0, False, 1.0),
        sensors=[], radios=[hf(5000), satcom_geo(), satcom_leo(), datalink(500), {"bearer": "wire", "freqMhz": 0, "powerW": 0, "rateKbps": 100000}],
        crew={"skill": 0.75, "cohesion": 0.75}, cost=50e6),
    rec("truck_plt", "Supply truck platoon", "convoy", "land", sidc="10031000001634000000", echelon="platoon",
        mover=wheeled(90, 25, 80, 25, 0.8, 0.6, 30, 0.45, 3), body=body(8, 2, 9, 2.5, 3.3, "soft"),
        sig=sig(0.55, 15, 0.5, (8, 15, 8, 18), 90, 0.6, True, 0.6),
        sensors=[eye(2000)], radios=[vhf(20, 2.5)], stores={"fuelL": 8 * 400, "cargoT": 8 * 10, "foodDays": 3},
        crew={"skill": 0.5, "cohesion": 0.55}, cost=8 * 250e3),
    rec("fuel_plt", "Fuel tanker platoon", "convoy", "land", sidc="10031000001634000000", echelon="platoon",
        mover=wheeled(85, 20, 85, 22, 0.8, 0.6, 35, 0.5, 3), body=body(6, 2, 10, 2.5, 3.4, "soft"),
        sig=sig(0.55, 15, 0.5, (8, 16, 8, 18), 90, 0.6, True, 0.6),
        sensors=[eye(2000)], radios=[vhf(20, 2.5)], stores={"fuelL": 6 * 400, "cargoT": 6 * 15, "foodDays": 3}, cost=6 * 300e3),
    rec("ambulance_sec", "Ambulance section", "medical", "land", sidc="10031000001613000000", echelon="section",
        mover=tracked(60, 30, 45), body=body(4, 2, 6.0, 3.0, 2.6, "lightArmour", armour=armour(20, 30, 14, 20, 10, 8)),
        sig=sig(0.5, 15, 0.5, (5, 12, 6, 14), 95, 0.6, True, 0.6),
        sensors=[eye(2000)], radios=[vhf(20, 2.5)], stores={"fuelL": 4 * 400, "medical": 80}, cost=4 * 1e6,
        note="Four casualty spaces each."),
    rec("aid_post", "Battalion aid post", "medical", "node", sidc="10031000001613000000",
        mover={"kind": "static"}, body=body(1, 12, 30, 30, 3, "soft"),
        sig=sig(0.5, 15, 5, (20, 20, 20, 30), 70, 0), radios=[vhf(20, 6)], stores={"medical": 300}, cost=1e6),
    rec("shorad_bty", "Short-range air defence battery", "gbad", "land", sidc="10031000001301000000", echelon="battery",
        mover=tracked(60, 30, 50), body=body(4, 3, 7.7, 3.7, 3.3, "lightArmour", armour=armour(40, 60, 20, 40, 15, 10)),
        sig=sig(0.6, 20, 0.5, (8, 18, 9, 20), 100, 0.6, True, 0.8),
        sensors=[radar(15e3, 30, 10.0, 25000, 1.0, 5.0, 4, 2.0, 4, snr=12, bw=2), thermal(0.2, 10000, 3), eye(5000)],
        mounts=[mount("aa_gun_35", 4, 320, he_35_prox=2000), mount("sam_short_launcher", 4, 4, sam_short=16)],
        radios=[vhf(50, 3), datalink(100)], stores={"fuelL": 4 * 700, "foodDays": 3}, crew={"skill": 0.65, "cohesion": 0.6}, cost=4 * 15e6),
    rec("manpads_team", "MANPADS team", "gbad", "land", sidc="10031000001301000000", echelon="team",
        mover=foot(4.0, 2.5), body=body(2, 2, 8, 8, 0.8, "personnel"),
        sig=sig(0.3, 6, 6, (1, 1, 1, 1), 45, 0.5),
        sensors=[eye(6000), thermal(0.3, 6000, 10)], mounts=[mount("manpads", 2, 1, sam_manpads=4)],
        radios=[vhf(5, 1.5)], crew={"skill": 0.6, "cohesion": 0.6}, cost=2 * 200e3),
    rec("mrsam_bty", "Medium-range SAM battery", "gbad", "land", sidc="10031000001301000000", echelon="battery",
        mover=wheeled(70, 20, 70, 25, 1.0, 0.7, 30, 1.0, 10), body=body(6, 3, 10, 3.0, 3.5, "soft"),
        sig=sig(0.6, 18, 0.5, (12, 25, 12, 30), 92, 0.6, True, 0.8),
        sensors=[radar(200e3, 38, 3.0, 160000, 6.0, 15.0, 0, 1.5, 12, snr=13, bw=2, duty=0.02),
                 radar(60e3, 40, 9.0, 100000, 1.0, 15.0, 24, 1.0, 8, snr=13, bw=5, duty=0.05)],
        mounts=[mount("sam_medium_launcher", 4, 4, sam_medium=16)], radios=[vhf(50, 6), datalink(200)],
        stores={"fuelL": 6 * 400, "foodDays": 5}, crew={"skill": 0.65, "cohesion": 0.65}, cost=6 * 40e6),
    rec("lrsam_bty", "Long-range SAM battery", "gbad", "land", sidc="10031000001301000000", echelon="battery",
        mover=wheeled(70, 15, 75, 20, 1.0, 0.6, 40, 1.2, 12), body=body(8, 3, 12, 3.0, 3.8, "soft"),
        sig=sig(0.65, 20, 0.5, (15, 30, 15, 40), 92, 0.6, True, 0.9),
        sensors=[radar(800e3, 42, 5.0, 220000, 4.0, 20.0, 100, 1.0, 10, snr=13, bw=4, duty=0.03)],
        mounts=[mount("sam_long_launcher", 6, 4, sam_long=24, sam_long_bmd=8)], radios=[vhf(50, 8), datalink(500), satcom_geo()],
        stores={"fuelL": 8 * 400, "foodDays": 5}, crew={"skill": 0.75, "cohesion": 0.7}, cost=1e9),
    rec("ew_comms", "Comms jammer detachment", "ewJammer", "land", sidc="10031000001505000000", echelon="detachment",
        mover=wheeled(80, 25, 70), body=body(2, 3, 9, 2.5, 3.5, "soft"),
        sig=sig(0.55, 15, 0.5, (8, 15, 8, 18), 90, 0.6, True, 0.6),
        sensors=[esm(-95, 0.02, 3.0, 3, 50000), eye(2000)],
        emitters=[{"kind": "commsJammer", "fminMhz": 20, "fmaxMhz": 3000, "erpW": 1500, "sectorDeg": 120, "antennaM": 10, "maxBands": 3}],
        radios=[vhf(20, 3)], stores={"fuelL": 2 * 300}, crew={"skill": 0.7, "cohesion": 0.6}, cost=2 * 5e6),
    rec("ew_gnss", "GNSS jammer detachment", "ewJammer", "land", sidc="10031000001505000000", echelon="detachment",
        mover=wheeled(80, 25, 70), body=body(1, 3, 8, 2.5, 3.5, "soft"),
        sig=sig(0.5, 15, 0.5, (8, 15, 8, 18), 90, 0.6, True, 0.6), sensors=[eye(2000)],
        emitters=[{"kind": "gnssJammer", "fminMhz": 1160, "fmaxMhz": 1610, "erpW": 200, "sectorDeg": 360, "antennaM": 10, "maxBands": 3}],
        radios=[vhf(20, 3)], cost=3e6, note="Denies GNSS out to about 25 km per module [P16]."),
    rec("ew_drone", "Counter-drone jammer team", "ewJammer", "land", sidc="10031000001505000000", echelon="team",
        mover=wheeled(80, 25, 70), body=body(1, 3, 6, 2.2, 2.2, "soft"),
        sig=sig(0.45, 12, 0.5, (5, 10, 5, 12), 85, 0.5, True, 0.4),
        sensors=[rf_detect(6000), radar(500, 25, 9.5, 5000, 1.0, 0.0, 10, 3.0, 3, snr=12, bw=20), thermal(0.3, 3000, 10), acoustic(800)],
        emitters=[{"kind": "commsJammer", "fminMhz": 400, "fmaxMhz": 6000, "erpW": 100, "sectorDeg": 360, "antennaM": 3, "maxBands": 4},
                  {"kind": "gnssJammer", "fminMhz": 1160, "fmaxMhz": 1610, "erpW": 20, "sectorDeg": 360, "antennaM": 3, "maxBands": 3}],
        mounts=[mount("shotgun", 2, 8, ball_shot=200), mount("interceptor_rack", 1, 4, interceptor_drone=12)],
        radios=[vhf(20, 2.5)], crew={"skill": 0.65, "cohesion": 0.6}, cost=1e6),
    rec("esm_team", "Electronic support (DF) team", "ewEsm", "land", sidc="10031000001505000000", echelon="team",
        mover=wheeled(80, 25, 70), body=body(2, 3, 6, 2.2, 2.4, "soft"),
        sig=sig(0.45, 12, 0.5, (5, 10, 5, 12), 85, 0.5, True, 0.4),
        sensors=[esm(-105, 0.02, 6.0, 2, 60000), eye(2000)], radios=[vhf(20, 6), datalink(50)],
        crew={"skill": 0.75, "cohesion": 0.65}, cost=2 * 2e6),
    rec("wlr", "Weapon-locating radar", "sensor", "land", sidc="10031000001501000000", echelon="detachment",
        mover=wheeled(80, 25, 70), body=body(1, 4, 9, 2.5, 3.5, "soft"),
        sig=sig(0.55, 15, 0.5, (8, 15, 8, 18), 90, 0.6, True, 0.6),
        sensors=[radar(50e3, 40, 5.5, 60000, 2.0, 50.0, 40, 1.5, 3, kind="wlr", snr=13, bw=5, duty=0.03)],
        radios=[vhf(50, 6), datalink(100)], crew={"skill": 0.7, "cohesion": 0.65}, cost=20e6,
        note="Detects shells and rockets in flight and backtracks the firing point."),
    rec("fpv_team", "FPV drone team (RF)", "droneTeam", "land", sidc="10031000001211000000", echelon="team",
        mover=foot(3.5, 2.0), body=body(1, 3, 10, 10, 0.8, "personnel"),
        sig=sig(0.3, 6, 6, (1, 1, 1, 1), 45, 0.5),
        sensors=[eye(2000), thermal(0.6, 1500, 12)], mounts=[mount("fpv_launch", 1, 1, fpv_drone=20)],
        radios=[vhf(5, 1.5), dronelink(5800, 1)], stores={"batteries": 30, "foodDays": 2}, crew={"skill": 0.55, "cohesion": 0.6}, cost=150e3),
    rec("fpv_fibre_team", "FPV drone team (fibre)", "droneTeam", "land", sidc="10031000001211000000", echelon="team",
        mover=foot(3.5, 2.0), body=body(1, 3, 10, 10, 0.8, "personnel"),
        sig=sig(0.3, 6, 6, (1, 1, 1, 1), 45, 0.5),
        sensors=[eye(2000), thermal(0.6, 1500, 12)], mounts=[mount("fpv_launch_fibre", 1, 1, fpv_fibre=10)],
        radios=[vhf(5, 1.5)], stores={"batteries": 20, "foodDays": 2}, crew={"skill": 0.6, "cohesion": 0.6}, cost=150e3),
    rec("bomber_team", "Heavy bomber-drone team", "droneTeam", "land", sidc="10031000001211000000", echelon="team",
        mover=wheeled(80, 25, 70), body=body(1, 4, 6, 2.2, 2.2, "soft"),
        sig=sig(0.4, 10, 0.5, (5, 10, 5, 12), 80, 0.5, True, 0.3),
        sensors=[eye(2000)], mounts=[mount("bomber_launch", 1, 1, bomber_drone=6)], radios=[vhf(10, 2.5), dronelink(900, 2)],
        stores={"batteries": 30}, crew={"skill": 0.6, "cohesion": 0.6}, cost=200e3),
    rec("recon_drone_team", "Reconnaissance drone team", "droneTeam", "land", sidc="10031000001213000000", echelon="team",
        mover=wheeled(80, 25, 70), body=body(1, 3, 5, 2.0, 2.0, "soft"),
        sig=sig(0.4, 10, 0.5, (4, 8, 4, 10), 80, 0.5, True, 0.3),
        sensors=[eye(2000)], radios=[vhf(10, 2.5), mesh(5, 4), satcom_leo()], carries=[("recon_quad", 6), ("isr_fixed_small", 2)],
        stores={"batteries": 40}, crew={"skill": 0.65, "cohesion": 0.6}, cost=500e3),
    rec("loiter_team", "Loitering munition team", "droneTeam", "land", sidc="10031000001211000000", echelon="team",
        mover=wheeled(80, 25, 70), body=body(1, 3, 6, 2.2, 2.2, "soft"),
        sig=sig(0.4, 10, 0.5, (5, 10, 5, 12), 80, 0.5, True, 0.3),
        sensors=[eye(2000)], mounts=[mount("loiter_launch", 1, 1, loiter_munition=8)], radios=[vhf(20, 3), dronelink(900, 5)],
        crew={"skill": 0.65, "cohesion": 0.6}, cost=300e3),
    rec("ugv_sec", "Logistics UGV section", "convoy", "land", sidc="10031000001634000000", echelon="section",
        mover=tracked(15, 10, 30, 30, 0.5, 0.5, 2, 0.1, 0.2), body=body(4, 0, 2.0, 1.2, 0.8, "soft"),
        sig=sig(0.25, 5, 0.5, (0.5, 0.8, 0.5, 1), 60, 0.5, False, 0.1),
        sensors=[eye(500)], radios=[mesh(2, 1)], stores={"cargoT": 4 * 0.3, "batteries": 8},
        cost=4 * 50e3, note="Last-mile delivery into the kill zone [P19]."),
    rec("coastal_bty", "Coastal anti-ship missile battery", "coastal", "land", sidc="10031000001304000000", echelon="battery",
        mover=wheeled(70, 20, 70), body=body(4, 3, 12, 3.0, 3.8, "soft"),
        sig=sig(0.6, 18, 0.5, (12, 25, 12, 30), 92, 0.6, True, 0.8),
        sensors=[radar(100e3, 36, 9.0, 250000, 6.0, 0.0, 10, 1.5, 15, snr=13, bw=2)],
        mounts=[mount("antiship_launcher", 4, 2, antiship_subsonic=8)], radios=[vhf(50, 6), datalink(100), satcom_geo()],
        crew={"skill": 0.65, "cohesion": 0.65}, cost=4 * 30e6),
    rec("depot", "Supply depot", "node", "node", sidc="10031000001634000000",
        mover={"kind": "static"}, body=body(1, 40, 300, 300, 8, "structure", missionKillPts=40, lossPts=120),
        sig=sig(0.8, 15, 8, (300, 300, 300, 500), 70, 0, False, 0.6),
        radios=[vhf(20, 10), hf(400), {"bearer": "wire", "freqMhz": 0, "powerW": 0, "rateKbps": 10000}],
        stores={"fuelL": 2_000_000, "cargoT": 8000, "foodDays": 60, "waterL": 500_000, "spares": 400, "medical": 2000}, cost=20e6),
    rec("railhead", "Railhead", "node", "node", sidc="10031000001634000000",
        mover={"kind": "static"}, body=body(1, 60, 800, 200, 6, "structure", missionKillPts=50, lossPts=200),
        sig=sig(0.8, 12, 6, (500, 500, 500, 800), 75, 0, False, 1.0),
        radios=[hf(400), {"bearer": "wire", "freqMhz": 0, "powerW": 0, "rateKbps": 10000}],
        stores={"fuelL": 5_000_000, "cargoT": 20000, "foodDays": 90}, cost=50e6),
    rec("bridge", "Road bridge", "node", "node", sidc="",
        mover={"kind": "static"}, body=body(1, 0, 120, 12, 8, "structure", missionKillPts=20, lossPts=40),
        sig=sig(0.7, 1, 1, (200, 400, 200, 600), 40, 0, False, 1.0), note="MLC 80."),
]

# ── air ────────────────────────────────────────────────────────────────────
def fixed(cruise, mx, ceiling, climb, g, fuel_kg_h, endurance):
    return {"kind": "fixedWing", "cruiseMs": cruise, "maxMs": mx, "ceilingM": ceiling, "climbMs": climb, "maxG": g, "fuelKgPerH": fuel_kg_h, "enduranceS": endurance}


air = [
    rec("fighter_flt", "Multirole fighter flight", "aircraft", "air", sidc="10030100001101000000", echelon="flight",
        mover=fixed(250, 600, 15000, 250, 9, 3000, 3 * 3600), body=body(2, 1, 15, 10, 5, "aircraft"),
        sig=sig(0.4, 300, 300, (3, 8, 4, 12), 130, 0.3),
        sensors=[radar(15e3, 34, 10.0, 160000, 3.0, 40.0, 20, 3.0, 0, snr=13, bw=5, duty=0.05), irst(0.1, 60000), esm(-70, 0.5, 18, 2, 150000, "rwr"), eye(15000)],
        mounts=[mount("aam_radar_rail", 2, 4, aam_radar=0), mount("aam_ir_rail", 2, 2, aam_ir=0), mount("gun_20_air", 2, 500, he_20=0)],
        radios=[uhf(20, 0), datalink(200)], emitters=[{"kind": "radarJammer", "fminMhz": 2000, "fmaxMhz": 18000, "erpW": 500, "sectorDeg": 120, "antennaM": 0, "maxBands": 2}],
        crew={"skill": 0.75, "cohesion": 0.7}, cost=2 * 80e6),
    rec("fighter_flt_5g", "Low-observable fighter flight", "aircraft", "air", sidc="10030100001101000000", echelon="flight",
        mover=fixed(260, 550, 15000, 250, 9, 3200, 3 * 3600), body=body(2, 1, 15.7, 10.7, 4.4, "aircraft"),
        sig=sig(0.35, 250, 250, (0.005, 0.05, 0.02, 0.1), 125, 0.3),
        sensors=[radar(20e3, 36, 10.0, 180000, 2.5, 40.0, 20, 2.5, 0, snr=13, bw=10, duty=0.03), irst(0.08, 70000), esm(-80, 0.5, 18, 1, 200000, "esm"), eye(15000)],
        mounts=[mount("aam_radar_rail", 2, 4, aam_radar=0), mount("asm_bay", 2, 2, gbu_glide=0)],
        radios=[uhf(20, 0), datalink(100)], crew={"skill": 0.8, "cohesion": 0.75}, cost=2 * 100e6),
    rec("strike_flt", "Strike flight", "aircraft", "air", sidc="10030100001101000000", echelon="flight",
        mover=fixed(230, 520, 14000, 200, 7.5, 3500, 3 * 3600), body=body(2, 2, 19, 13, 5.5, "aircraft"),
        sig=sig(0.45, 320, 320, (6, 15, 8, 20), 132, 0.3),
        sensors=[radar(10e3, 33, 10.0, 120000, 3.0, 2.0, 10, 3.0, 0, snr=13, bw=5), esm(-70, 0.5, 18, 2, 150000, "rwr"), thermal(0.05, 30000, 2), eye(15000)],
        mounts=[mount("asm_rail", 2, 4, asm_heavy=0), mount("gbu_rack", 2, 4, gbu_glide=0), mount("arm_rail", 2, 2, arm=0), mount("aam_ir_rail", 2, 2, aam_ir=0)],
        radios=[uhf(20, 0), datalink(200)], emitters=[{"kind": "radarJammer", "fminMhz": 2000, "fmaxMhz": 18000, "erpW": 800, "sectorDeg": 120, "antennaM": 0, "maxBands": 2}],
        crew={"skill": 0.75, "cohesion": 0.7}, cost=2 * 90e6),
    rec("aew", "Airborne early warning aircraft", "aircraft", "air", sidc="10030100001102000000",
        mover=fixed(160, 230, 11000, 15, 2.5, 3000, 10 * 3600), body=body(1, 15, 44, 44, 13, "aircraft"),
        sig=sig(0.6, 300, 300, (50, 150, 60, 200), 135, 0.3),
        sensors=[radar(1.0e6, 40, 3.2, 450000, 10.0, 30.0, 300, 2.0, 0, snr=13, bw=2, duty=0.02), esm(-85, 0.5, 18, 1, 450000)],
        radios=[uhf(100, 0), datalink(1000), satcom_geo()], crew={"skill": 0.8, "cohesion": 0.75}, cost=500e6),
    rec("attack_helo", "Attack helicopter pair", "aircraft", "air", sidc="10030100001102000000", echelon="section",
        mover={"kind": "rotary", "cruiseMs": 70, "maxMs": 80, "ceilingM": 5000, "climbMs": 12, "maxG": 3, "enduranceS": 2.5 * 3600, "fuelKgPerH": 500, "maxWindMs": 25},
        body=body(2, 2, 17.7, 14.6, 4.6, "aircraft", armour=armour(12, 20, 12, 20, 8, 5)),
        sig=sig(0.45, 120, 120, (8, 15, 8, 20), 110, 0.5),
        sensors=[thermal(0.05, 10000, 2), optic(0.04, 12000, 2), radar(500, 30, 35.0, 8000, 1.0, 0.5, 128, 3, 2, snr=12, bw=20), esm(-65, 0.5, 18, 5, 30000, "rwr"), eye(5000)],
        mounts=[mount("asm_helo", 2, 8, asm_helo=0), mount("gun_30_air", 2, 600, he_30=0)],
        radios=[uhf(20, 0), vhf(20, 0), datalink(50)], crew={"skill": 0.7, "cohesion": 0.7}, cost=2 * 35e6),
    rec("medevac_helo", "Medical evacuation helicopter", "aircraft", "air", sidc="10030100001102000000",
        mover={"kind": "rotary", "cruiseMs": 70, "maxMs": 80, "ceilingM": 5000, "climbMs": 10, "maxG": 2.5, "enduranceS": 2.2 * 3600, "fuelKgPerH": 600, "maxWindMs": 25},
        body=body(1, 4, 19.8, 16.4, 5.1, "aircraft"), sig=sig(0.5, 120, 120, (10, 20, 10, 25), 110, 0.5),
        sensors=[eye(5000)], radios=[uhf(20, 0), vhf(20, 0)], stores={"medical": 50}, cost=20e6, note="Six litters."),
    rec("ucav", "Medium-altitude armed drone", "aircraft", "air", sidc="10030100001101000000",
        mover=fixed(55, 70, 8000, 5, 2.5, 30, 24 * 3600), body=body(1, 0, 6.5, 12, 2.2, "aircraft"),
        sig=sig(0.3, 60, 60, (0.5, 1.5, 0.5, 2), 95, 0.5),
        sensors=[thermal(0.03, 20000, 1.5), optic(0.02, 25000, 1.5), eye(500)],
        mounts=[mount("gbu_small_rack", 1, 4, gbu_small=0)], radios=[{"bearer": "satcomGeo", "freqMhz": 14000, "powerW": 20, "gainDb": 25, "antennaM": 0, "rateKbps": 4000}, dronelink(5000, 10)],
        crew={"skill": 0.65, "cohesion": 0.6}, cost=5e6),
    rec("isr_fixed_small", "Small fixed-wing ISR drone", "drone", "air", sidc="10030100001101000000",
        mover=fixed(30, 40, 4000, 4, 2.0, 1.5, 8 * 3600) | {"maxWindMs": 18}, body=body(1, 0, 2.0, 3.0, 0.5, "aircraft"),
        sig=sig(0.25, 20, 20, (0.05, 0.15, 0.05, 0.3), 80, 0.5),
        sensors=[optic(0.05, 15000, 3), thermal(0.08, 8000, 4)], radios=[dronelink(900, 5)], cost=100e3),
    rec("recon_quad", "Reconnaissance quadcopter", "drone", "air", sidc="10030100001101000000",
        mover={"kind": "multirotor", "cruiseMs": 12, "maxMs": 20, "ceilingM": 1000, "climbMs": 5, "maxG": 2, "enduranceS": 35 * 60, "maxWindMs": 12},
        body=body(1, 0, 0.35, 0.35, 0.15, "aircraft"), sig=sig(0.2, 10, 10, (0.01, 0.01, 0.01, 0.02), 70, 1.0),
        sensors=[optic(0.15, 4000, 20), thermal(0.5, 1500, 30)], radios=[dronelink(2400, 0.5)], cost=3e3),
]

# ── flying munitions: entities in flight ───────────────────────────────────
def missile(mass, burnout, thrust, burn, cd, area, max_range, cruise_alt=0, sustain=False, kind="missile", max_g=30, max_ms=0, cruise_ms=0):
    m = {"kind": kind, "massKg": mass, "burnoutMassKg": burnout, "thrustN": thrust, "burnS": burn, "cd": cd, "areaM2": area,
         "maxRangeM": max_range, "cruiseAltM": cruise_alt, "sustain": sustain, "maxG": max_g}
    if max_ms: m["maxMs"] = max_ms
    if cruise_ms: m["cruiseMs"] = cruise_ms
    return m


def mun_rec(id, name, mover, l, rcs, ir, sensors=(), radios=(), cost=0, ac=90):
    return rec(id, name, "munition", "munition", mover=mover, body=body(1, 0, l, 0.3, 0.3, "munition"),
               sig=sig(0.3, ir, ir, (rcs, rcs, rcs, rcs), ac, 0.2), sensors=sensors, radios=radios, cost=cost)


flying = [
    mun_rec("m_atgm_heavy", "Heavy ATGM in flight", missile(29, 26, 4000, 2.0, 0.5, 0.0152, 5500, 0, True, max_g=10, max_ms=300, cruise_ms=250), 1.2, 0.02, 200, cost=20e3),
    mun_rec("m_atgm_ff", "Fire-and-forget ATGM in flight", missile(11.8, 10, 2500, 1.0, 0.6, 0.0127, 4000, 150, True, max_g=12, max_ms=190, cruise_ms=150), 1.1, 0.02, 200,
            sensors=[{"kind": "thermal", "fovDeg": 3, "regardDeg": 20, "mradPerCycle": 0.15, "maxRangeM": 4000}], cost=180e3),
    mun_rec("m_sam_manpads", "MANPADS missile", missile(10.1, 7, 3500, 2.5, 0.4, 0.0049, 6000, max_g=25, max_ms=750), 1.5, 0.01, 600,
            sensors=[irst(0.5, 6000)], cost=400e3),
    mun_rec("m_sam_short", "Short-range SAM", missile(85, 50, 15000, 3.0, 0.4, 0.0314, 12000, max_g=35, max_ms=1000), 2.9, 0.03, 600, cost=500e3),
    mun_rec("m_sam_medium", "Medium-range SAM", missile(330, 180, 45000, 5.0, 0.35, 0.0707, 50000, max_g=40, max_ms=1300), 4.5, 0.05, 700,
            sensors=[radar(200, 25, 12.0, 15000, 0.05, 0.0, 1, 10, 0, snr=12, bw=5)], cost=1.5e6),
    mun_rec("m_sam_long", "Long-range SAM", missile(900, 400, 150000, 8.0, 0.3, 0.1257, 160000, max_g=30, max_ms=1700), 5.3, 0.08, 800,
            sensors=[radar(500, 28, 12.0, 30000, 0.05, 0.0, 1, 10, 0, snr=12, bw=5)], cost=4e6),
    mun_rec("m_sam_long_bmd", "Hit-to-kill BMD interceptor", missile(310, 140, 60000, 6.0, 0.3, 0.0645, 35000, max_g=50, max_ms=1600), 5.2, 0.05, 800,
            sensors=[radar(300, 28, 35.0, 20000, 0.05, 0.0, 1, 10, 0, snr=12, bw=10)], cost=5e6),
    mun_rec("m_aam_ir", "Short-range IR air-to-air missile", missile(86, 60, 12000, 3.0, 0.4, 0.0127, 20000, max_g=50, max_ms=900), 2.9, 0.02, 600,
            sensors=[irst(0.3, 15000)], cost=500e3),
    mun_rec("m_aam_radar", "Radar air-to-air missile", missile(160, 100, 20000, 6.0, 0.35, 0.0254, 100000, max_g=40, max_ms=1300), 3.7, 0.04, 600,
            sensors=[radar(150, 25, 12.0, 20000, 0.05, 0.0, 1, 10, 0, snr=12, bw=5)], cost=1.2e6),
    mun_rec("m_asm_helo", "Helicopter-launched ASM", missile(49, 40, 8000, 2.5, 0.5, 0.0314, 8000, max_g=15, max_ms=450), 1.6, 0.02, 400, cost=150e3),
    mun_rec("m_asm_heavy", "Air-to-surface missile", missile(400, 300, 30000, 4.0, 0.4, 0.09, 60000, 50, True, max_g=12, max_ms=330, cruise_ms=300), 4.0, 0.1, 600,
            sensors=[{"kind": "thermal", "fovDeg": 3, "regardDeg": 20, "mradPerCycle": 0.05, "maxRangeM": 15000}], cost=900e3),
    mun_rec("m_arm", "Anti-radiation missile", missile(360, 220, 40000, 6.0, 0.35, 0.0645, 110000, max_g=25, max_ms=900), 4.2, 0.05, 600,
            sensors=[esm(-60, 0.5, 18, 1, 110000)], cost=900e3),
    mun_rec("m_gbu_glide", "Guided glide bomb", missile(250, 250, 0, 0.1, 0.25, 0.07, 110000, 0, False, max_g=4, max_ms=300), 1.8, 0.05, 1, cost=40e3),
    mun_rec("m_gbu_small", "Small guided bomb", missile(50, 50, 0, 0.1, 0.3, 0.02, 10000, max_g=4, max_ms=250), 1.0, 0.02, 1, cost=30e3),
    mun_rec("m_cruise", "Land-attack cruise missile", missile(1500, 1100, 30000, 10.0, 0.3, 0.2, 1600000, 50, True, max_g=6, max_ms=260, cruise_ms=240), 6.0, 0.1, 400,
            cost=2e6, ac=110),
    mun_rec("m_owa", "Long-range one-way attack drone", missile(200, 200, 1500, 2.0, 0.2, 0.5, 2000000, 1500, True, max_g=3, max_ms=55, cruise_ms=50), 3.5, 0.5, 80,
            radios=[], cost=35e3, ac=105),
    mun_rec("m_srbm", "Short-range ballistic missile", missile(3800, 1100, 250000, 25.0, 0.2, 0.7, 500000, kind="ballistic", max_g=20, max_ms=2100), 7.3, 0.3, 1500,
            cost=3e6, ac=140),
    mun_rec("m_mrbm", "Medium-range ballistic missile", missile(16000, 2500, 600000, 60.0, 0.2, 1.3, 2000000, kind="ballistic", max_g=20, max_ms=4000), 15, 0.5, 2000,
            cost=10e6, ac=150),
    mun_rec("m_antiship_subsonic", "Subsonic anti-ship missile", missile(700, 520, 15000, 3.0, 0.3, 0.13, 250000, 5, True, max_g=10, max_ms=280, cruise_ms=270), 4.6, 0.1, 400,
            sensors=[radar(1000, 28, 10.0, 40000, 0.1, 0.0, 1, 15, 0, snr=12, bw=10)], cost=1.5e6),
    mun_rec("m_antiship_super", "Supersonic anti-ship missile", missile(3000, 2200, 60000, 5.0, 0.3, 0.42, 300000, 10, True, max_g=15, max_ms=800, cruise_ms=750), 8.9, 0.3, 900,
            sensors=[radar(2000, 30, 10.0, 60000, 0.1, 0.0, 1, 15, 0, snr=12, bw=10)], cost=3e6),
    mun_rec("m_fpv", "FPV drone", {"kind": "multirotor", "cruiseMs": 25, "maxMs": 40, "ceilingM": 500, "climbMs": 8, "maxG": 3, "enduranceS": 10 * 60, "maxWindMs": 12, "maxRangeM": 15000},
            0.3, 0.01, 15, sensors=[optic(0.6, 1500, 120)], radios=[dronelink(5800, 0.8)], cost=500, ac=75),
    mun_rec("m_fpv_fibre", "Fibre-optic FPV drone", {"kind": "multirotor", "cruiseMs": 22, "maxMs": 35, "ceilingM": 400, "climbMs": 6, "maxG": 3, "enduranceS": 20 * 60, "maxWindMs": 12, "maxRangeM": 20000},
            0.35, 0.01, 15, sensors=[optic(0.6, 1500, 120)], cost=1500, ac=75),
    mun_rec("m_bomber", "Heavy bomber drone", {"kind": "multirotor", "cruiseMs": 15, "maxMs": 20, "ceilingM": 500, "climbMs": 4, "maxG": 1.5, "enduranceS": 25 * 60, "maxWindMs": 10, "maxRangeM": 12000},
            1.5, 0.1, 30, sensors=[thermal(0.4, 1500, 30)], radios=[dronelink(900, 2)], cost=15e3, ac=85),
    mun_rec("m_loiter", "Loitering munition", missile(12, 12, 400, 1.0, 0.3, 0.03, 40000, 1000, True, max_g=4, max_ms=80, cruise_ms=30) | {"enduranceS": 40 * 60},
            1.6, 0.05, 30, sensors=[optic(0.1, 4000, 20), thermal(0.2, 2500, 20)], radios=[dronelink(900, 3)], cost=35e3, ac=85),
    mun_rec("m_interceptor", "Interceptor drone", {"kind": "multirotor", "cruiseMs": 50, "maxMs": 80, "ceilingM": 3000, "climbMs": 25, "maxG": 6, "enduranceS": 8 * 60, "maxWindMs": 15, "maxRangeM": 10000},
            0.5, 0.02, 20, sensors=[optic(0.3, 2000, 40)], radios=[dronelink(2400, 1)], cost=5e3, ac=80),
    mun_rec("m_rocket_gmlrs", "Guided rocket in flight", missile(300, 180, 60000, 3.0, 0.3, 0.0452, 80000, kind="ballistic", max_g=10, max_ms=1100), 4.0, 0.05, 900, cost=160e3),
    mun_rec("m_torpedo", "Heavyweight torpedo", {"kind": "sub", "maxMs": 25, "cruiseMs": 20, "maxRangeM": 40000, "maxDepthM": 500, "enduranceS": 2000},
            6.0, 0.1, 0, sensors=[sonar_a(210, 20, 10, 4000), sonar_p(15, 8, 4000)], cost=4e6),
    mun_rec("m_usv", "Explosive uncrewed surface vessel", {"kind": "surface", "maxMs": 22, "cruiseMs": 15, "maxSeaState": 4, "maxRangeM": 800000, "enduranceS": 60 * 3600},
            5.5, 2.0, 30, sensors=[optic(0.3, 5000, 40), thermal(0.4, 3000, 40)], radios=[satcom_leo()], cost=250e3, ac=130),
]
for f in flying:
    if f["id"] == "m_usv":
        f["domain"] = "surface"

# ── sea ────────────────────────────────────────────────────────────────────
def ship(max_kn, cruise_kn, sea_state=7, l_km=60):
    return {"kind": "surface", "maxMs": max_kn * KT, "cruiseMs": cruise_kn * KT, "maxSeaState": sea_state, "fuelLPerKm": l_km}


sea = [
    rec("destroyer", "Air-defence destroyer", "warship", "surface", sidc="10033000001202000000",
        mover=ship(30, 18), body=body(1, 300, 155, 20, 30, "ship", missionKillPts=30, lossPts=80, tonnes=9000),
        sig=sig(0.7, 40, 20, (5000, 20000, 6000, 30000), 165, 1.5, False, 1.0),
        sensors=[radar(4e6, 42, 3.3, 350000, 1.0, 20.0, 200, 1.7, 40, snr=13, bw=4, duty=0.03), radar(30e3, 33, 9.5, 40000, 2.0, 0.0, 0, 1.2, 30, snr=12, bw=10),
                 esm(-85, 0.5, 18, 1, 400000), sonar_a(230, 25, 12, 30000), sonar_p(25, 8, 80000), thermal(0.08, 20000, 3, 30), eye(20000)],
        mounts=[mount("vls_long", 1, 48, sam_long=0), mount("vls_medium", 1, 32, sam_medium=0), mount("vls_strike", 1, 16, cruise=0),
                mount("antiship_canister", 1, 8, antiship_subsonic=0), mount("ciws", 2, 1500, he_20=4000), mount("naval_gun_127", 1, 600, he_127=0)],
        radios=[uhf(100, 30), hf(1000), satcom_geo(), satcom_leo(), datalink(1000)],
        emitters=[{"kind": "radarJammer", "fminMhz": 2000, "fmaxMhz": 18000, "erpW": 10000, "sectorDeg": 360, "antennaM": 30, "maxBands": 3},
                  {"kind": "decoy", "fminMhz": 8000, "fmaxMhz": 12000, "erpW": 100, "sectorDeg": 360, "antennaM": 1, "maxBands": 1}],
        stores={"fuelL": 2_000_000, "foodDays": 45, "spares": 200, "medical": 200}, crew={"skill": 0.75, "cohesion": 0.75}, cost=2e9),
    rec("frigate", "General-purpose frigate", "warship", "surface", sidc="10033000001202000000",
        mover=ship(28, 16), body=body(1, 180, 140, 18, 28, "ship", missionKillPts=22, lossPts=60, tonnes=6000),
        sig=sig(0.7, 35, 18, (3000, 12000, 4000, 18000), 160, 1.5, False, 1.0),
        sensors=[radar(1e6, 40, 3.0, 250000, 2.0, 20.0, 100, 1.5, 35, snr=13, bw=4, duty=0.03), esm(-80, 0.5, 18, 1, 350000), sonar_a(225, 24, 12, 25000), sonar_p(28, 6, 100000), eye(20000)],
        mounts=[mount("vls_medium", 1, 32, sam_medium=0), mount("antiship_canister", 1, 8, antiship_subsonic=0), mount("ciws", 1, 1500, he_20=3000), mount("torpedo_tubes", 2, 2, torpedo=6)],
        radios=[uhf(100, 30), hf(1000), satcom_geo(), datalink(1000)], stores={"fuelL": 1_000_000, "foodDays": 40},
        crew={"skill": 0.7, "cohesion": 0.7}, cost=1e9),
    rec("corvette", "Missile corvette", "warship", "surface", sidc="10033000001202000000",
        mover=ship(30, 15, 6), body=body(1, 90, 90, 13, 20, "ship", missionKillPts=12, lossPts=30, tonnes=2200),
        sig=sig(0.6, 30, 15, (1500, 5000, 2000, 8000), 155, 1.5, False, 0.9),
        sensors=[radar(200e3, 36, 9.0, 150000, 3.0, 20.0, 20, 2.0, 25, snr=13, bw=4), esm(-75, 0.5, 18, 2, 250000), eye(18000)],
        mounts=[mount("antiship_launcher", 2, 4, antiship_super=0), mount("vls_short", 1, 16, sam_short=0), mount("ciws", 1, 1000, he_20=2000)],
        radios=[uhf(50, 20), hf(500), satcom_geo(), datalink(500)], stores={"fuelL": 300_000, "foodDays": 15},
        crew={"skill": 0.6, "cohesion": 0.6}, cost=300e6),
    rec("patrol_boat", "Patrol boat", "warship", "surface", sidc="10033000001202000000",
        mover=ship(35, 20, 4), body=body(1, 15, 30, 6, 8, "ship", missionKillPts=3, lossPts=6, tonnes=200),
        sig=sig(0.5, 25, 10, (100, 300, 120, 400), 145, 2.0), sensors=[radar(10e3, 30, 9.4, 40000, 2.5, 5.0, 0, 2.0, 8), thermal(0.2, 8000, 6, 8), eye(12000)],
        mounts=[mount("gun_30_naval", 1, 300, he_30=1000), mount("hmg", 2, 200, ball_127=2000)], radios=[uhf(20, 8), vhf(20, 8)],
        stores={"fuelL": 20_000}, cost=15e6),
    rec("sub_ssk", "Diesel-electric submarine", "submarine", "subsurface", sidc="10033500001101000000",
        mover={"kind": "sub", "maxMs": 20 * KT, "cruiseMs": 6 * KT, "quietMs": 4 * KT, "maxDepthM": 300, "enduranceS": 3 * 24 * 3600},
        body=body(1, 50, 70, 7, 7, "ship", missionKillPts=4, lossPts=8, tonnes=2500),
        sig=sig(0.2, 5, 5, (10, 60, 10, 80), 110, 3.0, False, 0.7),
        sensors=[sonar_p(30, 4, 80000, 300), sonar_a(220, 25, 12, 20000), esm(-80, 0.5, 18, 2, 100000), optic(0.1, 15000, 6, 10)],
        mounts=[mount("torpedo_tubes", 6, 1, torpedo=12), mount("sub_antiship", 1, 4, antiship_subsonic=0)],
        radios=[{"bearer": "hf", "freqMhz": 8, "powerW": 400, "antennaM": 8, "rateKbps": 1.2}, satcom_geo()],
        crew={"skill": 0.75, "cohesion": 0.75}, cost=500e6),
    rec("usv_group", "Explosive USV group", "usvSwarm", "surface", sidc="10033000001202000000",
        mover=ship(42, 30, 4, 5), body=body(4, 0, 5.5, 1.5, 0.8, "ship", missionKillPts=1, lossPts=1, tonnes=1),
        sig=sig(0.3, 20, 5, (1, 2, 1, 2), 130, 2.0),
        sensors=[optic(0.3, 5000, 40), thermal(0.4, 3000, 40)], mounts=[mount("usv_self", 4, 1, usv_charge=0)], radios=[satcom_leo()],
        cost=4 * 250e3, note="Each boat is its own warhead (Magura-like) [ref R6.3]."),
    rec("minehunter", "Mine countermeasures vessel", "warship", "surface", sidc="10033000001202000000",
        mover=ship(15, 10, 5), body=body(1, 40, 52, 10, 12, "ship", missionKillPts=4, lossPts=8, tonnes=700),
        sig=sig(0.5, 20, 10, (300, 800, 300, 1000), 120, 1.0, False, 0.05), sensors=[sonar_a(210, 20, 10, 2000), radar(10e3, 30, 9.4, 40000, 2.5, 5.0), eye(12000)],
        mounts=[mount("hmg", 1, 200, ball_127=2000)], radios=[uhf(20, 10), satcom_geo()], cost=150e6),
    rec("tender", "Replenishment and reload ship", "warship", "surface", sidc="10033000001202000000",
        mover=ship(20, 15, 7), body=body(1, 120, 170, 25, 30, "ship", missionKillPts=12, lossPts=40, tonnes=20000),
        sig=sig(0.7, 30, 15, (8000, 30000, 8000, 40000), 160, 1.0, False, 1.0), sensors=[radar(20e3, 30, 9.4, 60000, 2.5, 5.0), eye(15000)],
        mounts=[mount("ciws", 1, 1500, he_20=3000)], radios=[uhf(50, 25), hf(1000), satcom_geo()],
        stores={"fuelL": 20_000_000, "cargoT": 6000, "foodDays": 90}, cost=500e6, note="Can reload VLS cells at sea in a calm sea state."),
    rec("merchant", "Merchant ship", "neutral", "surface", sidc="10043000001400000000",
        mover=ship(16, 12, 8, 40), body=body(1, 20, 190, 30, 35, "ship", missionKillPts=10, lossPts=40, tonnes=40000),
        sig=sig(0.7, 25, 15, (20000, 60000, 20000, 80000), 170, 1.0, False, 1.0), sensors=[radar(25e3, 30, 9.4, 50000, 2.5, 5.0), eye(15000)],
        radios=[uhf(25, 30), satcom_leo()], cost=50e6),
    rec("port", "Naval port", "node", "node", sidc="",
        mover={"kind": "static"}, body=body(1, 500, 1500, 800, 20, "structure", missionKillPts=80, lossPts=300),
        sig=sig(0.9, 20, 10, (5000, 5000, 5000, 8000), 90, 0, False, 1.0),
        sensors=[radar(50e3, 33, 3.0, 120000, 4.0, 5.0, 40, 1.5, 40)], radios=[hf(2000), satcom_geo(), {"bearer": "wire", "freqMhz": 0, "powerW": 0, "rateKbps": 100000}],
        stores={"fuelL": 50_000_000, "cargoT": 100000, "foodDays": 180}, cost=1e9),
    rec("airbase", "Air base", "node", "node", sidc="",
        mover={"kind": "static"}, body=body(1, 2000, 3000, 1000, 15, "structure", missionKillPts=60, lossPts=250),
        sig=sig(0.9, 20, 10, (5000, 5000, 5000, 8000), 120, 0, False, 1.0),
        sensors=[radar(100e3, 35, 2.8, 200000, 5.0, 10.0, 100, 1.5, 15)], radios=[uhf(200, 20), hf(2000), satcom_geo(), datalink(1000), {"bearer": "wire", "freqMhz": 0, "powerW": 0, "rateKbps": 100000}],
        stores={"fuelL": 20_000_000, "cargoT": 10000, "foodDays": 90}, cost=2e9),
]

# ── space ──────────────────────────────────────────────────────────────────
def orbit():
    return {"kind": "orbit"}


space = [
    rec("sat_eo", "Electro-optical imaging satellite", "satellite", "space", sidc="10030500001101000000",
        mover=orbit(), body=body(1, 0, 4, 2, 2, "satellite"), sig=sig(0.5, 20, 20, (2, 2, 2, 2), 0),
        sensors=[{"kind": "optic", "fovDeg": 1.5, "regardDeg": 60, "mradPerCycle": 0.001, "maxRangeM": 1500000, "gsdM": 0.5, "swathKm": 15}],
        radios=[{"bearer": "satcomGeo", "freqMhz": 8100, "powerW": 20, "gainDb": 10, "rateKbps": 800000}], cost=500e6),
    rec("sat_sar", "Radar imaging satellite", "satellite", "space", sidc="10030500001101000000",
        mover=orbit(), body=body(1, 0, 5, 3, 1, "satellite"), sig=sig(0.5, 20, 20, (3, 3, 3, 3), 0),
        sensors=[{"kind": "sar", "fovDeg": 3, "regardDeg": 60, "maxRangeM": 1500000, "gsdM": 1.0, "swathKm": 30, "peakW": 5000, "gainDb": 45, "freqGhz": 9.6}],
        radios=[{"bearer": "satcomGeo", "freqMhz": 8100, "powerW": 20, "gainDb": 10, "rateKbps": 500000}], cost=300e6),
    rec("sat_sigint", "Signals intelligence satellite", "satellite", "space", sidc="10030500001101000000",
        mover=orbit(), body=body(1, 0, 6, 4, 4, "satellite"), sig=sig(0.5, 20, 20, (5, 5, 5, 5), 0),
        sensors=[esm(-120, 0.1, 18, 0.5, 3000000)], cost=800e6),
    rec("sat_comms_geo", "Military communications satellite (GEO)", "satellite", "space", sidc="10030500001101000000",
        mover=orbit(), body=body(1, 0, 7, 30, 4, "satellite"), sig=sig(0.5, 20, 20, (20, 20, 20, 20), 0),
        radios=[{"bearer": "satcomGeo", "freqMhz": 8000, "powerW": 2000, "gainDb": 35, "rateKbps": 500000}], cost=1e9),
    rec("sat_leo_comms", "Commercial LEO broadband satellite", "satellite", "space", sidc="10030500001101000000",
        mover=orbit(), body=body(1, 0, 3, 10, 0.5, "satellite"), sig=sig(0.5, 20, 20, (1, 1, 1, 1), 0),
        radios=[{"bearer": "satcomLeo", "freqMhz": 12000, "powerW": 200, "gainDb": 35, "rateKbps": 20000000}], cost=1e6),
    rec("sat_gnss", "Navigation satellite", "satellite", "space", sidc="10030500001101000000",
        mover=orbit(), body=body(1, 0, 3, 18, 3, "satellite"), sig=sig(0.5, 20, 20, (5, 5, 5, 5), 0), cost=500e6),
    rec("sat_ew", "Missile warning satellite (GEO)", "satellite", "space", sidc="10030500001101000000",
        mover=orbit(), body=body(1, 0, 5, 10, 3, "satellite"), sig=sig(0.5, 20, 20, (5, 5, 5, 5), 0),
        sensors=[{"kind": "irst", "fovDeg": 20, "regardDeg": 20, "mradPerCycle": 0.02, "maxRangeM": 40000000}], cost=1.5e9),
]

# ── weapons ────────────────────────────────────────────────────────────────
def fc(rng=0.002, bias=0.25, disp=0.2, stab=True, lead=0.15):
    return {"rangeErrFrac": rng, "biasMrad": bias, "dispersionMrad": disp, "stabilised": stab, "leadErrFrac": lead}


def wpn(id, name, kind, munitions, rpm, mx, mn=0, muzzle=0, fire=None, reload=0, emplace=0, needs_track=False, on_move=True, anti_air=False, burst=1):
    w = {"id": id, "name": name, "kind": kind, "munitions": munitions, "rpm": rpm, "maxRangeM": mx, "minRangeM": mn, "burst": burst,
         "reloadS": reload, "emplaceS": emplace, "needsTrack": needs_track, "onMove": on_move, "antiAir": anti_air}
    if muzzle: w["muzzleMs"] = muzzle
    if fire: w["fireControl"] = fire
    return w


weapons = [
    wpn("gun_120", "120 mm smoothbore", "gun", ["apfsds_120", "heat_120"], 6, 4000, 50, 1650, fc(0.002, 0.25, 0.20, True, 0.12)),
    wpn("gun_125", "125 mm smoothbore with autoloader", "gun", ["apfsds_125", "heat_125", "atgm_gl"], 7, 4000, 50, 1700, fc(0.003, 0.35, 0.28, True, 0.18)),
    wpn("gun_30", "30 mm autocannon", "gun", ["apfsds_30", "he_30"], 200, 2500, 0, 1100, fc(0.003, 0.4, 0.6, True, 0.2), burst=6),
    wpn("gun_20_air", "20 mm aircraft cannon", "gun", ["he_20"], 3000, 1500, 0, 1000, fc(0.01, 1.0, 2.0, True, 0.1), burst=100),
    wpn("gun_30_air", "30 mm helicopter chain gun", "gun", ["he_30"], 600, 3000, 0, 800, fc(0.01, 1.0, 2.5, True, 0.2), burst=20),
    wpn("gun_30_naval", "30 mm naval gun", "gun", ["he_30"], 200, 3000, 0, 1000, fc(0.01, 1.0, 1.5, True, 0.2), burst=10),
    wpn("mg_coax", "7.62 mm coaxial machine gun", "smallArms", ["ball_762"], 600, 1100, 0, 830, fc(0.05, 2.0, 2.0, True, 0.3), burst=30),
    wpn("hmg", "12.7 mm heavy machine gun", "smallArms", ["ball_127"], 500, 1500, 0, 890, fc(0.05, 2.0, 2.5, False, 0.3), burst=20),
    wpn("rifles", "Section rifles", "smallArms", ["ball_556"], 60, 500, 0, 920, fc(0.1, 3.0, 3.0, False, 0.4), burst=20),
    wpn("lmg", "Light machine gun", "smallArms", ["ball_556"], 500, 800, 0, 920, fc(0.1, 3.0, 4.0, False, 0.4), burst=30),
    wpn("rpg", "Shoulder-fired rocket launcher", "gun", ["heat_rpg"], 4, 500, 20, 300, fc(0.1, 2.0, 2.5, False, 0.4), on_move=False),
    wpn("atgm_ff_launcher", "Fire-and-forget ATGM launcher", "atgm", ["atgm_ff"], 2, 4000, 65, fire=fc(0.002, 0, 0, False, 0), reload=20, on_move=False),
    wpn("atgm_cmd_launcher", "Command-guided ATGM launcher", "atgm", ["atgm_heavy"], 2, 5500, 100, fire=fc(0.002, 0, 0, False, 0), reload=30, on_move=False),
    wpn("atgm_vehicle", "Vehicle ATGM launcher", "atgm", ["atgm_heavy"], 2, 5500, 100, fire=fc(0.002, 0, 0, False, 0), reload=60, on_move=False),
    wpn("mortar_120", "120 mm mortar", "mortar", ["he_120m", "smoke_120m"], 6, 8000, 500, 320, fc(0.01, 3.0, 4.0, False, 0), emplace=60, on_move=False),
    wpn("how_155", "155 mm 52-calibre howitzer", "howitzer", ["he_155", "he_155_erfb", "gnss_155", "dpicm_155", "smoke_155"], 4, 40000, 3000, 900,
        fc(0.0, 1.5, 2.5, False, 0), emplace=60, on_move=False),
    wpn("how_152", "152 mm towed howitzer", "howitzer", ["he_152", "he_152_rap"], 3, 28000, 3000, 750, fc(0.0, 2.0, 3.0, False, 0), emplace=240, on_move=False),
    wpn("mlrs_launcher", "Guided rocket launcher", "rocket", ["gmlrs"], 12, 80000, 15000, fire=fc(0, 0, 0, False, 0), reload=600, emplace=60, on_move=False),
    wpn("line_charge", "Mine-clearing line charge", "gun", ["mclc"], 0.2, 150, 50, fire=fc(0.05, 0, 0, False, 0), on_move=False),
    wpn("aa_gun_35", "35 mm twin AA gun", "aaGun", ["he_35_prox"], 1100, 4000, 0, 1175, fc(0.002, 0.5, 1.2, True, 0.05), needs_track=True, anti_air=True, burst=30),
    wpn("ciws", "Close-in weapon system", "aaGun", ["he_20"], 4500, 2000, 0, 1100, fc(0.001, 0.3, 1.5, True, 0.03), needs_track=True, anti_air=True, burst=200),
    wpn("sam_short_launcher", "Short-range SAM launcher", "samShort", ["sam_short"], 6, 12000, 500, reload=300, needs_track=True, anti_air=True),
    wpn("manpads", "MANPADS", "samShort", ["sam_manpads"], 2, 6000, 500, reload=30, anti_air=True, on_move=False),
    wpn("sam_medium_launcher", "Medium-range SAM launcher", "samMedium", ["sam_medium"], 6, 50000, 2500, reload=900, emplace=300, needs_track=True, anti_air=True, on_move=False),
    wpn("sam_long_launcher", "Long-range SAM launcher", "samLong", ["sam_long", "sam_long_bmd"], 4, 160000, 3000, reload=1800, emplace=900, needs_track=True, anti_air=True, on_move=False),
    wpn("vls_short", "Short-range naval SAM cells", "samShort", ["sam_short"], 20, 12000, 500, needs_track=True, anti_air=True),
    wpn("vls_medium", "Medium-range naval SAM cells", "samMedium", ["sam_medium"], 20, 50000, 2000, needs_track=True, anti_air=True),
    wpn("vls_long", "Long-range naval SAM cells", "samLong", ["sam_long", "sam_long_bmd"], 12, 160000, 3000, needs_track=True, anti_air=True),
    wpn("vls_strike", "Strike cells", "cruise", ["cruise"], 4, 1600000, 20000),
    wpn("naval_gun_127", "127 mm naval gun", "howitzer", ["he_127"], 16, 24000, 500, 810, fc(0.0, 1.0, 2.0, True, 0)),
    wpn("antiship_canister", "Anti-ship missile canisters", "antiShip", ["antiship_subsonic"], 4, 250000, 10000),
    wpn("antiship_launcher", "Anti-ship missile launcher", "antiShip", ["antiship_subsonic", "antiship_super"], 4, 300000, 10000, reload=1800, on_move=False),
    wpn("sub_antiship", "Submarine-launched anti-ship missile", "antiShip", ["antiship_subsonic"], 1, 250000, 10000),
    wpn("torpedo_tubes", "Torpedo tubes", "torpedo", ["torpedo"], 2, 40000, 500, reload=600),
    wpn("aam_radar_rail", "Radar AAM rail", "aam", ["aam_radar"], 6, 100000, 1000, needs_track=True, anti_air=True),
    wpn("aam_ir_rail", "IR AAM rail", "aam", ["aam_ir"], 6, 20000, 300, anti_air=True),
    wpn("asm_rail", "ASM rail", "asm", ["asm_heavy"], 4, 60000, 2000),
    wpn("asm_helo", "Helicopter ASM rail", "asm", ["asm_helo"], 6, 8000, 500),
    wpn("asm_bay", "Internal weapons bay (ground attack)", "bomb", ["gbu_glide"], 4, 110000, 3000),
    wpn("gbu_rack", "Guided bomb rack", "bomb", ["gbu_glide"], 4, 110000, 3000),
    wpn("gbu_small_rack", "Small guided bomb rack", "bomb", ["gbu_small"], 2, 10000, 500),
    wpn("arm_rail", "Anti-radiation missile rail", "asm", ["arm"], 2, 110000, 5000),
    wpn("fpv_launch", "FPV drones (RF)", "drone", ["fpv_drone"], 2, 15000, 100, reload=60, on_move=False),
    wpn("fpv_launch_fibre", "FPV drones (fibre)", "drone", ["fpv_fibre"], 1, 20000, 100, reload=120, on_move=False),
    wpn("bomber_launch", "Heavy bomber drones", "drone", ["bomber_drone"], 1, 12000, 200, reload=300, on_move=False),
    wpn("loiter_launch", "Loitering munition launcher", "drone", ["loiter_munition"], 1, 40000, 1000, reload=300, on_move=False),
    wpn("interceptor_rack", "Interceptor drones", "interceptor", ["interceptor_drone"], 4, 10000, 100, reload=30, anti_air=True, on_move=False),
    wpn("shotgun", "Anti-drone shotgun", "smallArms", ["ball_shot"], 30, 60, 0, 400, fc(0.05, 3, 25, False, 0.2), anti_air=True, burst=2),
    wpn("strike_srbm", "Ballistic missile launcher", "ballistic", ["srbm"], 1, 500000, 50000, reload=3600, emplace=900, on_move=False),
    wpn("strike_mrbm", "Medium-range ballistic missile launcher", "ballistic", ["mrbm"], 1, 2000000, 300000, reload=7200, emplace=1800, on_move=False),
    wpn("owa_launch", "One-way attack drone launcher", "drone", ["owa"], 6, 2000000, 20000, reload=600, on_move=False),
    wpn("cruise_launcher", "Ground cruise missile launcher", "cruise", ["cruise"], 2, 1600000, 20000, reload=1800, emplace=600, on_move=False),
    wpn("usv_self", "The boat itself", "antiShip", ["usv_charge"], 1, 800000, 0),
    wpn("mine_layer", "Mine laying", "mine", ["mine_at", "mine_naval"], 30, 100, 0, on_move=True),
]

# ── munitions ──────────────────────────────────────────────────────────────
def mu(id, name, warhead, guidance="none", **kw):
    m = {"id": id, "name": name, "warhead": warhead, "guidance": guidance}
    m.update(kw)
    return m


munitions = [
    mu("apfsds_120", "120 mm APFSDS", "ke", penMm=650, penLossPerKm=35, costUsd=10e3),
    mu("heat_120", "120 mm HEAT-MP", "heat", penMm=480, lethalAreaM2=60, heKg=2.0, costUsd=7e3),
    mu("apfsds_125", "125 mm APFSDS", "ke", penMm=560, penLossPerKm=40, costUsd=6e3),
    mu("heat_125", "125 mm HEAT", "heat", penMm=500, lethalAreaM2=60, heKg=2.0, costUsd=4e3),
    mu("atgm_gl", "125 mm gun-launched ATGM", "tandem", "command", penMm=750, costUsd=60e3, cepM=0.6),
    mu("apfsds_30", "30 mm APFSDS-T", "ke", penMm=110, penLossPerKm=20, costUsd=80),
    mu("he_30", "30 mm HE-T", "he", lethalAreaM2=12, heKg=0.05, costUsd=50, damagePts=0.05),
    mu("he_20", "20 mm HE", "he", lethalAreaM2=6, heKg=0.01, costUsd=30, proxRadiusM=1, damagePts=0.02),
    mu("he_35_prox", "35 mm programmable airburst", "blastFrag", proxRadiusM=4, lethalAreaM2=20, heKg=0.1, costUsd=500),
    mu("ball_762", "7.62 mm ball", "ball", penMm=8, costUsd=1),
    mu("ball_127", "12.7 mm ball/AP", "ball", penMm=25, penLossPerKm=8, costUsd=3),
    mu("ball_556", "5.56 mm ball", "ball", penMm=5, costUsd=0.5),
    mu("ball_shot", "12-gauge anti-drone shot", "ball", penMm=0.5, costUsd=2, proxRadiusM=1.5),
    mu("heat_rpg", "RPG tandem HEAT rocket", "tandem", penMm=600, lethalAreaM2=15, heKg=1.0, costUsd=1500, reliability=0.9),
    mu("atgm_ff", "Fire-and-forget top-attack ATGM", "tandem", "imaging", penMm=750, topAttack=True, cepM=0.4, fliesAs="m_atgm_ff", costUsd=180e3, reliability=0.93),
    mu("atgm_heavy", "Heavy command-guided ATGM", "tandem", "command", penMm=1100, cepM=0.5, fliesAs="m_atgm_heavy", costUsd=20e3, reliability=0.92),
    mu("he_120m", "120 mm mortar HE", "he", lethalAreaM2=500, heKg=2.3, costUsd=1200, reliability=0.96),
    mu("smoke_120m", "120 mm mortar smoke", "he", lethalAreaM2=0, heKg=0, costUsd=1500),
    mu("he_155", "155 mm HE", "he", lethalAreaM2=900, heKg=10.8, costUsd=3000, reliability=0.96, damagePts=0.3),
    mu("he_155_erfb", "155 mm HE extended range", "he", lethalAreaM2=800, heKg=9.0, costUsd=6000, reliability=0.95, damagePts=0.3),
    mu("gnss_155", "155 mm GNSS-guided", "he", "gnssIns", lethalAreaM2=700, heKg=10.0, cepM=4, insDriftMps=4.0, costUsd=100e3, reliability=0.95, damagePts=0.4),
    mu("dpicm_155", "155 mm cluster (DPICM)", "submunition", lethalAreaM2=3000, heKg=8.0, costUsd=5000, reliability=0.9),
    mu("smoke_155", "155 mm smoke", "he", lethalAreaM2=0, costUsd=2000),
    mu("he_152", "152 mm HE", "he", lethalAreaM2=800, heKg=7.0, costUsd=1500, reliability=0.92, damagePts=0.25),
    mu("he_152_rap", "152 mm rocket-assisted HE", "he", lethalAreaM2=700, heKg=6.0, costUsd=4000, reliability=0.9, damagePts=0.25),
    mu("gmlrs", "Guided rocket, unitary", "he", "gnssIns", lethalAreaM2=2000, heKg=90, cepM=5, insDriftMps=3.0, fliesAs="m_rocket_gmlrs", costUsd=160e3, damagePts=2),
    mu("he_127", "127 mm naval HE", "he", lethalAreaM2=500, heKg=3.6, costUsd=5000, damagePts=0.4),
    mu("mclc", "Line charge", "he", heKg=800, costUsd=60e3),
    mu("sam_manpads", "MANPADS missile", "blastFrag", "imaging", proxRadiusM=1.5, cepM=1.0, fliesAs="m_sam_manpads", costUsd=400e3, reliability=0.9),
    mu("sam_short", "Short-range SAM", "blastFrag", "command", proxRadiusM=6, cepM=2, fliesAs="m_sam_short", costUsd=500e3),
    mu("sam_medium", "Medium-range SAM", "blastFrag", "activeRadar", proxRadiusM=10, cepM=3, fliesAs="m_sam_medium", costUsd=1.5e6),
    mu("sam_long", "Long-range SAM", "blastFrag", "semiActiveRadar", proxRadiusM=15, cepM=5, fliesAs="m_sam_long", costUsd=4e6),
    mu("sam_long_bmd", "Hit-to-kill BMD interceptor", "hitToKill", "activeRadar", proxRadiusM=0.5, cepM=0.3, fliesAs="m_sam_long_bmd", costUsd=5e6),
    mu("aam_ir", "IR AAM", "blastFrag", "imaging", proxRadiusM=5, cepM=2, fliesAs="m_aam_ir", costUsd=500e3),
    mu("aam_radar", "Radar AAM", "blastFrag", "activeRadar", proxRadiusM=8, cepM=3, fliesAs="m_aam_radar", costUsd=1.2e6),
    mu("asm_helo", "Helicopter ASM", "tandem", "laserSemiActive", penMm=1000, cepM=0.8, fliesAs="m_asm_helo", topAttack=True, costUsd=150e3),
    mu("asm_heavy", "Air-to-surface missile", "he", "imaging", heKg=100, lethalAreaM2=2500, cepM=2, penMm=600, fliesAs="m_asm_heavy", costUsd=900e3, damagePts=4),
    mu("arm", "Anti-radiation missile", "blastFrag", "antiRadiation", heKg=66, lethalAreaM2=1500, cepM=7, fliesAs="m_arm", costUsd=900e3, damagePts=2),
    mu("gbu_glide", "Guided glide bomb", "he", "gnssIns", heKg=100, lethalAreaM2=3000, cepM=5, insDriftMps=1.5, fliesAs="m_gbu_glide", costUsd=40e3, damagePts=5),
    mu("gbu_small", "Small guided bomb", "he", "laserSemiActive", heKg=10, lethalAreaM2=600, cepM=1.5, penMm=300, fliesAs="m_gbu_small", topAttack=True, costUsd=30e3, damagePts=1),
    mu("cruise", "Land-attack cruise missile", "he", "gnssIns", heKg=450, lethalAreaM2=8000, cepM=5, insDriftMps=0.5, fliesAs="m_cruise", costUsd=2e6, damagePts=12),
    mu("owa", "One-way attack drone warhead", "he", "gnssIns", heKg=50, lethalAreaM2=1500, cepM=10, insDriftMps=1.0, fliesAs="m_owa", costUsd=35e3, reliability=0.9, damagePts=4),
    mu("srbm", "SRBM warhead", "he", "gnssIns", heKg=480, lethalAreaM2=10000, cepM=10, insDriftMps=0.5, fliesAs="m_srbm", costUsd=3e6, damagePts=15),
    mu("mrbm", "MRBM warhead", "he", "ins", heKg=750, lethalAreaM2=15000, cepM=50, insDriftMps=0.2, fliesAs="m_mrbm", costUsd=10e6, damagePts=20),
    mu("antiship_subsonic", "Subsonic anti-ship missile", "he", "activeRadar", heKg=220, cepM=2, fliesAs="m_antiship_subsonic", costUsd=1.5e6, damagePts=12),
    mu("antiship_super", "Supersonic anti-ship missile", "he", "activeRadar", heKg=300, cepM=3, fliesAs="m_antiship_super", costUsd=3e6, damagePts=18),
    mu("torpedo", "Heavyweight torpedo", "he", "acoustic", heKg=250, cepM=3, fliesAs="m_torpedo", costUsd=4e6, damagePts=40),
    mu("usv_charge", "USV explosive charge", "he", "rfOperator", heKg=300, cepM=2, fliesAs="m_usv", costUsd=250e3, damagePts=15, reliability=0.85),
    mu("fpv_drone", "FPV drone with RPG warhead", "heat", "rfOperator", penMm=400, lethalAreaM2=20, heKg=1.0, cepM=0.8, fliesAs="m_fpv", costUsd=500, reliability=0.85),
    mu("fpv_fibre", "Fibre-optic FPV drone", "heat", "fibre", penMm=400, lethalAreaM2=20, heKg=1.0, cepM=0.8, fliesAs="m_fpv_fibre", costUsd=1500, reliability=0.85),
    mu("bomber_drone", "Bomber drone (drops)", "heat", "rfOperator", penMm=150, lethalAreaM2=60, heKg=2.5, cepM=2, fliesAs="m_bomber", topAttack=True, costUsd=15e3, reliability=0.8),
    mu("loiter_munition", "Loitering munition", "heat", "rfOperator", penMm=300, lethalAreaM2=60, heKg=3.0, cepM=1.0, fliesAs="m_loiter", topAttack=True, costUsd=35e3, reliability=0.85),
    mu("interceptor_drone", "Interceptor drone", "blastFrag", "rfOperator", proxRadiusM=3, cepM=2, fliesAs="m_interceptor", costUsd=5e3, reliability=0.85),
    mu("mine_at", "Anti-tank mine", "heat", penMm=300, heKg=6, costUsd=500, reliability=0.9),
    mu("mine_naval", "Influence sea mine", "he", heKg=300, costUsd=50e3, damagePts=15),
]

if __name__ == "__main__":
    write("land.json", land)
    write("air.json", air)
    write("sea.json", sea)
    write("space.json", space)
    write("munitions-flying.json", flying)
    write("weapons.json", weapons)
    write("munitions.json", munitions)
    print(f"{len(land)} land, {len(air)} air, {len(sea)} sea, {len(space)} space, {len(flying)} flying, {len(weapons)} weapons, {len(munitions)} munitions")
