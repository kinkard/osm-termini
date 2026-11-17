//! OSM route termini from a sorted planet PBF: one TSV per kind (ferry, train, coach), a row per route with two ends
//! and two names; the funnel and the coach operator list as Markdown on stdout.

mod coords;
mod pbf;
mod routes;
mod terminals;

use std::fmt::Write as _;
use std::path::PathBuf;
use std::time::Instant;

use coords::{Bits, Coords};
use pbf::Blobs;
use routes::{Drop, Kind, LONG, Named, RelKind};
use terminals::{Terminal, Terminals};

const USAGE: &str = "\
usage: osm-termini --pbf PLANET.osm.pbf [--out DIR]

Writes DIR/ferry.tsv, DIR/train.tsv, DIR/coach.tsv (DIR defaults to data): id, name_a, lat_a, lon_a, name_b,
lat_b, lon_b. The rules are in README.md. OSM data is ODbL 1.0, © OpenStreetMap contributors.";

fn arg(args: &[String], key: &str) -> Option<String> {
    let i = args.iter().position(|a| a == key)?;
    args.get(i + 1).cloned()
}

/// Tabs and line breaks to spaces: a TSV field.
fn field(s: &str) -> String {
    s.replace(['\t', '\n', '\r'], " ")
}

struct Timer(Instant, Vec<String>);

