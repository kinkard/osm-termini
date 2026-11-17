use std::fs::File;
use std::io::BufReader;
use std::ops::Range;
use std::path::{Path, PathBuf};

use osmpbf::{BlobReader, BlobType, ByteOffset, Element, PrimitiveBlock};
use rayon::prelude::*;

/// Element type of a block's first element.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    Node,
    Way,
    Relation,
}

pub struct Blobs {
    path: PathBuf,
    offsets: Vec<ByteOffset>,
    /// Index of the first blob whose first element is a way, and a relation.
    pub ways: usize,
    pub relations: usize,
}

fn reader(path: &Path) -> BlobReader<BufReader<File>> {
    BlobReader::seekable_from_path(path).expect("open pbf")
}

impl Blobs {
    pub fn open(path: &Path) -> osmpbf::Result<Self> {
        let mut r = BlobReader::seekable_from_path(path)?;
        let mut offsets = Vec::new();
        while let Some(h) = r.next_header_skip_blob() {
            let (header, offset) = h?;
            if matches!(header.blob_type(), BlobType::OsmData) {
                offsets.push(offset.expect("seekable reader"));
            }
        }
        let mut b = Blobs {
            path: path.to_path_buf(),
            offsets,
            ways: 0,
            relations: 0,
        };
        b.ways = b.first(Kind::Way, &mut r);
        b.relations = b.first(Kind::Relation, &mut r);
        Ok(b)
    }

    pub fn len(&self) -> usize {
        self.offsets.len()
    }

    fn block(&self, r: &mut BlobReader<BufReader<File>>, i: usize) -> PrimitiveBlock {
        let blob = r.blob_from_offset(self.offsets[i]).expect("read blob");
        blob.to_primitiveblock().expect("decode blob")
    }

    fn kind(&self, r: &mut BlobReader<BufReader<File>>, i: usize) -> Kind {
        match self.block(r, i).elements().next() {
            Some(Element::Way(_)) => Kind::Way,
            Some(Element::Relation(_)) => Kind::Relation,
            _ => Kind::Node,
        }
    }

    /// First blob of `kind` or later, by binary search over the sorted file.
    fn first(&self, kind: Kind, r: &mut BlobReader<BufReader<File>>) -> usize {
        let (mut lo, mut hi) = (0, self.offsets.len());
        while lo < hi {
            let mid = (lo + hi) / 2;
            if self.kind(r, mid) < kind {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        lo
    }

    /// Blobs of nodes, ways, relations; each range starts one blob early, for a block of mixed types.
    pub fn nodes(&self) -> Range<usize> {
        0..self.ways
    }

    pub fn ways_range(&self) -> Range<usize> {
        self.ways.saturating_sub(1)..self.relations
    }

    pub fn relations_range(&self) -> Range<usize> {
        self.relations.saturating_sub(1)..self.len()
    }

    /// `f` over the decoded blocks of `range`, in parallel; results in blob order.
    pub fn par_map<T, F>(&self, range: Range<usize>, f: F) -> Vec<T>
    where
        T: Send,
        F: Fn(&PrimitiveBlock) -> T + Sync + Send,
    {
        range
            .into_par_iter()
            .with_min_len(16)
            .map_init(|| reader(&self.path), |r, i| f(&self.block(r, i)))
            .collect()
    }
}
