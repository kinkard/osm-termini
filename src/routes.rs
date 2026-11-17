use std::collections::BTreeMap;

use osmpbf::RelMemberType;
use plane_projection::PlaneProjection;
use rustc_hash::{FxHashMap, FxHashSet};

use crate::coords::Bits;
use crate::pbf::Blobs;
use crate::terminals::{Grid, LatLon, Terminal, Terminals, assemble, quick};

/// Metres within which ends are one landing, a terminal keeps a shared node an end, and a stop route's ends make a loop.
pub const LANDING_M: f64 = 50.0;
/// Metres within which each end of a unit meets a different end of a route.
pub const MERGE_M: f64 = 500.0;

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

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Tags {
    pub name: String,
    pub from: String,
    pub to: String,
}

impl Tags {
    pub fn read<'a>(tags: impl Iterator<Item = (&'a str, &'a str)>) -> Self {
        let mut t = Tags::default();
        for (k, v) in tags {
            match k {
                "name" => t.name = v.trim().to_string(),
                "from" => t.from = v.trim().to_string(),
                "to" => t.to = v.trim().to_string(),
                _ => {}
            }
        }
        t
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RelKind {
    /// `type=route` + `route=ferry`.
    Ferry,
    /// `type=route` + `route=train` + `public_transport:version=2`.
    Train,
    /// Another `type=route` + `route=train` relation.
    OtherTrain,
    /// `type=route` + `route=bus` + "flix" in `operator` or `network` + `public_transport:version=2`.
    Coach,
    /// Another `type=route` + `route=bus` relation with "flix" in `operator` or `network`.
    OtherCoach,
    /// `amenity=ferry_terminal`: its member ways are a terminal.
    Terminal,
}

#[derive(Debug, Default)]
struct RelTags<'a> {
    ty: &'a str,
    route: &'a str,
    v2: bool,
    terminal: bool,
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
            "amenity" => t.terminal = v == "ferry_terminal",
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
        "train" if route && t.v2 => Some(RelKind::Train),
        "train" if route => Some(RelKind::OtherTrain),
        "bus" if route && t.flix && t.v2 => Some(RelKind::Coach),
        "bus" if route && t.flix => Some(RelKind::OtherCoach),
        _ if t.terminal => Some(RelKind::Terminal),
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

/// A ferry relation member on the route's path: not a stop or platform.
pub fn path_role(role: &str) -> bool {
    !role.starts_with("stop") && !role.starts_with("platform")
}

pub struct Rel {
    pub id: i64,
    pub kind: RelKind,
    pub tags: Tags,
    /// Ferry and terminal relations: member ways, any role; trains and coaches: stop and platform members; others: none.
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

/// Pass 1: ferry, train, flix bus and ferry terminal relations, in id order; the bus tag counts.
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
                        RelKind::Ferry | RelKind::Terminal => ty == 'w',
                        RelKind::Train | RelKind::Coach => STOPS.contains(&role),
                        RelKind::OtherTrain | RelKind::OtherCoach => false,
                    };
                    keep.then(|| Member::new(ty, m.member_id, role))
                });
                out.push(Rel {
                    id: r.id(),
                    kind,
                    tags: Tags::read(r.tags()),
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

pub struct Way {
    pub id: i64,
    pub tags: Tags,
    pub refs: Vec<i64>,
    /// `route=ferry` on the way itself.
    pub tagged: bool,
}

#[derive(Default)]
pub struct Ways {
    /// `route=ferry` ways and ferry relations' path member ways, in id order.
    pub ferry: Vec<Way>,
    /// `amenity=ferry_terminal` ways and terminal relations' member ways: id, refs, the way's own tag; in id order.
    pub terminal: Vec<(i64, Vec<i64>, bool)>,
    /// Train and coach stop and platform member ways: id, first node; in id order.
    pub first: Vec<(i64, i64)>,
}

/// Pass 2; `ferry`, `terminal`, `stops`: the relation member ways wanted.
pub fn ways(blobs: &Blobs, ferry: &Bits, terminal: &Bits, stops: &Bits) -> Ways {
    let per_block = blobs.par_map(blobs.ways_range(), |b| {
        let mut out = Ways::default();
        for g in b.groups() {
            for w in g.ways() {
                let id = w.id();
                let q = quick(w.tags());
                let tagged = q.route == "ferry";
                if tagged || ferry.has(id) {
                    out.ferry.push(Way {
                        id,
                        tags: Tags::read(w.tags()),
                        refs: w.refs().collect(),
                        tagged,
                    });
                }
                if q.terminal || terminal.has(id) {
                    out.terminal.push((id, w.refs().collect(), q.terminal));
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
        out.ferry.extend(w.ferry);
        out.terminal.extend(w.terminal);
        out.first.extend(w.first);
    }
    out.ferry.dedup_by_key(|w| w.id);
    out.terminal.dedup_by_key(|t| t.0);
    out.first.dedup_by_key(|f| f.0);
    out
}

/// Terminal relations' rings, closed, as terminal ways (points unset); and the member ways left open.
pub fn terminal_rings(rels: &[Rel], ways: &Ways) -> (Vec<Terminal>, usize) {
    let refs = |id: i64| {
        let i = ways.terminal.binary_search_by_key(&id, |t| t.0).ok()?;
        Some(ways.terminal[i].1.as_slice())
    };
    let (mut out, mut open) = (Vec::new(), 0);
    for r in rels.iter().filter(|r| r.kind == RelKind::Terminal) {
        let members: Vec<&[i64]> = r.members.iter().filter_map(|m| refs(m.id)).collect();
        let (rings, o) = assemble(&members);
        open += o;
        out.extend(rings.into_iter().map(|refs| Terminal {
            refs,
            pts: Vec::new(),
        }));
    }
    (out, open)
}

/// Distinct ferry ways holding each of `nodes`.
pub fn held(ways: &[Way], nodes: impl Iterator<Item = i64>) -> FxHashMap<i64, u32> {
    let mut held: FxHashMap<i64, (u32, usize)> = nodes.map(|n| (n, (0, usize::MAX))).collect();
    for (i, w) in ways.iter().enumerate() {
        for n in &w.refs {
            if let Some(h) = held.get_mut(n)
                && h.1 != i
            {
                *h = (h.0 + 1, i);
            }
        }
    }
    held.into_iter().map(|(n, (c, _))| (n, c)).collect()
}

/// End nodes of `ways` in order, a way's first node first, each node once.
pub fn end_nodes<'a>(ways: impl Iterator<Item = &'a Way>) -> Vec<i64> {
    let mut out: Vec<i64> = Vec::new();
    for w in ways {
        for n in [w.refs.first(), w.refs.last()].into_iter().flatten() {
            if !out.contains(n) {
                out.push(*n);
            }
        }
    }
    out
}

/// Single-linkage groups of points within `m` metres, by first member; each group at its first point.
pub fn landings(pts: &[LatLon], m: f64) -> Vec<LatLon> {
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
    let mut seen = FxHashSet::default();
    (0..pts.len())
        .filter(|&i| seen.insert(group[i]))
        .map(|i| pts[i])
        .collect()
}

/// Why an OSM unit is not kept.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Drop {
    /// One landing, or a stop route's ends within `LANDING_M`.
    Loop,
    /// A ferry with other than two landings, or a stop route with fewer than two stop or platform members.
    Ends,
    /// A stop route's first or last stop without coordinates.
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

/// A ferry unit's ends: two landings, else why not.
pub fn ferry_ends(pts: &[LatLon]) -> Result<[LatLon; 2], Drop> {
    match landings(pts, LANDING_M).as_slice() {
        &[a, b] => Ok([a, b]),
        [_] => Err(Drop::Loop),
        _ => Err(Drop::Ends),
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
    match PlaneProjection::new(a.0).distance(a, b) <= LANDING_M {
        true => Err(Drop::Loop),
        false => Ok([a, b]),
    }
}

/// Units to routes in order: a unit joins the first earlier route whose two ends each lie within `m` of a different
/// end of its own, else starts one with its ends. Returns each unit's route and whether it joined reversed (its first
/// end at the route's second), and each route's ends.
pub fn merge(units: &[[LatLon; 2]], m: f64) -> (Vec<(usize, bool)>, Vec<[LatLon; 2]>) {
    let near = |a: LatLon, b: LatLon| PlaneProjection::new(a.0).distance(a, b) <= m;
    let mut grid = Grid::new(0.01);
    let (mut of, mut routes): (Vec<(usize, bool)>, Vec<[LatLon; 2]>) = (Vec::new(), Vec::new());
    for &[a, b] in units {
        let mut cands = grid.near(a, m);
        cands.extend(grid.near(b, m));
        let forward = |r: &[LatLon; 2]| near(r[0], a) && near(r[1], b);
        let reversed = |r: &[LatLon; 2]| near(r[0], b) && near(r[1], a);
        let joins = cands
            .into_iter()
            .filter(|&r| forward(&routes[r]) || reversed(&routes[r]))
            .min();
        match joins {
            Some(r) => of.push((r, !forward(&routes[r]))),
            None => {
                grid.insert(a, routes.len());
                grid.insert(b, routes.len());
                of.push((routes.len(), false));
                routes.push([a, b]);
            }
        }
    }
    (of, routes)
}

/// The first and last part of a name split at the first of `SEPS` it holds, trimmed; `None` when one is empty.
pub fn split_name(name: &str) -> Option<[&str; 2]> {
    let sep = SEPS.iter().find(|s| name.contains(*s))?;
    let parts: Vec<&str> = name.split(sep).map(str::trim).collect();
    let (a, b) = (parts[0], parts[parts.len() - 1]);
    (!a.is_empty() && !b.is_empty()).then_some([a, b])
}

/// Byte offset of the `(` that opens a parenthesised group ending `s`.
fn trailing_group(s: &str) -> Option<usize> {
    if !s.ends_with(')') {
        return None;
    }
    let mut depth = 0;
    for (i, c) in s.char_indices().rev() {
        match c {
            ')' => depth += 1,
            '(' if depth == 1 => return Some(i),
            '(' => depth -= 1,
            _ => {}
        }
    }
    None
}

/// A name without its trailing parenthesised tags, "Gedser (S)" → "Gedser"; the name itself when nothing would remain.
pub fn strip_tags(name: &str) -> &str {
    let mut s = name.trim();
    while let Some(open) = trailing_group(s) {
        s = s[..open].trim_end();
    }
    if s.is_empty() { name.trim() } else { s }
}

/// Where a route's names came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Named {
    FromTo,
    Name,
}

/// A route's names from its units (tags, joined reversed): `from` and `to` of the first with both set, else the parts
/// of the first name that splits; swapped for a reversed unit, then `strip_tags`.
pub fn names<'a>(
    units: impl Iterator<Item = (&'a Tags, bool)> + Clone,
) -> Option<([String; 2], Named)> {
    let mut it = units.clone();
    let from_to = (it.find(|(t, _)| !t.from.is_empty() && !t.to.is_empty()))
        .map(|(t, rev)| ([t.from.as_str(), t.to.as_str()], rev, Named::FromTo));
    let split = || {
        let mut it = units.clone();
        it.find_map(|(t, rev)| split_name(&t.name).map(|n| (n, rev, Named::Name)))
    };
    let (mut n, rev, by) = from_to.or_else(split)?;
    if rev {
        n.swap(0, 1);
    }
    Some((n.map(|s| strip_tags(s).to_string()), by))
}

/// One OSM unit: a ferry relation or way, a train or coach relation, and its fate.
pub struct Unit {
    pub kind: Kind,
    /// `r` relation, `w` way.
    pub osm_type: char,
    pub id: i64,
    pub tags: Tags,
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

/// Every OSM unit, relations first, each kind in id order: ferry relations, `route=ferry` ways no ferry relation
/// holds, train relations, coach relations.
pub struct Extract {
    pub units: Vec<Unit>,
    /// Ferry way end nodes without coordinates, and ends dropped as shared nodes.
    pub missing: usize,
    pub joins: usize,
}

/// Ends and drops of every unit from the passes' output; `road` sorted, `at` a node's (lat, lon).
pub fn extract(
    rels: &[Rel],
    ways: &Ways,
    road: &[i64],
    terminals: &Terminals,
    at: impl Fn(i64) -> Option<LatLon>,
) -> Extract {
    let way_of: FxHashMap<i64, usize> = (ways.ferry.iter().enumerate())
        .map(|(i, w)| (w.id, i))
        .collect();
    let ferry_rels: Vec<&Rel> = rels.iter().filter(|r| r.kind == RelKind::Ferry).collect();
    let paths: Vec<Vec<&Way>> = (ferry_rels.iter())
        .map(|r| {
            (r.members.iter())
                .filter(|m| path_role(&m.role))
                .filter_map(|m| way_of.get(&m.id).map(|&w| &ways.ferry[w]))
                .collect()
        })
        .collect();
    let members: FxHashSet<i64> = (ferry_rels.iter())
        .flat_map(|r| r.members.iter().map(|m| m.id))
        .collect();
    // `ways.ferry`, tagged ways and path members, are the ferry ways: a node 2+ of them hold needs a highway or terminal.
    let counts = held(&ways.ferry, end_nodes(ways.ferry.iter()).into_iter());
    let on_road = |n: i64| road.binary_search(&n).is_ok();
    let (mut missing, mut joins) = (0, 0);
    let mut ends_of = |ws: &[&Way]| -> Vec<LatLon> {
        let mut pts = Vec::new();
        for n in end_nodes(ws.iter().copied()) {
            let Some(p) = at(n) else {
                missing += 1;
                continue;
            };
            let shared = counts.get(&n).copied().unwrap_or(0) >= 2;
            if shared && !on_road(n) && terminals.nearest(p, LANDING_M).is_none() {
                joins += 1;
                continue;
            }
            pts.push(p);
        }
        pts
    };
    let mut units = Vec::new();
    for (r, path) in ferry_rels.iter().zip(&paths) {
        units.push(Unit {
            kind: Kind::Ferry,
            osm_type: 'r',
            id: r.id,
            tags: r.tags.clone(),
            ends: ferry_ends(&ends_of(path)),
        });
    }
    for w in (ways.ferry.iter()).filter(|w| w.tagged && !members.contains(&w.id)) {
        units.push(Unit {
            kind: Kind::Ferry,
            osm_type: 'w',
            id: w.id,
            tags: w.tags.clone(),
            ends: ferry_ends(&ends_of(&[w])),
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
    Extract {
        units,
        missing,
        joins,
    }
}

/// A route: its units (indices into `Extract::units`), the first's ends, and its names, oriented to those ends.
pub struct Route {
    pub kind: Kind,
    pub units: Vec<usize>,
    pub ends: [LatLon; 2],
    pub names: Option<([String; 2], Named)>,
}

/// The kept units of each kind merged into routes (`merge`), named (`names`).
pub fn routes(units: &[Unit]) -> Vec<Route> {
    let mut out = Vec::new();
    for kind in Kind::ALL {
        let kept: Vec<usize> = (0..units.len())
            .filter(|&u| units[u].kind == kind && units[u].ends.is_ok())
            .collect();
        let ends: Vec<[LatLon; 2]> = kept.iter().map(|&u| units[u].ends.unwrap()).collect();
        let (of, rs) = merge(&ends, MERGE_M);
        let mut members: Vec<Vec<(usize, bool)>> = vec![Vec::new(); rs.len()];
        for (i, (r, rev)) in of.into_iter().enumerate() {
            members[r].push((kept[i], rev));
        }
        for (ends, us) in rs.into_iter().zip(members) {
            let names = names(us.iter().map(|&(u, rev)| (&units[u].tags, rev)));
            out.push(Route {
                kind,
                units: us.into_iter().map(|(u, _)| u).collect(),
                ends,
                names,
            });
        }
    }
    out
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

    fn way(id: i64, refs: &[i64], tagged: bool) -> Way {
        Way {
            id,
            tags: Tags::default(),
            refs: refs.to_vec(),
            tagged,
        }
    }

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
    fn ferry_end_rule() {
        // Single linkage: 0–40 m and 40–80 m are one landing though 0–80 m is over 50 m; 200 m is another.
        let pts = [north(0.0), north(200.0), north(40.0), north(80.0)];
        assert_eq!(landings(&pts, 50.0), [north(0.0), north(200.0)]);
        assert_eq!(ferry_ends(&pts), Ok([north(0.0), north(200.0)]));
        assert_eq!(ferry_ends(&pts[2..]), Err(Drop::Loop));
        assert_eq!(
            ferry_ends(&[north(0.0), north(100.0), north(200.0)]),
            Err(Drop::Ends)
        );
        assert_eq!(ferry_ends(&[]), Err(Drop::Ends));

        // Way 1 (10 → 20) and way 2 (10 → 30, parallel) share node 10; way 3 (30 → 40) splits the route at 30.
        let ws = [
            way(1, &[10, 11, 20], true),
            way(2, &[10, 12, 30], true),
            way(3, &[30, 40], true),
        ];
        assert_eq!(end_nodes(ws.iter()), [10, 20, 30, 40]);
        let h = held(&ws, [10, 20, 30, 40, 11].into_iter());
        assert_eq!((h[&10], h[&20], h[&30], h[&40], h[&11]), (2, 1, 2, 1, 1));
        // Node 10 is an end by its highway, 30 by a terminal within 50 m; with neither, both are joins.
        let at: FxHashMap<i64, LatLon> = [
            (10, north(0.0)),
            (20, north(5000.0)),
            (30, north(9000.0)),
            (40, north(20000.0)),
        ]
        .into_iter()
        .collect();
        let rels = vec![Rel {
            id: 7,
            kind: RelKind::Ferry,
            tags: Tags::default(),
            members: vec![
                Member::new('w', 2, ""),
                Member::new('w', 3, "forward"),
                Member::new('w', 1, "platform"),
            ],
        }];
        let wset = Ways {
            ferry: ws.into_iter().collect(),
            ..Ways::default()
        };
        let term = |m: f64| Terminal {
            refs: Vec::new(),
            pts: vec![north(9000.0 + m)],
        };
        let run = |road: &[i64], t: Vec<Terminal>| {
            let x = extract(&rels, &wset, road, &Terminals::new(t), |n| {
                at.get(&n).copied()
            });
            let got: Vec<(String, Result<[LatLon; 2], Drop>)> =
                x.units.iter().map(|u| (u.osm_id(), u.ends)).collect();
            (got, x.joins)
        };
        // The relation's path is ways 2 and 3 (way 1 is a platform member); way 1 is a relation member, not a unit.
        let (got, joins) = run(&[10], vec![term(30.0)]);
        assert_eq!(
            got,
            [("relation/7".to_string(), Err(Drop::Ends))],
            "landings 10, 30, 40: a stop at 30"
        );
        assert_eq!(joins, 0);
        let (got, joins) = run(&[10], vec![term(60.0)]);
        assert_eq!(
            got,
            [("relation/7".to_string(), Ok([north(0.0), north(20000.0)]))],
            "the terminal 60 m away keeps 30 a join"
        );
        assert_eq!(joins, 1);
        let (got, joins) = run(&[], vec![]);
        assert_eq!((got[0].1, joins), (Err(Drop::Loop), 2), "only 40 is left");
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
    fn merge_rule() {
        let p = |km_n: f64, km_e: f64| (54.0 + km_n / 111.195, 11.0 + km_e / 65.4);
        let units = [
            [p(0.0, 0.0), p(10.0, 0.0)],
            // Reversed and 300 m off at each end: joins route 0, reversed.
            [p(10.3, 0.0), p(0.0, 0.3)],
            // One end 700 m off: a new route.
            [p(0.0, 0.0), p(10.7, 0.0)],
            // Both ends near route 0's first end only: a new route.
            [p(0.0, 0.1), p(0.1, 0.0)],
            // Within 500 m of routes 0 and 2: the first.
            [p(0.0, 0.0), p(10.4, 0.0)],
        ];
        let (of, routes) = merge(&units, 500.0);
        assert_eq!(
            of,
            [(0, false), (0, true), (1, false), (2, false), (0, false)]
        );
        assert_eq!(
            routes,
            [units[0], units[2], units[3]],
            "a route keeps its first unit's ends"
        );
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
            Some(["RE 1: Hamburg", "Lübeck"])
        );
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
    fn tag_strip_rule() {
        assert_eq!(strip_tags("Gedser (S)"), "Gedser");
        assert_eq!(strip_tags("Trelleborg (SE)"), "Trelleborg");
        assert_eq!(strip_tags("Rostock (Überseehafen)"), "Rostock", "any tag");
        assert_eq!(
            strip_tags("Puttgarden (D) (Fehmarn)"),
            "Puttgarden",
            "every trailing tag"
        );
        assert_eq!(strip_tags("Lindau (Bodensee (Insel))"), "Lindau", "nested");
        assert_eq!(strip_tags("Frankfurt(Main)"), "Frankfurt");
        assert_eq!(
            strip_tags("Frankfurt (Main) Hbf"),
            "Frankfurt (Main) Hbf",
            "not trailing"
        );
        assert_eq!(strip_tags("(S)"), "(S)", "nothing would remain");
        assert_eq!(strip_tags("(A) (B)"), "(A) (B)");
        assert_eq!(strip_tags("Foo)"), "Foo)", "no opening");
        assert_eq!(strip_tags("Bar (x"), "Bar (x");
    }

    #[test]
    fn names_rule() {
        let two = |a: &str, b: &str| [a.to_string(), b.to_string()];
        let units = [
            tags("X - Y", "Alpha", ""),
            tags("P → Q", "", ""),
            tags("", "Beta", "Gamma (DK)"),
        ];
        let fwd = |us: &[Tags]| names(us.iter().map(|t| (t, false)));
        assert_eq!(fwd(&units), Some((two("Beta", "Gamma"), Named::FromTo)));
        assert_eq!(fwd(&units[..2]), Some((two("X", "Y"), Named::Name)));
        assert_eq!(fwd(&units[1..2]), Some((two("P", "Q"), Named::Name)));
        assert_eq!(fwd(&[tags("Fähre", "", "")]), None);
        assert_eq!(
            fwd(&[tags("Gedser (S) - Rostock", "", "")]),
            Some((two("Gedser", "Rostock"), Named::Name))
        );
        // Names from a unit that joined reversed are swapped onto the route's ends.
        let rev = [(&units[1], false), (&units[2], true)];
        assert_eq!(
            names(rev.iter().copied()),
            Some((two("Gamma", "Beta"), Named::FromTo))
        );
        let rev = [(&units[1], true), (&units[0], false)];
        assert_eq!(
            names(rev.iter().copied()),
            Some((two("Q", "P"), Named::Name))
        );
    }

    #[test]
    fn routes_rule() {
        let p = |km: f64| (54.0 + km / 111.195, 11.0);
        let unit = |id: i64, ends: Result<[LatLon; 2], Drop>, t: Tags| Unit {
            kind: Kind::Train,
            osm_type: 'r',
            id,
            tags: t,
            ends,
        };
        let units = [
            unit(1, Ok([p(0.0), p(50.0)]), tags("Train 1", "", "")),
            unit(
                2,
                Ok([p(50.1), p(0.2)]),
                tags("", "Lübeck Hbf", "Kiel (SH)"),
            ),
            unit(3, Err(Drop::Loop), tags("", "A", "B")),
            unit(4, Ok([p(0.0), p(90.0)]), tags("Train 4", "", "")),
        ];
        let rs = routes(&units);
        assert_eq!(rs.len(), 2);
        assert_eq!(
            (rs[0].units.as_slice(), rs[0].ends),
            ([0, 1].as_slice(), [p(0.0), p(50.0)])
        );
        let n = rs[0].names.as_ref().unwrap();
        assert_eq!(
            (n.0[0].as_str(), n.0[1].as_str()),
            ("Kiel", "Lübeck Hbf"),
            "unit 2 runs 50 → 0 km"
        );
        assert_eq!(rs[1].names, None);
    }
}