impl Timer {
    fn lap(&mut self, what: &str, detail: String) {
        let row = format!(
            "| {what} | {:.1} | {detail} |",
            self.0.elapsed().as_secs_f64()
        );
        eprintln!("{row}");
        self.1.push(row);
        self.0 = Instant::now();
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(pbf) = arg(&args, "--pbf").map(PathBuf::from) else {
        eprintln!("{USAGE}");
        std::process::exit(2);
    };
    let out = PathBuf::from(arg(&args, "--out").unwrap_or_else(|| "data".into()));
    std::fs::create_dir_all(&out).expect("create the output directory");
    let all = Instant::now();
    let mut tm = Timer(Instant::now(), Vec::new());

    let blobs = Blobs::open(&pbf).expect("read pbf");
    tm.lap("blob index", format!("{} blobs", blobs.len()));

    let (rels, bus) = routes::relations(&blobs);
    let n_rel = |k: RelKind| rels.iter().filter(|r| r.kind == k).count();
    tm.lap(
        "relations",
        format!(
            "{} ferry, {} train PTv2, {} other train, {} flix bus PTv2, {} other flix bus, {} ferry terminal",
            n_rel(RelKind::Ferry),
            n_rel(RelKind::Train),
            n_rel(RelKind::OtherTrain),
            n_rel(RelKind::Coach),
            n_rel(RelKind::OtherCoach),
            n_rel(RelKind::Terminal)
        ),
    );

    let member_ways = |kinds: &[RelKind], keep: fn(&routes::Member) -> bool| {
        let ids: Vec<i64> = (rels.iter().filter(|r| kinds.contains(&r.kind)))
            .flat_map(|r| {
                (r.members.iter())
                    .filter(|m| m.ty == 'w' && keep(m))
                    .map(|m| m.id)
            })
            .collect();
        Bits::new(ids.into_iter())
    };
    let ways = routes::ways(
        &blobs,
        &member_ways(&[RelKind::Ferry], |m| routes::path_role(&m.role)),
        &member_ways(&[RelKind::Terminal], |_| true),
        &member_ways(&[RelKind::Train, RelKind::Coach], |_| true),
    );
    tm.lap(
        "ways",
        format!(
            "{} ferry ways ({} path members only), {} terminal ways and members, {} stop ways",
            ways.ferry.len(),
            ways.ferry.iter().filter(|w| !w.tagged).count(),
            ways.terminal.len(),
            ways.first.len(),
        ),
    );

    let ends = routes::end_nodes(ways.ferry.iter());
    let road = terminals::roads(&blobs, &Bits::new(ends.iter().copied()));
    tm.lap(
        "highways",
        format!(
            "{} ferry way end nodes, {} on a highway",
            ends.len(),
            road.len()
        ),
    );

    let mut need: Vec<i64> = ends;
    need.extend(ways.terminal.iter().flat_map(|t| t.1.iter().copied()));
    need.extend(ways.first.iter().map(|f| f.1));
    need.extend(
        (rels
            .iter()
            .filter(|r| matches!(r.kind, RelKind::Train | RelKind::Coach)))
        .flat_map(|r| r.members.iter().filter(|m| m.ty == 'n').map(|m| m.id)),
    );
    need.sort_unstable();
    need.dedup();
    let coords = Coords::new(need);
    let term_nodes: Vec<Terminal> = coords::nodes(&blobs, &coords, |_, at, mut tags| {
        Iterator::any(&mut tags, |(k, v)| k == "amenity" && v == "ferry_terminal").then(|| {
            Terminal {
                refs: Vec::new(),
                pts: vec![at],
            }
        })
    });
    tm.lap(
        "nodes",
        format!(
            "{} needed, {} found; {} terminal nodes",
            coords.len(),
            coords.found(),
            term_nodes.len()
        ),
    );

    let pts = |refs: &[i64]| refs.iter().filter_map(|&n| coords.get(n)).collect();
    let term_ways = (ways.terminal.iter().filter(|t| t.2)).map(|(_, refs, _)| Terminal {
        refs: refs.clone(),
        pts: pts(refs),
    });
    let n_term_ways = ways.terminal.iter().filter(|t| t.2).count();
    let (mut rings, open) = routes::terminal_rings(&rels, &ways);
    for r in &mut rings {
        r.pts = pts(&r.refs);
    }
    let n_term = format!(
        "terminals: {} nodes, {n_term_ways} ways, {} relation rings ({open} member ways left open)",
        term_nodes.len(),
        rings.len()
    );
    let terminals = Terminals::new(
        term_nodes
            .into_iter()
            .chain(term_ways)
            .chain(rings)
            .collect(),
    );
    let x = routes::extract(&rels, &ways, &road, &terminals, |n| coords.get(n));
    let rs = routes::routes(&x.units);
    tm.lap(
        "units, routes",
        format!(
            "{} units, {} routes; {} shared ferry nodes dropped, {} ferry end nodes without coordinates",
            x.units.len(),
            rs.len(),
            x.joins,
            x.missing
        ),
    );

    let mut bytes = 0;
    for kind in Kind::ALL {
        let mut tsv = String::from("id\tname_a\tlat_a\tlon_a\tname_b\tlat_b\tlon_b\n");
        for r in rs.iter().filter(|r| r.kind == kind) {
            let Some(([a, b], _)) = &r.names else {
                continue;
            };
            let [(lat_a, lon_a), (lat_b, lon_b)] = r.ends;
            let _ = writeln!(
                tsv,
                "{}\t{}\t{lat_a:.7}\t{lon_a:.7}\t{}\t{lat_b:.7}\t{lon_b:.7}",
                x.units[r.units[0]].osm_id(),
                field(a),
                field(b),
            );
        }
        bytes += tsv.len();
        std::fs::write(out.join(format!("{}.tsv", kind.code())), &tsv).expect("write tsv");
    }
    tm.lap("write", format!("{bytes} B"));

    println!("| step | s | what |\n|---|---|---|");
    for l in &tm.1 {
        println!("{l}");
    }
    println!("\ntotal {:.0} s; {n_term}\n", all.elapsed().as_secs_f64());
    report(&x, &rs);
    coach(&bus);
}

fn report(x: &routes::Extract, rs: &[routes::Route]) {
    let drops = routes::drops(&x.units);
    let mut head = String::from("| kind | OSM units |");
    for d in Drop::ALL {
        let _ = write!(head, " {} |", d.code());
    }
    head += " units kept | merged | routes | no names | written | names from / to | names split | same names |";
    println!("{head}\n|{}", "---|".repeat(head.matches('|').count() - 1));
    for kind in Kind::ALL {
        let n = |d: Option<Drop>| drops.get(&(kind, d)).copied().unwrap_or(0);
        let units: usize = (drops.iter().filter(|((k, _), _)| *k == kind))
            .map(|(_, c)| c)
            .sum();
        let kr: Vec<&routes::Route> = rs.iter().filter(|r| r.kind == kind).collect();
        let by = |b: Named| {
            kr.iter()
                .filter(|r| r.names.as_ref().is_some_and(|n| n.1 == b))
                .count()
        };
        let same = (kr.iter())
            .filter(|r| r.names.as_ref().is_some_and(|(n, _)| n[0] == n[1]))
            .count();
        let named = by(Named::FromTo) + by(Named::Name);
        let mut row = format!("| {} | {units} |", kind.code());
        for d in Drop::ALL {
            let _ = write!(row, " {} |", n(Some(d)));
        }
        let _ = write!(
            row,
            " {} | {} | {} | {} | {named} | {} | {} | {same} |",
            n(None),
            n(None) - kr.len(),
            kr.len(),
            kr.len() - named,
            by(Named::FromTo),
            by(Named::Name),
        );
        println!("{row}");
    }
    println!(
        "\nloop: one landing, or the first and last stop within 50 m; ends: a ferry with other than two landings, or \
         fewer than two stop / platform members; missing: the first or last stop without coordinates; not PTv2: no \
         public_transport:version=2; merged: joined an earlier route of the kind; no names: no unit with from and \
         to, nor a name that splits; same names: written, name_a equal to name_b.\n"
    );
}

fn coach(bus: &routes::Bus) {
    println!(
        "type=route + route=bus relations: {}; flix in operator or network: {}; route=coach relations (any type): {}, \
         of them flix: {}",
        bus.all, bus.flix, bus.coach_route, bus.coach_route_flix
    );
    let tags: Vec<String> = (0..LONG.len())
        .map(|i| format!("{}={} {}", LONG[i].0, LONG[i].1, bus.by_tag[i]))
        .collect();
    println!(
        "with a coach or long-distance tag: {} ({})\n",
        bus.long,
        tags.join(", ")
    );
    println!("| operator | relations |\n|---|---|");
    for (op, n) in bus.top(20) {
        let op = if op.is_empty() { "(none)" } else { op };
        println!("| {} | {n} |", op.replace('|', "\\|"));
    }
}
