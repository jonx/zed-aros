//! CPU sprite atlas: etagere shelf packing over `Vec<u8>` textures.
//!
//! Ported from `gpui_wgpu::wgpu_atlas` but backed by plain CPU byte buffers
//! (Monochrome = 1 byte/px alpha coverage, Polychrome/Subpixel = 4 byte/px
//! RGBA) so the tiny-skia renderer can sample tiles directly.

use anyhow::{Context as _, Result};
use collections::FxHashMap;
use etagere::{BucketedAtlasAllocator, size2};
use gpui::{
    AtlasKey, AtlasTextureId, AtlasTextureKind, AtlasTextureList, AtlasTile, Bounds, DevicePixels,
    PlatformAtlas, Point, Size,
};
use parking_lot::Mutex;
use std::borrow::Cow;

const DEFAULT_ATLAS_SIZE: i32 = 1024;
const MAX_ATLAS_SIZE: i32 = 4096;

fn device_size_to_etagere(size: Size<DevicePixels>) -> etagere::Size {
    size2(size.width.0, size.height.0)
}

fn etagere_point_to_device(point: etagere::Point) -> Point<DevicePixels> {
    Point {
        x: DevicePixels(point.x),
        y: DevicePixels(point.y),
    }
}

fn bytes_per_pixel(kind: AtlasTextureKind) -> usize {
    match kind {
        AtlasTextureKind::Monochrome => 1,
        AtlasTextureKind::Polychrome | AtlasTextureKind::Subpixel => 4,
    }
}

/// A tile's pixels copied out for the renderer to composite.
pub(crate) struct TileBytes {
    pub width: usize,
    pub height: usize,
    pub bytes_per_pixel: usize,
    pub data: Vec<u8>,
}

pub(crate) struct CpuAtlas(Mutex<CpuAtlasState>);

struct CpuAtlasState {
    monochrome: AtlasTextureList<CpuAtlasTexture>,
    polychrome: AtlasTextureList<CpuAtlasTexture>,
    subpixel: AtlasTextureList<CpuAtlasTexture>,
    tiles_by_key: FxHashMap<AtlasKey, AtlasTile>,
}

struct CpuAtlasTexture {
    id: AtlasTextureId,
    allocator: BucketedAtlasAllocator,
    bytes: Vec<u8>,
    width: usize,
    bytes_per_pixel: usize,
    live_atlas_keys: u32,
}

impl CpuAtlas {
    pub(crate) fn new() -> Self {
        CpuAtlas(Mutex::new(CpuAtlasState {
            monochrome: AtlasTextureList::default(),
            polychrome: AtlasTextureList::default(),
            subpixel: AtlasTextureList::default(),
            tiles_by_key: FxHashMap::default(),
        }))
    }

    /// Copy a tile's pixels out for compositing. `None` if the texture is gone.
    pub(crate) fn read_tile(&self, tile: &AtlasTile) -> Option<TileBytes> {
        let state = self.0.lock();
        let texture = state.texture(tile.texture_id)?;
        let bpp = texture.bytes_per_pixel;
        let stride = texture.width * bpp;
        let x = tile.bounds.origin.x.0 as usize;
        let y = tile.bounds.origin.y.0 as usize;
        let w = tile.bounds.size.width.0 as usize;
        let h = tile.bounds.size.height.0 as usize;

        let mut data = vec![0u8; w * h * bpp];
        for row in 0..h {
            let src = (y + row) * stride + x * bpp;
            let dst = row * w * bpp;
            data[dst..dst + w * bpp].copy_from_slice(&texture.bytes[src..src + w * bpp]);
        }
        Some(TileBytes {
            width: w,
            height: h,
            bytes_per_pixel: bpp,
            data,
        })
    }
}

impl PlatformAtlas for CpuAtlas {
    fn get_or_insert_with<'a>(
        &self,
        key: &AtlasKey,
        build: &mut dyn FnMut() -> Result<Option<(Size<DevicePixels>, Cow<'a, [u8]>)>>,
    ) -> Result<Option<AtlasTile>> {
        let mut state = self.0.lock();
        if let Some(tile) = state.tiles_by_key.get(key) {
            return Ok(Some(*tile));
        }
        let Some((size, bytes)) = build()? else {
            return Ok(None);
        };
        let tile = state
            .allocate(size, key.texture_kind())
            .context("failed to allocate atlas tile")?;
        state.upload(tile.texture_id, tile.bounds, &bytes);
        state.tiles_by_key.insert(key.clone(), tile);
        Ok(Some(tile))
    }

