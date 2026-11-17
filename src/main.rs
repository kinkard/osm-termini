use std::fmt::Write as _;
use std::path::PathBuf;
use std::time::Instant;

use clap::Parser;
use coords::Coords;
use pbf::Blobs;
use routes::{Drop, Kind, RelKind};
use rustc_hash::FxHashSet;

mod coords;
mod pbf;
mod routes;

/// Writes termini.tsv to the output directory: lat, lon, lang, name, ids.
#[derive(Parser)]
struct Config {
    /// OSM PBF sorted by type then id, as the planet is.
    #[arg(long)]
    pbf: PathBuf,
    /// Output directory for termini.tsv.
    #[arg(long, default_value = "data")]
    out: PathBuf,
}

fn main() {
    let Config { pbf, out } = Config::parse();
    std::fs::create_dir_all(&out).expect("create the output directory");
    let all = Instant::now();
    let mut tm = Timer(Instant::now(), Vec::new());

    let blobs = Blobs::open(&pbf).expect("read pbf");
    tm.lap("blob index", format!("{} blobs", blobs.len()));

    let rels = routes::relations(&blobs);
    let n_rel = |k: RelKind| rels.iter().filter(|r| r.kind == k).count();
    tm.lap(
        "relations",
        format!(
            "{} ferry, {} shuttle, {} train PTv2, {} other train, {} flix bus PTv2, {} other flix bus",
            n_rel(RelKind::Ferry),
            n_rel(RelKind::Shuttle),
            n_rel(RelKind::Train),
            n_rel(RelKind::OtherTrain),
            n_rel(RelKind::Coach),
            n_rel(RelKind::OtherCoach),
        ),
    );

    let member_ways = |kinds: &[RelKind]| -> FxHashSet<i64> {
        (rels.iter().filter(|r| kinds.contains(&r.kind)))
            .flat_map(|r| r.members.iter().filter(|m| m.ty == 'w').map(|m| m.id))
            .collect()
    };
    let ways = routes::ways(
        &blobs,
        &member_ways(&[RelKind::Ferry, RelKind::Shuttle]),
        &member_ways(&[RelKind::Train, RelKind::Coach]),
    );
    tm.lap(
        "ways",
        format!(
            "{} ferry and shuttle ways and member ways, {} stop ways",
            ways.path.len(),
            ways.first.len(),
        ),
    );

    let mut need: Vec<i64> = (ways.path.iter())
        .flat_map(|w| [w.refs.first(), w.refs.last()])
        .flatten()
        .copied()
        .collect();
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
    coords::nodes(&blobs, &coords);
    tm.lap(
        "nodes",
        format!("{} needed, {} found", coords.len(), coords.found()),
    );

    let units = routes::extract(&rels, &ways, |n| coords.get(n));
    let (places, swapped) = routes::places(&units);
    tm.lap(
        "units, places",
        format!(
            "{} units, {} places, {} ferry name pairs swapped",
            units.len(),
            places.len(),
            swapped.len()
        ),
    );

    let mut tsv = String::from("lat\tlon\tlang\tname\tids\n");
    for p in &places {
        let _ = writeln!(
            tsv,
            "{:.7}\t{:.7}\t{}\t{}\t{}",
            p.at.0,
            p.at.1,
            p.lang,
            field(&p.name),
            p.ids.join(",")
        );
    }
    std::fs::write(out.join("termini.tsv"), &tsv).expect("write tsv");
    tm.lap("write", format!("{} B", tsv.len()));

    println!("| step | s | what |\n|---|---|---|");
    for l in &tm.1 {
        println!("{l}");
    }
    println!("\ntotal {:.0} s\n", all.elapsed().as_secs_f64());
    report(&units, &places, &swapped);
}

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

fn report(units: &[routes::Unit], places: &[routes::Place], swapped: &[(usize, &str)]) {
    let drops = routes::drops(units);
    let mut head = String::from("| kind | OSM units |");
    for d in Drop::ALL {
        let _ = write!(head, " {} |", d.code());
    }
    head += " units kept |";
    println!("{head}\n|{}", "---|".repeat(head.matches('|').count() - 1));
    for kind in Kind::ALL {
        let n = |d: Option<Drop>| drops.get(&(kind, d)).copied().unwrap_or(0);
        let units: usize = (drops.iter().filter(|((k, _), _)| *k == kind))
            .map(|(_, c)| c)
            .sum();
        let mut row = format!("| {} | {units} |", kind.code());
        for d in Drop::ALL {
            let _ = write!(row, " {} |", n(Some(d)));
        }
        let _ = write!(row, " {} |", n(None));
        println!("{row}");
    }
    let langs: std::collections::BTreeSet<&str> = places.iter().map(|p| p.lang.as_str()).collect();
    let plain = places.iter().filter(|p| p.lang.is_empty()).count();
    println!(
        "\n{} rows: {plain} plain names, {} in {} languages; {} ferry name pairs swapped, e.g. {}\n",
        places.len(),
        places.len() - plain,
        langs.len() - langs.contains("") as usize,
        swapped.len(),
        (swapped.iter().take(10))
            .map(|&(u, lang)| format!("{} {lang}", units[u].osm_id())
                .trim_end()
                .to_string())
            .collect::<Vec<_>>()
            .join(", ")
    );
}
