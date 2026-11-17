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

The 2026-11-09 planet gives 33,849 rows, 12,327 of them in 151 languages beyond the plain tags.

## Routes and ends

Ferry and shuttle train routes are `type=route` relations with `route=ferry` or `route=shuttle_train`, and ways with
those tags that are not members of such a relation; their ends are the first way's first node and the last way's last
node. Trains are PTv2 `route=train` relations, coaches PTv2 `route=bus` relations with "flix" in `operator` or
`network`; their ends are the first and last `stop` member, or the first and last stop or platform member when there are
fewer than two stops, as routes may list their platforms after all their stops.

## Names

In each language, a route's two ends are named by `from` and `to`; else by the first and last part of a `name` that
splits outside brackets at a spaced separator: ` - `, ` – `, ` — `, ` -> `, ` --> `, ` → `, ` => `, ` ↔ `, ` <-> `,
` ⇄ `, ` to `, or else inside its trailing bracket: "JR Kyoto Line (Osaka - Kyoto)"; else a lone `from`, `to` or part
names its end: "Ferry to Crow Island".

A name loses its route label: the text before a colon ("Flixbus 718: Toulon"), leading route and train codes ("G5810",
"IC-01 Eupen", "HSR 108 Zuoying") and route words with their number ("Ruta Golfito", "Linie P1", "Balsa Bertioga",
"Fähre Kaub"); a name that is only a label is dropped. A name must contain a letter and be more than a generic word
(ferry, landing, quay, pier, jetty, wharf, dock, port, harbour, harbor, terminal, station, stop, boat, line, route,
gare, bahnhof, hauptbahnhof, busbahnhof, hafen, fähre, anleger): not "7:00" or "Ferry".

"A (B)" and "A/B" are two names, "A" and "B", when each part has more than 3 letters; a lowercase or letterless "(b)" is
dropped ("Tampa (seasonal)" is "Tampa", "Zachodni (11)" is "Zachodni") and so is a generic "A" ("Wharf (Bhaucha
Dhakka)"); else the name stays as tagged: "Alderney (GBG)", "Frankfurt (Main) Hbf".

A ferry's first node need not be its `from` end, so a ferry's names swap ends when other routes name them more often
within 5 km of the other end; names in another language also count the route's plain names, and follow them on a tie.

A train's or coach's end stop adds its own names, in every language; an unnamed stop takes its platform's within 1 km. A
trailing platform number goes: "Museum, Platform 2" is "Museum", "peron 1a" nothing. The route's names swap ends when
they agree with the stops' crosswise, and stand at an end only if its stop's name says nothing (only generic words:
those of stop names in 10 or more 0.1° cells, as "station", "peron"), agrees with them, or other routes name them within
5 km: a route with missing stops starts somewhere else. Names agree when they share a word, words compared by their
first 5 letters, a shorter one by prefix: "Göteborgs central" and "Göteborg C", "長津田駅" and "長津田".

The same name in the same language within 1 km is one row.

## Run

```
cargo run --release -- --pbf planet.osm.pbf --out data
```

## Licence

Code: MIT OR Apache-2.0, at your option ([LICENSE-MIT](LICENSE-MIT), [LICENSE-APACHE](LICENSE-APACHE)).

Data: `termini.tsv` is derived from OpenStreetMap and licensed under the [Open Database License
1.0](https://opendatacommons.org/licenses/odbl/1-0/), © OpenStreetMap contributors.
