use plane_projection::PlaneProjection;
use rayon::prelude::*;
use rustc_hash::FxHashMap;

use crate::coords::Bits;
use crate::pbf::Blobs;

/// (lat, lon) in degrees.
pub type LatLon = (f64, f64);

#[derive(Debug, Default, PartialEq)]
pub struct Quick<'a> {
    pub route: &'a str,
    pub terminal: bool,
    pub highway: bool,
}

pub fn quick<'a>(tags: impl Iterator<Item = (&'a str, &'a str)>) -> Quick<'a> {
    let mut q = Quick::default();
    for (k, v) in tags {
        match k {
            "route" => q.route = v,
            "amenity" => q.terminal = v == "ferry_terminal",
            "highway" => q.highway = true,
            _ => {}
        }
    }
    q
}

/// Highway pass: nodes of `ferry` that a non-ferry `highway=*` way (any value) holds, sorted, each once.
pub fn roads(blobs: &Blobs, ferry: &Bits) -> Vec<i64> {
    let per_block = blobs.par_map(blobs.ways_range(), |b| {
        let mut out = Vec::new();
        for g in b.groups() {
            for w in g.ways() {
                let q = quick(w.tags());
                if !q.highway || q.route == "ferry" {
                    continue;
                }
                out.extend(w.refs().filter(|&n| ferry.has(n)));
            }
        }
        out
    });
    let mut all: Vec<i64> = per_block.into_iter().flatten().collect();
    all.par_sort_unstable();
    all.dedup();
    all
}

/// An `amenity=ferry_terminal` node, way, or relation ring.
#[derive(Clone, Debug, Default)]
pub struct Terminal {
    /// Node ids, for a way or ring; empty for a node.
    pub refs: Vec<i64>,
    /// One point for a node.
    pub pts: Vec<LatLon>,
}

/// Even-odd point in ring, x = lon, y = lat.
pub fn point_in_ring(ring: &[LatLon], p: LatLon) -> bool {
    let mut inside = false;
    let Some(&last) = ring.last() else {
        return false;
    };
    let mut a = last;
    for &b in ring {
        if (a.0 > p.0) != (b.0 > p.0) && p.1 < a.1 + (p.0 - a.0) * (b.1 - a.1) / (b.0 - a.0) {
            inside = !inside;
        }
        a = b;
    }
    inside
}

/// Metres from `at` to a terminal: 0 inside a closed way, else to its nearest node or segment.
pub fn terminal_m(t: &Terminal, at: LatLon) -> f64 {
    let proj = PlaneProjection::new(at.0);
    let closed = t.pts.len() >= 4 && t.refs.first() == t.refs.last();
    if closed && point_in_ring(&t.pts, at) {
        return 0.0;
    }
    match t.pts.as_slice() {
        [] => f64::INFINITY,
        [p] => proj.distance(at, *p),
        pts => pts
            .windows(2)
            .map(|s| proj.distance_to_segment(at, (s[0], s[1])))
            .fold(f64::INFINITY, f64::min),
    }
}

/// Grid buckets over degrees, for "within m metres" candidates.
pub struct Grid {
    cell: f64,
    map: FxHashMap<(i32, i32), Vec<usize>>,
}

impl Grid {
    pub fn new(cell: f64) -> Self {
        Grid {
            cell,
            map: FxHashMap::default(),
        }
    }

    fn key(&self, at: LatLon) -> (i32, i32) {
        (
            (at.0 / self.cell).floor() as i32,
            (at.1 / self.cell).floor() as i32,
        )
    }

    pub fn insert(&mut self, at: LatLon, i: usize) {
        self.map.entry(self.key(at)).or_default().push(i);
    }

    /// Every cell of a box [min lat, min lon, max lat, max lon], up to `max` cells; false when over.
    pub fn insert_box(&mut self, b: [f64; 4], i: usize, max: usize) -> bool {
        let (lo, hi) = (self.key((b[0], b[1])), self.key((b[2], b[3])));
        if ((hi.0 - lo.0 + 1) as usize) * ((hi.1 - lo.1 + 1) as usize) > max {
            return false;
        }
        for y in lo.0..=hi.0 {
            for x in lo.1..=hi.1 {
                self.map.entry((y, x)).or_default().push(i);
            }
        }
        true
    }

    /// Items in the cells within `m` metres of `at`: a superset, unsorted, may repeat.
    pub fn near(&self, at: LatLon, m: f64) -> Vec<usize> {
        let dy = (m / (111_320.0 * self.cell)).ceil() as i32;
        let cos = at.0.to_radians().cos().max(0.01);
        let dx = (m / (111_320.0 * self.cell * cos)).ceil().min(1e4) as i32;
        let (y, x) = self.key(at);
        let mut out = Vec::new();
        for yy in y - dy..=y + dy {
            for xx in x - dx..=x + dx {
                if let Some(v) = self.map.get(&(yy, xx)) {
                    out.extend(v);
                }
            }
        }
        out
    }
}

