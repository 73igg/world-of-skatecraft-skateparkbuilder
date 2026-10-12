//! Builder extension: immutable identities refer to models in each player's own game files.
mod search;
mod wow;
use anyhow::{bail, Context, Result};
pub use search::{search, Search, Sort};
use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};
use std::io::{Read, Write};
use std::path::Path;
pub use wow::{index_wow, load_wow};

pub const MAX_FACES: usize = 8_000_000;
pub const MAX_PACKAGE_BYTES: u64 = 256 * 1024 * 1024;
pub fn asset_id(game: u8, path: &str) -> u64 {
    let normalized = path.replace('\\', "/").to_lowercase();
    let hash = Sha1::digest(normalized.as_bytes());
    (u64::from(game) << 62) | (u64::from_le_bytes(hash[..8].try_into().unwrap()) & ((1 << 62) - 1))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn asset_names_are_normalized_and_games_have_distinct_identities() {
        assert_eq!(
            asset_id(1, "World\\Props\\Bench.m2"),
            asset_id(1, "world/props/bench.m2")
        );
        assert_ne!(asset_id(1, "park/bench"), asset_id(2, "park/bench"));
    }
    #[test]
    fn material_batching_preserves_faces_uvs_colors_and_blend_order() {
        let batch = Batch {
            positions: vec![[0., 0., 0.], [1., 0., 0.], [0., 0., 1.]],
            normals: vec![[0., 1., 0.]; 3],
            uvs: vec![[0., 0.], [1., 0.], [0., 1.]],
            colors: vec![],
            indices: vec![0, 2, 1],
            texture: Some("test.png".into()),
            alpha: 0,
            cutoff: 0.5,
            two_sided: false,
            repeat_u: true,
            repeat_v: true,
        };
        let mut colored = batch.clone();
        colored.colors = vec![[0.5, 0.25, 1., 1.]; 3];
        let mut blend = batch.clone();
        blend.alpha = 2;
        let mut pack = Package {
            version: 3,
            id: asset_id(1, "test"),
            batches: vec![batch, colored, blend.clone(), blend],
            collision: vec![[[0., 0., 0.], [0., 0., 1.], [1., 0., 0.]]],
            rails: vec![],
            bounds: [[0., 0., 0.], [1., 0., 1.]],
        };
        pack.batch_static_materials();
        pack.validate(pack.id).unwrap();
        assert_eq!(pack.batches.len(), 3);
        assert_eq!(pack.batches[0].indices, vec![0, 2, 1, 3, 5, 4]);
        assert_eq!(&pack.batches[0].uvs[3..], &[[0., 0.], [1., 0.], [0., 1.]]);
        assert_eq!(&pack.batches[0].colors[..3], &[[1.; 4]; 3]);
        assert_eq!(&pack.batches[0].colors[3..], &[[0.5, 0.25, 1., 1.]; 3]);
        assert!(pack.batches[1..]
            .iter()
            .all(|b| b.alpha == 2 && b.indices == [0, 2, 1]));
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Entry {
    pub id: u64,
    pub game: String,
    pub name: String,
    pub category: String,
    pub path: String,
    #[serde(default)]
    pub aliases: Vec<String>,
    #[serde(default)]
    pub package: Option<String>,
    #[serde(default)]
    pub bounds: Option<[[f32; 3]; 2]>,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Batch {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    #[serde(default)]
    pub colors: Vec<[f32; 4]>,
    pub indices: Vec<u32>,
    pub texture: Option<String>,
    #[serde(default)]
    pub alpha: u8,
    #[serde(default = "cutoff")]
    pub cutoff: f32,
    #[serde(default)]
    pub two_sided: bool,
    #[serde(default = "yes")]
    pub repeat_u: bool,
    #[serde(default = "yes")]
    pub repeat_v: bool,
}
fn cutoff() -> f32 {
    0.5
}
fn yes() -> bool {
    true
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Package {
    pub version: u32,
    pub id: u64,
    pub batches: Vec<Batch>,
    pub collision: Vec<[[f32; 3]; 3]>,
    #[serde(default)]
    pub rails: Vec<Vec<[f32; 3]>>,
    pub bounds: [[f32; 3]; 2],
}
impl Package {
    /// Merge compatible opaque batches while retaining every vertex, tint, UV and face.
    pub fn batch_static_materials(&mut self) {
        let mut groups = std::collections::HashMap::new();
        let mut combined = Vec::<Batch>::new();
        for mut batch in std::mem::take(&mut self.batches) {
            if batch.alpha >= 2 {
                combined.push(batch);
                continue;
            }
            let key = (
                batch.texture.clone(),
                batch.alpha,
                batch.cutoff.to_bits(),
                batch.two_sided,
                batch.repeat_u,
                batch.repeat_v,
            );
            if let Some(&index) = groups.get(&key) {
                let target: &mut Batch = &mut combined[index];
                let offset = target.positions.len() as u32;
                if target.colors.is_empty() {
                    target.colors.resize(target.positions.len(), [1.; 4]);
                }
                if batch.colors.is_empty() {
                    batch.colors.resize(batch.positions.len(), [1.; 4]);
                }
                target.positions.extend(batch.positions);
                target.normals.extend(batch.normals);
                target.uvs.extend(batch.uvs);
                target.colors.extend(batch.colors);
                target
                    .indices
                    .extend(batch.indices.into_iter().map(|i| i + offset));
            } else {
                groups.insert(key, combined.len());
                combined.push(batch);
            }
        }
        self.batches = combined;
    }
    pub fn validate(&self, expected: u64) -> Result<()> {
        if self.version != 3 || self.id != expected || !(1..=2).contains(&(self.id >> 62)) {
            bail!("Object package identity or version does not match its catalog entry");
        }
        let mut faces = 0;
        for batch in &self.batches {
            faces += batch.indices.len() / 3;
            let n = batch.positions.len();
            if n == 0
                || !batch.indices.len().is_multiple_of(3)
                || batch.indices.iter().any(|&i| i as usize >= n)
                || batch
                    .positions
                    .iter()
                    .flatten()
                    .any(|v| !v.is_finite() || v.abs() > 100_000.)
                || batch.normals.len() != n
                || batch.uvs.len() != n
                || !batch.colors.is_empty() && batch.colors.len() != n
                || batch
                    .normals
                    .iter()
                    .flatten()
                    .chain(batch.uvs.iter().flatten())
                    .chain(batch.colors.iter().flatten())
                    .any(|v| !v.is_finite())
                || !batch.cutoff.is_finite()
                || !(0.0..=1.0).contains(&batch.cutoff)
                || batch.alpha > 3
            {
                bail!("Object has invalid render geometry or materials");
            }
        }
        if faces == 0 || faces > MAX_FACES || self.collision.len() > MAX_FACES {
            bail!("Object geometry exceeds the supported range: {faces} render faces, {} collision faces",self.collision.len());
        }
        if self
            .collision
            .iter()
            .flatten()
            .flatten()
            .chain(self.bounds.iter().flatten())
            .any(|v| !v.is_finite() || v.abs() > 100_000.)
            || self
                .rails
                .iter()
                .any(|r| r.len() < 2 || r.iter().flatten().any(|v| !v.is_finite()))
        {
            bail!("Object geometry exceeds the supported range");
        }
        for i in 0..3 {
            if self.bounds[0][i] > self.bounds[1][i] {
                bail!("Object bounds are inverted");
            }
        }
        Ok(())
    }
    pub fn ground_origin(&mut self) -> Result<()> {
        let mut min = glam::Vec3::splat(f32::INFINITY);
        let mut max = glam::Vec3::splat(f32::NEG_INFINITY);
        for p in self.batches.iter().flat_map(|b| &b.positions) {
            let v = glam::Vec3::from_array(*p);
            min = min.min(v);
            max = max.max(v);
        }
        if !min.is_finite() || !max.is_finite() {
            bail!("Object has no visible geometry");
        }
        let offset = glam::Vec3::new((min.x + max.x) * 0.5, min.y, (min.z + max.z) * 0.5);
        for p in self
            .batches
            .iter_mut()
            .flat_map(|b| &mut b.positions)
            .chain(self.collision.iter_mut().flatten())
            .chain(self.rails.iter_mut().flatten())
        {
            *p = (glam::Vec3::from_array(*p) - offset).to_array();
        }
        self.bounds = [(min - offset).to_array(), (max - offset).to_array()];
        self.validate(self.id)
    }
    pub fn read(path: &Path, expected: u64) -> Result<Self> {
        let mut file =
            std::fs::File::open(path).with_context(|| format!("Opening {}", path.display()))?;
        let mut magic = [0; 4];
        file.read_exact(&mut magic)?;
        if &magic != b"SPK3" {
            bail!("Object cache has an unsupported format");
        }
        let data = flate2::read::ZlibDecoder::new(file).take(MAX_PACKAGE_BYTES + 1);
        let value: Self = serde_json::from_reader(data).context("Reading object cache")?;
        value.validate(expected)?;
        Ok(value)
    }
    pub fn write(&self, path: &Path) -> Result<()> {
        self.validate(self.id)?;
        let temporary = path.with_extension("spk.new");
        let mut file = std::fs::File::create(&temporary)?;
        file.write_all(b"SPK3")?;
        let mut compressed = flate2::write::ZlibEncoder::new(file, flate2::Compression::fast());
        serde_json::to_writer(&mut compressed, self)?;
        compressed.finish()?.sync_all()?;
        std::fs::rename(temporary, path)?;
        Ok(())
    }
}
