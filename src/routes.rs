use std::collections::BTreeMap;

use osmpbf::RelMemberType;
use plane_projection::PlaneProjection;
use rustc_hash::{FxHashMap, FxHashSet};

use crate::coords::{Bits, LatLon};
use crate::pbf::Blobs;

/// Metres within which a unit's two ends make a loop.
pub const LOOP_M: f64 = 50.0;
/// Metres within which the same name in the same language is one place.
pub const SAME_M: f64 = 1000.0;
/// Metres within which other units' names vote for a ferry's orientation.
pub const VOTE_M: f64 = 5000.0;

/// Roles of a train or coach route's stops and platforms.
pub const STOPS: [&str; 6] = [
    "stop",
    "stop_entry_only",
    "stop_exit_only",
    "platform",
    "platform_entry_only",
    "platform_exit_only",
];

/// Separators of a route name, in the order tried.
pub const SEPS: [&str; 11] = [
    " - ", " – ", " — ", " -> ", " --> ", " → ", " => ", " ↔ ", " <-> ", " ⇄ ", " to ",
];

/// Words that are not a place name on their own, any case.
pub const GENERIC: [&str; 23] = [
    "ferry",
    "landing",
    "quay",
    "pier",
    "jetty",
    "wharf",
    "dock",
    "port",
    "harbour",
    "harbor",
    "terminal",
    "station",
    "stop",
    "boat",
    "line",
    "route",
    "gare",
    "bahnhof",
    "hauptbahnhof",
    "busbahnhof",
    "hafen",
    "fähre",
    "anleger",
];

/// Words of a route label before the place, any case: "Ruta Golfito", "Balsa Bertioga", "Tuyến phà Chì".
pub const ROUTE_WORDS: [&str; 22] = [
    "ruta",
    "rute",
    "rota",
    "línea",
    "linea",
    "linie",
    "ligne",
    "linija",
    "route",
    "travessia",
    "balsa",
    "barca",
    "lancha",
    "ferry",
    "boat",
    "speedboat",
    "fähre",
    "autofähre",
    "tuyến",
    "phà",
    "đò",
    "tàu",
];

/// Tags that mark a `route=bus` relation as a coach or long-distance route, for the operator list.
pub const LONG: [(&str, &str); 5] = [
    ("bus", "coach"),
    ("coach", "yes"),
    ("bus", "long_distance"),
    ("bus", "long-distance"),
    ("service", "long_distance"),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Kind {
    Ferry,
    Train,
    Coach,
}

impl Kind {
    pub const ALL: [Kind; 3] = [Kind::Ferry, Kind::Train, Kind::Coach];

    pub fn code(self) -> &'static str {
        match self {
            Kind::Ferry => "ferry",
            Kind::Train => "train",
            Kind::Coach => "coach",
        }
    }
}

/// `name`, `from` and `to` in one language.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Tags {
    pub name: String,
    pub from: String,
    pub to: String,
}

/// `Tags` by language: "" for `name`, `from`, `to`; "es" for `name:es`, `from:es`, `to:es`.
pub type Langs = BTreeMap<String, Tags>;

/// A language suffix: "es", "zh-Hant", "ja-Latn"; not "etymology:wikidata" or "left".
fn is_lang(s: &str) -> bool {
    let (lang, script) = s.split_once('-').unwrap_or((s, ""));
    (2..=3).contains(&lang.len())
        && lang.bytes().all(|b| b.is_ascii_lowercase())
        && script.bytes().all(|b| b.is_ascii_alphanumeric())
}

pub fn read_langs<'a>(tags: impl Iterator<Item = (&'a str, &'a str)>) -> Langs {
    let mut out = Langs::new();
    for (k, v) in tags {
        let (key, lang) = k.split_once(':').unwrap_or((k, ""));
        if !matches!(key, "name" | "from" | "to") || !(lang.is_empty() || is_lang(lang)) {
            continue;
        }
        let t = out.entry(lang.to_string()).or_default();
        let v = v.trim().to_string();
        match key {
            "name" => t.name = v,
            "from" => t.from = v,
            _ => t.to = v,
        }
    }
    out
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RelKind {
    /// `type=route` + `route=ferry`.
    Ferry,
    /// `type=route` + `route=shuttle_train`.
    Shuttle,
    /// `type=route` + `route=train` + `public_transport:version=2`.
    Train,
    /// Another `type=route` + `route=train` relation.
    OtherTrain,
    /// `type=route` + `route=bus` + "flix" in `operator` or `network` + `public_transport:version=2`.
    Coach,
    /// Another `type=route` + `route=bus` relation with "flix" in `operator` or `network`.
    OtherCoach,
}

#[derive(Debug, Default)]
struct RelTags<'a> {
    ty: &'a str,
    route: &'a str,
    v2: bool,
    flix: bool,
    operator: &'a str,
    /// Indices into `LONG` of the tags it carries.
    long: Vec<usize>,
}

fn rel_tags<'a>(tags: impl Iterator<Item = (&'a str, &'a str)>) -> RelTags<'a> {
    let mut t = RelTags::default();
    for (k, v) in tags {
        match k {
            "type" => t.ty = v,
            "route" => t.route = v,
            "public_transport:version" => t.v2 = v == "2",
            "operator" | "network" => {
                t.flix |= v.to_lowercase().contains("flix");
                if k == "operator" {
                    t.operator = v.trim();
                }
            }
            _ => {}
        }
        t.long
            .extend((0..LONG.len()).filter(|&i| LONG[i] == (k, v)));
    }
    t
}

