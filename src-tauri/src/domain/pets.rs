// Pet manifest parsing. Mirrors ClaudePet/src/shared/pets.js.
//
// Each `pet/<id>/pet.json` is normalized into a [PetManifest] with seven
// animations (idle/thinking/tool/waiting/success/error/run). The spritesheet
// size is read straight from the WebP header bytes — no image crate needed.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use super::paths::pet_dir;

pub const DEFAULT_FRAME_WIDTH: u32 = 192;
pub const DEFAULT_FRAME_HEIGHT: u32 = 208;

#[derive(Debug, Clone, Serialize)]
pub struct AnimationDef {
    pub frames: Vec<u32>,
    pub fps: u32,
    pub looped: bool,
}

/// The seven animations, with default frame ranges mirroring pets.js.
/// `frames` start indices follow the sprite sheet's row layout.
pub fn default_animations() -> Vec<(&'static str, AnimationDef)> {
    vec![
        ("idle", AnimationDef { frames: (0..6).collect(), fps: 6, looped: true }),
        ("thinking", AnimationDef { frames: (8..16).collect(), fps: 8, looped: true }),
        ("tool", AnimationDef { frames: (16..24).collect(), fps: 10, looped: true }),
        ("waiting", AnimationDef { frames: (24..28).collect(), fps: 5, looped: true }),
        ("success", AnimationDef { frames: (32..37).collect(), fps: 10, looped: false }),
        ("error", AnimationDef { frames: (40..48).collect(), fps: 6, looped: true }),
        ("run", AnimationDef { frames: (8..16).collect(), fps: 12, looped: true }),
    ]
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Anchor {
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawAnimation {
    #[serde(default)]
    pub frames: Vec<u32>,
    #[serde(default)]
    pub fps: u32,
    #[serde(default, rename = "loop")]
    pub r#loop: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawManifest {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub spritesheet_path: String,
    #[serde(default)]
    pub frame_width: Option<u32>,
    #[serde(default)]
    pub frame_height: Option<u32>,
    #[serde(default)]
    pub columns: Option<u32>,
    #[serde(default)]
    pub rows: Option<u32>,
    #[serde(default)]
    pub default_scale: Option<f64>,
    #[serde(default)]
    pub anchor: Option<Anchor>,
    #[serde(default)]
    pub animations: std::collections::HashMap<String, RawAnimation>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PetManifest {
    pub id: String,
    pub display_name: String,
    pub description: String,
    pub kind: String,
    pub spritesheet_path: String,
    pub spritesheet_file: PathBuf,
    pub image_width: u32,
    pub image_height: u32,
    pub frame_width: u32,
    pub frame_height: u32,
    pub columns: u32,
    pub rows: u32,
    pub default_scale: f64,
    pub anchor: Anchor,
    pub animations: std::collections::HashMap<String, AnimationDef>,
}

/// Read WebP width/height from the file header bytes (VP8X / VP8L / VP8 ).
/// Straight port of pets.js#readWebpSize.
pub fn read_webp_size(path: &Path) -> Option<(u32, u32)> {
    let data = std::fs::read(path).ok()?;
    if data.len() < 30 || &data[0..4] != b"RIFF" || &data[8..12] != b"WEBP" {
        return None;
    }
    let chunk = &data[12..16];
    match chunk {
        b"VP8X" => {
            // VP8X: 10 bytes header after "VP8X", then 3 bytes width, 3 bytes height (1-based).
            if data.len() < 30 {
                return None;
            }
            let w = u32::from(data[24]) | (u32::from(data[25]) << 8) | (u32::from(data[26]) << 16);
            let h = u32::from(data[27]) | (u32::from(data[28]) << 8) | (u32::from(data[29]) << 16);
            Some((w + 1, h + 1))
        }
        b"VP8L" => {
            // VP8L lossless: signature byte at 20, then 14 bits width, 14 bits height.
            if data.len() < 25 || data[20] != 0x2f {
                return None;
            }
            let bits = u32::from(data[21]) | (u32::from(data[22]) << 8) | (u32::from(data[23]) << 16) | (u32::from(data[24]) << 24);
            let w = (bits & 0x3fff) + 1;
            let h = ((bits >> 14) & 0x3fff) + 1;
            Some((w, h))
        }
        b"VP8 " => {
            // VP8 (lossy): width/height are 16-bit LE at offset 26/28.
            if data.len() < 30 {
                return None;
            }
            let w = u32::from(data[26]) | (u32::from(data[27]) << 8);
            let h = u32::from(data[28]) | (u32::from(data[29]) << 8);
            Some((w & 0x3fff, h & 0x3fff))
        }
        _ => None,
    }
}

/// Scan `pet/` for `<id>/pet.json` and normalize each into a [PetManifest].
pub fn list_pets() -> Vec<PetManifest> {
    let dir = pet_dir();
    let mut pets = Vec::new();
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return pets;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let manifest_path = path.join("pet.json");
        let Ok(text) = std::fs::read_to_string(&manifest_path) else {
            continue;
        };
        let Ok(raw) = serde_json::from_str::<RawManifest>(&text) else {
            continue;
        };
        if let Some(pet) = normalize_manifest(&path, raw) {
            pets.push(pet);
        }
    }
    pets.sort_by(|a, b| a.display_name.cmp(&b.display_name));
    pets
}

fn normalize_manifest(dir: &Path, raw: RawManifest) -> Option<PetManifest> {
    let id = if raw.id.is_empty() {
        dir.file_name()?.to_string_lossy().to_string()
    } else {
        raw.id
    };
    let display_name = if raw.display_name.is_empty() {
        id.clone()
    } else {
        raw.display_name
    };

    let spritesheet_rel = if raw.spritesheet_path.is_empty() {
        "spritesheet.webp".to_string()
    } else {
        raw.spritesheet_path
    };
    let spritesheet_file = dir.join(&spritesheet_rel);

    let frame_width = raw.frame_width.unwrap_or(DEFAULT_FRAME_WIDTH);
    let frame_height = raw.frame_height.unwrap_or(DEFAULT_FRAME_HEIGHT);

    let (image_width, image_height) = read_webp_size(&spritesheet_file).unwrap_or((0, 0));

    let columns = raw.columns.unwrap_or_else(|| {
        if frame_width > 0 {
            (image_width / frame_width).max(1)
        } else {
            1
        }
    });
    let rows = raw.rows.unwrap_or_else(|| {
        if frame_height > 0 {
            (image_height / frame_height).max(1)
        } else {
            1
        }
    });

    let default_scale = raw.default_scale.unwrap_or(3.0);
    let anchor = raw.anchor.unwrap_or(Anchor { x: 0.5, y: 1.0 });

    // Merge per-manifest animations over the defaults.
    let mut animations = std::collections::HashMap::new();
    for (name, def) in default_animations() {
        let merged = if let Some(r) = raw.animations.get(name) {
            AnimationDef {
                frames: if r.frames.is_empty() { def.frames.clone() } else { r.frames.clone() },
                fps: if r.fps == 0 { def.fps } else { r.fps },
                looped: r.r#loop,
            }
        } else {
            def
        };
        animations.insert(name.to_string(), merged);
    }

    Some(PetManifest {
        id,
        display_name,
        description: raw.description,
        kind: "character".to_string(),
        spritesheet_path: spritesheet_rel,
        spritesheet_file,
        image_width,
        image_height,
        frame_width,
        frame_height,
        columns,
        rows,
        default_scale,
        anchor,
        animations,
    })
}
