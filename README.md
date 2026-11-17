# osm-termini

Two ends and two place names per OpenStreetMap ferry, train and coach route, as a test set for geocoders: search a
name and expect a result near an end, or reverse-geocode an end and expect a name.

Built from the OSM planet `planet-261109.osm.pbf` (2026-11-09).

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

## Units

A unit is one OSM relation or way. Each kind has its own units and end rule.

| kind | units | ends |
|---|---|---|
| ferry | relations `type=route` + `route=ferry`; ways `route=ferry` that no such relation holds | end nodes of the unit's ways (relation member ways whose role does not start with `stop` or `platform`) |
| train | relations `type=route` + `route=train` + `public_transport:version=2` | the first and last member with role `stop`, `stop_entry_only`, `stop_exit_only`, `platform`, `platform_entry_only` or `platform_exit_only`; a node as is, a way by its first node, a relation member skipped |
| coach | relations `type=route` + `route=bus` + `public_transport:version=2` whose `operator` or `network` contains "flix", any case | as train |

**Ferry ends.** A node held by two or more ferry ways is not an end unless a non-ferry `highway=*` way holds it or an
`amenity=ferry_terminal` (node, way, or relation ring) lies within 50 m. Ends within 50 m of each other (single
linkage) are one landing. A unit needs exactly two landings.

**Dropped units:**

| reason | rule |
|---|---|
| loop | one landing; a train or coach whose first and last stop lie within 50 m |
| ends | a ferry with other than two landings; a train or coach with fewer than two stop or platform members |
| missing | a train's or coach's first or last stop without coordinates |
| not PTv2 | a `type=route` + `route=train` relation, or a flix `type=route` + `route=bus` relation, without `public_transport:version=2` |

## Routes

Units are taken relations first, then ways, each in id order. A unit joins the first earlier route of its kind whose
two ends each lie within 500 m of a different end of its own, in either order; else it starts a route. This merges the
two directions of a line and duplicates. A route keeps its first unit's ends.

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

The PBF must be sorted by type then id, as the planet is. A blob index, then four passes: relations, ways, highway
ways, nodes; on the 2026-11-09 planet about 135 s and 3.7 GB maximum resident memory on an 18-core Apple machine.

Markdown on stdout: the counts per kind and reason, and the 20 most frequent `operator` values of `type=route` +
`route=bus` relations with `bus=coach`, `coach=yes`, `bus=long_distance`, `bus=long-distance` or
`service=long_distance` (candidates for more coach operators; not in `coach.tsv`).

## Licence

Data: the TSVs are derived from OpenStreetMap and are licensed under the
[Open Database License 1.0](https://opendatacommons.org/licenses/odbl/1-0/), © OpenStreetMap contributors.

Code: MIT OR Apache-2.0, at your option ([LICENSE-MIT](LICENSE-MIT), [LICENSE-APACHE](LICENSE-APACHE)).