fn rel_kind(t: &RelTags) -> Option<RelKind> {
    let route = t.ty == "route";
    match t.route {
        "ferry" if route => Some(RelKind::Ferry),
        "shuttle_train" if route => Some(RelKind::Shuttle),
        "train" if route && t.v2 => Some(RelKind::Train),
        "train" if route => Some(RelKind::OtherTrain),
        "bus" if route && t.flix && t.v2 => Some(RelKind::Coach),
        "bus" if route && t.flix => Some(RelKind::OtherCoach),
        _ => None,
    }
}

/// A relation member: `n` node, `w` way, `r` relation.
#[derive(Clone, Debug, PartialEq)]
pub struct Member {
    pub ty: char,
    pub id: i64,
    pub role: String,
}

impl Member {
    pub fn new(ty: char, id: i64, role: &str) -> Self {
        Member {
            ty,
            id,
            role: role.to_string(),
        }
    }
}

pub struct Rel {
    pub id: i64,
    pub kind: RelKind,
    pub tags: Langs,
    /// Ferries and shuttles: member ways but stops and platforms; trains and coaches: stop and platform members; others:
    /// none.
    pub members: Vec<Member>,
}

/// `type=route` + `route=bus` relations by tags, for the coach operator list.
#[derive(Debug, Default)]
pub struct Bus {
    pub all: usize,
    /// "flix" in `operator` or `network`.
    pub flix: usize,
    /// Relations with a `LONG` tag, and how many carry each.
    pub long: usize,
    pub by_tag: [usize; LONG.len()],
    /// `operator` values of the relations with a `LONG` tag; "" for none.
    pub operators: FxHashMap<String, usize>,
    /// `route=coach` relations (any `type`), and of them with "flix".
    pub coach_route: usize,
    pub coach_route_flix: usize,
}

impl Bus {
    fn add(&mut self, t: &RelTags) {
        if t.route == "coach" {
            self.coach_route += 1;
            self.coach_route_flix += t.flix as usize;
        }
        if t.ty != "route" || t.route != "bus" {
            return;
        }
        self.all += 1;
        self.flix += t.flix as usize;
        if t.long.is_empty() {
            return;
        }
        self.long += 1;
        let mut long = t.long.clone();
        long.dedup();
        long.iter().for_each(|&i| self.by_tag[i] += 1);
        *self.operators.entry(t.operator.to_string()).or_default() += 1;
    }

    fn extend(&mut self, o: Bus) {
        self.all += o.all;
        self.flix += o.flix;
        self.long += o.long;
        (0..LONG.len()).for_each(|i| self.by_tag[i] += o.by_tag[i]);
        for (k, v) in o.operators {
            *self.operators.entry(k).or_default() += v;
        }
        self.coach_route += o.coach_route;
        self.coach_route_flix += o.coach_route_flix;
    }

    /// The `n` most frequent operators, ties by name.
    pub fn top(&self, n: usize) -> Vec<(&str, usize)> {
        let mut v: Vec<(&str, usize)> = (self.operators.iter())
            .map(|(k, &c)| (k.as_str(), c))
            .collect();
        v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
        v.truncate(n);
        v
    }
}

/// Pass 1: ferry, shuttle, train and flix bus relations, in id order; the bus tag counts.
pub fn relations(blobs: &Blobs) -> (Vec<Rel>, Bus) {
    let per_block = blobs.par_map(blobs.relations_range(), |b| {
        let (mut out, mut bus) = (Vec::new(), Bus::default());
        for g in b.groups() {
            for r in g.relations() {
                let t = rel_tags(r.tags());
                bus.add(&t);
                let Some(kind) = rel_kind(&t) else {
                    continue;
                };
                let members = r.members().filter_map(|m| {
                    let ty = match m.member_type {
                        RelMemberType::Node => 'n',
                        RelMemberType::Way => 'w',
                        RelMemberType::Relation => 'r',
                    };
                    let role = m.role().unwrap_or_default();
                    let keep = match kind {
                        RelKind::Ferry | RelKind::Shuttle => {
                            ty == 'w' && !role.starts_with("stop") && !role.starts_with("platform")
                        }
                        RelKind::Train | RelKind::Coach => STOPS.contains(&role),
                        RelKind::OtherTrain | RelKind::OtherCoach => false,
                    };
                    keep.then(|| Member::new(ty, m.member_id, role))
                });
                out.push(Rel {
                    id: r.id(),
                    kind,
                    tags: read_langs(r.tags()),
                    members: members.collect(),
                });
            }
        }
        (out, bus)
    });
    let (mut rels, mut bus) = (Vec::new(), Bus::default());
    for (r, b) in per_block {
        rels.extend(r);
        bus.extend(b);
    }
    rels.dedup_by_key(|r| r.id);
    (rels, bus)
}

/// `route=ferry` a ferry, `route=shuttle_train` a train.
fn route_kind<'a>(mut tags: impl Iterator<Item = (&'a str, &'a str)>) -> Option<Kind> {
    match tags.find(|(k, _)| *k == "route")?.1 {
        "ferry" => Some(Kind::Ferry),
        "shuttle_train" => Some(Kind::Train),
        _ => None,
    }
}

pub struct Way {
    pub id: i64,
    /// `route_kind` of the way's own tags.
    pub kind: Option<Kind>,
    pub tags: Langs,
    pub refs: Vec<i64>,
}

#[derive(Default)]
pub struct Ways {
    /// Ferry and shuttle train ways, and ferry and shuttle relations' member ways, in id order.
    pub path: Vec<Way>,
    /// Train and coach stop and platform member ways: id, first node; in id order.
    pub first: Vec<(i64, i64)>,
}

