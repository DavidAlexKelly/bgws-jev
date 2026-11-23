# IR & EO Systems Handbook, vol. 2 (1993): Atmospheric Propagation of Radiation

**Source:** Frederick G. Smith (ed.), *Atmospheric Propagation of
Radiation*, vol. 2 of *The Infrared & Electro-Optical Systems Handbook*,
J. S. Accetta and D. L. Shumaker (executive eds.), ERIM and SPIE, 1993.

**File:** `raw/sensing/irhb-1993-v2-atmospheric-propagation.pdf` (333 PDF
pages; text layer, but tables and graphs come through poorly). Read on
23 November 2026: chapter 1 (Thomas and Duncan, "Atmospheric
Transmission"), the sections on aerosols, fog and visibility. Printed
page = PDF page − 13.

## What it is

The physics of absorption, scattering, turbulence and refraction
between source and sensor, and the codes that compute them (LOWTRAN,
MODTRAN, FASCODE). Most of the quantitative content is in graphs of
transmittance against wavelength, which do not reduce to a few figures.

## What we use

**Visual range** (§1.4.5.1, p. 123 [136]): R_v = (1/β_sca) ln(C/ε), where
C is the target's inherent contrast and ε the observer's threshold. The
**meteorological range** is the same for a black target (C = 1) and a 2%
threshold: R_m = 3.912/β_sca. This is the definition behind every
"visibility in km" figure, including the world layer's
`env.visibilityKm`.

**LOWTRAN default visibilities** (Table 1.35, p. 129 [142]):

| Aerosol model | Visibility |
|---|---|
| Rural, clear | 23 km |
| Rural, hazy | 5 km |
| Maritime | 23 km |
| Urban | 5 km |
| Tropospheric | 50 km |
| Advection fog | **0.2 km** |
| Radiation fog | **0.5 km** |

The world layer's weather presets (`world/environment.ts`) sit inside
these: clear 20 km, overcast 12 km, rain 5 km, snow 2 km, fog 0.4 km.
They are consistent and need no change.

**Fog and thermal sights** (Table 1.5, p. 12 [25]; Table 1.31, p. 109
[122]): fog droplets have radii of **1–10 µm**, the same size as the 8–12
µm band. So fog scatters long-wave infrared strongly, unlike haze, whose
particles are under 1 µm. Table 1.31 gives the extinction of heavy
advective and moderate radiative fog. The OCR of its columns is too
garbled to take numbers from without the page image.

## Not applied, and why

- **Thermal sights in fog.** The world layer gives thermal sensors an
  unlimited range in fog (`sensorConditions`: `maxRangeM: Infinity` for
  thermal). By this volume, that is too generous for thick fog. The fix
  needs an LWIR extinction for fog, which is in Table 1.31 but not
  legible in the text layer. It is listed under open items.
- **`haze_ratio`** (the Rust engine's visible-haze figure): this volume
  defines visibility and gives the LOWTRAN haze defaults, but has no
  single "haze ratio". The Rust figure still needs its own definition
  before a source can be matched to it.
