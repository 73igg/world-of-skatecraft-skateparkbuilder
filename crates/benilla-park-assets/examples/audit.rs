//! Read-only local catalog audit; private output stays in the requested local state folder.
use benilla_park_assets::{Entry, Package};
use std::collections::BTreeMap;
use std::path::PathBuf;
fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let cache = PathBuf::from(args.next().expect("object cache folder"));
    #[derive(serde::Deserialize)]
    struct Catalog {
        entries: Vec<Entry>,
    }
    let catalog: Catalog =
        serde_json::from_slice(&std::fs::read(cache.join("skate-catalog.json"))?)?;
    for entry in &catalog.entries {
        let pack = Package::read(&cache.join(entry.package.as_ref().unwrap()), entry.id)?;
        assert!(
            !pack.collision.is_empty(),
            "{} must have usable collision",
            entry.name
        );
        assert!(
            (pack.bounds[0][1]).abs() < 0.00001,
            "{} must have a ground origin",
            entry.name
        );
    }
    println!("PASS {} native Skate object packages: identity, finite complete geometry, materials, collision and ground pivots",catalog.entries.len());
    if let Some(wow) = args.next() {
        let mut chain = benilla_formats::Chain::open(std::path::Path::new(&wow))?;
        let aliases: BTreeMap<String, Vec<String>> =
            serde_json::from_slice(&std::fs::read(cache.join("wow-names.json"))?)?;
        let entries = benilla_park_assets::index_wow(&mut chain, &aliases)?;
        std::fs::write(
            cache.join("wow-catalog-audit.json"),
            serde_json::to_vec(&entries)?,
        )?;
        println!(
            "PASS WoW index: {} unique objects with names, aliases and dimensions",
            entries.len()
        );
        for query in ["mailbox", "barrel", "bench", "stone", "stormwind"] {
            let hits = benilla_park_assets::search(
                &entries,
                &benilla_park_assets::Search {
                    text: query.into(),
                    ..Default::default()
                },
            );
            let first = hits.first().map(|&i| &entries[i]);
            println!(
                "SEARCH {query}: {} matches; first={:?}",
                hits.len(),
                first.map(|e| (&e.name, &e.path))
            );
            if let Some(entry) = first {
                let pack = benilla_park_assets::load_wow(&mut chain, entry)?;
                println!(
                    "PASS WoW {}: {} batches, {} collision faces",
                    entry.name,
                    pack.batches.len(),
                    pack.collision.len()
                );
            }
        }
    }
    Ok(())
}