/// Pass 2; `path`, `stops`: the relation member ways wanted.
pub fn ways(blobs: &Blobs, path: &Bits, stops: &Bits) -> Ways {
    let per_block = blobs.par_map(blobs.ways_range(), |b| {
        let mut out = Ways::default();
        for g in b.groups() {
            for w in g.ways() {
                let id = w.id();
                let kind = route_kind(w.tags());
                if kind.is_some() || path.has(id) {
                    out.path.push(Way {
                        id,
                        kind,
                        tags: read_langs(w.tags()),
                        refs: w.refs().collect(),
                    });
                }
                if stops.has(id)
                    && let Some(n) = w.refs().next()
                {
                    out.first.push((id, n));
                }
            }
        }
        out
    });
    let mut out = Ways::default();
    for w in per_block {
        out.path.extend(w.path);
        out.first.extend(w.first);
    }
    out.path.dedup_by_key(|w| w.id);
    out.first.dedup_by_key(|f| f.0);
    out
}

/// Why an OSM unit is not kept.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Drop {
    /// A stop route's ends within `LOOP_M`.
    Loop,
    /// A ferry or shuttle relation with none of its ways, or a stop route with fewer than two stop or platform members.
    Ends,
    /// An end without coordinates.
    Missing,
    /// No `public_transport:version=2`.
    Version,
}

impl Drop {
    pub const ALL: [Drop; 4] = [Drop::Loop, Drop::Ends, Drop::Missing, Drop::Version];

    pub fn code(self) -> &'static str {
        match self {
            Drop::Loop => "loop",
            Drop::Ends => "ends",
            Drop::Missing => "missing",
            Drop::Version => "not PTv2",
        }
    }
}

/// A train's or coach's ends: its first and last stop or platform member of node or way type, a way by its first
/// node (`at`).
pub fn stop_ends(
    members: &[Member],
    at: impl Fn(&Member) -> Option<LatLon>,
) -> Result<[LatLon; 2], Drop> {
    let stops: Vec<&Member> = (members.iter())
        .filter(|m| m.ty != 'r' && STOPS.contains(&m.role.as_str()))
        .collect();
    let (Some(first), Some(last)) = (stops.first(), stops.last()) else {
        return Err(Drop::Ends);
    };
    if stops.len() < 2 {
        return Err(Drop::Ends);
    }
    let (Some(a), Some(b)) = (at(first), at(last)) else {
        return Err(Drop::Missing);
    };
    match PlaneProjection::new(a.0).distance(a, b) <= LOOP_M {
        true => Err(Drop::Loop),
        false => Ok([a, b]),
    }
}

/// A name ending in a bracket: the text before it and inside it, trimmed; "A (B (C))" is "A" and "B (C)".
fn trailing(name: &str) -> Option<(&str, &str)> {
    let body = name.strip_suffix(')')?;
    let mut depth = 0;
    for (i, c) in body.char_indices().rev() {
        match c {
            ')' => depth += 1,
            '(' if depth == 0 => return Some((body[..i].trim(), body[i + 1..].trim())),
            '(' => depth -= 1,
            _ => {}
        }
    }
    None
}

/// The first and last part of a name at `sep` outside brackets.
fn split_outside<'a>(name: &'a str, sep: &str) -> Option<[&'a str; 2]> {
    let outside = |i: usize| name[..i].matches('(').count() == name[..i].matches(')').count();
    let mut at = name
        .match_indices(sep)
        .map(|(i, _)| i)
        .filter(|&i| outside(i));
    let first = at.next()?;
    let last = at.last().unwrap_or(first);
    Some([&name[..first], &name[last + sep.len()..]])
}

/// The first and last part of a name split outside brackets at the first of `SEPS` it holds there, else of its trailing
/// bracket: "JR Kyoto Line (Osaka - Kyoto)"; trimmed, `None` when one is empty. A label before a colon is cut first, as
/// it may hold a separator: "AREX - Todas las paradas: Seoul → Incheon".
pub fn split_name(name: &str) -> Option<[&str; 2]> {
    if let Some((_, rest)) = name.rsplit_once([':', '：'])
        && let Some(parts) = split_name(rest)
    {
        return Some(parts);
    }
    let Some([a, b]) = SEPS.iter().find_map(|sep| split_outside(name, sep)) else {
        return split_name(trailing(name)?.1);
    };
    let (a, b) = (a.trim(), b.trim());
    (!a.is_empty() && !b.is_empty()).then_some([a, b])
}

/// A route or train code: uppercase letters, digits and dashes with a digit and a letter, "G5810", "IC-01"; not "Bender-3".
fn code(s: &str) -> bool {
    s.chars()
        .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '-')
        && s.chars().any(|c| c.is_ascii_digit())
        && s.chars().any(|c| c.is_ascii_uppercase())
}

/// A name without its route label: the text before a colon, leading codes, `ROUTE_WORDS` and their number, a train
/// type and its number. "Flixbus 718: Toulon", "Rute Speedboat Tarakan", "HSR 108 Zuoying" are the last word; "G5810"
/// and "Linie P1" are empty.
pub fn unlabel(name: &str) -> &str {
    let mut s = name
        .rsplit_once([':', '：'])
        .map_or(name, |(_, s)| s)
        .trim();
    loop {
        let (head, rest) = s.split_once(' ').unwrap_or((s, ""));
        let rest = rest.trim_start();
        let (next, after) = rest.split_once(' ').unwrap_or((rest, ""));
        let numbered = next.starts_with(|c: char| c.is_ascii_digit());
        s = if code(head) {
            rest
        } else if !rest.is_empty() && ROUTE_WORDS.iter().any(|w| head.eq_ignore_ascii_case(w)) {
            if code(next) || numbered {
                after.trim_start()
            } else {
                rest
            }
        } else if (2..=4).contains(&head.len())
            && head.chars().all(|c| c.is_ascii_uppercase())
            && numbered
        {
            after.trim_start()
        } else {
            return s;
        };
    }
}

