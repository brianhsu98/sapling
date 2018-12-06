// Copyright Facebook, Inc. 2018

use std::ops::Range;
use std::path::Path;

use indexedlog::log::{IndexDef, IndexOutput, Log};
use types::errors::{KeyError, Result};
use types::node::Node;

#[derive(Debug, Fail)]
#[fail(display = "Node Map Error: {:?}", _0)]
struct NodeMapError(String);

impl From<NodeMapError> for KeyError {
    fn from(err: NodeMapError) -> Self {
        KeyError::new(err.into())
    }
}

/// A persistent bidirectional mapping between two Nodes
///
/// [NodeMap] is implemented on top of [indexedlog::log::Log] to store a mapping between two kinds
/// of nodes.
pub struct NodeMap {
    log: Log,
}

impl NodeMap {
    pub fn open(dir: impl AsRef<Path>) -> Result<Self> {
        // Update the index every 100KB, i.e. every 256 entries
        let lag = 100 * 1024;
        let first_index = |_data: &[u8]| vec![IndexOutput::Reference(0..20)];
        let second_index = |_data: &[u8]| vec![IndexOutput::Reference(20..40)];
        Ok(NodeMap {
            log: Log::open(
                dir,
                vec![
                    IndexDef {
                        func: Box::new(first_index),
                        name: "first",
                        lag_threshold: lag,
                    },
                    IndexDef {
                        func: Box::new(second_index),
                        name: "second",
                        lag_threshold: lag,
                    },
                ],
            )?,
        })
    }

    pub fn flush(&mut self) -> Result<()> {
        Ok(self.log.flush()?)
    }

    pub fn add(&mut self, first: &Node, second: &Node) -> Result<()> {
        let mut buf = Vec::with_capacity(40);
        buf.extend_from_slice(first.as_ref());
        buf.extend_from_slice(second.as_ref());
        self.log.append(buf).map_err(|e| e.into())
    }

    pub fn lookup_by_first(&self, first: &Node) -> Result<Option<Node>> {
        self.lookup(first, 0, 20..40)
    }

    pub fn lookup_by_second(&self, second: &Node) -> Result<Option<Node>> {
        self.lookup(second, 1, 0..20)
    }

    fn lookup(&self, key: &Node, index_id: usize, range: Range<usize>) -> Result<Option<Node>> {
        let mut lookup_iter = self.log.lookup(index_id, key)?;
        Ok(match lookup_iter.next() {
            Some(result) => Some(Node::from_slice(&result?[range])?),
            None => None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    quickcheck! {
        fn test_roundtrip(pairs: Vec<(Node, Node)>) -> bool {
            let mut pairs = pairs;
            if pairs.len() == 0 {
                return true;
            }

            let dir = TempDir::new().unwrap();
            let mut map = NodeMap::open(dir).unwrap();
            let last = pairs.pop().unwrap();
            for (first, second) in pairs.iter() {
                map.add(&first, &second).unwrap();
            }

            for (first, second) in pairs.iter() {
                if first != &map.lookup_by_second(second).unwrap().unwrap() {
                    return false;
                }
                if second != &map.lookup_by_first(first).unwrap().unwrap() {
                    return false;
                }
            }

            for value in vec![last.0, last.1].iter() {
                if !map.lookup_by_first(value).unwrap().is_none() {
                    return false;
                }
                if !map.lookup_by_second(value).unwrap().is_none() {
                    return false;
                }

            }
            true
        }
    }
}
