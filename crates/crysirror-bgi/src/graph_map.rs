//! Native tile-table policy; tile pixels and retained cache are engine jobs.
use super::*;
#[derive(Default)]
pub(super) struct NativeMap {
    grid: Option<[u32; 4]>,
    source: Option<([u32; 2], Vec<u16>)>,
    atlas: Option<(i32, u64, i32)>,
    viewport: Option<[u32; 4]>,
    current: Vec<u16>,
    desired: Vec<u16>,
}
impl BgiVm {
    pub(super) fn native_map_call(
        &mut self,
        host: &mut dyn HostServices,
        vm: &Machine,
        call: &NativeCall,
    ) -> Result<Option<i32>> {
        let a = &call.args;
        if call.id == 0x70 {
            let id = self.graph.allocate(0xa0000000, 8, Kind::Map);
            ensure!(id != 0, "BGI map pool exhausted");
            self.graph.maps.insert(id, Default::default());
            return Ok(Some(id));
        }
        let object = a[0];
        ensure!(
            self.graph.tree.kind(object) == Some(Kind::Map)
                && self.graph.maps.contains_key(&object),
            "BGI invalid map handle"
        );
        if call.id == 0x71 {
            if let Some(parent) = self.graph.tree.parent(object) {
                if let Some(members) = self.graph.members.get_mut(&parent) {
                    members.remove(&object);
                }
            }
            self.hide_graph_object(host, object)?;
            self.remove_sprite_composition(host, object)?;
            self.graph.images.remove(object);
            self.graph.maps.remove(&object);
            self.graph.crops.remove(&object);
            self.graph.objects.remove(&object);
            self.graph.fixed_vectors.remove(&object);
            self.graph.raster_offsets.remove(&object);
            self.graph.tree.remove(object);
            return Ok(None);
        }
        if call.id == 0x74 {
            self.graph.tree.set_enabled(object, a[1] != 0);
            self.graph.objects.entry(object).or_default().native.enabled = u32::from(a[1] != 0);
            return Ok(None);
        }
        if call.id == 0x76 {
            let [cols, rows, cw, ch] = [a[1] as u32, a[2] as u32, a[3] as u32, a[4] as u32];
            ensure!(
                cw.wrapping_sub(1) <= 255 && ch.wrapping_sub(1) <= 255,
                "BGI invalid map cell size"
            );
            ensure!(
                cols > 0 && rows > 0 && cols <= 1024 / cw && rows <= 768 / ch,
                "BGI invalid map viewport size"
            );
            let map = self.graph.maps.get_mut(&object).unwrap();
            map.grid = Some([cols, rows, cw, ch]);
            map.atlas = None;
            map.viewport = None;
            map.current = vec![u16::MAX; ((cols + 1) * (rows + 1)) as usize];
            map.desired = vec![0; map.current.len()];
            self.graph.images.remove(object);
            self.remove_sprite_composition(host, object)?;
            self.hide_graph_object(host, object)?;
            self.graph.crops.remove(&object);
            return Ok(None);
        }
        if call.id == 0x78 {
            let [width, height] = [a[1] as u32, a[2] as u32];
            let count = width
                .checked_mul(height)
                .filter(|n| *n > 0)
                .context("BGI map tile dimensions")?;
            let len = count
                .checked_mul(2)
                .filter(|n| *n <= 64 * 1024 * 1024)
                .context("BGI map tile table limit")?;
            let bytes = vm.memory(a[3] as u32, len as usize)?;
            let data = bytes
                .chunks_exact(2)
                .map(|b| u16::from_le_bytes(b.try_into().unwrap()))
                .collect();
            let map = self.graph.maps.get_mut(&object).unwrap();
            map.source = Some(([width, height], data));
            map.viewport = None;
            for (current, desired) in map.current.iter_mut().zip(&map.desired) {
                *current = !*desired;
            }
            self.graph.images.remove(object);
            self.remove_sprite_composition(host, object)?;
            self.hide_graph_object(host, object)?;
            return Ok(None);
        }
        if call.id == 0x75 {
            target_checks::priority(a[6])?;
            target_checks::effect_level(a[5])?;
            target_checks::effect_mode(a[4])?;
            target_checks::bitmap(a[3])?;
            let bitmap = self.bitmap(a[3])?.1.clone();
            let grid = self.graph.maps[&object]
                .grid
                .context("BGI uninitialized map")?;
            ensure!(
                bitmap.size[0] / grid[2] > 0 && bitmap.size[1] / grid[3] > 0,
                "BGI map atlas does not fit cells"
            );
            self.graph.maps.get_mut(&object).unwrap().atlas =
                Some((a[3], bitmap.generation, bitmap.format));
            self.graph.images.remove(object);
            self.graph.position(object, a[1], a[2]);
            let p = self.graph.objects.entry(object).or_default();
            p.format_resource = Some(a[3]);
            p.blend_mode = a[4];
            p.native.blend_mode = a[4];
            p.set_alpha_parameter(a[5]);
            p.native.priority = a[6] as u32;
            p.native.draw_enabled = 1;
            self.graph.sort(object);
            return self.rebuild_native_map(object).map(|_| None);
        }
        if call.id == 0x79 {
            let map = self.graph.maps.get_mut(&object).unwrap();
            let [cols, rows, cw, ch] = map.grid.context("BGI uninitialized map")?;
            let (size, data) = map.source.as_ref().context("BGI map tile data absent")?;
            let [sw, sh] = *size;
            let [sx, sy, ox, oy] = [a[1] as u32, a[2] as u32, a[3] as u32, a[4] as u32];
            ensure!(
                sx < sw && sy < sh && ox < cw && oy < ch,
                "BGI invalid map display coordinates"
            );
            if map.viewport.is_none_or(|v| v[0] != sx || v[1] != sy) {
                if a[5] == 0 {
                    map.desired.fill(u16::MAX);
                }
                for y in 0..=rows {
                    let mut yy = sy + y;
                    if yy >= sh {
                        if a[5] == 0 {
                            break;
                        }
                        yy %= sh;
                    }
                    for x in 0..=cols {
                        let mut xx = sx + x;
                        if xx >= sw {
                            if a[5] == 0 {
                                break;
                            }
                            xx %= sw;
                        }
                        map.desired[(y * (cols + 1) + x) as usize] = data[(yy * sw + xx) as usize];
                    }
                }
            }
            map.viewport = Some([sx, sy, ox, oy]);
        } else if call.id == 0x7a {
            let map = self.graph.maps.get_mut(&object).unwrap();
            ensure!(map.grid.is_some(), "BGI uninitialized map");
            let mut cursor = 0;
            for current in &mut map.current {
                if i32::from(*current) == a[1] {
                    *current = !map.desired.get(cursor).copied().unwrap_or(0);
                    cursor += 1;
                }
                cursor += 1;
            }
        }
        self.rebuild_native_map(object)?;
        Ok(None)
    }
    fn rebuild_native_map(&mut self, object: i32) -> Result<()> {
        let map = self.graph.maps.get_mut(&object).context("BGI map state")?;
        let Some([cols, rows, cw, ch]) = map.grid else {
            return Ok(());
        };
        let Some([_, _, ox, oy]) = map.viewport else {
            return Ok(());
        };
        let Some((atlas, generation, format)) = map.atlas else {
            return Ok(());
        };
        if self
            .bitmaps
            .get(&(atlas as u32))
            .is_none_or(|b| b.generation != generation)
        {
            self.graph
                .crops
                .insert(object, [ox, oy, cols * cw, rows * ch]);
            self.set_graph_plan(
                object,
                GraphPlan {
                    inputs: vec![GraphInput::Solid(
                        [(cols + 1) * cw, (rows + 1) * ch],
                        [0; 4],
                    )],
                    quad: None,
                    source_format: None,
                    warp: None,
                    compound: None,
                    projection: None,
                    operation: None,
                    format,
                    force_opaque: format == 1,
                    filters: Vec::new(),
                },
            );
            return Ok(());
        }
        self.graph.images.expired.remove(&object);
        let updates = map
            .current
            .iter()
            .zip(&map.desired)
            .enumerate()
            .filter_map(|(index, (old, new))| (old != new).then_some((index as u32, *new)))
            .collect::<Vec<_>>();
        self.graph
            .crops
            .insert(object, [ox, oy, cols * cw, rows * ch]);
        if updates.is_empty() && self.graph.images.painted.contains_key(&object) {
            return Ok(());
        }
        map.current.clone_from(&map.desired);
        let cache = if let Some(cache) = self.graph.images.painted.get(&object) {
            GraphInput::Frozen(cache.image.clone())
        } else {
            GraphInput::Solid([(cols + 1) * cw, (rows + 1) * ch], [0; 4])
        };
        let plan = GraphPlan {
            inputs: vec![cache, GraphInput::Bitmap(atlas)],
            quad: None,
            source_format: None,
            warp: None,
            compound: None,
            projection: None,
            operation: Some(Raster::Display {
                spec: DisplayRaster::TileCache {
                    columns: cols + 1,
                    rows: rows + 1,
                    cell: [cw, ch],
                    updates,
                },
            }),
            format,
            force_opaque: format == 1,
            filters: Vec::new(),
        };
        self.set_graph_plan(object, plan);
        Ok(())
    }
}