/// A place name: a letter in it and more than a `GENERIC` word; not "7:00" or "Ferry".
fn place(s: &str) -> bool {
    s.chars().any(char::is_alphabetic) && !GENERIC.iter().any(|g| s.eq_ignore_ascii_case(g))
}

/// The names a tagged name holds: "A (B)" and "A/B" are "A" and "B" when each part is a `place` of more than 3 letters;
/// a lowercase "(b)" and a generic "A" are no names: "Tampa (seasonal)", "Wharf (Bhaucha Dhakka)"; else the name as
/// tagged: "Alderney (GBG)", "Batayporã/MS".
pub fn parts(name: &str) -> Vec<&str> {
    let ok = |s: &str| place(s) && s.chars().filter(|c| c.is_alphabetic()).count() > 3;
    if let Some((outer, inner)) = trailing(name) {
        let lower = inner.starts_with(char::is_lowercase);
        match (ok(outer), ok(inner) && !lower) {
            (true, true) => return [parts(outer), parts(inner)].concat(),
            (true, false) if lower => return parts(outer),
            (false, true) if !place(outer) => return parts(inner),
            _ => {}
        }
    }
    if !name.contains('(') && name.contains('/') {
        let ps: Vec<&str> = (name.split('/').map(str::trim))
            .filter(|p| !GENERIC.iter().any(|g| p.eq_ignore_ascii_case(g)))
            .collect();
        if !ps.is_empty() && ps.iter().all(|p| ok(p)) {
            return ps;
        }
    }
    vec![name]
}

/// The names of a unit's two ends in one language: `from` and `to`, else the parts of a `name` that splits, else a lone
/// `from` or `to` or part: "Ferry to Crow Island"; `unlabel`ed `place` names only.
pub fn end_names<'a>(t: &'a Tags) -> [Option<&'a str>; 2] {
    let p = |s: &'a str| Some(unlabel(s)).filter(|s| place(s));
    if let (Some(a), Some(b)) = (p(&t.from), p(&t.to)) {
        return [Some(a), Some(b)];
    }
    let [a, b] = split_name(&t.name).map_or([None, None], |[a, b]| [p(a), p(b)]);
    if a.is_some() && b.is_some() {
        return [a, b];
    }
    [p(&t.from).or(a), p(&t.to).or(b)]
}

/// One OSM unit: a ferry or shuttle relation or way, a train or coach relation, and its fate.
pub struct Unit {
    pub kind: Kind,
    /// `r` relation, `w` way.
    pub osm_type: char,
    pub id: i64,
    pub tags: Langs,
    pub ends: Result<[LatLon; 2], Drop>,
}

impl Unit {
    pub fn osm_id(&self) -> String {
        let ty = if self.osm_type == 'r' {
            "relation"
        } else {
            "way"
        };
        format!("{ty}/{}", self.id)
    }
}

/// Every OSM unit: ferry and shuttle relations, then ferry and shuttle ways no such relation holds, then train
/// relations, then coach relations; each in id order.
pub struct Extract {
    pub units: Vec<Unit>,
}

/// Ends and drops of every unit from the passes' output; `at` a node's (lat, lon).
pub fn extract(rels: &[Rel], ways: &Ways, at: impl Fn(i64) -> Option<LatLon>) -> Extract {
    let way_of: FxHashMap<i64, &Way> = ways.path.iter().map(|w| (w.id, w)).collect();
    // A path's first way's first node and its last way's last node.
    let path_ends = |ws: &[&Way]| {
        let a = ws.first().and_then(|w| w.refs.first());
        let b = ws.last().and_then(|w| w.refs.last());
        let (Some(&a), Some(&b)) = (a, b) else {
            return Err(Drop::Ends);
        };
        match (at(a), at(b)) {
            (Some(a), Some(b)) => Ok([a, b]),
            _ => Err(Drop::Missing),
        }
    };
    let mut units = Vec::new();
    let mut members = FxHashSet::default();
    for r in rels
        .iter()
        .filter(|r| matches!(r.kind, RelKind::Ferry | RelKind::Shuttle))
    {
        members.extend(r.members.iter().map(|m| m.id));
        let path: Vec<&Way> = (r.members.iter())
            .filter_map(|m| way_of.get(&m.id).copied())
            .collect();
        units.push(Unit {
            kind: match r.kind {
                RelKind::Ferry => Kind::Ferry,
                _ => Kind::Train,
            },
            osm_type: 'r',
            id: r.id,
            tags: r.tags.clone(),
            ends: path_ends(&path),
        });
    }
    for w in ways.path.iter().filter(|w| !members.contains(&w.id)) {
        let Some(kind) = w.kind else {
            continue;
        };
        units.push(Unit {
            kind,
            osm_type: 'w',
            id: w.id,
            tags: w.tags.clone(),
            ends: path_ends(&[w]),
        });
    }
    let first = |id: i64| {
        let i = ways.first.binary_search_by_key(&id, |f| f.0).ok()?;
        at(ways.first[i].1)
    };
    let stop_at = |m: &Member| match m.ty {
        'n' => at(m.id),
        _ => first(m.id),
    };
    for (kind, ptv2, other) in [
        (Kind::Train, RelKind::Train, RelKind::OtherTrain),
        (Kind::Coach, RelKind::Coach, RelKind::OtherCoach),
    ] {
        for r in rels.iter().filter(|r| r.kind == ptv2 || r.kind == other) {
            units.push(Unit {
                kind,
                osm_type: 'r',
                id: r.id,
                tags: r.tags.clone(),
                ends: match r.kind == ptv2 {
                    true => stop_ends(&r.members, stop_at),
                    false => Err(Drop::Version),
                },
            });
        }
    }
    Extract { units }
}

