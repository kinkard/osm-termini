use std::sync::atomic::{AtomicU64, Ordering};

use crate::pbf::Blobs;

/// (lat, lon) in degrees.
pub type LatLon = (f64, f64);

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

    pub fn get(&self, id: i64) -> Option<(f64, f64)> {
        let i = self.ids.binary_search(&id).ok()?;
        let v = self.packed[i].load(Ordering::Relaxed);
        (v != MISSING).then(|| {
            let (lat, lon) = ((v >> 32) as u32 as i32, v as u32 as i32);
            (lat as f64 / 1e7, lon as f64 / 1e7)
        })
    }

    /// Stores the node if wanted; `k` is a cursor into `ids`, walked forward while ids ascend.
    pub fn put(&self, k: &mut Option<usize>, id: i64, lat: i32, lon: i32) {
        let mut i = match *k {
            Some(i) if i == 0 || self.ids[i - 1] < id => i,
            _ => self.ids.partition_point(|&x| x < id),
        };
        while i < self.ids.len() && self.ids[i] < id {
            i += 1;
        }
        if self.ids.get(i) == Some(&id) {
            self.packed[i].store(pack(lat, lon), Ordering::Relaxed);
        }
        *k = Some(i);
    }
}

/// Node pass: fills `coords`.
pub fn nodes(blobs: &Blobs, coords: &Coords) {
    blobs.par_map(blobs.nodes(), |b| {
        for g in b.groups() {
            let mut k = None;
            for n in g.dense_nodes() {
                coords.put(&mut k, n.id(), n.decimicro_lat(), n.decimicro_lon());
            }
            let mut k = None;
            for n in g.nodes() {
                coords.put(&mut k, n.id(), n.decimicro_lat(), n.decimicro_lon());
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coords_fill_wanted_ids_only() {
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