    fn remove(&self, key: &AtlasKey) {
        let mut state = self.0.lock();
        let Some(tile) = state.tiles_by_key.remove(key) else {
            return;
        };
        let id = tile.texture_id;
        let list = state.list_mut(id.kind);
        let Some(slot) = list.textures.get_mut(id.index as usize) else {
            return;
        };
        if let Some(mut texture) = slot.take() {
            texture.allocator.deallocate(tile.tile_id.into());
            texture.live_atlas_keys = texture.live_atlas_keys.saturating_sub(1);
            if texture.live_atlas_keys == 0 {
                list.free_list.push(id.index as usize);
            } else {
                *slot = Some(texture);
            }
        }
    }
}

impl CpuAtlasState {
    fn list(&self, kind: AtlasTextureKind) -> &AtlasTextureList<CpuAtlasTexture> {
        match kind {
            AtlasTextureKind::Monochrome => &self.monochrome,
            AtlasTextureKind::Polychrome => &self.polychrome,
            AtlasTextureKind::Subpixel => &self.subpixel,
        }
    }

    fn list_mut(&mut self, kind: AtlasTextureKind) -> &mut AtlasTextureList<CpuAtlasTexture> {
        match kind {
            AtlasTextureKind::Monochrome => &mut self.monochrome,
            AtlasTextureKind::Polychrome => &mut self.polychrome,
            AtlasTextureKind::Subpixel => &mut self.subpixel,
        }
    }

    fn texture(&self, id: AtlasTextureId) -> Option<&CpuAtlasTexture> {
        self.list(id.kind)
            .textures
            .get(id.index as usize)
            .and_then(|t| t.as_ref())
    }

    fn allocate(
        &mut self,
        size: Size<DevicePixels>,
        kind: AtlasTextureKind,
    ) -> Option<AtlasTile> {
        if let Some(tile) = self
            .list_mut(kind)
            .iter_mut()
            .rev()
            .find_map(|texture| texture.allocate(size))
        {
            return Some(tile);
        }
        let index = self.push_texture(size, kind);
        self.list_mut(kind).textures[index]
            .as_mut()
            .and_then(|texture| texture.allocate(size))
    }

    fn push_texture(&mut self, min_size: Size<DevicePixels>, kind: AtlasTextureKind) -> usize {
        let width = min_size.width.0.clamp(DEFAULT_ATLAS_SIZE, MAX_ATLAS_SIZE);
        let height = min_size.height.0.clamp(DEFAULT_ATLAS_SIZE, MAX_ATLAS_SIZE);
        let size = Size {
            width: DevicePixels(width),
            height: DevicePixels(height),
        };
        let bpp = bytes_per_pixel(kind);

        let list = self.list_mut(kind);
        let index = list.free_list.pop().unwrap_or(list.textures.len());
        let texture = CpuAtlasTexture {
            id: AtlasTextureId {
                index: index as u32,
                kind,
            },
            allocator: BucketedAtlasAllocator::new(device_size_to_etagere(size)),
            bytes: vec![0u8; (width as usize) * (height as usize) * bpp],
            width: width as usize,
            bytes_per_pixel: bpp,
            live_atlas_keys: 0,
        };
        if index == list.textures.len() {
            list.textures.push(Some(texture));
        } else {
            list.textures[index] = Some(texture);
        }
        index
    }

    fn upload(&mut self, id: AtlasTextureId, bounds: Bounds<DevicePixels>, bytes: &[u8]) {
        let Some(texture) = self
            .list_mut(id.kind)
            .textures
            .get_mut(id.index as usize)
            .and_then(|t| t.as_mut())
        else {
            return;
        };
        let bpp = texture.bytes_per_pixel;
        let stride = texture.width * bpp;
        let x = bounds.origin.x.0 as usize;
        let y = bounds.origin.y.0 as usize;
        let w = bounds.size.width.0 as usize;
        let h = bounds.size.height.0 as usize;
        let src_stride = w * bpp;
        for row in 0..h {
            let src = row * src_stride;
            let dst = (y + row) * stride + x * bpp;
            if src + src_stride <= bytes.len() && dst + src_stride <= texture.bytes.len() {
                texture.bytes[dst..dst + src_stride].copy_from_slice(&bytes[src..src + src_stride]);
            }
        }
    }
}

impl CpuAtlasTexture {
    fn allocate(&mut self, size: Size<DevicePixels>) -> Option<AtlasTile> {
        let allocation = self.allocator.allocate(device_size_to_etagere(size))?;
        let tile = AtlasTile {
            texture_id: self.id,
            tile_id: allocation.id.into(),
            padding: 0,
            bounds: Bounds {
                origin: etagere_point_to_device(allocation.rectangle.min),
                size,
            },
        };
        self.live_atlas_keys += 1;
        Some(tile)
    }
}
