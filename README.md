# osm-termini

It's hard to measure geocoding quality without a proper dataset of names and coordinates. `osm-termini` was born from the idea that the end points of OSM ferry, train and coach routes could serve for that.

Check https://wiki.openstreetmap.org/wiki/Planet.osm and https://download.geofabrik.de for where to download an OSM PBF.

N.B.: The PBF must be sorted by type then id, as the planet is. `osm-termini` relies on it to binary-search blobs for faster scans.

## Output

`termini.tsv` in the output directory (`data/` by default): a row per name at a place, from ferry, car shuttle train,
train and coach routes.

| column | value |
|---|---|
| `lat`, `lon` | the place, degrees, 7 decimals |
| `lang` | empty for `name`, `from`, `to`; `es` for `name:es`, `from:es`, `to:es`; `zh-Hant`, ... |
| `name` | as tagged |
| `ids` | the OSM routes naming it there, comma separated: `relation/123,way/456` |

The 2026-11-09 planet gives 31,857 rows, 9,252 of them in 92 languages beyond the plain tags.

## Routes and ends

- Ferries, shuttle trains: `type=route` relations with `route=ferry` or `route=shuttle_train`, and such ways, relation
  members too. Ends: the tips of the first and last way away from the rest of the path; a round trip's far end is its
  node farthest from the start.
- Trains: PTv2 `route=train` relations. Coaches: PTv2 `route=bus` relations with "flix" in `operator` or `network`.
  Ends: the first and last `stop` member, else stop or platform member; platforms may follow all the stops.

## Names

- Per language: `from` and `to`; else the first and last part of `name` split outside brackets at ` - `, ` – `, ` — `,
  ` -> `, ` --> `, ` → `, ` => `, ` ↔ `, ` <-> `, ` ⇄ `, ` to `, else inside its trailing bracket ("JR Kyoto Line
  (Osaka - Kyoto)"); else a lone `from`, `to` or part ("Ferry to Crow Island").
- Labels go: text before a colon ("Flixbus 718: Toulon"), a remark after it in a split name ("Gedser: 2h"), leading
  codes ("G5810", "IC-01 Eupen", "HSR 108 Zuoying"), route words and their number ("Ruta Golfito", "Linie P1", "Fähre
  Kaub"). A label-only name goes.
- A name needs a letter and more than a generic word (ferry, landing, quay, pier, jetty, wharf, dock, port, harbour,
  harbor, terminal, station, stop, boat, line, route, gare, bahnhof, hauptbahnhof, busbahnhof, hafen, fähre, anleger).
- "A (B)", "A/B": two names when each part has more than 3 letters. A lowercase or letterless "(b)" goes ("Tampa
  (seasonal)", "Zachodni (11)"), so does a generic "A" ("Wharf (Bhaucha Dhakka)"). Else as tagged: "Alderney (GBG)".
- Ferries: names swap ends when other routes name them more often within 5 km of the other end; `from` and `to` count
  double, a relation and its ways do not vote for each other. Other languages also count the plain names, and follow
  them on a tie.
- A name at both ends is written once, at the first: "Thun - Thun".
- One row per name and language within 1 km.

## Run

```
cargo run --release -- --pbf planet.osm.pbf --out data
```

## Licence

Code: MIT OR Apache-2.0, at your option ([LICENSE-MIT](LICENSE-MIT), [LICENSE-APACHE](LICENSE-APACHE)).

Data: `termini.tsv` is derived from OpenStreetMap and licensed under the [Open Database License
1.0](https://opendatacommons.org/licenses/odbl/1-0/), © OpenStreetMap contributors.
