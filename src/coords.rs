use std::sync::atomic::{AtomicU64, Ordering};

use rustc_hash::FxHashMap;

use crate::pbf::Blobs;
use crate::routes::{Langs, read_langs};

/// (lat, lon) in degrees.
pub type LatLon = (f64, f64);

pub struct Bits(Vec<u64>);

impl Bits {
    pub fn new(ids: impl Iterator<Item = i64> + Clone) -> Self {
        let max = ids.clone().max().unwrap_or(0).max(0) as usize;
        let mut v = vec![0u64; max / 64 + 1];
        for id in ids.filter(|&i| i >= 0) {
            v[id as usize / 64] |= 1 << (id as usize % 64);
        }
        Bits(v)
    }

    pub fn has(&self, id: i64) -> bool {
        id >= 0
            && self
                .0
                .get(id as usize / 64)
                .is_some_and(|w| w & (1 << (id as usize % 64)) != 0)
    }
}

/// Node coordinates by id: lat, lon in 1e-7°, packed; `MISSING` until the node pass fills it.
pub struct Coords {
    ids: Vec<i64>,
    packed: Vec<AtomicU64>,
}

const MISSING: u64 = u64::MAX;

fn pack(lat: i32, lon: i32) -> u64 {
    ((lat as u32 as u64) << 32) | lon as u32 as u64
}

impl Coords {
    /// `ids` sorted and deduplicated.
    pub fn new(ids: Vec<i64>) -> Self {
        let packed = ids.iter().map(|_| AtomicU64::new(MISSING)).collect();
        Coords { ids, packed }
    }

    pub fn len(&self) -> usize {
        self.ids.len()
    }

    pub fn found(&self) -> usize {
        self.packed
            .iter()
            .filter(|a| a.load(Ordering::Relaxed) != MISSING)
            .count()
    }

    /// (lat, lon) in degrees.
    pub fn get(&self, id: i64) -> Option<(f64, f64)> {
        let i = self.ids.binary_search(&id).ok()?;
        let v = self.packed[i].load(Ordering::Relaxed);
        (v != MISSING).then(|| {
            let (lat, lon) = ((v >> 32) as u32 as i32, v as u32 as i32);
            (lat as f64 / 1e7, lon as f64 / 1e7)
        })
    }

    /// Stores the node if wanted, and tells whether it was; `k` is a cursor into `ids`, walked forward while ids ascend.
    pub fn put(&self, k: &mut Option<usize>, id: i64, lat: i32, lon: i32) -> bool {
        let mut i = match *k {
            Some(i) if i == 0 || self.ids[i - 1] < id => i,
            _ => self.ids.partition_point(|&x| x < id),
        };
        while i < self.ids.len() && self.ids[i] < id {
            i += 1;
        }
        *k = Some(i);
        let wanted = self.ids.get(i) == Some(&id);
        if wanted {
            self.packed[i].store(pack(lat, lon), Ordering::Relaxed);
        }
        wanted
    }
}

/// Node pass: fills `coords`; the names of the `named` nodes, sorted ids among `coords`'.
pub fn nodes(blobs: &Blobs, coords: &Coords, named: &[i64]) -> FxHashMap<i64, Langs> {
    let per_block = blobs.par_map(blobs.nodes(), |b| {
        let mut out = Vec::new();
        for g in b.groups() {
            let mut k = None;
            for n in g.dense_nodes() {
                if coords.put(&mut k, n.id(), n.decimicro_lat(), n.decimicro_lon())
                    && named.binary_search(&n.id()).is_ok()
                {
                    out.push((n.id(), read_langs(n.tags())));
                }
            }
            let mut k = None;
            for n in g.nodes() {
                if coords.put(&mut k, n.id(), n.decimicro_lat(), n.decimicro_lon())
                    && named.binary_search(&n.id()).is_ok()
                {
                    out.push((n.id(), read_langs(n.tags())));
                }
            }
        }
        out
    });
    per_block.into_iter().flatten().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bits_and_coords() {
        let b = Bits::new([3, 64, 1000].into_iter());
        assert!(b.has(3) && b.has(64) && b.has(1000));
        assert!(!b.has(4) && !b.has(-1) && !b.has(5000));

        let c = Coords::new(vec![2, 5, 9]);
        let mut k = None;
        for (id, lat) in [(1, 10), (2, 20), (5, 50), (7, 70), (9, 90)] {
            c.put(&mut k, id, lat, -lat);
        }
        // Out of order: the cursor restarts.
        c.put(&mut k, 2, 21, -21);
        assert_eq!(c.get(2), Some((21e-7, -21e-7)));
        assert_eq!(c.get(5), Some((50e-7, -50e-7)));
        assert_eq!(c.get(1), None);
        assert_eq!(c.found(), 3);
    }
}
