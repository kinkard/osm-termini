# osm-termini

It's hard to measure geocoding quality without a proper dataset of names and coordinates. `osm-termini` was born from an idea that it's possible to use terminal points of OSM ferry, train and coach routes for that.

Check https://wiki.openstreetmap.org/wiki/Planet.osm and https://download.geofabrik.de for various options where to get OSM PBF.

N.B.: The PBF must be sorted by type then id, as the planet is. `osm-termini` relies on that fact as it uses binary search over blobs for faster scans.

## Output

One TSV per kind in the output directory (`data/` by default): `ferry.tsv`, `train.tsv`, `coach.tsv`.

| column | value |
|---|---|
| `id` | OSM type and id of the route's first unit: `relation/123`, `way/456` |
| `name_a` | name of end a |
| `lat_a`, `lon_a` | end a, degrees, 7 decimals |
| `name_b` | name of end b |
| `lat_b`, `lon_b` | end b |

Rows from the 2026-11-09 planet: 5,740 ferry, 7,803 train, 746 coach.

## Names

From the route's units in order: `from` and `to` of the first unit with both set; else the first unit whose `name`
splits at a spaced separator, first and last part. Separators, the first of this list the name holds: ` - `, ` – `,
` — `, ` -> `, ` --> `, ` → `, ` => `, ` ↔ `, ` <-> `, ` ⇄ `, ` to `.

Then trailing parenthesised tags are removed from each name, every one and nested ones included:
"Gedser (S)" → "Gedser", "Trelleborg (SE)" → "Trelleborg"; a name that would become empty is kept as is. Values are
trimmed and tabs or line breaks become spaces; nothing else is cleaned. A route without names is dropped.

**Names to ends.** The names follow the unit they came from: `from` or the first part at that unit's first end, swapped
when the unit joined the route reversed. For train and coach the first end is the first stop, so `name_a` belongs to
end a. A ferry's first end is its first way's first node, which `from` need not match: for ferries the pairing is not
guaranteed.

## Run

```
cargo run --release -- --pbf planet.osm.pbf --out data
```

## Licence

Code: MIT OR Apache-2.0, at your option ([LICENSE-MIT](LICENSE-MIT), [LICENSE-APACHE](LICENSE-APACHE)).

Data: the TSVs are derived from OpenStreetMap and are licensed under the
[Open Database License 1.0](https://opendatacommons.org/licenses/odbl/1-0/), © OpenStreetMap contributors.
