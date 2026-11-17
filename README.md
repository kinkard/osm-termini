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

The 2026-11-09 planet gives 28,353 rows, 8,177 of them in 88 languages beyond the plain tags.

## Routes and ends

- Ferries, shuttle trains: `type=route` relations with `route=ferry` or `route=shuttle_train`, and such ways outside
  them. Ends: the first way's first node, the last way's last node.
- Trains: PTv2 `route=train` relations. Coaches: PTv2 `route=bus` relations with "flix" in `operator` or `network`.
  Ends: the first and last `stop` member, else stop or platform member; platforms may follow all the stops.

## Names

- Per language: `from` and `to`; else the first and last part of `name` split outside brackets at ` - `, ` – `, ` — `,
  ` -> `, ` --> `, ` → `, ` => `, ` ↔ `, ` <-> `, ` ⇄ `, ` to `, else inside its trailing bracket ("JR Kyoto Line
  (Osaka - Kyoto)"); else a lone `from`, `to` or part ("Ferry to Crow Island").
- Labels go: text before a colon ("Flixbus 718: Toulon"), leading codes ("G5810", "IC-01 Eupen", "HSR 108 Zuoying"),
  route words and their number ("Ruta Golfito", "Linie P1", "Fähre Kaub"). A label-only name goes.
- A name needs a letter and more than a generic word (ferry, landing, quay, pier, jetty, wharf, dock, port, harbour,
  harbor, terminal, station, stop, boat, line, route, gare, bahnhof, hauptbahnhof, busbahnhof, hafen, fähre, anleger).
- "A (B)", "A/B": two names when each part has more than 3 letters. A lowercase or letterless "(b)" goes ("Tampa
  (seasonal)", "Zachodni (11)"), so does a generic "A" ("Wharf (Bhaucha Dhakka)"). Else as tagged: "Alderney (GBG)".
- Ferries: names swap ends when other routes name them more often within 5 km of the other end. Other languages also
  count the plain names, and follow them on a tie.
- Trains, coaches: end stop names only check the route's names and are not written.
  - Names swap ends when they match the stops crosswise.
  - An end's name goes when its stop's name disagrees and no other route names it within 5 km: missing stops.
  - Stop names of generic words ("station", "peron": words in 10 or more 0.1° cells) and platform numbers say nothing.
  - An unnamed stop takes its platform's name within 1 km.
  - Names match on a word, by its first 5 letters or a prefix: "Göteborgs central", "Göteborg C"; "長津田駅", "長津田".
- One row per name and language within 1 km.

## Run

```
cargo run --release -- --pbf planet.osm.pbf --out data
```

## Licence

Code: MIT OR Apache-2.0, at your option ([LICENSE-MIT](LICENSE-MIT), [LICENSE-APACHE](LICENSE-APACHE)).

Data: `termini.tsv` is derived from OpenStreetMap and licensed under the [Open Database License
1.0](https://opendatacommons.org/licenses/odbl/1-0/), © OpenStreetMap contributors.