/// Terminals with points, in a grid of 0.01° cells by their bbox.
pub struct Terminals {
    pub list: Vec<Terminal>,
    grid: Grid,
}

impl Terminals {
    pub fn new(list: Vec<Terminal>) -> Self {
        let mut grid = Grid::new(0.01);
        for (i, t) in list.iter().enumerate().filter(|(_, t)| !t.pts.is_empty()) {
            let b = t
                .pts
                .iter()
                .fold([f64::MAX, f64::MAX, f64::MIN, f64::MIN], |b, p| {
                    [b[0].min(p.0), b[1].min(p.1), b[2].max(p.0), b[3].max(p.1)]
                });
            // Over 400 cells (~20 km across): not a terminal to measure from.
            grid.insert_box(b, i, 400);
        }
        Terminals { list, grid }
    }

    /// The nearest terminal within `m` metres: index and metres.
    pub fn nearest(&self, at: LatLon, m: f64) -> Option<(usize, f64)> {
        let mut c = self.grid.near(at, m);
        c.sort_unstable();
        c.dedup();
        c.into_iter()
            .map(|i| (i, terminal_m(&self.list[i], at)))
            .filter(|&(_, d)| d <= m)
            .min_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)))
    }
}

/// Closed rings from a relation's ways (node id lists): ways joined at shared end nodes, reversed where needed; a closed
/// way is a ring by itself. Returns the rings (first == last, 4+ nodes) and the count of ways left in open chains.
pub fn assemble(ways: &[&[i64]]) -> (Vec<Vec<i64>>, usize) {
    let (mut rings, mut open_ways) = (Vec::new(), 0);
    let mut open: Vec<&[i64]> = Vec::new();
    for w in ways.iter().filter(|w| w.len() >= 2) {
        if w.first() == w.last() {
            if w.len() >= 4 {
                rings.push(w.to_vec());
            }
        } else {
            open.push(w);
        }
    }
    let mut ends: FxHashMap<i64, Vec<usize>> = FxHashMap::default();
    for (i, w) in open.iter().enumerate() {
        ends.entry(w[0]).or_default().push(i);
        ends.entry(*w.last().unwrap()).or_default().push(i);
    }
    let mut used = vec![false; open.len()];
    for start in 0..open.len() {
        if used[start] {
            continue;
        }
        used[start] = true;
        let mut chain = open[start].to_vec();
        let mut n = 1;
        loop {
            let end = *chain.last().unwrap();
            if end == chain[0] {
                if chain.len() >= 4 {
                    rings.push(chain);
                }
                break;
            }
            let next = ends
                .get(&end)
                .and_then(|v| v.iter().copied().find(|&j| !used[j]));
            let Some(j) = next else {
                open_ways += n;
                break;
            };
            used[j] = true;
            n += 1;
            let w = open[j];
            if w[0] == end {
                chain.extend_from_slice(&w[1..]);
            } else {
                chain.extend(w.iter().rev().skip(1));
            }
        }
    }
    (rings, open_ways)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rings() {
        // 1-2-3, 5-4-3 (reversed), 5-6-1: one ring; a closed way; 20-21 and 21-22-20 close; 30-31 stays open.
        let ways: [&[i64]; 6] = [
            &[1, 2, 3],
            &[5, 4, 3],
            &[10, 11, 12, 10],
            &[5, 6, 1],
            &[20, 21],
            &[21, 22, 20],
        ];
        let (rings, open) = assemble(&ways);
        assert_eq!(rings[0], [10, 11, 12, 10]);
        assert_eq!(rings[1], [1, 2, 3, 4, 5, 6, 1]);
        assert_eq!(rings[2], [20, 21, 22, 20]);
        assert_eq!(open, 0);
        assert_eq!(assemble(&[&[30, 31], &[32, 33, 34]]).1, 2);
    }

    #[test]
    fn terminal_distance() {
        let sq = Terminal {
            refs: vec![1, 2, 3, 4, 1],
            pts: vec![
                (0.0, 0.0),
                (0.0, 0.01),
                (0.01, 0.01),
                (0.01, 0.0),
                (0.0, 0.0),
            ],
        };
        assert_eq!(terminal_m(&sq, (0.005, 0.005)), 0.0, "inside");
        let d = terminal_m(&sq, (0.005, 0.0101));
        assert!((d - 11.1).abs() < 0.5, "{d}");
        let ts = Terminals::new(vec![sq]);
        assert!(ts.nearest((0.005, 0.0101), 50.0).is_some());
        assert!(ts.nearest((0.005, 0.0111), 50.0).is_none());
    }
}
