# Naked-eye and binocular search: leads, not yet read in the original

> **Status, 23 November 2026.**
>
> - **Read:** Maxey & Caviness (1972) and the full report, Caviness, Maxey & McPherson (1972)
>   (`notes/caviness-1972-target-detection-and-range-estimation.md`). Camouflaged men on foot at
>   100–300 m were found in 2–7 s on average. Only 54% were found in thick cover, and none at
>   300 m there. The land engine is 3–5 times faster and has no ceiling, so this is now a
>   calibration target.
> - **Still to get:**
>   - Grau (2022);
>   - Louis (1964, AD0445050);
>   - Richardson (1968);
>   - ARI TR-79-A13 (ADA077862);
>   - the TETAM Phase IB companion paper.
>
>   The Grau tables and ARI TR-79-A13 would give P∞ for tanks.

Howe (1993) says the NVESD search model "is not applicable to search with
the unaided eye, or even with optically aided eyes (using binoculars)".
Most infantry search that way.

A web search on 21 November 2026 found the leads below. Each was seen
**only in search snippets**: the hosts holding the documents refused this
environment (apps.dtic.mil, archive.org, media.defense.gov,
benning.army.mil). **None of these numbers is used in the model.** To use
one, download it in a browser and add it to `docs/sources/raw/`.

| Lead | What the snippets say | Would mean |
|---|---|---|
| L. W. Grau, "Polar Nights, White Nights, and Normal Days and Nights: Arctic Ground Target Identification and Engagement", *Journal of Indo-Pacific Affairs*, Sep–Oct 2022; reprinted in *Infantry*, Spring 2023 (media.defense.gov/2022/Sep/28/2003087082/-1/-1/1/01%20GRAU_COMMENTARY.PDF; benning.army.mil/infantry/magazine/issues/2023/Spring/pdf/Grau-Polar_8.pdf) | Soviet daylight trials on the Kola peninsula, with and without binoculars. "The unaided eye could detect a tank in a forest 2.5 kilometers away in 7 seconds." The table snippets give: a tank against forest found in 3 s at 500 m and 7 s at 2,500 m; a tank on a grass field in 9 s at 500 m and not at 2,500 m; a whitewashed tank in snowy forest, not at all, or in 44–51 s. ATGM and machine-gun crews seen without optics up to 900 m. | τ of about 3–7 s with P∞ near 1 for a tank against forest at 0.5–2.5 km, the same order as Howe's 3.4/P∞. About 45–50 s for a camouflaged target. |
| N. B. Louis, *The Effects of Observer Location and View Method on Target Detection*, 1964, DTIC AD0445050 | For the first minute of search the unaided eye was as effective as binoculars; over a longer search, binoculars did better. | Binoculars mainly raise P∞ (late or long-range finds), not the early rate. |
| Richardson, 1968 (Combat Developments Command, ground observers) | With supported binoculars, detectability was greater at all ranges; background contrast had a strong effect. | Binoculars raise P∞. |
| Maxey & Caviness, *Target Detection in the Field*, HumRRO Professional Paper 11-72, 1972 | Unaided and aided viewing of personnel and vehicles. | The likeliest primary source of field numbers. |
| ARI TR-79-A13 (DTIC ADA077862) | A stationary tank "will not generally be visible at ranges greater than 2000 meters" except on very open terrain. | A check on the eye's P∞ against range. |
| Poe, 1974, US Army Missile R&D Lab RD-75-30 (DTIC ADA017599) | "Average glimpse rate is in the neighborhood of 3/sec." | About 0.33 s per fixation, as Howe's t_f ≈ 0.3 s. |
| The TETAM Phase IB companion paper (same symposium as Bryson) | Players called "DETECT" on advancing vehicles, "which allows the calculation of time to detect given an opportunity". | Field times for the naked eye and binoculars; not yet found. |

**What the model does now:** eyes and binoculars use the same search
model as sights, through their resolution in the catalogue. If Grau's
figures hold, that is about right for tanks against forest. The open
question is P∞ for small or camouflaged targets.
