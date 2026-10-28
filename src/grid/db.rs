use std::collections::HashMap;

use swofty_formats::gvdb::Gvdb;
use swofty_formats::hash::fnv1a64;

/// The game database with its records indexed by key: the formats crate looks a
/// key up by walking every record, and a grid reads thousands of them.
pub struct Db {
    pub gvdb: Gvdb,
    index: HashMap<u64, usize>,
}

impl Db {
    pub fn new(gvdb: Gvdb) -> Db {
        let index = gvdb.records.iter().enumerate().map(|(i, r)| (r.key, i)).collect();
        Db { gvdb, index }
    }

    pub fn reindex(&mut self) {
        self.index = self
            .gvdb
            .records
            .iter()
            .enumerate()
            .map(|(i, r)| (r.key, i))
            .collect();
    }

    pub fn has_key(&self, key: u64) -> bool {
        self.index.contains_key(&key)
    }

    pub fn put_key(&mut self, key: u64, tag: u8, value: Vec<u8>) {
        match self.index.get(&key) {
            Some(&i) => {
                let r = &mut self.gvdb.records[i];
                r.tag = tag;
                r.value = value;
            }
            None => {
                self.index.insert(key, self.gvdb.records.len());
                self.gvdb
                    .records
                    .push(swofty_formats::gvdb::Record { key, tag, value });
            }
        }
    }

    pub fn text(&self, path: &str) -> Option<String> {
        let i = *self.index.get(&fnv1a64(path))?;
        Some(self.gvdb.records[i].text().into_owned())
    }

    pub fn field(&self, row: &str, name: &str) -> String {
        self.text(&format!("{row}.{name}")).unwrap_or_default()
    }

    pub fn int(&self, row: &str, name: &str) -> Option<i64> {
        self.field(row, name).trim().parse::<f64>().ok().map(|v| v as i64)
    }

    pub fn count(&self, container: &str, row: &str) -> usize {
        self.text(&format!("{container}._{row}_instancecount_"))
            .and_then(|t| t.trim().parse().ok())
            .unwrap_or(0)
    }

    /// The paths of a table's rows, up to its row count.
    pub fn rows(&self, container: &str, row: &str) -> Vec<String> {
        (0..self.count(container, row))
            .map(|i| format!("{container}.{row}_{i}"))
            .collect()
    }

    /// The row of a table with this label.
    pub fn find(&self, container: &str, row: &str, label: &str) -> Option<String> {
        self.rows(container, row)
            .into_iter()
            .find(|r| self.field(r, "_metadata_") == label)
    }
}
