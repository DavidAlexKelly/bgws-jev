# Routing raster, format 2: ground, slope, and links

Replaces the routing-chunk build so that **rivers and roads never compete
for pixels**. Drop-in for the Foundry transform: same cells, same 50 m
pixels, same `terrain.bin` and `slope.bin`, plus one new plane, `links.bin`.

## The problem it fixes

Format 1 kept one class per 50 m pixel. Rivers (5–30 m) and roads (5–10 m)
are both thinner than that, so where they ran side by side they landed in the
same pixels and one had to lose:

- roads were painted last, so every road pixel beside a river punched a hole
  in it, and the pathfinder walked across;
- water was dilated to keep thin rivers continuous, which made rivers 150 m
  wide in the raster;
- `_clear_tiny_components` removed short river pieces that were real.

## The three planes

| file | per pixel | holds |
|---|---|---|
| `terrain.bin` | uint8 class | the ground: open, rough, very rough, building, water **areas**, nodata. No roads. No thin rivers. |
| `slope.bin` | uint8 | unchanged |
| `links.bin` | uint8 bits | the moves **between** pixels: river walls and road links |

`links.bin` bits, geographic, stored on the pixel the move leaves going east
or north (so each move is stored exactly once):

| bit | name | meaning |
|---|---|---|
| `0x01` | `WALL_E` | a river crosses the move to the east neighbour |
| `0x02` | `WALL_N` | a river crosses the move to the north neighbour |
| `0x04` | `ROAD_E` | road link to the east neighbour |
| `0x08` | `ROAD_N` | road link to the north neighbour |
| `0x10` | `ROAD_NE` | road link to the north-east neighbour |
| `0x20` | `ROAD_NW` | road link to the north-west neighbour |

## The move rules (client)

1. **Off-road, orthogonal:** blocked by a wall on that move.
2. **Off-road, diagonal:** blocked when *both* right-angle routes round the
   corner are walled. That is exactly when the walls separate the two pixels,
   so a one-pixel diagonal river has no corner gaps.
3. **By road:** follow a road link. Ignores walls and the terrain class, which
   is how a bridge crosses water and a road crosses a DEM gap.
4. Off-road moves still use the terrain plane for the ground they enter.

`client/links.ts` implements these against any chunk loader
(`LinksAt = (x, y) => byte`, global pixel coordinates, y north), so moves
across a chunk seam need nothing special.

## How the builder guarantees "roads don't break, rivers don't leak"

`myproject/terrain/links.py`:

- **Walls** are traced from the river's own geometry: every
  centre-to-centre move a waterway centreline or water-area outline crosses
  gets a wall. They're watertight at any pixel size and need no width.
- **Real crossings** are found geometrically: road ∩ water. Only a road link
  within 1.5 px of one may cross a wall. Bridge/ford tags aren't needed;
  untagged crossings in the data are bridges or culverts in reality.
- **Road vertices** snap to the nearest pixel centre *on the same side of the
  water* (checked against the geometry, out to 3 px).
- **Roads** are walked sample by sample (every half pixel). A step blocked
  by a wall away from a real crossing is replaced by the open step closest
  to the road; at each vertex a short search reconnects the chain. So a road
  hugging a bank stays on its bank and stays continuous.
- If geometry narrower than a pixel leaves no route (a sharp tongue of water
  with no pixel centre on the road's side), the road is kept continuous and
  the segment is counted as **`forced`**, which is logged per cell. On
  realistic rivers this is zero; it only appeared on deliberately extreme
  saw-tooth rivers in the stress test.

Walls and roads are traced over the cell **plus a 4-pixel margin**, and
features are read over that margin (`build.read_bounds`), so two neighbouring
cells trace the same geometry the same way and their roads meet at the seam.
That needs the cell's PMTiles archive to include features a few pixels past
the cell edge (vector tiles normally carry a buffer). If yours are clipped
exactly at the cell edge, roads crossing a seam can be off by a pixel there.

## Files

```
routing_raster/
  myproject/terrain/links.py         walls, crossings, snapping, road tracing (new)
  myproject/terrain/build.py         build_cell: the three planes (replaces build.py)
  myproject/datasets/raster_chunks.py the transform (replaces your existing file)
  client/links.ts                    move rules for the pathfinder
  client/links.test.ts               vitest, on bytes written by the Python builder
  tests/                             pytest: watertightness, continuity, end to end
```

`classes.py`, `grid.py` and `vector.py` are unchanged and not included. The
tests stand in for them when they're absent.

## Integrating

1. Copy `links.py` and `build.py` into `myproject/terrain/`, and replace your
   transform with `raster_chunks.py`. It writes to a **new dataset**,
   `[DK] Rasters v2`, because format-1 clients read roads from
   `terrain.bin` and format 2 no longer paints them there. Keep the old
   dataset until the client has switched.
2. In the pathfinder (`shared/routing`): load `links.bin` alongside the other
   planes and generate neighbours with `moveKind` from `client/links.ts`:
   - `"road"`: road cost; no terrain check;
   - `"offRoad"`: the existing terrain and slope cost;
   - `"blocked"`: skip.
   A pixel's "is on road" for display is `onRoad`.
3. Watch the per-cell log line: `walls`, `roads … crossings, forced`. A cell
   with many `forced` is worth a look.

## Cost

- One more uint8 plane per cell (~20 MB raw, mostly zeros; compresses to a
  small fraction).
- Build time on a 4000 × 4000 test grid with 52,000 km of road and
  24,000 km of river: walls 0.2 s, roads 6 s.
- Client: one extra byte lookup per move (three for a diagonal).

## Running the tests

```
pip install numpy scipy shapely rasterio pytest
cd routing_raster && python -m pytest -q tests
npx vitest run routing_raster
```