/// A name at a place: the point of its first mention, and the units that name it there.
#[derive(Debug, PartialEq)]
pub struct Place {
    pub at: LatLon,
    pub lang: String,
    pub name: String,
    pub ids: Vec<String>,
}

/// Single-linkage groups of points within `m` metres: each point's group, the index of the group's first point.
fn groups(pts: &[LatLon], m: f64) -> Vec<usize> {
    let mut group: Vec<usize> = (0..pts.len()).collect();
    for i in 0..pts.len() {
        let proj = PlaneProjection::new(pts[i].0);
        for j in 0..i {
            if group[j] != group[i] && proj.distance(pts[i], pts[j]) <= m {
                let (from, to) = (group[i].max(group[j]), group[i].min(group[j]));
                group
                    .iter_mut()
                    .filter(|g| **g == from)
                    .for_each(|g| *g = to);
            }
        }
    }
    group
}

/// A kept unit's names in one language: unit, language, the names of its ends, its ends.
type Named<'a> = (usize, &'a str, [Option<&'a str>; 2], [LatLon; 2]);

/// A row per language, name and place (`SAME_M`). A ferry's names swap ends by majority of other units' names within
/// `VOTE_M`; other languages also count the unit's plain names and follow them on a tie.
pub fn places(units: &[Unit]) -> (Vec<Place>, Vec<(usize, &str)>) {
    let mut named: Vec<Named> = Vec::new();
    for (u, unit) in units.iter().enumerate() {
        let Ok(ends) = unit.ends else {
            continue;
        };
        for (lang, t) in &unit.tags {
            let n = end_names(t);
            if n.iter().any(Option::is_some) {
                named.push((u, lang, n, ends));
            }
        }
    }
    let mut seen: FxHashMap<&str, Vec<(usize, LatLon)>> = FxHashMap::default();
    for &(u, _, n, ends) in &named {
        for (s, at) in n.iter().zip(ends) {
            if let Some(s) = s {
                seen.entry(s).or_default().push((u, at));
            }
        }
    }
    // Units other than `u` naming `s` within `VOTE_M` of `at`, in any language.
    let votes = |u: usize, s: Option<&str>, at: LatLon| {
        let Some(s) = s else {
            return 0;
        };
        let proj = PlaneProjection::new(at.0);
        let mut us: Vec<usize> = (seen[s].iter())
            .filter(|&&(v, p)| v != u && proj.distance(at, p) <= VOTE_M)
            .map(|&(v, _)| v)
            .collect();
        us.sort_unstable();
        us.dedup();
        us.len()
    };
    let mut swapped = Vec::new();
    let mut mentions: BTreeMap<(&str, &str), Vec<(LatLon, usize)>> = BTreeMap::new();
    // The current unit's plain names, after the vote, and whether they swapped; `named` has them first.
    let mut plain: Option<(usize, [Option<&str>; 2], bool)> = None;
    for &(u, lang, mut n, [a, b]) in &named {
        let mut keep = votes(u, n[0], a) + votes(u, n[1], b);
        let mut flip = votes(u, n[0], b) + votes(u, n[1], a);
        let mut tie = false;
        if let Some((pu, pn, ps)) = plain
            && pu == u
        {
            let own = |s: Option<&str>, e: usize| (s.is_some() && s == pn[e]) as usize;
            keep += own(n[0], 0) + own(n[1], 1);
            flip += own(n[0], 1) + own(n[1], 0);
            tie = ps;
        }
        let swap = units[u].kind == Kind::Ferry && (flip > keep || flip == keep && tie);
        if swap {
            n.swap(0, 1);
            swapped.push((u, lang));
        }
        if lang.is_empty() {
            plain = Some((u, n, swap));
        }
        for (s, at) in n.iter().zip([a, b]) {
            for s in s.map(parts).unwrap_or_default() {
                mentions.entry((lang, s)).or_default().push((at, u));
            }
        }
    }
    let mut out = Vec::new();
    for ((lang, name), ms) in mentions {
        let pts: Vec<LatLon> = ms.iter().map(|m| m.0).collect();
        let group = groups(&pts, SAME_M);
        for (i, &(at, _)) in ms.iter().enumerate().filter(|&(i, _)| group[i] == i) {
            let mut ids: Vec<String> = (ms.iter().zip(&group))
                .filter(|&(_, &g)| g == i)
                .map(|(m, _)| units[m.1].osm_id())
                .collect();
            ids.sort_unstable();
            ids.dedup();
            out.push(Place {
                at,
                lang: lang.to_string(),
                name: name.to_string(),
                ids,
            });
        }
    }
    out.sort_by(|p, q| {
        (p.lang.cmp(&q.lang).then(p.name.cmp(&q.name)))
            .then(p.at.0.total_cmp(&q.at.0))
            .then(p.at.1.total_cmp(&q.at.1))
    });
    (out, swapped)
}

pub fn drops(units: &[Unit]) -> BTreeMap<(Kind, Option<Drop>), usize> {
    let mut out = BTreeMap::new();
    for u in units {
        *out.entry((u.kind, u.ends.err())).or_default() += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn north(m: f64) -> LatLon {
        (54.0 + m / 111_195.0, 11.0)
    }

    fn tags(name: &str, from: &str, to: &str) -> Tags {
        Tags {
            name: name.into(),
            from: from.into(),
            to: to.into(),
        }
    }

    #[test]
    fn way_units() {
        let way = |id: i64, kind: Option<Kind>, refs: &[i64]| Way {
            id,
            kind,
            tags: Langs::default(),
            refs: refs.to_vec(),
        };
        let ways = Ways {
            path: vec![
                way(1, Some(Kind::Ferry), &[10, 11, 12]),
                way(2, Some(Kind::Ferry), &[10, 11, 10]),
                way(3, Some(Kind::Train), &[12, 13]),
                way(4, Some(Kind::Ferry), &[12, 99]),
                way(5, None, &[14, 15]),
                way(6, Some(Kind::Ferry), &[15, 16]),
            ],
            ..Ways::default()
        };
        let at = |n: i64| (n < 99).then(|| north((n - 10) as f64 * 1000.0));
        // Relation 7 runs over ways 5 and 6, an untagged and a ferry way; relation 8 has none of its ways.
        let rel = |id: i64, ways: &[i64]| Rel {
            id,
            kind: RelKind::Ferry,
            tags: Langs::default(),
            members: ways.iter().map(|&w| Member::new('w', w, "")).collect(),
        };
        let rels = [rel(7, &[5, 6]), rel(8, &[9])];
        let got: Vec<(Kind, String, Result<[LatLon; 2], Drop>)> = (extract(&rels, &ways, at).units)
            .into_iter()
            .map(|u| (u.kind, u.osm_id(), u.ends))
            .collect();
        assert_eq!(
            got,
            [
                (
                    Kind::Ferry,
                    "relation/7".into(),
                    Ok([north(4000.0), north(6000.0)])
                ),
                (Kind::Ferry, "relation/8".into(), Err(Drop::Ends)),
                (Kind::Ferry, "way/1".into(), Ok([north(0.0), north(2000.0)])),
                (Kind::Ferry, "way/2".into(), Ok([north(0.0), north(0.0)]),),
                (
                    Kind::Train,
                    "way/3".into(),
                    Ok([north(2000.0), north(3000.0)])
                ),
                (Kind::Ferry, "way/4".into(), Err(Drop::Missing)),
            ]
        );

        let kind = |tags: &[(&str, &str)]| route_kind(tags.iter().copied());
        assert_eq!(
            kind(&[("name", "X"), ("route", "ferry")]),
            Some(Kind::Ferry)
        );
        assert_eq!(kind(&[("route", "shuttle_train")]), Some(Kind::Train));
        assert_eq!(kind(&[("route", "train")]), None);
        assert_eq!(kind(&[("railway", "rail")]), None);
    }

    #[test]
    fn stop_end_rule() {
        let m = |ty: char, id: i64, role: &str| Member::new(ty, id, role);
        let at: FxHashMap<(char, i64), LatLon> = [
            (('n', 1), north(0.0)),
            (('w', 2), north(10.0)),
            (('n', 3), north(3000.0)),
            (('w', 4), north(6000.0)),
        ]
        .into_iter()
        .collect();
        let ends = |ms: &[Member]| stop_ends(ms, |m| at.get(&(m.ty, m.id)).copied());
        // First and last stop or platform member, a way by its first node; relations and route ways skipped.
        let ms = [
            m('r', 9, "platform"),
            m('n', 1, "stop_entry_only"),
            m('w', 2, "platform"),
            m('n', 3, "stop"),
            m('w', 4, "platform_exit_only"),
            m('r', 8, "stop"),
        ];
        assert_eq!(ends(&ms), Ok([north(0.0), north(6000.0)]));
        assert_eq!(ends(&ms[..3]), Err(Drop::Loop), "0 and 10 m");
        assert_eq!(ends(&ms[..2]), Err(Drop::Ends), "one stop");
        assert_eq!(
            ends(&[m('n', 1, "stop"), m('n', 5, "stop")]),
            Err(Drop::Missing)
        );
        assert_eq!(
            ends(&[m('n', 1, ""), m('n', 3, "station")]),
            Err(Drop::Ends)
        );

        let kind = |tags: &[(&str, &str)]| rel_kind(&rel_tags(tags.iter().copied()));
        let (route, v2) = (("type", "route"), ("public_transport:version", "2"));
        assert_eq!(kind(&[route, ("route", "train"), v2]), Some(RelKind::Train));
        assert_eq!(
            kind(&[route, ("route", "train")]),
            Some(RelKind::OtherTrain)
        );
        assert_eq!(kind(&[("type", "route_master"), ("route", "train")]), None);
        assert_eq!(kind(&[route, ("route", "ferry")]), Some(RelKind::Ferry));
        assert_eq!(kind(&[("route", "ferry")]), None);
        assert_eq!(
            kind(&[route, ("route", "shuttle_train")]),
            Some(RelKind::Shuttle)
        );
        // Coaches: "flix" in operator or network, any case.
        let bus = ("route", "bus");
        assert_eq!(
            kind(&[route, bus, v2, ("operator", "FlixBus DACH")]),
            Some(RelKind::Coach)
        );
        assert_eq!(
            kind(&[route, bus, v2, ("network", "Flixbus")]),
            Some(RelKind::Coach)
        );
        assert_eq!(
            kind(&[route, bus, ("network", "FLIXBUS")]),
            Some(RelKind::OtherCoach)
        );
        assert_eq!(kind(&[route, bus, v2, ("operator", "BlaBlaCar Bus")]), None);
        assert_eq!(
            kind(&[("type", "route_master"), bus, v2, ("network", "Flixbus")]),
            None
        );

        let t = rel_tags(
            [
                route,
                bus,
                ("bus", "coach"),
                ("service", "long_distance"),
                ("operator", " Ouibus "),
            ]
            .into_iter(),
        );
        let mut b = Bus::default();
        b.add(&t);
        b.add(&rel_tags([route, bus, ("operator", "Ouibus")].into_iter()));
        assert_eq!((b.all, b.long, b.by_tag), (2, 1, [1, 0, 0, 0, 1]));
        assert_eq!(b.top(5), [("Ouibus", 1)]);
    }

    #[test]
    fn name_split_rule() {
        assert_eq!(split_name("Rostock - Gedser"), Some(["Rostock", "Gedser"]));
        assert_eq!(
            split_name("A – B – C"),
            Some(["A", "C"]),
            "first and last part"
        );
        assert_eq!(
            split_name("RE 1: Hamburg => Lübeck"),
            Some(["Hamburg", "Lübeck"])
        );
        assert_eq!(
            split_name("Airport Railroad Express - Todas las paradas: Seoul Station → Incheon T2"),
            Some(["Seoul Station", "Incheon T2"])
        );
        assert_eq!(
            split_name("Rostock - Gedser: 2h"),
            Some(["Rostock", "Gedser: 2h"]),
            "nothing splits after the colon"
        );
        // Not inside brackets; the trailing bracket's parts when nothing splits outside.
        assert_eq!(
            split_name("Frankfurt (Main - Süd) - Berlin"),
            Some(["Frankfurt (Main - Süd)", "Berlin"])
        );
        assert_eq!(
            split_name("Línea JR Takarazuka (Osaka - Sasayamaguchi)"),
            Some(["Osaka", "Sasayamaguchi"])
        );
        assert_eq!(
            split_name("京成電鉄 金町線 (京成高砂 → 京成金町)"),
            Some(["京成高砂", "京成金町"])
        );
        assert_eq!(split_name("Wanaka Water Taxi (Mou Waho)"), None);
        assert_eq!(split_name("Calais --> Dover"), Some(["Calais", "Dover"]));
        assert_eq!(
            split_name("Ferry to Isle of Wight"),
            Some(["Ferry", "Isle of Wight"])
        );
        assert_eq!(split_name("Dover <-> Calais"), Some(["Dover", "Calais"]));
        assert_eq!(split_name("Blankenese-Cranz"), None);
        assert_eq!(split_name("Rostock <=> Gedser"), None, "not a separator");
        assert_eq!(split_name("Rostock - "), None, "an empty part");
        // The first separator of the list the name holds, wherever it stands.
        assert_eq!(split_name("A → B - C"), Some(["A → B", "C"]));
    }

    #[test]
    fn unlabel_rule() {
        let cases = [
            (
                "Flixbus 718: Toulon, Gare routière",
                "Toulon, Gare routière",
            ),
            ("Flixbus N831: Amsterdam Sloterdijk", "Amsterdam Sloterdijk"),
            ("G5624：香港西九龍", "香港西九龍"),
            ("TER C25 : Strasbourg", "Strasbourg"),
            ("Ruta Barra del Colorado Norte", "Barra del Colorado Norte"),
            ("Rute Speedboat Tarakan", "Tarakan"),
            ("Ligne Pointe-à-Pitre", "Pointe-à-Pitre"),
            ("Balsa Bertioga", "Bertioga"),
            ("Tuyến phà Chì", "Chì"),
            ("Ferry 612 Komiža (Vis)", "Komiža (Vis)"),
            ("F14 Autofähre Kleinzschachwitz", "Kleinzschachwitz"),
            ("F10 Fähre Heidenau", "Heidenau"),
            ("KA15 Tanjung Malim", "Tanjung Malim"),
            ("IC-01 Eupen", "Eupen"),
            ("HSR 108 Zuoying", "Zuoying"),
            ("KBS 975 Ulm", "Ulm"),
            ("G5810", ""),
            ("Ruta T-848", ""),
            ("Linie P1", ""),
            ("Route 3", ""),
            // Not labels: a route word alone, numbers that are part of a place, lowercase ordinals.
            ("Rota", "Rota"),
            ("Ferryland", "Ferryland"),
            ("1st Street", "1st Street"),
            ("'t Schouw", "'t Schouw"),
            ("25 de Mayo", "25 de Mayo"),
            ("Pier 11", "Pier 11"),
            ("Frankfurt (Main) Hbf", "Frankfurt (Main) Hbf"),
            ("Bender-3", "Bender-3"),
            ("Marconistraat/M4H", "Marconistraat/M4H"),
        ];
        for (name, want) in cases {
            assert_eq!(unlabel(name), want, "{name}");
        }
    }

    #[test]
    fn parts_rule() {
        assert_eq!(parts("Achele (Murnau)"), ["Achele", "Murnau"]);
        assert_eq!(
            parts("Addax/Sandfill/Maroko (Victoria Island)"),
            ["Addax", "Sandfill", "Maroko", "Victoria Island"]
        );
        assert_eq!(parts("Fribourg / Freiburg"), ["Fribourg", "Freiburg"]);
        assert_eq!(parts("Alderney (GBG)"), ["Alderney (GBG)"], "too short");
        assert_eq!(parts("Batayporã/MS"), ["Batayporã/MS"]);
        assert_eq!(parts("Hermannswerder/Fähre"), ["Hermannswerder"]);
        assert_eq!(parts("Brücke 3/4"), ["Brücke 3/4"]);
        assert_eq!(parts("Frankfurt (Main) Hbf"), ["Frankfurt (Main) Hbf"]);
        assert_eq!(
            parts("Landungsbrücken (Brücke 3/4)"),
            ["Landungsbrücken", "Brücke 3/4"]
        );
        assert_eq!(parts("Bondorf (bei Herrenberg)"), ["Bondorf"]);
        assert_eq!(parts("St Martin's (low tide)"), ["St Martin's"]);
        assert_eq!(parts("Wharf (Bhaucha Dhakka)"), ["Bhaucha Dhakka"]);
        assert_eq!(parts("(on request)"), ["(on request)"]);
        assert_eq!(parts("Komiža (Vis (Otok))"), ["Komiža", "Vis (Otok)"]);
    }

    #[test]
    fn end_names_rule() {
        let names = |name: &str, from: &str, to: &str| {
            let t = tags(name, from, to);
            end_names(&t).map(|n| n.map(str::to_string))
        };
        let some = |s: &str| Some(s.to_string());
        assert_eq!(names("X - Y", "A", "B"), [some("A"), some("B")]);
        assert_eq!(names("X - Y", "A", ""), [some("X"), some("Y")]);
        assert_eq!(
            names("Korpo rutt", "Verkan (Korpo)", ""),
            [some("Verkan (Korpo)"), None]
        );
        assert_eq!(names("", "", "B"), [None, some("B")]);
        assert_eq!(
            names("Thalwil - Thalwil", "", ""),
            [some("Thalwil"), some("Thalwil")]
        );
        assert_eq!(names("7:00 - 19:00", "", ""), [None, None]);
        assert_eq!(
            names("Ferry to Crow Island", "", ""),
            [None, some("Crow Island")]
        );
        assert_eq!(names("Boat to X", "Y", ""), [some("Y"), some("X")]);
        assert_eq!(names("Fähre", "", ""), [None, None]);
        assert_eq!(names("Bahnhof - Hafen", "", ""), [None, None]);
        assert_eq!(
            names("Flixbus 728: Genova => Milano", "", ""),
            [some("Genova"), some("Milano")]
        );
        assert_eq!(names("", "Linie P1", "Hamburg"), [None, some("Hamburg")]);
        assert_eq!(
            names("Dublin Bay Ferry (Dún Laoghaire - Howth)", "", ""),
            [some("Dún Laoghaire"), some("Howth")]
        );

        let langs = read_langs(
            [
                ("name", "Propriano - Marseille"),
                ("name:es", "Marsella - Propriano"),
                ("from:zh-Hant", "甲"),
                ("name:etymology:wikidata", "Q1"),
                ("name:left", "L"),
                ("alt_name", "Z"),
            ]
            .into_iter(),
        );
        assert_eq!(langs.keys().collect::<Vec<_>>(), ["", "es", "zh-Hant"]);
    }

    #[test]
    fn places_rule() {
        let p = |km: f64| (54.0 + km / 111.195, 11.0);
        let unit = |kind: Kind, id: i64, ends: [LatLon; 2], tags: &[(&str, &str)]| Unit {
            kind,
            osm_type: 'r',
            id,
            tags: read_langs(tags.iter().copied()),
            ends: Ok(ends),
        };
        let units = [
            // Two trains put "A" at 0 km and "B" at 50 km.
            unit(
                Kind::Train,
                1,
                [p(0.0), p(50.0)],
                &[("from", "A"), ("to", "B")],
            ),
            unit(
                Kind::Train,
                2,
                [p(0.5), p(50.0)],
                &[("from", "A"), ("to", "B")],
            ),
            // A ferry tagged the other way round: swapped. Its "es" names follow "A" across languages.
            unit(
                Kind::Ferry,
                3,
                [p(0.2), p(49.5)],
                &[("name", "B - A"), ("name:es", "Be - A")],
            ),
            // A ferry the votes cannot place keeps its order; so would a train.
            unit(
                Kind::Ferry,
                4,
                [p(100.0), p(120.0)],
                &[("from", "C"), ("to", "D")],
            ),
            unit(
                Kind::Train,
                5,
                [p(50.0), p(0.0)],
                &[("from", "A"), ("to", "B")],
            ),
        ];
        let (places, swapped) = places(&units);
        assert_eq!(swapped, [(2, ""), (2, "es")]);
        let got: Vec<(&str, &str, LatLon, String)> = (places.iter())
            .map(|q| (q.lang.as_str(), q.name.as_str(), q.at, q.ids.join(",")))
            .collect();
        let ids = |s: &str| s.to_string();
        assert_eq!(
            got,
            [
                ("", "A", p(0.0), ids("relation/1,relation/2,relation/3")),
                ("", "A", p(50.0), ids("relation/5")),
                ("", "B", p(0.0), ids("relation/5")),
                ("", "B", p(50.0), ids("relation/1,relation/2,relation/3")),
                ("", "C", p(100.0), ids("relation/4")),
                ("", "D", p(120.0), ids("relation/4")),
                ("es", "A", p(0.2), ids("relation/3")),
                ("es", "Be", p(49.5), ids("relation/3")),
            ]
        );
    }
}
